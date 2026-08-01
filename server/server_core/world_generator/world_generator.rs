use std::collections::HashMap;
use std::sync::{Arc, MutexGuard};
use std::sync::{Mutex, PoisonError};

use anyhow::{anyhow, bail, Result};
use noise::Perlin;
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};

use crate::server::server_core::world_generator::biome::Biome;
use crate::server::server_core::world_generator::noise::{convolve, turbulence};
use crate::shared::blocks::{BlockId, Blocks};
use crate::shared::mod_manager::ModManager;
use crate::shared::walls::{WallId, Walls};

pub struct WorldGenerator {
    pub(super) biomes: Arc<Mutex<Vec<Biome>>>,
}

impl WorldGenerator {
    pub fn new() -> Self {
        Self {
            biomes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn get_biomes(&self) -> MutexGuard<'_, Vec<Biome>> {
        self.biomes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// This function creates a array of biome ids and returns it along with the width of the world.
    /// Each biome id is for each column of the world.
    fn generate_biome_ids(&self, min_width: i32) -> Result<(Vec<i32>, i32)> {
        let mut biome_ids = Vec::new();
        let mut width = 0;

        // walk on the graph of biomes
        // initial biome is random
        let mut curr_biome = rand::random::<i32>().abs() % self.get_biomes().len() as i32;
        while width < min_width {
            // determine the width of the current biome
            // the width is a random number between the min and max width
            let biomes = self.get_biomes();
            let biome = biomes.get(curr_biome as usize).ok_or_else(|| anyhow!("Biome with id {} does not exist!", curr_biome))?;
            let biome_width = (rand::random::<u32>() % (biome.max_width - biome.min_width) + biome.min_width) as i32;
            for _ in 0..biome_width {
                biome_ids.push(curr_biome);
            }
            width += biome_width;

            // determine the next biome
            // the next biome is chosen randomly based on the weights of the edges
            let mut total_weight = 0;
            for (weight, _) in &biome.adjacent_biomes {
                total_weight += weight;
            }
            let mut rand = rand::random::<i32>().abs() % total_weight;
            for (weight, next_biome) in &biome.adjacent_biomes {
                rand -= weight;
                if rand < 0 {
                    curr_biome = *next_biome;
                    break;
                }
            }
        }

        Ok((biome_ids, width))
    }

    /// The noise parameters are used to determine the start and end noise of each ore.
    fn generate_ore_noise_parameters(&self, blocks: &Blocks, width: i32, biome_ids: &[i32], rng: &mut StdRng) -> Result<Vec<(BlockId, Perlin, f32, Vec<(f32, f32)>)>> {
        let mut ores_start_end_noises = HashMap::new();

        for block_id in blocks.get_all_block_ids() {
            ores_start_end_noises.insert(block_id, (vec![-1.0; width as usize], vec![-1.0; width as usize]));
        }

        for x in 0..width {
            let biomes = self.get_biomes();
            let biome = biomes
                .get(*biome_ids.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))? as usize)
                .ok_or_else(|| anyhow!("invalid biome id"))?;
            for ore in &biome.ores {
                *ores_start_end_noises
                    .get_mut(&ore.block)
                    .ok_or_else(|| anyhow!("invalid block"))?
                    .0
                    .get_mut(x as usize)
                    .ok_or_else(|| anyhow!("invalid coordinate"))? = ore.start_noise;
                *ores_start_end_noises
                    .get_mut(&ore.block)
                    .ok_or_else(|| anyhow!("invalid block"))?
                    .1
                    .get_mut(x as usize)
                    .ok_or_else(|| anyhow!("invalid coordinate"))? = ore.end_noise;
            }
        }

        let convolution_size = 50;
        // convolve the noise parameters
        for block_id in blocks.get_all_block_ids() {
            for _ in 0..5 {
                let start_noises = convolve(&ores_start_end_noises.get(&block_id).ok_or_else(|| anyhow!("invalid block id"))?.0, convolution_size);
                let end_noises = convolve(&ores_start_end_noises.get(&block_id).ok_or_else(|| anyhow!("invalid block id"))?.1, convolution_size);
                ores_start_end_noises.insert(block_id, (start_noises, end_noises));
            }
        }

        let mut ores_perlin_noises = Vec::new();
        for (block_id, (start_noises, end_noises)) in &ores_start_end_noises {
            let mut commonness = 0.0;
            // commonness is the average difference between the start and end noise
            for (start_noise, end_noise) in start_noises.iter().zip(end_noises.iter()) {
                commonness += f32::abs(start_noise - end_noise);
            }
            commonness /= width as f32;

            ores_perlin_noises.push((
                *block_id,
                Perlin::new(rng.next_u32()),
                commonness,
                start_noises
                    .iter()
                    .zip(end_noises.iter())
                    .map(|(start_noise, end_noise)| (*start_noise, *end_noise))
                    .collect::<Vec<_>>(),
            ));
        }

        // sort ores by commonness, so that the most common ores are generated first
        ores_perlin_noises.sort_by(|(_, _, commonness1, _), (_, _, commonness2, _)| commonness2.partial_cmp(commonness1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(ores_perlin_noises)
    }

    /// Compute the terrain surface height for a column, given the turbulence
    /// value at that column. Pure so it can be unit tested.
    fn terrain_height(noise_turbulence: f32, min_h: f32, max_h: f32, amplitude: f32, world_height: i32) -> i32 {
        ((noise_turbulence + 1.0) * (max_h - min_h) * amplitude) as i32 + min_h as i32 + world_height * 2 / 3
    }

    /// This function generates the heights of the terrain.
    fn generate_heights(rng: &mut StdRng, width: i32, height: i32, min_heights: &[f32], max_heights: &[f32], amplitudes: &[f32], frequencies: &[f32]) -> Result<Vec<i32>> {
        let terrain_noise = Perlin::new(rng.next_u32());
        let mut heights = Vec::new();
        for x in 0..width {
            let amplitude = *amplitudes.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?;
            let frequency = *frequencies.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?;
            let turbulence_val = turbulence(&terrain_noise, x as f32 / frequency, 0.0);
            let min_h = *min_heights.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?;
            let max_h = *max_heights.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?;
            heights.push(Self::terrain_height(turbulence_val, min_h, max_h, amplitude, height));
        }
        Ok(heights)
    }

    /// This generates a column of world, before giving it to the mod to generate the rest of the world.
    fn generate_column(
        &self,
        height: i32,
        x: i32,
        heights: &[i32],
        cave_noise: &Perlin,
        cave_threshold: (f32, f32),
        blocks: &Blocks,
        walls: &Walls,
        biome_id: i32,
        ores_noises: &Vec<(BlockId, Perlin, f32, Vec<(f32, f32)>)>,
    ) -> Result<(Vec<BlockId>, Vec<WallId>)> {
        let mut curr_block_terrain = vec![BlockId::undefined(); height as usize];
        let mut curr_wall_terrain = vec![WallId::undefined(); height as usize];

        let terrain_noise_val = *heights.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?;
        let mut walls_height = *heights.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?;
        if let Some(height2) = heights.get(x as usize + 1) {
            walls_height = i32::min(walls_height, *height2);
        }
        if x > 0 {
            if let Some(height2) = heights.get(x as usize - 1) {
                walls_height = i32::min(walls_height, *height2);
            }
        }

        let biome_base_block = self.get_biomes().get(biome_id as usize).ok_or_else(|| anyhow!("invalid biome id"))?.base_block;
        let biome_base_wall = self.get_biomes().get(biome_id as usize).ok_or_else(|| anyhow!("invalid biome id"))?.base_wall;

        for y in 0..height {
            let terrain_height = height - y;

            let (cave_noise_val, cave_threshold) = {
                if terrain_height > terrain_noise_val {
                    (0.0, 0.0)
                } else {
                    (
                        f32::abs(turbulence(cave_noise, x as f32 / 80.0, y as f32 / 80.0)),
                        y as f32 / height as f32 * (cave_threshold.1 - cave_threshold.0) + cave_threshold.0,
                    )
                }
            };

            let curr_block = if terrain_height > terrain_noise_val || cave_threshold > cave_noise_val {
                blocks.air()
            } else {
                // generate ores
                let mut block = biome_base_block;

                for (block_id, noise, _commonness, noise_thresholds) in ores_noises {
                    let start_noise = noise_thresholds.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?.0;

                    let end_noise = noise_thresholds.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?.1;

                    let noise_threshold = (y as f32 / height as f32) * end_noise + (1.0 - y as f32 / height as f32) * start_noise;

                    if noise_threshold < -0.99 {
                        continue;
                    }

                    if noise_threshold > 0.99 {
                        block = *block_id;
                        continue;
                    }

                    let noise_val = turbulence(noise, x as f32 / 20.0, y as f32 / 20.0);

                    if noise_val <= noise_threshold {
                        block = *block_id;
                    }
                }

                block
            };

            *curr_block_terrain.get_mut(y as usize).ok_or_else(|| anyhow!("invalid y coordinate"))? = curr_block;

            if terrain_height < walls_height {
                *curr_wall_terrain.get_mut(y as usize).ok_or_else(|| anyhow!("invalid y coordinate"))? = biome_base_wall;
            } else {
                *curr_wall_terrain.get_mut(y as usize).ok_or_else(|| anyhow!("invalid y coordinate"))? = walls.clear;
            }
        }

        Ok((curr_block_terrain, curr_wall_terrain))
    }

    /// This lets the mod finish generating terrain
    fn call_mod_to_generate(&self, mods: &mut ModManager, biome_id: i32, mut curr_terrain: Vec<Vec<BlockId>>, curr_heights: &[i32], width: i32, height: i32) -> Result<Vec<Vec<BlockId>>> {
        let biomes = self.get_biomes();
        let curr_biome2 = biomes.get(biome_id as usize).ok_or_else(|| anyhow!("invalid biome id"))?;
        if let Some(generator_function) = &curr_biome2.generator_function {
            curr_terrain = mods
                .get_mod(curr_biome2.mod_id)
                .ok_or_else(|| anyhow!("invalid mod id"))?
                .call_function(generator_function, (curr_terrain, curr_heights.to_owned(), width, height))?;
        }

        Ok(curr_terrain)
    }

    #[allow(clippy::too_many_lines)] // TODO: split this function up
    pub fn generate(&self, world: (&mut Blocks, &mut Walls), mods: &mut ModManager, min_width: i32, height: i32, seed: u64, status_text: &Mutex<String>) -> Result<()> {        let start_time = std::time::Instant::now();

        let blocks = world.0;
        let walls = world.1;
        // create a random number generator with seed
        let mut rng = StdRng::seed_from_u64(seed);

        if self.get_biomes().is_empty() {
            bail!("No biomes were added! Cannot generate world!")
        }

        let (biome_ids, width) = self.generate_biome_ids(min_width)?;

        let mut min_heights = Vec::new();
        let mut max_heights = Vec::new();
        let mut amplitudes = Vec::new();
        let mut frequencies = Vec::new();
        let mut max_cave_thresholds = Vec::new();

        println!("Creating a world with size {width}x{height}");

        // extract the min and max heights from the biomes
        for biome_id in &biome_ids {
            let biomes = self.get_biomes();
            let biome = biomes.get(*biome_id as usize).ok_or_else(|| anyhow!("Biome with id {} does not exist!", *biome_id))?;
            min_heights.push(biome.min_terrain_height as f32);
            max_heights.push(biome.max_terrain_height as f32);
            amplitudes.push(biome.terrain_amplitude);
            frequencies.push(biome.terrain_frequency);
            max_cave_thresholds.push(biome.cave_density);
        }

        // tasks are for loading bar
        let mut current_task = 0;
        let total_tasks = width;

        let mut next_task = || {
            current_task += 1;
            *status_text.lock().unwrap_or_else(PoisonError::into_inner) = format!("Generating world {}%", (current_task as f32 / total_tasks as f32 * 100.0) as i32);
        };

        "Generating world".clone_into(&mut status_text.lock().unwrap_or_else(PoisonError::into_inner));
        blocks.create((width as u32, height as u32));

        let mut block_terrain = vec![vec![BlockId::undefined(); height as usize]; width as usize];
        let mut wall_terrain = Vec::with_capacity(width as usize);

        let ores_noises = self.generate_ore_noise_parameters(blocks, width, &biome_ids, &mut rng)?;

        let mut min_cave_thresholds = vec![0.0; width as usize];
        // max_cave_thresholds is collected per-biome from cave_density above.

        // convolve the min and max heights and cave thresholds
        let convolution_size = 50;
        for _ in 0..5 {
            min_heights = convolve(&min_heights, convolution_size);
            max_heights = convolve(&max_heights, convolution_size);
            min_cave_thresholds = convolve(&min_cave_thresholds, convolution_size);
            max_cave_thresholds = convolve(&max_cave_thresholds, convolution_size);
            amplitudes = convolve(&amplitudes, convolution_size);
            frequencies = convolve(&frequencies, convolution_size);
        }

        let cave_noise = Perlin::new(rng.next_u32());

        let mut curr_terrain = Vec::new();
        let mut curr_heights = Vec::new();
        let mut prev_x = 0;

        let heights = Self::generate_heights(&mut rng, width, height, &min_heights, &max_heights, &amplitudes, &frequencies)?;

        for x in 0..width {
            next_task();

            curr_heights.push(*heights.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?);

            let (block_column, wall_column) = self.generate_column(
                height,
                x,
                &heights,
                &cave_noise,
                (
                    *min_cave_thresholds.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?,
                    *max_cave_thresholds.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?,
                ),
                blocks,
                walls,
                *biome_ids.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?,
                &ores_noises,
            )?;

            curr_terrain.push(block_column);
            wall_terrain.push(wall_column);

            if x == width - 1 || biome_ids.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))? != biome_ids.get((x + 1) as usize).ok_or_else(|| anyhow!("invalid x coordinate"))? {
                curr_terrain = self.call_mod_to_generate(
                    mods,
                    *biome_ids.get(x as usize).ok_or_else(|| anyhow!("invalid x coordinate"))?,
                    curr_terrain,
                    &curr_heights,
                    x - prev_x + 1,
                    height,
                )?;

                for y2 in 0..height {
                    for x2 in prev_x..=x {
                        *block_terrain
                            .get_mut(x2 as usize)
                            .ok_or_else(|| anyhow!("invalid x coordinate"))?
                            .get_mut(y2 as usize)
                            .ok_or_else(|| anyhow!("invalid y coordinate"))? = *curr_terrain
                            .get((x2 - prev_x) as usize)
                            .ok_or_else(|| anyhow!("invalid x coordinate"))?
                            .get(y2 as usize)
                            .ok_or_else(|| anyhow!("invalid y coordinate"))?;
                    }
                }

                curr_terrain.clear();
                curr_heights.clear();
                prev_x = x + 1;
            }
        }

        blocks.create_from_block_ids(&block_terrain)?;
        walls.create_from_wall_ids(&wall_terrain)?;

        println!("World generated in {}ms", start_time.elapsed().as_millis());

        if current_task != total_tasks {
            println!("Not all tasks were completed! {current_task} != {total_tasks}");
        }

        Ok(())
    }
}

