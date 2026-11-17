use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool exposed to agents (chat tools, sandbox tools, MCP tools).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments object.
    pub input_schema: Value,
}
