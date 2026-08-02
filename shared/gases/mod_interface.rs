use anyhow::Result;
use std::sync::{Arc, Mutex, PoisonError};

use crate::shared::gases::{Gases, GasId, GasType};
use crate::shared::mod_manager::ModManager;

// make GasId lua compatible
impl rlua::FromLua<'_> for GasId {
    fn from_lua(value: rlua::Value, _context: rlua::Context) -> rlua::Result<Self> {
        match value {
            rlua::Value::UserData(ud) => Ok(*ud.borrow::<Self>()?),
            _ => unreachable!(),
        }
    }
}

impl rlua::UserData for GasId {
    // implement equals comparison for GasId
    fn add_methods<'lua, M: rlua::UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_meta_method(rlua::MetaMethod::Eq, |_, this, other: Self| Ok(this.id == other.id));
    }
}

/// Initializes the gases mod interface: registers Lua functions that let mods
/// declare new gas types and look up gas ids by name.
pub fn init_gases_mod_interface(mods: &mut ModManager, gases: &Arc<Mutex<Gases>>) -> Result<()> {
    let gases_clone = gases.clone();
    mods.add_global_function("register_gas_type", move |_lua, (name, density): (String, f32)| {
        let result = Gases::register_new_gas_type(
            &mut gases_clone.lock().unwrap_or_else(PoisonError::into_inner),
            GasType::new(name, density),
        )
        .map_err(|e| rlua::Error::RuntimeError(e.to_string()))?;
        Ok(result)
    })?;

    let gases_clone = gases.clone();
    mods.add_global_function("get_gas_id_by_name", move |_lua, name: String| {
        let gases = &gases_clone.lock().unwrap_or_else(PoisonError::into_inner);
        match gases.get_gas_id_by_name(&name) {
            Some(id) => Ok(id),
            None => Err(rlua::Error::RuntimeError("Gas type not found".to_owned())),
        }
    })?;

    Ok(())
}