/// A special-purpose, fully deterministic "flat test world" created when the
/// world is named "test" with seed 123. It has a flat grass surface with some
/// trees and clearly separated, labeled test sections for exercising world
/// features (building, mining, lighting, digging, etc.).
impl WorldGenerator {
    pub fn generate_test(&self, world: (&mut Blocks, &mut Walls), _mods: &mut ModManager, _min_width: i32, _height: i32, status_text: &Mutex<String>) -> Result<()> {
        let blocks = world.0;
        let walls = world.1;

        // The test world is deliberately small so it loads instantly and every
        // test section is within easy walking distance.
        let width = 512i32;
        let width_u = width as u32;
        let height_u = 256u32;

        // Rows at or below this index are filled ground; rows above are air.
        let ground = 180i32;

        // Resolve block/wall ids from the base game mod by name.
        let b = |name: &str| blocks.get_block_id_by_name(name).unwrap_or_else(|_| blocks.air());
        let air = b("air");
        let dirt = b("dirt");
        let grass = b("grass_block");
        let wood = b("wood");
        let branch = b("branch");
        let leaves = b("leaves");
        let canopy = b("canopy");
        let stone = b("stone");
        let stone_block = b("stone_block");
        let copper = b("copper_ore");
        let iron = b("iron_ore");
        let tin = b("tin_ore");
        let torch = b("torch");
        let wood_planks = b("wood_planks");
        let dirt_wall = walls.get_wall_id_by_name("dirt").unwrap_or(walls.clear);

        println!("Creating a flat test world with size {width}x{height_u}");

        *status_text.lock().unwrap_or_else(PoisonError::into_inner) = "Generating test world".to_owned();
        blocks.create((width_u, height_u));
        walls.create((width_u, height_u));

        let mut block_terrain = vec![vec![air; height_u as usize]; width as usize];
        let mut wall_terrain = vec![vec![walls.clear; height_u as usize]; width as usize];

        // --- Flat ground: filled with dirt, grass on the surface. ---
        for x in 0..width {
            for y in ground..height_u as i32 {
                block_terrain[x as usize][y as usize] = if y == ground { grass } else { dirt };
                wall_terrain[x as usize][y as usize] = dirt_wall;
            }
        }

        // --- A few trees along the surface for decoration. ---
        for x in (20..width - 20).step_by(24) {
            place_test_tree(&mut block_terrain, x, ground - 1, width, height_u as i32, wood, branch, leaves, canopy);
        }

        // --- Clearly separated, labeled test sections. ---
        // A labeled sign is just a small stone pillar marker; the feature itself
        // sits on the grass a couple blocks ahead so it is easy to walk up to.
        let mut cursor = 24;
        for (name, feature) in TEST_SECTIONS {
            place_test_label(&mut block_terrain, &mut cursor, ground, width, height_u as i32, stone, name);
            feature(&mut block_terrain, cursor, ground, height_u as i32, &SectionCtx { width, air, dirt, wood_planks, stone_block, copper, iron, tin, torch, stone });
            cursor += 30;
        }

        // --- Fill the remaining span with trees so it never looks empty. ---
        for x in (cursor..width - 8).step_by(20) {
            place_test_tree(&mut block_terrain, x, ground - 1, width, height_u as i32, wood, branch, leaves, canopy);
        }

        blocks.create_from_block_ids(&block_terrain)?;
        walls.create_from_wall_ids(&wall_terrain)?;

        println!("World generated in {}ms", std::time::Instant::now().elapsed().as_millis());
        Ok(())
    }
}

