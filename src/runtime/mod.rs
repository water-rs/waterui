pub mod app;
mod entry;
pub mod error;
pub mod fullscreen;
// Inspection is a TCP endpoint that also launches the inspector application on
// the developer's machine; a browser page can do neither, so on wasm the
// endpoint is absent rather than present and permanently unable to work.
#[cfg(all(feature = "inspector", not(target_arch = "wasm32")))]
pub mod inspector;
pub mod metadata;
pub mod realization;
#[cfg(feature = "snackbar")]
pub mod snackbar;
pub mod task;
pub mod window;

pub use entry::entry;
