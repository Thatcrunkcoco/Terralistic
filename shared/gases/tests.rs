#![allow(clippy::unwrap_used)]
#![cfg(test)]
mod tests {
    use crate::shared::gases::{GasCell, GasId, Gases, GasType};

    fn new_test_gas(name: &str, density: f32) -> GasType {
        GasType::new(name.to_owned(), density)
    }

    #[test]
    fn starts_empty() {
        let gases = Gases::new();
        assert_eq!(gases.get_num_gas_types(), 0);
        assert_eq!(gases.get_all_gas_type_ids().len(), 0);
    }

    #[test]
    fn register_assigns_dense_ids_in_order() {
        let mut gases = Gases::new();
        let o2 = gases.register_new_gas_type(new_test_gas("o2", 1.43)).unwrap();
        let co2 = gases.register_new_gas_type(new_test_gas("co2", 1.98)).unwrap();
        let h2 = gases.register_new_gas_type(new_test_gas("h2", 0.09)).unwrap();
        assert_eq!(o2.raw(), 0);
        assert_eq!(co2.raw(), 1);
        assert_eq!(h2.raw(), 2);
        assert_eq!(gases.get_num_gas_types(), 3);
    }

    #[test]
    fn register_empty_name_rejected() {
        let mut gases = Gases::new();
        assert!(gases.register_new_gas_type(new_test_gas("", 1.0)).is_err());
    }

    #[test]
    fn register_duplicate_name_rejected() {
        let mut gases = Gases::new();
        assert!(gases.register_new_gas_type(new_test_gas("o2", 1.43)).is_ok());
        assert!(gases.register_new_gas_type(new_test_gas("o2", 1.43)).is_err());
        assert!(gases.register_new_gas_type(new_test_gas("co2", 1.98)).is_ok());
    }

    #[test]
    fn register_non_finite_density_rejected() {
        let mut gases = Gases::new();
        assert!(gases.register_new_gas_type(new_test_gas("nan_gas", f32::NAN)).is_err());
        assert!(gases.register_new_gas_type(new_test_gas("inf_gas", f32::INFINITY)).is_err());
        assert!(gases.register_new_gas_type(new_test_gas("neg_inf_gas", f32::NEG_INFINITY)).is_err());
        // a normal density is fine
        assert!(gases.register_new_gas_type(new_test_gas("ok_gas", 1.0)).is_ok());
    }

    #[test]
    fn get_by_id_round_trip() {
        let mut gases = Gases::new();
        let id = gases.register_new_gas_type(new_test_gas("o2", 1.43)).unwrap();
        let gas = gases.get_gas_type(id).unwrap();
        assert_eq!(gas.name, "o2");
        assert_eq!(gas.get_id(), id);
    }

    #[test]
    fn get_by_id_unknown_is_err() {
        let gases = Gases::new();
        // an id that was never registered cannot be resolved
        assert!(gases.get_gas_type(crate::shared::gases::GasId::from_raw(999)).is_err());
    }

    #[test]
    fn get_id_by_name() {
        let mut gases = Gases::new();
        let o2 = gases.register_new_gas_type(new_test_gas("o2", 1.43)).unwrap();
        assert_eq!(gases.get_gas_id_by_name("o2"), Some(o2));
        assert_eq!(gases.get_gas_id_by_name("co2"), None);
    }