/// Block ids the test sections need.
struct SectionCtx {
    width: i32,
    air: BlockId,
    dirt: BlockId,
    wood_planks: BlockId,
    stone_block: BlockId,
    copper: BlockId,
    iron: BlockId,
    tin: BlockId,
    torch: BlockId,
    stone: BlockId,
}

/// A test section is a named feature plus a builder that stamps it into the
/// world at `x` (the left edge of the feature), with the surface at `ground`
/// and the world `height` tall.
type SectionBuilder = fn(&mut Vec<Vec<BlockId>>, x: i32, ground: i32, height: i32, ctx: &SectionCtx);

const TEST_SECTIONS: &[(&str, SectionBuilder)] = &[
    ("pillar", build_pillar_section),
    ("pit", build_pit_section),
    ("mount", build_mount_section),
    ("ores", build_ores_section),
    ("platform", build_platform_section),
    ("torches", build_torches_section),
    ("house", build_house_section),
    ("wall", build_wall_section),
];

/// Write `block` at (x, y) only if it is inside the world bounds.
fn tset(terrain: &mut Vec<Vec<BlockId>>, x: i32, y: i32, width: i32, height: i32, block: BlockId) {
    if x >= 0 && x < width && y >= 0 && y < height {
        terrain[x as usize][y as usize] = block;
    }
}

