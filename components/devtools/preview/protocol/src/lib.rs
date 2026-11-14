//! Shared contracts the `water` CLI and generated `WaterUI` preview binaries
//! speak: the `WATERUI_PREVIEW_RUN_CONFIG` run configuration every backend's
//! generated binary reads ([`run`]), the Hydrolysis MCP session
//! configuration ([`hydrolysis`]) and the benchmark report wire format
//! ([`bench`]).

pub mod bench;
pub mod hydrolysis;
pub mod run;
