use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::{Mutex, PoisonError};

use anyhow::Result;

use crate::client::game::chat::ClientChat;
use crate::client::game::debug_menu::DebugMenu;
use crate::client::game::entities::ClientEntities;
use crate::client::game::floating_text::FloatingTextManager;
use crate::client::game::framerate_measurer::FramerateMeasurer;
use crate::client::game::gas_overlay::GasOverlayProvider;
use crate::client::game::gases::ClientGases;
use crate::client::game::overlay::Overlay;
use crate::client::game::substance::SubstanceRenderer;
use crate::client::game::zombies::ClientZombies;
use crate::client::game::health::ClientHealth;
use crate::client::game::inventory::ClientInventory;
use crate::client::game::items::ClientItems;
use crate::client::game::lights::ClientLights;
use crate::client::game::pause_menu::PauseMenu;
use crate::client::game::players::ClientPlayers;
use crate::client::game::respawn_screen::RespawnScreen;
use crate::client::global_settings::GlobalSettings;
use crate::client::settings::Settings;
use crate::libraries::events;
use crate::libraries::events::EventManager;
use crate::libraries::graphics as gfx;
use crate::shared::entities::PositionComponent;

use super::background::Background;
use super::block_selector::BlockSelector;
use super::blocks::ClientBlocks;
use super::camera::Camera;
use super::capture::FrameCapture;
use super::mod_manager::ClientModManager;
use super::networking::ClientNetworking;
use super::script::ScriptDriver;
use super::walls::ClientWalls;