/// Place a small marker above the surface that reads as a label. We can't draw
/// text, so we use a stone pillar to mark the start of each section.
fn place_test_label(
    terrain: &mut Vec<Vec<BlockId>>,
    cursor: &mut i32,
    ground: i32,
    width: i32,
    height: i32,
    stone: BlockId,
    _name: &str,
) {
    for h in ground - 3..ground {
        tset(terrain, *cursor, h, width, height, stone);
    }
    *cursor += 6;
}

/// Place a tree matching the normal world's appearance: a wood trunk with
/// branch/leaves on the sides and a big bushy canopy block crowning the top.
fn place_test_tree(terrain: &mut Vec<Vec<BlockId>>, x: i32, surface_y: i32, width: i32, height: i32, wood: BlockId, branch: BlockId, leaves: BlockId, canopy: BlockId) {
    let tree_height = 8;
    let trunk_top = surface_y - tree_height;

    // trunk
    for y in trunk_top..=surface_y {
        tset(terrain, x, y, width, height, wood);
    }

    // side wood branches at the top
    tset(terrain, x - 1, trunk_top, width, height, wood);
    tset(terrain, x + 1, trunk_top, width, height, wood);

    // branch + leaves columns going up on both sides
    let mut ly = trunk_top - 2;
    while ly > trunk_top - 6 {
        tset(terrain, x - 1, ly, width, height, branch);
        tset(terrain, x - 2, ly, width, height, leaves);
        tset(terrain, x + 1, ly, width, height, branch);
        tset(terrain, x + 2, ly, width, height, leaves);
        ly -= 2;
    }

    // bushy green crown (the 5x5 canopy multiblock) at the top
    tset(terrain, x - 2, trunk_top - 5, width, height, canopy);
}

