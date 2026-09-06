use waterui::Plugin as _;
use waterui::theme::{FontSettings, Theme};
use waterui_backend_core::frame_signals::FrameSignals;
use waterui_backend_core::time::Instant;
use waterui_core::Environment;
use waterui_dew::{DewRenderer, FontSources};
use waterui_text::font::{FontWeight, ResolvedFont};

use std::path::PathBuf;

#[allow(dead_code, reason = "each integration test binary uses its own subset")]
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

/// A bare renderer for the tests that inspect a display list directly instead
/// of pumping a board.
///
/// It picks its faces the way `HostBoard` does: the system collection on a
/// desktop build, and the repository's own test faces in the firmware shape,
/// which has no `system-fonts` feature and therefore no `FontSources::System`.
/// `DewRenderer::default()` exists only in the former, so calling it here is
/// what kept these tests from compiling in any other shape.
#[allow(dead_code, reason = "each integration test binary uses its own subset")]
pub fn test_renderer() -> DewRenderer {
    #[cfg(feature = "system-fonts")]
    let fonts = FontSources::System;
    #[cfg(not(feature = "system-fonts"))]
    let fonts = FontSources::bundled(&[
        include_bytes!("../../../../testing/fonts/Roboto-Regular.ttf"),
        include_bytes!("../../../../testing/fonts/Roboto-Bold.ttf"),
    ]);
    DewRenderer::new(FrameSignals::new(Instant::now()), fonts)
}

// Only the performance simulation uses this; other integration tests include
// `support` for `test_environment` alone.
#[allow(dead_code, reason = "each integration test binary uses its own subset")]
pub mod simulation;

/// Where a test writes the PNG (or report) it exports for visual review.
///
/// Everything lands under the artifact root `waterui-testing` shares with CI
/// (`WATERUI_TEST_ARTIFACTS_DIR`, uploaded with every run), in a `dew`
/// subdirectory, and under the platform temp directory when that variable is
/// unset — so the exports work on every host rather than only where `/tmp`
/// exists.
#[allow(dead_code, reason = "each integration test binary uses its own subset")]
pub fn export_path(file_name: &str) -> PathBuf {
    let directory = waterui_testing::artifact_root().join("dew");
    std::fs::create_dir_all(&directory).expect("the dew export directory must be creatable");
    directory.join(file_name)
}
