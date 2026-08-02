/// A registered gas *type*.
///
/// A `GasType` describes *what* a gas is — its name and physical properties —
/// not where it is. Concrete, per-tile gas state lives elsewhere (see the
/// future gas-layer / flow-simulation work); this type is purely configuration
/// registered once at world/mod load.
///
/// This is intentionally designed to scale to large numbers of gases (ONI-style
/// games can have dozens or more distinct atmospheres). Identity in the hot loop
/// is a dense integer `GasId` (an index into the registry), so per-cell storage
/// and lookups stay cheap no matter how many distinct gas types exist. Gas types
/// themselves are stored in a contiguous `Vec`.
#[derive(Clone)]
pub struct GasType {
    /// Unique, non-empty name used to look the gas up from Lua and config.
    pub name: String,
    /// Relative density of the gas. Heavier gases (CO₂) sink; lighter gases
    /// (H₂, O₂) rise. This drives how gases layer and settle during flow.
    /// Density is currently a required registration field so the property is
    /// always present; its semantics are exercised when flow simulation lands.
    pub density: f32,
    /// Dense registry index. Assigned automatically at registration; `-1`
    /// indicates an unregistered placeholder.
    pub(super) id: i32,
}

impl GasType {
    /// Creates a gas type. The id is a placeholder; it is assigned when the
    /// type is registered with a registry.
    #[must_use]
    pub const fn new(name: String, density: f32) -> Self {
        Self { name, density, id: -1 }
    }

    /// Returns this gas's dense registry id.
    #[must_use]
    pub const fn get_id(&self) -> GasId {
        GasId { id: self.id }
    }
}

/// Identifies a gas *type* in the world's registry.
///
/// A `GasId` is a dense integer index into the gas-type registry. It is meant
/// to be the compact, Copy, cheap-to-store handle used in per-cell gas data and
/// in the hot simulation loop — never a string or a heap-allocated reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct GasId {
    pub(super) id: i32,
}

impl GasId {
    /// The reserved id used for an *empty* / vacuum cell (pressure zero, no gas).
    /// This is distinct from the `-1` unregistered placeholder: `-1` means "no
    /// valid id yet" while `NONE` is a real, meaningful state meaning "no gas".
    pub const NONE: GasId = GasId { id: -2 };

    /// Constructs a gas id from a raw registry index. Used by systems that
    /// reconstruct gas references from resolved/serialized data.
    #[must_use]
    pub const fn from_raw(id: i32) -> Self {
        Self { id }
    }

    /// The raw underlying registry index.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.id
    }

    /// Whether this id represents an empty / vacuum cell.
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.id == Self::NONE.id
    }
}