/// A small tower of stone blocks to test stacking/jumping.
fn build_pillar_section(terrain: &mut Vec<Vec<BlockId>>, x: i32, ground: i32, height: i32, ctx: &SectionCtx) {
    let top = ground - 3;
    for y in top..=ground {
        for dx in 0..3 {
            tset(terrain, x + dx, y, ctx.width, height, ctx.stone_block);
        }
    }
}

/// A shallow pit dug into the ground you can jump back out of.
fn build_pit_section(terrain: &mut Vec<Vec<BlockId>>, x: i32, ground: i32, height: i32, ctx: &SectionCtx) {
    let bottom = ground + 3;
    for y in ground + 1..=bottom {
        for dx in 0..4 {
            tset(terrain, x + dx, y, ctx.width, height, ctx.air);
        }
    }
    // floor of the pit
    for dx in 0..4 {
        tset(terrain, x + dx, bottom + 1, ctx.width, height, ctx.dirt);
    }
}

/// Small stone steps to test climbing/walking up slopes.
fn build_mount_section(terrain: &mut Vec<Vec<BlockId>>, x: i32, ground: i32, height: i32, ctx: &SectionCtx) {
    for step in 0..3 {
        let level = ground - step;
        for dx in 0..(4 - step) {
            tset(terrain, x + dx, level, ctx.width, height, ctx.stone);
        }
    }
}

