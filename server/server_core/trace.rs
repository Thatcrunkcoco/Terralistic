//! Sim trace + headless sim-dump harness (Tier 1 numeric test tooling).
//!
//! Gives the agent / tests numeric ground truth for the server sim: entity
//! trajectories (speed, smoothness, jump detection) and gas-layer evolution
//! (fnv checksum + per-type totals), plus a deterministic end-state JSON dump
//! for golden-state regression testing.
//!
//! CSV schema (v1):
//!   line0:      `#trace v1`
//!   entity row: `e,<t_ms>,<id>,<kind>,<x>,<y>,<vx>,<vy>`
//!   gas row:    `g,<t_ms>,<fnv64>,<active_cells>,<name>=<amount>,...`
//! `active_cells` counts cells with amount > 0.0. Per-type values are emitted
//! in registry (id) order.

use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{anyhow, Result};

use crate::shared::entities::{Entities, PhysicsComponent, PositionComponent, ZombieComponent};
use crate::shared::gases::{GasCell, GasId, GasLayer};
// NOTE: diagnostic borrows of `Entities` are mutable because hecs queries
// require `&mut World`; trace/dump calls take `&mut Entities` but never write.

pub const TRACE_SCHEMA: u32 = 1;
pub const DUMP_SCHEMA: u32 = 1;

#[derive(Debug, Clone)]
pub struct SimTraceConfig {
    /// CSV path; `Some` enables tracing.
    pub trace_path: Option<String>,
    /// Trace interval in simulated milliseconds.
    pub trace_ms: i32,
    /// End-state dump path; `Some` enables the dump on stop.
    pub dump_path: Option<String>,
    /// Auto-stop after this much simulated time (5ms sub-ticks).
    pub duration_ms: Option<i32>,
}

impl Default for SimTraceConfig {
    fn default() -> Self {
        Self { trace_path: None, trace_ms: 250, dump_path: None, duration_ms: None }
    }
}

/// Runtime state of the trace. Lives on `Server`; configured before start,
/// stepped from the update loop, and finalized from `stop`.
pub struct SimTrace {
    config: Option<SimTraceConfig>,
    writer: Option<BufWriter<std::fs::File>>,
    last_trace_ms: i32,
    last_sim_ms: i32,
    dumped: bool,
}

impl Default for SimTrace {
    fn default() -> Self {
        Self::new()
    }
}

impl SimTrace {
    #[must_use]
    pub const fn new() -> Self {
        Self { config: None, writer: None, last_trace_ms: -1, last_sim_ms: 0, dumped: false }
    }

    pub fn set_config(&mut self, config: SimTraceConfig) {
        self.config = Some(config);
    }

    /// Records the latest simulated time so `stop` can produce a dump even
    /// when the run ended without hitting `--duration` first.
    pub fn note_sim_ms(&mut self, sim_ms: i32) {
        self.last_sim_ms = sim_ms;
    }

    /// The configured auto-stop threshold, if any.
    #[must_use]
    pub fn duration_ms(&self) -> Option<i32> {
        self.config.as_ref().and_then(|config| config.duration_ms)
    }

    /// Last simulated time recorded by the update loop.
    #[must_use]
    pub const fn last_sim_ms(&self) -> i32 {
        self.last_sim_ms
    }

    /// Emits trace rows when the interval elapsed. The file opens lazily on
    /// the first emitted row, after the schema headers are written.
    pub fn maybe_trace(&mut self, t_ms: i32, sim: &mut SimStateView) -> Result<()> {
        let Some(config) = self.config.as_ref() else { return Ok(()); };
        if config.trace_path.is_none() {
            return Ok(());
        }

        if self.writer.is_none() {
            self.writer = Some(open_trace_file(config.trace_path.as_deref().ok_or_else(|| anyhow!("missing trace path"))?)?);
        }

        if self.last_trace_ms >= 0 && t_ms - self.last_trace_ms < config.trace_ms {
            return Ok(());
        }
        self.last_trace_ms = t_ms;

        let writer = self.writer.as_mut().ok_or_else(|| anyhow!("trace writer not initialized"))?;
        write_entity_rows(writer, t_ms, sim.entities)?;
        write_gas_row(writer, t_ms, sim.gas_layer, &sim.gas_names)?;
        writer.flush().map_err(|e| anyhow!(e.to_string()))?;
        Ok(())
    }

