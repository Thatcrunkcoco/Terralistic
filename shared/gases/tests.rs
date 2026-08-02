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
}
