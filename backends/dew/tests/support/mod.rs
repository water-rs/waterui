use waterui::Plugin as _;
use waterui::theme::{FontSettings, Theme};
use waterui_core::Environment;
use waterui_text::font::{FontWeight, ResolvedFont};

pub fn test_environment() -> Environment {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    waterui_testing::install_test_executor();

    let mut environment = Environment::new();
    Theme::new()
        .fonts(
            FontSettings::new()
                .body(ResolvedFont::new(16.0, FontWeight::Normal))
                .title(ResolvedFont::new(24.0, FontWeight::Normal))
                .headline(ResolvedFont::new(22.0, FontWeight::Normal))
                .subheadline(ResolvedFont::new(20.0, FontWeight::Normal))
                .caption(ResolvedFont::new(12.0, FontWeight::Normal))
                .footnote(ResolvedFont::new(11.0, FontWeight::Normal)),
        )
        .install(&mut environment);
    environment
}

// Only the performance simulation uses this; other integration tests include
// `support` for `test_environment` alone.
#[allow(dead_code, reason = "each integration test binary uses its own subset")]
pub mod simulation;