    /// Writes the deterministic JSON dump. Idempotent; only writes once.
    pub fn write_dump(&mut self, sim: &mut SimStateView) -> Result<()> {
        if self.dumped {
            return Ok(());
        }
        let Some(dump_path) = self
            .config
            .as_ref()
            .and_then(|config| config.dump_path.clone())
        else {
            return Ok(());
        };

        self.dumped = true;
        let json = serde_json::to_vec_pretty(&build_dump(sim))?;
        std::fs::write(Path::new(&dump_path), json)?;
        Ok(())
    }
}

/// Everything a trace/dump needs from the server, built once per tick by
/// `Server` from internal state (split-borrow friendly). `entities` is mutable
/// only because hecs queries need `&mut World`.
pub struct SimStateView<'a> {
    pub entities: &'a mut Entities,
    pub gas_layer: &'a GasLayer,
    /// (id, name) pairs for every registered gas type, in id order.
    pub gas_names: Vec<(GasId, String)>,
    pub sim_ms: i32,
    pub world_seed: u64,
    pub world_size: (u32, u32),
}

/// Derives the canonical kind string for an entity.
#[must_use]
pub fn kind_of(has_zombie_component: bool) -> &'static str {
    if has_zombie_component { "zombie" } else { "player" }
}

/// fnv-1a 64-bit over raw bytes. Pure and deterministic.
#[must_use]
pub fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

/// Serializes the layer into a canonical byte stream for hashing:
/// `[w u32][h u32][per cell: gas id i32][amount f32 bits]`, then fnv-64.
#[must_use]
pub fn layer_checksum(layer: &GasLayer) -> u64 {
    let (width, height) = layer.get_size();
    let mut bytes = Vec::with_capacity(8 + layer.cells().len() * 8);
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    for cell in layer.cells() {
        bytes.extend_from_slice(&cell.gas.raw().to_le_bytes());
        bytes.extend_from_slice(&cell.amount.to_bits().to_le_bytes());
    }
    fnv64(&bytes)
}

/// Per-registered-type total amounts, in id order. Empty names use `?`.
#[must_use]
pub fn gas_totals<'names>(layer: &GasLayer, names: &'names [(GasId, String)]) -> Vec<(&'names str, f32)> {
    let mut totals = vec![0.0_f32; names.len()];
    for cell in layer.cells() {
        let raw = cell.gas.raw();
        if raw >= 0 {
            if let Some(total) = totals.get_mut(raw as usize) {
                *total += cell.amount;
            }
        }
    }
    names
        .iter()
        .zip(totals)
        .map(|((_, name), total)| (name.as_str(), total))
        .collect()
}

/// Count of cells with any substance.
#[must_use]
pub fn active_cells(layer: &GasLayer) -> usize {
    layer.cells().iter().filter(|cell: &&GasCell| cell.amount > 0.0).count()
}

pub fn write_entity_rows(writer: &mut impl Write, t_ms: i32, entities: &mut Entities) -> Result<()> {
    let mut rows: Vec<String> = Vec::new();
    for (entity, (position, physics, zombie)) in
        &mut entities.ecs.query::<(&PositionComponent, &PhysicsComponent, Option<&ZombieComponent>)>()
    {
        let kind = kind_of(zombie.is_some());
        let id = entities.get_id_from_entity(entity).map_or(0, |id| id.raw());
        rows.push(format!(
            "{t_ms},{id},{kind},{:.4},{:.4},{:.4},{:.4}",
            position.x(),
            position.y(),
            physics.velocity_x,
            physics.velocity_y
        ));
    }
    for row in rows {
        writeln!(writer, "e,{row}")?;
    }
    Ok(())
}

