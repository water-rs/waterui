//! What the CPU painter refuses, and that each refusal names the view or
//! capability and the reason, instead of failing somewhere incidental.
//!
//! A view that needs a GPU device is unsupported on the CPU painter for
//! good. A build with both painters refuses it when the CPU target mounts
//! it; a build without the `gpu` feature compiles no realization for it and
//! refuses it at measure and node build. A filtered view runs on the CPU
//! raster unless the raster cannot run it, which its refusal names.

use waterui::ViewExt as _;
use waterui::graphics::Color;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::backend::{AnyEffect, FilteredView, OutputSize, ParamGuards};
use waterui_graphics::filter_view::filters;
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui_graphics::{
    ExternalFrameSource, ExternalFrameView, FrameOutput, GpuContent, GpuContentView,
};

use super::{MinimalTestTheme, pumped_test_environment};
use crate::{HeadlessRuntime, OffscreenPainter};

/// Mounts `view` on the CPU painter, whichever painters the build has, and
/// renders one frame.
fn render_on_cpu(view: impl Fn() -> AnyView + 'static, theme: MinimalTestTheme) {
    let painter = OffscreenPainter::open_cpu()
        .unwrap_or_else(|error| panic!("the CPU painter must open: {error}"));
    let mut runtime = HeadlessRuntime::new_for_tests_on_context(
        painter,
        pumped_test_environment(),
        AnyViewBuilder::<AnyView>::new(view),
        64,
        64,
        theme,
    );
    let _ = runtime.pump_snapshot();
}

/// GPU content that draws nothing: only its kind is under test.
struct NoContent;

impl GpuContent for NoContent {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {}

    fn render(&mut self, _frame: &mut GpuFrame<'_>) {}
}

/// An external frame source that publishes nothing.
struct NoFrames;

impl ExternalFrameSource for NoFrames {
    fn start(&mut self, _output: FrameOutput) {}
}

#[test]
#[cfg_attr(
    all(debug_assertions, hydrolysis_gpu),
    should_panic(
        expected = "hydrolysis: GpuContentView needs a GPU device, but this runtime renders on the \
                    Cherenkov CPU backend"
    )
)]
#[cfg_attr(
    all(debug_assertions, not(hydrolysis_gpu)),
    should_panic(
        expected = "hydrolysis: GpuContentView needs a GPU device, but this Hydrolysis build has \
                    no GPU painter"
    )
)]
fn gpu_content_is_refused_by_name() {
    render_on_cpu(
        || AnyView::new(GpuContentView::new(NoContent)),
        MinimalTestTheme::default(),
    );
}

#[test]
#[cfg_attr(
    all(debug_assertions, hydrolysis_gpu),
    should_panic(
        expected = "hydrolysis: ExternalFrameView needs a GPU device, but this runtime renders on \
                    the Cherenkov CPU backend"
    )
)]
#[cfg_attr(
    all(debug_assertions, not(hydrolysis_gpu)),
    should_panic(
        expected = "hydrolysis: ExternalFrameView needs a GPU device, but this Hydrolysis build \
                    has no GPU painter"
    )
)]
fn an_external_frame_is_refused_by_name() {
    render_on_cpu(
        || AnyView::new(ExternalFrameView::new(NoFrames)),
        MinimalTestTheme::default(),
    );
}

#[test]
#[should_panic(
    expected = "hydrolysis: a filtered view cannot run on the Cherenkov CPU backend: the view \
                declares an output size, which the CPU raster does not resize to"
)]
fn a_filter_output_size_is_refused_with_its_reason() {
    render_on_cpu(
        || {
            AnyView::new(
                FilteredView {
                    content: AnyView::new(
                        ().size(32.0, 32.0).background(Color::srgb_hex("#36343B")),
                    ),
                    effect: AnyEffect::filter(filters::Blur(2.0f32)),
                    guards: ParamGuards::default(),
                }
                .output_size(OutputSize::Scale(0.5)),
            )
        },
        MinimalTestTheme::default(),
    );
}
