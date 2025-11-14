use std::sync::OnceLock;

use color_eyre::eyre::Result;
use serde::Serialize;

/// Supported output formats for CLI commands.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

impl OutputFormat {
    /// Check if the format expects JSON payloads.
    #[must_use]
    pub const fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

static GLOBAL_OUTPUT_FORMAT: OnceLock<OutputFormat> = OnceLock::new();

/// Store the desired global output format used across CLI commands.
pub fn set_global_output_format(format: OutputFormat) {
    let _ = GLOBAL_OUTPUT_FORMAT.set(format);
}

/// Access the configured global output format, defaulting to human-friendly logs.
#[must_use]
pub fn global_output_format() -> OutputFormat {
    *GLOBAL_OUTPUT_FORMAT.get().unwrap_or(&OutputFormat::Human)
}

/// Emit a JSON payload to stdout.
///
/// # Errors
/// Returns an error if the payload cannot be serialized.
pub fn emit_json<T>(payload: &T) -> Result<()>
where
    T: Serialize,
{
    println!("{}", serde_json::to_string(payload)?);
    Ok(())
}