pub fn write_gas_row(writer: &mut impl Write, t_ms: i32, layer: &GasLayer, gas_names: &[(GasId, String)]) -> Result<()> {
    let totals = gas_totals(layer, gas_names);
    let mut row = format!("g,{t_ms},{},{},", layer_checksum(layer), active_cells(layer));
    for (name, total) in totals {
        row.push_str(&format!("{name}={:.4},", total));
    }
    writeln!(writer, "{row}")?;
    Ok(())
}

fn open_trace_file(path: &str) -> Result<BufWriter<std::fs::File>> {
    let file = std::fs::File::create(Path::new(path))?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "#trace v{TRACE_SCHEMA}")?;
    Ok(writer)
}

// ---- JSON dump ----

#[derive(serde_derive::Serialize)]
struct DumpEntity {
    id: u32,
    kind: &'static str,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

#[derive(serde_derive::Serialize)]
struct DumpGas {
    fnv64: u64,
    active_cells: usize,
    /// name -> total amount, fixed-point stable through string formatting.
    per_type: Vec<String>,
}

#[derive(serde_derive::Serialize)]
struct SimDump {
    schema: u32,
    sim_ms: i32,
    seed: u64,
    world: (u32, u32),
    entities: Vec<DumpEntity>,
    gas: DumpGas,
    /// fnv-64 over the canonical payload built *before* header fields
    /// (sim_ms etc.) are mixed in — this is the golden-comparison value.
    payload_fnv64: u64,
}

fn collect_entities(entities: &mut Entities) -> Vec<DumpEntity> {
    let mut out = Vec::new();
    for (entity, (position, physics, zombie)) in
        &mut entities.ecs.query::<(&PositionComponent, &PhysicsComponent, Option<&ZombieComponent>)>()
    {
        out.push(DumpEntity {
            id: entities.get_id_from_entity(entity).map_or(0, |id| id.raw()),
            kind: kind_of(zombie.is_some()),
            x: position.x(),
            y: position.y(),
            vx: physics.velocity_x,
            vy: physics.velocity_y,
        });
    }
    out.sort_by_key(|ent| ent.id);
    out
}

/// Canonical payload for the golden hash: formatted entity rows + gas
/// checksum + per-type totals — deliberately excluding sim_ms/seed/world so
/// runs of different durations hash identically for the same sim state.
#[must_use]
fn payload_bytes(sim: &mut SimStateView) -> Vec<u8> {
    let mut bytes = Vec::new();
    for entity in collect_entities(sim.entities) {
        bytes.extend_from_slice(
            format!("e,{},{},{:.4},{:.4},{:.4},{:.4}\n", entity.id, entity.kind, entity.x, entity.y, entity.vx, entity.vy).as_bytes(),
        );
    }
    bytes.extend_from_slice(&layer_checksum(sim.gas_layer).to_le_bytes());
    for (_, total) in gas_totals(sim.gas_layer, &sim.gas_names) {
        bytes.extend_from_slice(&total.to_bits().to_le_bytes());
    }
    bytes
}

fn build_dump(sim: &mut SimStateView) -> SimDump {
    let entities = collect_entities(sim.entities);
    let (fnv64_value, active) = (layer_checksum(sim.gas_layer), active_cells(sim.gas_layer));
    let per_type: Vec<String> = gas_totals(sim.gas_layer, &sim.gas_names)
        .iter()
        .map(|(name, total)| format!("{name}={total:.4}"))
        .collect();
    let payload_fnv64 = fnv64(&payload_bytes(sim));

    SimDump {
        schema: DUMP_SCHEMA,
        sim_ms: sim.sim_ms,
        seed: sim.world_seed,
        world: sim.world_size,
        entities,
        gas: DumpGas { fnv64: fnv64_value, active_cells: active, per_type },
        payload_fnv64,
    }
}


#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::shared::entities::spawn_zombie;
    use crate::shared::gases::GasLayer;

