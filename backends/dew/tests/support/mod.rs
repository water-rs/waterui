use waterui::Plugin as _;
use waterui::theme::{FontSettings, Theme};
use waterui_backend_core::frame_signals::FrameSignals;
use waterui_backend_core::time::Instant;
use waterui_core::Environment;
use waterui_dew::{DewRenderer, FontSources};
use waterui_text::font::{FontWeight, ResolvedFont};

use std::path::PathBuf;

use waterui_testing::TestArtifacts;

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

/// Where a test writes the PNG it exports for visual review.
///
/// Everything lands in `waterui-testing`'s canonical artifact layout
/// (`<root>/dew/<case>/<stage>.png`, with the root taken from
/// `WATERUI_TEST_ARTIFACTS_DIR` when CI sets it and the platform temp directory
/// otherwise), so the images ride along with the uploaded CI artifacts and the
/// test report can list them — on every host, not only where `/tmp` exists.
#[allow(dead_code, reason = "each integration test binary uses its own subset")]
pub fn export_path(case: &str, stage: &str) -> PathBuf {
    let path = TestArtifacts::new("dew").snapshot_path(case, stage);
    std::fs::create_dir_all(path.parent().expect("a snapshot path has a case directory"))
        .expect("the dew export directory must be creatable");
    path
}

/// Where a test writes a non-image report, beside its images in the same case
/// directory.
#[allow(dead_code, reason = "each integration test binary uses its own subset")]
pub fn report_path(case: &str, file_name: &str) -> PathBuf {
    let directory = TestArtifacts::new("dew").case_dir(case);
    std::fs::create_dir_all(&directory).expect("the dew report directory must be creatable");
    directory.join(file_name)
}
