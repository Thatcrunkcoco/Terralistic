use crate::shared::blocks::BlockId;
use crate::shared::walls::WallId;

#[derive(Clone)]
pub(super) struct Ore {
    pub block: BlockId,
    pub start_noise: f32,
    pub end_noise: f32,
}

#[derive(Clone)]
pub(super) struct Biome {
    pub min_width: u32,
    pub max_width: u32,
    pub min_terrain_height: u32,
    pub max_terrain_height: u32,
    // How strongly terrain rises from min_terrain_height toward max_terrain_height.
    // 1.0 = the full min..max range; 0.0 produces flat terrain at min_terrain_height.
    pub terrain_amplitude: f32,
    // Wavelength (noise divisor) for terrain height. Smaller = more frequent hills,
    // larger = broader rolling terrain. 150.0 is the historical default.
    pub terrain_frequency: f32,
    // How cave-filled the underground is (0.0 = none, higher = more caves).
    // 0.15 is the historical default.
    pub cave_density: f32,
    pub base_block: BlockId,
    pub base_wall: WallId,
    // the first element is connection weight, the second is the biome id
    pub adjacent_biomes: Vec<(i32, i32)>,
    pub mod_id: i32,
    pub generator_function: Option<String>,
    pub ores: Vec<Ore>,
}

impl Biome {
    pub const fn new(mod_id: i32) -> Self {
        Self {
            min_width: 0,
            max_width: 0,
            min_terrain_height: 0,
            max_terrain_height: 0,
            terrain_amplitude: 1.0,
            terrain_frequency: 150.0,
            cave_density: 0.15,
            base_block: BlockId::undefined(),
            base_wall: WallId::undefined(),
            adjacent_biomes: Vec::new(),
            mod_id,
            generator_function: None,
            ores: Vec::new(),
        }
    }
}
