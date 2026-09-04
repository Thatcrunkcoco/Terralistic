use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};

use crate::shared::gases::{GasId, GasType};

/// Stores all registered gas types.
///
/// Gas types are held in a contiguous `Vec` indexed by their dense `GasId`, with
/// a secondary name -> id map for config-time lookups. This mirrors the existing
/// item/block/wall registries and is the scalable foundation called out in the
/// gas-type module: adding hundreds of gas types only grows the `Vec` and never
/// slows per-cell behavior, because cells only ever hold a compact `GasId`.
#[derive(Default)]
pub struct Gases {
    pub(super) gas_types: Vec<GasType>,
    gas_types_by_name: HashMap<String, GasId>,
}

impl Gases {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a gas type, validating that the name is non-empty and unique
    /// among registered gases. The assigned `GasId` is returned.
    pub fn register_new_gas_type(&mut self, mut gas_type: GasType) -> Result<GasId> {
        if gas_type.name.is_empty() {
            bail!("Cannot register a gas type with an empty name");
        }
        if self.gas_types_by_name.contains_key(&gas_type.name) {
            bail!("A gas type named \"{}\" already exists", gas_type.name);
        }
        // Reject non-finite density so downstream flow logic can always rely on
        // the value being a real number.
        if !gas_type.density.is_finite() {
            bail!(
                "Cannot register gas type \"{}\" with a non-finite density",
                gas_type.name
            );
        }
        let id = GasId {
            id: self.gas_types.len() as i32,
        };
        gas_type.id = id.id;
        self.gas_types.push(gas_type);
        self.gas_types_by_name
            .insert(self.gas_types.last().unwrap().name.clone(), id);
        Ok(id)
    }

    /// Returns the gas type with the given id.
    pub fn get_gas_type(&self, id: GasId) -> Result<&GasType> {
        self.gas_types
            .get(id.id as usize)
            .ok_or_else(|| anyhow!("gas type not found"))
    }

    /// Returns the gas id with the given name, if registered.
    pub fn get_gas_id_by_name(&self, name: &str) -> Option<GasId> {
        self.gas_types_by_name.get(name).copied()
    }

    /// Returns the number of registered gas types.
    #[must_use]
    pub fn get_num_gas_types(&self) -> usize {
        self.gas_types.len()
    }

    /// Returns all registered gas ids in registration order.
    #[must_use]
    pub fn get_all_gas_type_ids(&self) -> Vec<GasId> {
        (0..self.gas_types.len() as i32).map(GasId::from_raw).collect()
    }

    /// Returns the name of the gas type with the given id, if it is a
    /// registered (non-negative) id. Used by diagnostic tooling to label
    /// per-type gas totals.
    #[must_use]
    pub fn get_gas_name(&self, id: GasId) -> Option<&str> {
        if id.raw() < 0 {
            return None;
        }
        self.gas_types.get(id.raw() as usize).map(|gas_type| gas_type.name.as_str())
    }
}