/// Optional harness configuration threaded through `run_game` by PrivateWorld.
/// Empty (default) in normal play; with capture/script content set the loop
/// runs scripted input + frame capture and ticks deterministically.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Default, Clone)]
pub struct RunGameExtras {
    pub script_path: Option<String>,
    pub capture_dir: Option<String>,
    /// capture every Nth rendered frame; 0 disables cadence captures
    pub capture_interval: u32,
    pub capture_max_frames: Option<u32>,
    pub capture_timeout_ms: Option<u64>,
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
pub fn run_game(
    graphics: &mut gfx::GraphicsContext,
    server_port: u16,
    server_address: String,
    player_name: &str,
    settings: &Rc<RefCell<Settings>>,
    global_settings: &Rc<RefCell<GlobalSettings>>,
    debug: bool,
    extras: RunGameExtras,
) -> Result<()> {
    let deterministic = extras.capture_dir.is_some() || extras.script_path.is_some();
    // load base game mod
    let mut pre_events = EventManager::new();
    let mut networking = ClientNetworking::new(server_port, server_address);
    networking.init(player_name.to_owned())?;
    while networking.is_welcoming() {
        // wait 1 ms
        std::thread::sleep(std::time::Duration::from_millis(1));
        networking.check_thread_for_errors()?;
    }

    networking.update(&mut pre_events)?;
    networking.start_receiving();

    let timer = std::time::Instant::now();

    let loading_text = Arc::new(Mutex::new("Loading".to_owned()));
    let loading_text2 = loading_text;

    let temp_fn = || -> Result<(ClientModManager, ClientBlocks, ClientWalls, ClientEntities, ClientItems, ClientGases, ClientNetworking)> {
        "Loading mods".clone_into(&mut loading_text2.lock().unwrap_or_else(PoisonError::into_inner));
        let mut mods = ClientModManager::new();
        let mut blocks = ClientBlocks::new();
        let walls = ClientWalls::new(&mut blocks.get_blocks());
        let entities = ClientEntities::new();
        let mut items = ClientItems::new();
        let mut gases = ClientGases::new();

        while let Some(event) = pre_events.pop_event() {
            mods.on_event(&event)?;
            blocks.on_event(&event, &mut pre_events, &mut networking)?;
            walls.on_event(&event)?;
            gases.on_event(&event)?;
            items.on_event(&event, &mut entities.get_entities(), &mut pre_events)?;
        }

        blocks.init(&items.get_items_arc(), &mut mods.mod_manager)?;
        walls.init(&mut mods.mod_manager)?;
        gases.init(&mut mods.mod_manager)?;
        items.init(&mut mods.mod_manager, &entities.get_entities_arc())?;

        "Initializing mods".clone_into(&mut loading_text2.lock().unwrap_or_else(PoisonError::into_inner));
        mods.init()?;

        anyhow::Ok((mods, blocks, walls, entities, items, gases, networking))
    };
    // if the init fails, we clear the loading text so the error can be displayed
    let result = temp_fn()?;
    loading_text2.lock().unwrap_or_else(PoisonError::into_inner).clear();

    let mut mods = result.0;
    let mut blocks = result.1;
    let mut walls = result.2;
    let entities = result.3;
    let mut items = result.4;
    let mut networking = result.6;
    let mut gases = result.5;

    let mut background = Background::new();
    let mut inventory = ClientInventory::new();
    let mut lights = ClientLights::new();
    let mut events = EventManager::new();
    let mut camera = Camera::new();
    let mut players = ClientPlayers::new(player_name);
    let mut block_selector = BlockSelector::new();
    let mut pause_menu = PauseMenu::new(graphics, settings.clone(), global_settings.clone());
    let mut debug_menu = DebugMenu::new();
    let mut gas_overlay = Overlay::new(debug);
    let mut substance_renderer = SubstanceRenderer::new();
    let mut zombies = ClientZombies::new();
    let mut framerate_measurer = FramerateMeasurer::new(deterministic);
    let mut chat = ClientChat::new(graphics);
    let mut health = ClientHealth::new();
    let mut floating_text = FloatingTextManager::new();
    let mut respawn_screen = RespawnScreen::new();

    background.init()?;
    inventory.init(graphics);
    lights.init(&blocks.get_blocks(), settings)?;

    blocks.load_resources(&mods.mod_manager)?;
    walls.load_resources(&mods.mod_manager)?;
    items.load_resources(&mods.mod_manager)?;
    camera.load_resources(graphics);
    players.load_resources(&mods.mod_manager)?;
    zombies.load_resources(&mods.mod_manager)?;
    health.load_resources(&mods.mod_manager)?;

    pause_menu.init(graphics);
    debug_menu.init();
    gas_overlay.init(graphics, &GasOverlayProvider::new(&gases));
    chat.init(graphics);
    respawn_screen.init(graphics);

    // print the time it took to initialize
    println!("Game joined in {}ms", timer.elapsed().as_millis());

    // Harness (capture/script) setup. Deterministic runs drop the fps
    // limiter and vsync (frames run as fast as they render), cancel the
    // real_scale easing so window-size math is stable from frame 0, and
    // advance the sim by a fixed number of sub-ticks per frame.
    let mut script_driver = extras.script_path.as_ref().map(|path| ScriptDriver::from_file(path)).transpose()?;
    let mut capture = extras.capture_dir.as_ref().map(|dir| FrameCapture::new(dir, extras.capture_interval, extras.capture_max_frames));
    if deterministic {
        graphics.disable_fps_limit();
        graphics.enable_vsync(false);
        graphics.set_scale_immediate(graphics.real_scale());
    }
    let run_started = std::time::Instant::now();
    // (spec, pressed, started while pressing)
    let mut world_click_steering: Option<(super::script::WorldClickSpec, bool, std::time::Instant)> = None;

    'main_loop: while graphics.is_window_open() {
        framerate_measurer.update();

        let frame_timer = std::time::Instant::now();

        // Scripted play: pump the driver *before* the event drain so events
        // it injects land in this very frame. It needs the camera position
        // (of the previous frame) for world-click resolution.
        let mut script_status = if let Some(driver) = script_driver.as_mut() {
            let camera_pos = camera.get_position();
            let status = driver.update(graphics, camera_pos.0, camera_pos.1)?;
            // client-side teleport: move the main player's position directly;
            // the server adopts client-reported positions so this sticks
            if let Some((x, y)) = status.tp {
                if let Some(main_player) = players.get_main_player() {
                    let mut entities_lock = entities.get_entities();
                    if let Ok(position_component) = entities_lock.ecs.query_one_mut::<&mut PositionComponent>(main_player) {
                        position_component.set_x(x);
                        position_component.set_y(y);
                    }
                    if let Ok(physics_component) = entities_lock.ecs.query_one_mut::<&mut crate::shared::entities::PhysicsComponent>(main_player) {
                        physics_component.velocity_x = 0.0;
                        physics_component.velocity_y = 0.0;
                    }
                }
            }
            status
        } else {
            super::script::ScriptStatus::default()
        };

        // Scripted world click steering: the script moves the mouse override
        // near the target block; each frame nudge it until the block selector
        // actually selects the target, then inject the press/release. This is
        // immune to any world<->screen unit drift because it closes on the
        // same math the game itself uses to resolve clicks.
        if let Some(spec) = script_status.world_click.take() {
            world_click_steering = Some((spec, false, std::time::Instant::now()));
        }
        if let Some((spec, pressed, started)) = &mut world_click_steering {
            if !*pressed {
                let selected = BlockSelector::get_selected_block(graphics, &camera);
                let err_x = spec.block_x as f32 - selected.0 as f32;
                let err_y = spec.block_y as f32 - selected.1 as f32;
                if err_x.abs() < 0.9 && err_y.abs() < 0.9 {
                    graphics.inject_event(gfx::Event::KeyPress(spec.button, false));
                    *pressed = true;
                } else {
                    let render_block_width = crate::shared::blocks::RENDER_BLOCK_WIDTH;
                    spec.mouse_x += err_x * render_block_width;
                    spec.mouse_y += err_y * render_block_width;
                    graphics.set_scripted_mouse_pos(Some(gfx::FloatPos(spec.mouse_x, spec.mouse_y)));
                }
            } else if started.elapsed().as_millis() as u64 >= spec.hold_ms {
                graphics.inject_event(gfx::Event::KeyRelease(spec.button, false));
                world_click_steering = None;
            }
        }

        while let Some(event) = graphics.get_event() {
            events.push_event(events::Event::new(event));
        }

        graphics.block_key_states = chat.is_selected();

        networking.update(&mut events)?;
        mods.update()?;
        blocks.update(framerate_measurer.get_delta_time(), &mut events)?;
        walls.update(framerate_measurer.get_delta_time(), &mut events)?;

        if let Some(main_player) = players.get_main_player() {
            let player_pos = entities.get_entities().ecs.get::<&PositionComponent>(main_player)?.deref().clone();

            camera.set_position(player_pos.x(), player_pos.y());
        }

        while framerate_measurer.has_5ms_passed() {
            camera.update_ms(graphics);
            players.controls_enabled = !camera.is_detached();
            players.update(graphics, &mut entities.get_entities(), &mut networking, &blocks.get_blocks())?;
            entities.get_entities().update_entities_ms(&blocks.get_blocks(), &mut events)?;
            zombies.update(&mut entities.get_entities())?;
        }

        respawn_screen.is_shown = players.get_main_player().is_none() && !players.is_waiting_for_player();

        items.update(&mut events);

        background.render(graphics, &camera);
        walls.render(graphics, &camera, &frame_timer)?;
        blocks.render(graphics, &camera /*&frame_timer*/)?;
        // Draw the always-on procedural substance layer (gases + liquids) right
        // after the terrain, before players/items/HUD, so fluids sit beneath
        // entities like a cartoony backdrop of roiling gas and bubbling liquid.
        substance_renderer.render(graphics, &camera, &gases, &blocks.get_blocks())?;
        // Draw the gas overlay immediately after the terrain (background/walls/
        // blocks) but before players/items/HUD, so its gray wash only desaturates
        // the world and the gas cells remain the focus while entities stay readable.
        gas_overlay.render(graphics, &camera, &GasOverlayProvider::new(&gases))?;
        players.render(graphics, &mut entities.get_entities(), &camera);
        items.render(graphics, &camera, &mut entities.get_entities())?;
        zombies.render(graphics, &camera, &mut entities.get_entities())?;
        floating_text.render(graphics, &camera);
        lights.render(graphics, &camera, &blocks.get_blocks(), settings, &frame_timer)?;
        camera.render(graphics);
        block_selector.render(graphics, &mut networking, &camera)?;
        inventory.render(graphics, &items, &mut networking, &blocks.get_blocks())?;
        health.render(graphics);
        // The gas overlay's toggle icon is HUD and must render on top of the world
        // (the gray wash + gas cells themselves rendered much earlier).
        gas_overlay.render_hud(graphics, &GasOverlayProvider::new(&gases));
        chat.render(graphics);
        respawn_screen.render(graphics);

        pause_menu.render(graphics);

        debug_menu.render(
            graphics,
            &[
                format!("FPS: {}", framerate_measurer.get_fps()),
                format!("{:.2} ms max", framerate_measurer.get_max_frame_time()),
                format!("{:.2} ms avg", framerate_measurer.get_avg_frame_time()),
            ],
        );

        while let Some(event) = events.pop_event() {
            if chat.on_event(&event, graphics, &mut networking)? {
                continue;
            }
            // The gas overlay's on-screen toggle icon consumes mouse press/release
            // events that land on it, so they must not also reach the world
            // handlers below (block_selector would mine the block under the cursor,
            // and an off-world corner click would crash the server on an
            // out-of-bounds coordinate). Run it before the world handlers and skip
            // them entirely when it consumes the event.
            if gas_overlay.on_event(&event, graphics)? {
                continue;
            }
            inventory.on_event(&event, &mut networking, &items, &mut blocks.get_blocks(), &mut events)?;
            mods.on_event(&event)?;
            blocks.on_event(&event, &mut events, &mut networking)?;
            walls.on_event(&event)?;
            gases.on_event(&event)?;
            entities.on_event(&event, &mut events, &players, &mut networking)?;
            zombies.on_event(&event, &mut entities.get_entities())?;
            items.on_event(&event, &mut entities.get_entities(), &mut events)?;
            block_selector.on_event(graphics, &mut networking, &camera, &event, &mut events)?;
            players.on_event(&event, &mut entities.get_entities())?;
            lights.on_event(&event, &blocks.get_blocks())?;
            camera.on_event(&event);
            health.on_event(&event, graphics, &mut floating_text, &players, &entities.get_entities());
            if pause_menu.on_event(&event, graphics) {
                break 'main_loop;
            }
            debug_menu.on_event(&event);
            respawn_screen.on_event(&event, graphics, &mut networking)?;
        }

        // Keep the server's live gas snapshots flowing. The always-on substance
        // renderer needs fresh layer data every tick to show roiling gas and
        // bubbling liquid, so live updates are requested unconditionally while
        // the world is open (the G-hotkey debug overlay is an additional view on
        // the same data, not the only consumer of it).
        gases.request_live_updates(true, &mut networking)?;

        framerate_measurer.update_post_render();

        graphics.update_window();

        let script_shot = script_status.shot;
        if script_status.exit {
            graphics.close_window();
        }

        if let Some(capture) = capture.as_mut() {
            capture.try_capture(graphics, script_shot)?;
            if capture.is_done() {
                capture.write_manifest()?;
                graphics.close_window();
            }
        }

        if let Some(timeout) = extras.capture_timeout_ms {
            if run_started.elapsed().as_millis() as u64 >= timeout {
                if let Some(capture) = capture.as_ref() {
                    capture.write_manifest()?;
                }
                graphics.close_window();
            }
        }
    }

    if let Some(capture) = capture {
        capture.write_manifest()?;
        println!("Captured {} frames", capture.frames_taken());
    }

    lights.stop(settings)?;
    networking.stop()?;
    mods.stop()?;

    Ok(())
}
