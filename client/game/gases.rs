use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::shared::gases::{init_gases_mod_interface, Gases};
use crate::shared::mod_manager::ModManager;

/// Client-side gas registry.
///
/// The client does not simulate gas flow (that happens on the server), but it
/// still runs the base game mod's Lua, which declares gas types via
/// `terralistic_register_gas_type` / `terralistic_get_gas_id_by_name`. Those
/// globals must be registered here too, or Lua init would error out and the
/// client would fail to join the world.
pub struct ClientGases {
    gases: Arc<Mutex<Gases>>,
}

impl ClientGases {
    #[must_use]
    pub fn new() -> Self {
        Self {
            gases: Arc::new(Mutex::new(Gases::new())),
        }
    }

    pub fn init(&self, mods: &mut ModManager) -> Result<()> {
        init_gases_mod_interface(mods, &self.gases)
    }
}

impl Default for ClientGases {
    fn default() -> Self {
        Self::new()
    }
}
