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
#[non_exhaustive]
pub struct McpRunConfig {
    /// Viewport width in logical units.
    width: u32,
    /// Viewport height in logical units.
    height: u32,
    /// Display scale factor applied to the runtime.
    scale_factor: f64,
}

impl McpRunConfig {
    /// Viewport width in logical units.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Viewport height in logical units.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Display scale factor applied to the runtime.
    #[must_use]
    pub const fn scale_factor(&self) -> f64 {
        self.scale_factor
    }
}

impl McpRunConfig {
    /// A session of a `width` by `height` logical-unit viewport at
    /// `scale_factor`.
    #[must_use]
    pub const fn new(width: u32, height: u32, scale_factor: f64) -> Self {
        Self {
            width,
            height,
            scale_factor,
        }
    }
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
