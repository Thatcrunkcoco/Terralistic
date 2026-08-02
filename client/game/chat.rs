use anyhow::Result;

use crate::client::game::networking::ClientNetworking;
use crate::libraries::events::Event;
use crate::libraries::graphics as gfx;
use crate::shared::chat::ChatPacket;
use crate::shared::packet::Packet;
use gfx::{BaseUiElement, UiElement};

pub struct ChatLine {
    texture: gfx::Texture,
    transparency: i32,
    scale: f32,
}

impl ChatLine {
    pub fn new(graphics: &gfx::GraphicsContext, text: &str) -> Self {
        let (texture, scale) = if let Some(terminal_font) = graphics.terminal_font.as_ref() {
            let surface = terminal_font.render_text(text);
            (gfx::Texture::load_from_surface(&surface), 1.0)
        } else {
            let font = graphics.font_mono.as_ref().map_or(&graphics.font, |mono_font| mono_font);
            (gfx::Texture::load_from_surface(&font.create_text_surface(text, None)), 2.0)
        };

        Self {
            texture,
            transparency: 255,
            scale,
        }
    }

    pub fn render(&mut self, graphics: &mut gfx::GraphicsContext, pos: gfx::FloatPos, focused: bool) {
        let target_transparency = if focused { 255 } else { 0 };

        self.transparency = target_transparency;

        if self.transparency == 0 {
            return;
        }

        self.texture.render(graphics, self.scale, pos, None, false, Some(gfx::Color::new(255, 255, 255, self.transparency as u8)));
    }

    pub fn get_size(&self) -> gfx::FloatSize {
        gfx::FloatSize(self.texture.get_texture_size().0 * self.scale, self.texture.get_texture_size().1 * self.scale)
    }
}

pub struct ClientChat {
    back_rect: gfx::RenderRect,
    text_input: gfx::TextInput,
    chat_lines: Vec<ChatLine>,
    waiting_for_t: bool,
    visible: bool,
}

//TODO make this a UI element
impl ClientChat {
    pub fn new(graphics: &gfx::GraphicsContext) -> Self {
        Self {
            text_input: gfx::TextInput::new(graphics),
            back_rect: gfx::RenderRect::new(gfx::FloatPos(0.0, 0.0), gfx::FloatSize(0.0, 0.0)),
            chat_lines: Vec::new(),
            waiting_for_t: false,
            visible: false,
        }
    }

    pub fn init(&mut self, graphics: &gfx::GraphicsContext) {
        self.text_input.orientation = gfx::BOTTOM_LEFT;
        self.text_input.pos = gfx::FloatPos(gfx::SPACING, -gfx::SPACING);
        self.text_input.border_color = gfx::BORDER_COLOR;

        // use the terminal font (natural size) when available, otherwise the
        // pixel font scaled up.
        if graphics.terminal_font.is_some() {
            self.text_input.scale = 1.0;
            self.text_input.use_terminal_font = true;
        } else {
            self.text_input.scale = 3.0;
            self.text_input.use_terminal_font = false;
        }

        self.back_rect.fill_color = gfx::TRANSPARENT;
        self.back_rect.orientation = gfx::BOTTOM_LEFT;
        self.back_rect.pos = gfx::FloatPos(gfx::SPACING, -gfx::SPACING);
        self.back_rect.size.1 = self.text_input.get_size().1;
        self.back_rect.blur_radius = gfx::BLUR;
        self.back_rect.smooth_factor = 1.0;
        self.back_rect.shadow_intensity = gfx::SHADOW_INTENSITY;
    }

    pub fn render(&mut self, graphics: &mut gfx::GraphicsContext) {
        let window_container = gfx::Container::default(graphics);
        if self.visible {
            if self.text_input.selected {
                self.back_rect.size.0 = gfx::TEXT_INPUT_WIDTH * self.text_input.scale;
            } else {
                self.back_rect.size.0 = gfx::TEXT_INPUT_WIDTH * self.text_input.scale * 0.6;
            }

            self.back_rect.update(graphics, &window_container);
            self.back_rect.render(graphics, &window_container);

            self.text_input.width = self.back_rect.get_container(graphics, &window_container).rect.size.0 / self.text_input.scale;
            self.text_input.update(graphics, &window_container);
            self.text_input.render(graphics, &window_container);
        }

        let mut curr_y = graphics.get_window_size().1 - gfx::SPACING - self.text_input.get_size().1;
        for line in self.chat_lines.iter_mut().rev() {
            curr_y -= line.get_size().1;
            line.render(graphics, gfx::FloatPos(gfx::SPACING, curr_y), self.text_input.selected);
        }
    }

    pub fn on_event(&mut self, event: &Event, graphics: &mut gfx::GraphicsContext, networking: &mut ClientNetworking) -> Result<bool> {
        if let Some(event) = event.downcast::<gfx::Event>() {
            if let gfx::Event::TextInput(..) = event {
                if self.waiting_for_t {
                    self.waiting_for_t = false;
                    return Ok(true);
                }
            }

            self.text_input.on_event(graphics, event, &gfx::Container::default(graphics));

            if let gfx::Event::KeyPress(gfx::Key::Enter, ..) = event {
                if self.text_input.selected && !self.text_input.get_text().is_empty() {
                    networking.send_packet(Packet::new(ChatPacket {
                        message: self.text_input.get_text().clone(),
                    })?)?;

                    self.text_input.set_text(String::new());
                }
            } else if let gfx::Event::KeyPress(gfx::Key::T, ..) = event {
                if !self.is_selected() {
                    self.visible = true;
                    self.text_input.selected = true;
                    self.waiting_for_t = true;
                }
            } else if let gfx::Event::KeyPress(gfx::Key::Escape, ..) = event {
                if self.is_selected() {
                    self.visible = false;
                    self.text_input.selected = false;
                    return Ok(true);
                }
            }
        } else if let Some(event) = event.downcast::<Packet>() {
            if let Some(packet) = event.try_deserialize::<ChatPacket>() {
                self.chat_lines.push(ChatLine::new(graphics, &packet.message));
            }
        }
        Ok(self.is_selected() && event.downcast::<gfx::Event>().is_some())
    }

    pub const fn is_selected(&self) -> bool {
        self.visible
    }
}
