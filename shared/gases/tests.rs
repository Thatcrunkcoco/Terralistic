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
                pressure_rate: 0.08,
                buoyancy_rate: 0.02,
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
}