    #[test]
    fn fnv64_is_deterministic() {
        assert_eq!(fnv64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv64(b"a"), fnv64(b"a"));
        assert_ne!(fnv64(b"a"), fnv64(b"b"));
    }

    #[test]
    fn kind_of_choices() {
        assert_eq!(kind_of(true), "zombie");
        assert_eq!(kind_of(false), "player");
    }

    #[test]
    fn layer_checksum_is_stable_and_content_sensitive() {
        let mut layer = GasLayer::new();
        layer.create((4, 2), GasId::from_raw(0), 100.0);
        let baseline = layer_checksum(&layer);
        assert_eq!(layer_checksum(&layer), baseline);
        layer.set_cell(0, 0, GasCell::new(GasId::from_raw(1), 50.0)).unwrap();
        assert_ne!(layer_checksum(&layer), baseline);
    }

    #[test]
    fn gas_totals_aggregate_per_type_in_registry_order() {
        let mut layer = GasLayer::new();
        layer.create((2, 2), GasId::from_raw(0), 10.0);
        layer.set_cell(0, 0, GasCell::new(GasId::from_raw(1), 5.0)).unwrap();
        let names = vec![(GasId::from_raw(0), "air".to_owned()), (GasId::from_raw(1), "o2".to_owned())];
        let totals = gas_totals(&layer, &names);
        assert_eq!(totals, vec![("air", 30.0), ("o2", 5.0)]);
    }

    #[test]
    fn active_cells_counts_only_filled_cells() {
        let mut layer = GasLayer::new();
        layer.create((2, 2), GasId::from_raw(0), 10.0);
        assert_eq!(active_cells(&layer), 4);
        layer.set_cell(0, 0, GasCell::new(GasId::NONE, 0.0)).unwrap();
        assert_eq!(active_cells(&layer), 3);
    }

    #[test]
    fn entity_rows_use_zombie_kind_and_fixed_floats() {
        let mut entities = Entities::new();
        let _ = spawn_zombie(&mut entities, 12.0, 34.0);

        let mut buffer: Vec<u8> = Vec::new();
        write_entity_rows(&mut buffer, 125, &mut entities).unwrap();
        let text = String::from_utf8(buffer).unwrap();
        assert_eq!(text, "e,125,1,zombie,12.0000,34.0000,0.0000,0.0000\n");
    }

    #[test]
    fn gas_row_includes_totals_fallen_back_to_question_mark() {
        let layer = {
            let mut layer = GasLayer::new();
            layer.create((1, 1), GasId::from_raw(0), 100.0);
            layer
        };
        let names = vec![(GasId::from_raw(0), "air".to_owned())];
        let mut buffer: Vec<u8> = Vec::new();
        write_gas_row(&mut buffer, 250, &layer, &names).unwrap();
        let text = String::from_utf8(buffer).unwrap();
        assert!(text.starts_with("g,250,"), "{text:?}");
        assert!(text.contains("air=100.0000"), "{text:?}");
    }

    #[test]
    fn dump_payload_excludes_header_fields() {
        let mut entities = Entities::new();
        let _ = spawn_zombie(&mut entities, 1.0, 2.0);
        let mut layer = GasLayer::new();
        layer.create((2, 1), GasId::from_raw(0), 60.0);
        let names = vec![(GasId::from_raw(0), "air".to_owned())];

        let first = {
            let mut sim = SimStateView { entities: &mut entities, gas_layer: &layer, gas_names: names.clone(), sim_ms: 100, world_seed: 123, world_size: (2, 1) };
            payload_bytes(&mut sim)
        };
        let second = {
            let mut sim = SimStateView { entities: &mut entities, gas_layer: &layer, gas_names: names, sim_ms: 999, world_seed: 555, world_size: (4, 4) };
            payload_bytes(&mut sim)
        };
        assert_eq!(fnv64(&first), fnv64(&second));
        assert_eq!(first, second);
    }
}
