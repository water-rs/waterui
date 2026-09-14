//! Build-time metadata embedded into the `water` CLI binary.

/// Exact Android Kotlin compiler version required by the embedded Android backend/runtime.
pub const ANDROID_KOTLIN_VERSION: &str = env!("WATERUI_CLI_ANDROID_KOTLIN_VERSION");

/// Git repository reference embedded into the CLI binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendReference {
    /// Git remote URL for the backend repository.
    pub repository_url: &'static str,
    /// Git ref the scaffold pins.
    pub revision: &'static str,
}

/// Embedded TUI backend repository reference.
///
/// The experimental TUI backend is not a workspace submodule, so its pin is a
/// source literal rather than a build-script value — bump `revision` when the
/// CLI starts depending on a newer `waterui-tui` API.
pub const TUI_BACKEND: BackendReference = BackendReference {
    repository_url: "https://github.com/water-rs/tui",
    revision: "4782df8a39626a9f24fd27b909d884924e2bce95",
};
