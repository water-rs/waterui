use clap::ValueEnum;
use color_eyre::eyre::Result;
use serde::Serialize;
use std::sync::OnceLock;

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[clap(rename_all = "kebab-case")]
#[derive(Default)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}


impl OutputFormat {
    pub fn is_json(self) -> bool {
        matches!(self, OutputFormat::Json)
    }
}

static GLOBAL_OUTPUT_FORMAT: OnceLock<OutputFormat> = OnceLock::new();

pub fn set_global_output_format(format: OutputFormat) {
    let _ = GLOBAL_OUTPUT_FORMAT.set(format);
}

pub fn global_output_format() -> OutputFormat {
    *GLOBAL_OUTPUT_FORMAT.get().unwrap_or(&OutputFormat::Human)
}

pub fn emit_json<T>(payload: &T) -> Result<()>
where
    T: Serialize,
{
    println!("{}", serde_json::to_string(payload)?);
    Ok(())
}
