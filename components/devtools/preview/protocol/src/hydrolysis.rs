//! Hydrolysis MCP session protocol.
//!
//! The CLI drives the generated Hydrolysis MCP binary with a single JSON
//! run configuration (passed as a file path through
//! [`MCP_RUN_CONFIG_ENV`]). The preview run configuration lives in
//! [`crate::run`], shared with every backend's generated preview binary.

use serde::{Deserialize, Serialize};

/// Environment variable carrying the path of the JSON-encoded
/// [`McpRunConfig`] for the generated MCP binary.
pub const MCP_RUN_CONFIG_ENV: &str = "WATERUI_HYDROLYSIS_MCP_RUN_CONFIG";

/// One headless MCP session of a generated Hydrolysis binary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpRunConfig {
    /// Viewport width in logical units.
    pub width: u32,
    /// Viewport height in logical units.
    pub height: u32,
    /// Display scale factor applied to the runtime.
    pub scale_factor: f64,
}

#[cfg(test)]
mod tests {
    use super::McpRunConfig;

    #[test]
    fn mcp_run_config_round_trips() {
        let config = McpRunConfig {
            width: 390,
            height: 844,
            scale_factor: 2.0,
        };
        let json = serde_json::to_string(&config).expect("config serializes");
        let parsed: McpRunConfig = serde_json::from_str(&json).expect("config deserializes");
        assert_eq!(parsed, config);
    }
}