/// Columns of each ore type, embedded in stone, to test mining different ores.
fn build_ores_section(terrain: &mut Vec<Vec<BlockId>>, x: i32, ground: i32, height: i32, ctx: &SectionCtx) {
    let ores = [ctx.copper, ctx.iron, ctx.tin];
    for (i, ore) in ores.iter().enumerate() {
        for y in ground + 1..ground + 4 {
            tset(terrain, x + i as i32, y, ctx.width, height, *ore);
        }
    }
}

/// Low wooden steps to test vertical building and jumping.
fn build_platform_section(terrain: &mut Vec<Vec<BlockId>>, x: i32, ground: i32, height: i32, ctx: &SectionCtx) {
    for step in 0..3 {
        let level = ground - step;
        for dx in 0..3 {
            tset(terrain, x + dx, level, ctx.width, height, ctx.wood_planks);
        }
    }
}

/// A row of torches embedded in dirt to test light emission.
fn build_torches_section(terrain: &mut Vec<Vec<BlockId>>, x: i32, ground: i32, height: i32, ctx: &SectionCtx) {
    for i in 0..6 {
        tset(terrain, x + i * 2, ground - 1, ctx.width, height, ctx.torch);
    }
}

/// A small low house (wood planks with a doorway) to test building.
fn build_house_section(terrain: &mut Vec<Vec<BlockId>>, x: i32, ground: i32, height: i32, ctx: &SectionCtx) {
    let wall_top = ground - 3;
    // walls
    for y in wall_top..=ground {
        tset(terrain, x, y, ctx.width, height, ctx.wood_planks);
        tset(terrain, x + 4, y, ctx.width, height, ctx.wood_planks);
    }
    // ceiling
    for dx in 0..5 {
        tset(terrain, x + dx, wall_top, ctx.width, height, ctx.wood_planks);
    }
    // doorway
    tset(terrain, x + 4, ground, ctx.width, height, ctx.air);
    tset(terrain, x + 4, ground - 1, ctx.width, height, ctx.air);
}

