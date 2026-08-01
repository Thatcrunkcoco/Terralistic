use anyhow::{bail, Result};
use serde_derive::{Deserialize, Serialize};

use crate::shared::blocks::Blocks;

/// Struct that contains all the information about a tool
pub struct Tool {
    pub name: String,
    pub id: ToolId,
}

impl Tool {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            name: String::new(),
            id: ToolId::new(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolId {
    pub(super) id: i32,
}

impl ToolId {
    #[must_use]
    pub const fn new() -> Self {
        Self { id: -1 }
    }
}

impl Blocks {
    /// Adds a new tool type to the world, validating that the name is non-empty
    /// and unique among registered tools.
    pub fn register_new_tool_type(&mut self, mut tool: Tool) -> Result<ToolId> {
        if tool.name.is_empty() {
            bail!("Cannot register a tool with an empty name");
        }
        if self.tool_types.iter().any(|t| t.name == tool.name) {
            bail!("A tool named \"{}\" already exists", tool.name);
        }
        let id = self.tool_types.len() as i32;
        tool.id = ToolId { id };
        self.tool_types.push(tool);
        Ok(ToolId { id })
    }

    /// Returns the tool type that has the specified name
    #[must_use]
    pub fn get_tool_id_by_name(&self, name: &String) -> Option<ToolId> {
        for tool_type in &self.tool_types {
            if tool_type.name == *name {
                return Some(tool_type.id);
            }
        }
        None
    }

    /// Returns the reference to the Tool with the specified id.
    #[must_use]
    pub fn get_tool_by_id(&self, id: ToolId) -> Option<&Tool> {
        self.tool_types.iter().find(|&tool_type| tool_type.id == id)
    }
}