    #[test]
    fn get_all_ids_in_order() {
        let mut gases = Gases::new();
        gases.register_new_gas_type(new_test_gas("a", 1.0)).unwrap();
        gases.register_new_gas_type(new_test_gas("b", 2.0)).unwrap();
        gases.register_new_gas_type(new_test_gas("c", 3.0)).unwrap();
        let ids = gases.get_all_gas_type_ids();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0].raw(), 0);
        assert_eq!(ids[1].raw(), 1);
        assert_eq!(ids[2].raw(), 2);
    }

    #[test]
    fn density_preserved() {
        let mut gases = Gases::new();
        let id = gases.register_new_gas_type(new_test_gas("co2", 1.98)).unwrap();
        assert_eq!(gases.get_gas_type(id).unwrap().density, 1.98);
    }

    // --- GasLayer tests ---

    use crate::shared::gases::GasLayer;

    #[test]
    fn gas_layer_full_world_serialize_size() {
        // Simulate the test world's full layer to measure the compressed payload.
        let mut layer = GasLayer::new();
        let air = GasId::from_raw(0);
        let co2 = GasId::from_raw(1);
        let oxygen = GasId::from_raw(2);
        let hydrogen = GasId::from_raw(3);
        layer.create((512, 256), air, 100.0);
        // a small gas room like seed_test_gases fills: one 5x5 box per gas
        for y in 174..179 {
            for x in 250..255 { layer.set_cell(x, y, GasCell::new(co2, 100.0)).unwrap(); }
            for x in 256..261 { layer.set_cell(x, y, GasCell::new(oxygen, 100.0)).unwrap(); }
            for x in 262..267 { layer.set_cell(x, y, GasCell::new(hydrogen, 100.0)).unwrap(); }
        }
        let bytes = layer.serialize().unwrap();
        let raw = bincode::serialize(&layer).unwrap();
        eprintln!("full-size gas layer: raw={} bytes, snap-compressed={} bytes", raw.len(), bytes.len());
        assert!(bytes.len() < 65_000, "payload too big for framed tcp: {}", bytes.len());
        // verify round-trip through the sparse patch
        let mut decoded = GasLayer::new();
        decoded.deserialize(&bytes).unwrap();
        assert_eq!(decoded.get_size(), layer.get_size());
        assert_eq!(decoded.get_cell(251, 175).unwrap().gas, co2);
        assert_eq!(decoded.get_cell(257, 175).unwrap().gas, oxygen);
        assert_eq!(decoded.get_cell(266, 178).unwrap().gas, hydrogen);
        assert_eq!(decoded.get_cell(252, 100).unwrap().gas, air);
    }

    #[test]
    fn gas_layer_update_chunks_reconstruct_layer() {
        // A layer where many cells differ from the base, like a full open world
        // after gas flow has redistributed pressures. It must split into bounded
        // chunks that reconstruct the original layer when re-applied.
        let mut layer = GasLayer::new();
        let air = GasId::from_raw(0);
        let co2 = GasId::from_raw(1);
        layer.create((64, 64), air, 100.0);
        // vary pressure/gas across ~half the cells so the patch is large
        for x in 0..64 {
            for y in 0..64 {
                if (x + y) % 2 == 0 {
                    layer.set_cell(x, y, GasCell::new(if x % 3 == 0 { co2 } else { air }, 150.0)).unwrap();
                }
            }
        }

        let chunks = layer.update_chunks(400);
        assert!(chunks.len() > 1, "expected multiple chunks, got {}", chunks.len());
        assert!(chunks[0].start_of_frame);
        assert!(!chunks[1].start_of_frame);

        // No single chunk may approach the framed-TCP wire limit. Each entry is
        // ~12 bytes on the wire, so a 400-cell cap keeps every chunk tiny.
        for chunk in &chunks {
            let bytes = bincode::serialize(chunk).unwrap();
            assert!(bytes.len() < 65_000, "chunk too big for framed tcp: {}", bytes.len());
            assert!(chunk.indexes.len() <= 400);
        }

        // Rebuild the layer from the chunks and compare against the original.
        let mut decoded = GasLayer::new();
        for chunk in &chunks {
            decoded.apply_chunk(chunk);
        }
        assert_eq!(decoded.get_size(), layer.get_size());
        for x in [0, 10, 40, 63] {
            for y in [0, 15, 33, 63] {
                assert_eq!(decoded.get_cell(x, y).unwrap().gas, layer.get_cell(x, y).unwrap().gas, "gas mismatch at {x},{y}");
                assert!((decoded.get_cell(x, y).unwrap().pressure - layer.get_cell(x, y).unwrap().pressure).abs() < 0.001, "pressure mismatch at {x},{y}");
            }
        }
    }

    #[test]
    fn gas_layer_update_chunks_single_chunk_when_uniform() {
        // A uniform layer (only the base differs from nothing) still yields one
        // well-formed frame chunk that does not reset an existing client to empty.
        let mut layer = GasLayer::new();
        let air = GasId::from_raw(0);
        layer.create((10, 10), air, 100.0);
        let chunks = layer.update_chunks(400);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].start_of_frame);
        assert_eq!(chunks[0].indexes.len(), 0);

        let mut decoded = GasLayer::new();
        decoded.apply_chunk(&chunks[0]);
        assert_eq!(decoded.get_size(), (10, 10));
        assert_eq!(decoded.get_cell(5, 5).unwrap().gas, air);
        assert_eq!(decoded.get_cell(5, 5).unwrap().pressure, 100.0);
    }

    #[test]
    fn gas_layer_starts_empty() {
        let layer = GasLayer::new();
        assert_eq!(layer.get_size(), (0, 0));
        assert_eq!(layer.cells().len(), 0);
    }

    #[test]
    fn gas_layer_create_sizes_and_fills() {
        let mut layer = GasLayer::new();
        let o2 = GasId::from_raw(0);
        layer.create((10, 20), o2, 100.0);
        assert_eq!(layer.get_size(), (10, 20));
        assert_eq!(layer.cells().len(), 200);
        // every cell is filled with the requested gas + pressure
        let cell = layer.get_cell(5, 5).unwrap();
        assert_eq!(cell.gas, o2);
        assert_eq!(cell.pressure, 100.0);
    }

    #[test]
    fn gas_layer_set_get_round_trip() {
        let mut layer = GasLayer::new();
        let o2 = GasId::from_raw(0);
        let co2 = GasId::from_raw(1);
        layer.create((50, 50), o2, 1.0);

        layer.set_cell(0, 0, GasCell::new(co2, 9.0)).unwrap();
        layer.set_cell(49, 49, GasCell::new(co2, 25.0)).unwrap();
        assert_eq!(layer.get_cell(0, 0).unwrap().gas, co2);
        assert_eq!(layer.get_cell(0, 0).unwrap().pressure, 9.0);
        assert_eq!(layer.get_cell(49, 49).unwrap().pressure, 25.0);
    }

    #[test]
    fn gas_layer_out_of_bounds() {
        let mut layer = GasLayer::new();
        let o2 = GasId::from_raw(0);
        layer.create((50, 50), o2, 1.0);
        assert!(layer.get_cell(-1, -1).is_err());
        assert!(layer.get_cell(50, 50).is_err());
        assert!(layer.get_cell(100, 5).is_err());
        assert!(layer.set_cell(-1, 0, GasCell::new(o2, 1.0)).is_err());
        assert!(layer.set_cell(0, 999, GasCell::new(o2, 1.0)).is_err());
    }

    #[test]
    fn gas_layer_serialize_round_trip() {
        let mut layer = GasLayer::new();
        let o2 = GasId::from_raw(0);
        let co2 = GasId::from_raw(1);
        layer.create((5, 5), o2, 1.0);
        layer.set_cell(2, 3, GasCell::new(co2, 42.0)).unwrap();

        let bytes = bincode::serialize(&layer).unwrap();
        let decoded: GasLayer = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.get_size(), (5, 5));
        assert_eq!(decoded.get_cell(2, 3).unwrap().gas, co2);
        assert_eq!(decoded.get_cell(2, 3).unwrap().pressure, 42.0);
        assert_eq!(decoded.get_cell(0, 0).unwrap().gas, o2);
    }

    #[test]
    fn gas_layer_snap_serialize_round_trip() {
        let mut layer = GasLayer::new();
        let air = GasId::from_raw(0);
        let co2 = GasId::from_raw(1);
        layer.create((40, 20), air, 100.0);
        // sprinkle a second gas so compression exercises mixed content
        layer.set_cell(10, 15, GasCell::new(co2, 250.0)).unwrap();

        let bytes = layer.serialize().unwrap();
        let mut decoded = GasLayer::new();
        decoded.deserialize(&bytes).unwrap();

        assert_eq!(decoded.get_size(), (40, 20));
        assert_eq!(decoded.get_cell(10, 15).unwrap().gas, co2);
        assert_eq!(decoded.get_cell(10, 15).unwrap().pressure, 250.0);
        assert_eq!(decoded.get_cell(1, 1).unwrap().gas, air);
        assert_eq!(decoded.get_cell(1, 1).unwrap().pressure, 100.0);
    }

    // --- GasFlow tests ---

    use crate::shared::gases::{GasFlow, GasFlowParams};

    /// See-through / solid map used by flow tests to control where gas may flow.
    struct OpenMap {
        open: Vec<bool>,
        w: u32,
        h: u32,
    }

    impl OpenMap {
        fn new(w: u32, h: u32) -> Self {
            Self { open: vec![true; (w * h) as usize], w, h }
        }

        fn set_solid(&mut self, x: i32, y: i32) {
            self.open[((x as u32) * self.h + y as u32) as usize] = false;
        }

        fn is_open(&self, x: i32, y: i32) -> bool {
            if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
                return false;
            }
            self.open[((x as u32) * self.h + y as u32) as usize]
        }
    }

    struct GasTestWorld {
        gases: Gases,
        layer: GasLayer,
        flow: GasFlow,
        map: OpenMap,
    }

    fn defaults() -> (GasId, GasId) {
        // two gases: light (index 0) and heavy (index 1)
        (GasId::from_raw(0), GasId::from_raw(1))
    }

    impl GasTestWorld {
        fn new(w: u32, h: u32, fill_gas: GasId, fill_pressure: f32) -> Self {
            let mut gases = Gases::new();
            // register light + heavy so ids are stable (0, 1)
            gases.register_new_gas_type(GasType::new("light".to_owned(), 1.0)).unwrap();
            gases.register_new_gas_type(GasType::new("heavy".to_owned(), 3.0)).unwrap();
            let mut layer = GasLayer::new();
            layer.create((w, h), fill_gas, fill_pressure);
            let flow = GasFlow::with_params(GasFlowParams {
                pressure_rate: 80.0,
                buoyancy_rate: 100.0,
            });
            let map = OpenMap::new(w, h);
            Self { gases, layer, flow, map }
        }

        fn tick(&mut self) {
            let open = &self.map;
            let is_open = |x: i32, y: i32| open.is_open(x, y);
            let gases = &self.gases;
            let density = move |g: GasId| {
                if g.is_none() {
                    0.0
                } else {
                    gases.get_gas_type(g).map(|t| t.density).unwrap_or(0.0)
                }
            };
            self.flow.tick(&mut self.layer, &is_open, &density);
        }

        fn activate_all(&mut self) {
            let (w, h) = self.layer.get_size();
            self.flow.activate_all(w, h);
        }

        fn p(&self, x: i32, y: i32) -> f32 {
            self.layer.get_cell(x, y).unwrap().pressure
        }
    }

    #[test]
    fn horizontal_pressure_equalizes_and_stays_positive() {
        // A 10x2 world = 20 open cells; one cell holds 100, so the converged
        // uniform pressure is 100/20 = 5 across every cell.
        let (light, _heavy) = defaults();
        let mut world = GasTestWorld::new(10, 2, light, 0.0);
        world.layer.set_cell(0, 0, GasCell::new(light, 100.0)).unwrap();
        world.activate_all();

        // Nothing should ever go negative as the gas spreads.
        for _ in 0..2000 {
            world.tick();
            assert!(world.p(0, 0) >= 0.0, "pressure went negative");
            assert!(world.p(9, 1) >= 0.0, "pressure went negative");
        }
        let left = world.p(0, 0);
        let right = world.p(9, 1);
        assert!((left - 5.0).abs() < 1.0, "left end did not reach uniform: {left}");
        assert!((right - 5.0).abs() < 1.0, "right end did not reach uniform: {right}");
    }

    #[test]
    fn open_system_converges_to_uniform_pressure_and_conserves_mass() {
        // 4x1 world = 4 open cells; 80 total, so uniform is 80/4 = 20.
        let (light, _heavy) = defaults();
        let mut world = GasTestWorld::new(4, 1, light, 0.0);
        world.layer.set_cell(0, 0, GasCell::new(light, 80.0)).unwrap();
        world.activate_all();
        for _ in 0..4000 {
            world.tick();
        }
        for _ in 0..4000 {
            world.tick();
        }
        let mut sum = 0.0;
        for x in 0..4 {
            sum += world.p(x, 0);
            assert!((world.p(x, 0) - 20.0).abs() < 1.0, "cell {x} not uniform: {}", world.p(x, 0));
        }
        // Mass should be conserved (none vanishes, none is created).
        assert!((sum - 80.0).abs() < 1.0, "mass not conserved: total {sum}");
    }

    #[test]
    fn sealed_pocket_is_contained() {
        // A 5x5 world: a sealed 1-cell pocket in the center, vacuum outside, and
        // every border cell solid so gas cannot leak out.
        let (light, _heavy) = defaults();
        let mut world = GasTestWorld::new(5, 5, light, 0.0);
        // make the pocket (2,2) open, everything else solid
        for x in 0..5 {
            for y in 0..5 {
                if (x, y) != (2, 2) {
                    world.map.set_solid(x, y);
                }
            }
        }
        world.layer.set_cell(2, 2, GasCell::new(light, 100.0)).unwrap();
        world.activate_all();

        for _ in 0..200 {
            world.tick();
        }
        // The pocket keeps all its gas (no open neighbor to leak to).
        assert_eq!(world.p(2, 2), 100.0);
    }

    #[test]
    fn solid_wall_blocks_flow() {
        // 5x3 world, solid wall at x=2. Left region (x=0,1 over 3 rows = 6 cells)
        // receives all 300 gas, so it equalizes to 300/6 = 50 each. The right
        // region stays vacuum because the wall blocks any flow across x=2.
        let (light, _heavy) = defaults();
        let mut world = GasTestWorld::new(5, 3, light, 0.0);
        for y in 0..3 {
            world.map.set_solid(2, y);
        }
        for y in 0..3 {
            world.layer.set_cell(1, y, GasCell::new(light, 100.0)).unwrap();
        }
        world.activate_all();

        for _ in 0..2000 {
            world.tick();
        }
        // Left region equalizes to 50 each.
        for y in 0..3 {
            assert!((world.p(1, y) - 50.0).abs() < 1.0, "left cell {y} not uniform: {}", world.p(1, y));
        }
        // Right region stays a vacuum (fully cut off by the solid wall).
        for y in 0..3 {
            assert_eq!(world.p(3, y), 0.0, "gas leaked through the wall at row {y}");
        }
    }

    #[test]
    fn heavier_gas_sinks() {
        let (light, heavy) = defaults();
        let mut world = GasTestWorld::new(1, 2, light, 100.0);
        // top (y=0) is heavy, bottom (y=1) is light, equal pressure: heavy should sink
        world.layer.set_cell(0, 0, GasCell::new(heavy, 100.0)).unwrap();
        world.layer.set_cell(0, 1, GasCell::new(light, 100.0)).unwrap();
        world.activate_all();

        let before_bottom = world.p(0, 1);
        for _ in 0..20 {
            world.tick();
        }
        // The heavy gas migrated down: bottom gained pressure, top lost it.
        assert!(world.p(0, 1) > before_bottom, "bottom should gain, got {}", world.p(0, 1));
        assert!(world.p(0, 0) < 100.0, "top should lose, got {}", world.p(0, 0));
    }

    #[test]
    fn stable_layering_does_not_move() {
        // The stable arrangement (light above, heavy below, equal pressure) is a
        // rest state for buoyancy: nothing should drift (within float tolerance).
        let (light, heavy) = defaults();
        let mut world = GasTestWorld::new(1, 2, light, 100.0);
        world.layer.set_cell(0, 0, GasCell::new(light, 100.0)).unwrap();
        world.layer.set_cell(0, 1, GasCell::new(heavy, 100.0)).unwrap();
        world.activate_all();

        for _ in 0..200 {
            world.tick();
        }
        assert!((world.p(0, 0) - 100.0).abs() < 0.01, "top drifted: {}", world.p(0, 0));
        assert!((world.p(0, 1) - 100.0).abs() < 0.01, "bottom drifted: {}", world.p(0, 1));
    }

    #[test]
    fn test_world_boxes_survive_flow_and_produce_diff() {
    // Reproduce the flat test world scenario: a 512x256 world filled with air
    // (raw 0) at pressure 100, containing three sealed 5x5 boxes in a row, one
    // gas each — co2 x[250..255], oxygen x[256..261], hydrogen x[262..267].
    // Activate-all, run many flow ticks, then check update_chunks still reports
    // the box pockets as differing from the air base.
    let w = 512u32; let h = 256u32;
    let air = GasId::from_raw(0);
    let co2 = GasId::from_raw(1);
    let oxy = GasId::from_raw(2);
    let hyd = GasId::from_raw(3);
    let mut gases = Gases::new();
    let _ = gases.register_new_gas_type(GasType::new("air".to_owned(), 1.2));
    let _ = gases.register_new_gas_type(GasType::new("co2".to_owned(), 1.98));
    let _ = gases.register_new_gas_type(GasType::new("oxygen".to_owned(), 1.43));
    let _ = gases.register_new_gas_type(GasType::new("hydrogen".to_owned(), 0.09));

    let mut layer = GasLayer::new();
    layer.create((w, h), air, 100.0);
    let flow = GasFlow::with_params(GasFlowParams { pressure_rate: 80.0, buoyancy_rate: 100.0 });

    // Build an OpenMap with the box walls solid (matching stone_box). The boxes
    // occupy interiors y in [174..179], with a shared 1-block wall between
    // neighbors: walls at x in [249, 255, 261, 267].
    let mut map = OpenMap::new(w, h);
    let seal_box = |map: &mut OpenMap, x0: i32, x1: i32, y0: i32, y1: i32| {
        for x in (x0)..=(x1) { map.set_solid(x, y0 - 1); map.set_solid(x, y1); }
        for y in (y0)..=(y1) { map.set_solid(x0 - 1, y); map.set_solid(x1, y); }
    };
    seal_box(&mut map, 250, 255, 174, 179);
    seal_box(&mut map, 256, 261, 174, 179);
    seal_box(&mut map, 262, 267, 174, 179);

    // Seed each box with a single gas.
    for x in 250..255 { for y in 174..179 { layer.set_cell(x, y, GasCell::new(co2, 100.0)).unwrap(); } }
    for x in 256..261 { for y in 174..179 { layer.set_cell(x, y, GasCell::new(oxy, 100.0)).unwrap(); } }
    for x in 262..267 { for y in 174..179 { layer.set_cell(x, y, GasCell::new(hyd, 100.0)).unwrap(); } }

    let mut flow = flow;
    flow.activate_all(w, h);

    // After seeding, before flow: update_chunks must report >0 diff cells.
    let pre_chunks = layer.update_chunks(3000);
    let pre_diff: usize = pre_chunks.iter().map(|c| c.indexes.len()).sum();
    eprintln!("PRE-FLOW diff cells = {pre_diff}");
    assert!(pre_diff > 0, "boxes not seen as differing from base before flow!");

    // Run flow ticks enough for containment + relayer to settle (the sealed
    // pockets must persist, so a moderate iteration count is plenty).
    for _ in 0..2_000 {
        flow.tick(&mut layer, &|x: i32, y: i32| map.is_open(x, y),
            &|g| if g.is_none() { 0.0 } else { gases.get_gas_type(g).map(|t| t.density).unwrap_or(0.0) });
    }

    let post_chunks = layer.update_chunks(3000);
    let post_diff: usize = post_chunks.iter().map(|c| c.indexes.len()).sum();
    let base = post_chunks.first().map(|c| format!("{}@{}", c.base.gas.raw(), c.base.pressure as i32)).unwrap();
    eprintln!("POST-FLOW diff cells = {post_diff}, base = {base}");
    let cell = |x: i32, y: i32| -> String {
        match layer.get_cell(x, y) {
            Ok(c) => format!("{}@{}", c.gas.raw(), c.pressure as i32),
            Err(_) => "ERR".to_owned(),
        }
    };
    eprintln!("status co2(252,174)={} oxygen(258,174)={} hydrogen(264,174)={}",
        cell(252,174), cell(258,174), cell(264,174));
    assert!(post_diff > 0, "boxes disappeared from diff after flow!");
    }
}