/// Low stone blocks to test stepping/tunneling.
fn build_wall_section(terrain: &mut Vec<Vec<BlockId>>, x: i32, ground: i32, height: i32, ctx: &SectionCtx) {
    let top = ground - 3;
    for y in top..=ground {
        for dx in 0..3 {
            tset(terrain, x + dx, y, ctx.width, height, ctx.stone_block);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorldGenerator;

    /// Helper to call the private pure function.
    fn height(turbulence: f32, min: f32, max: f32, amplitude: f32, world_height: i32) -> i32 {
        WorldGenerator::terrain_height(turbulence, min, max, amplitude, world_height)
    }

    #[test]
    fn amplitude_zero_is_flat_at_min() {
        // amplitude 0 produces a perfectly flat surface at min + bias,
        // regardless of turbulence or max height.
        let bias = 100 * 2 / 3;
        assert_eq!(height(0.0, 10.0, 50.0, 0.0, 100), 10 + bias);
        assert_eq!(height(2.0, 10.0, 50.0, 0.0, 100), 10 + bias);
        assert_eq!(height(-1.0, 30.0, 400.0, 0.0, 200), 30 + 200 * 2 / 3);
    }

    #[test]
    fn amplitude_one_matches_original_formula() {
        // amplitude 1.0 reproduces the historical formula exactly.
        // (turbulence + 1) ranges 0..2, so turbulence -1 sits at min + bias
        // and turbulence +1 reaches 2*(max-min) + min + bias.
        let world_h = 100;
        let bias = world_h * 2 / 3;
        assert_eq!(height(-1.0, 10.0, 50.0, 1.0, world_h), 10 + bias);
        assert_eq!(height(1.0, 10.0, 50.0, 1.0, world_h), 2 * (50 - 10) + 10 + bias);
        assert_eq!(height(0.0, 10.0, 50.0, 1.0, world_h), (50 - 10) + 10 + bias);
    }

    #[test]
    fn higher_amplitude_raises_terrain() {
        // increasing amplitude monotonically raises the surface.
        let flat_amp = height(0.5, 0.0, 100.0, 0.0, 100);
        let half = height(0.5, 0.0, 100.0, 0.5, 100);
        let full = height(0.5, 0.0, 100.0, 1.0, 100);
        assert!(flat_amp < half);
        assert!(half < full);
    }
}
