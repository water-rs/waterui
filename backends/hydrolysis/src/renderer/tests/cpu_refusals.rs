//! What the CPU painter refuses, and that each refusal names the view or
//! capability and the reason, instead of failing somewhere incidental.
//!
//! A view that needs a GPU device is unsupported on the CPU painter for
//! good. A build with both painters refuses it when the CPU target mounts
//! it; a build without the `gpu` feature compiles no realization for it and
//! refuses it at measure and node build. Shader effects (filtered views) and
//! a theme's backdrop shaders are refused until the CPU backend realizes
//! them (water-rs/waterui#4065), which retires those two tests.

use cherenkov_record::{BackdropShaderSource, MaterialShader};
use waterui::graphics::Color;
use waterui::{FilterViewExt as _, ViewExt as _};
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui_graphics::{
    ExternalFrameSource, ExternalFrameView, FrameOutput, GpuContent, GpuContentView,
};

use super::{ChromePlan, MinimalTestTheme, pumped_test_environment};
use crate::{HeadlessRuntime, OffscreenPainter};

/// Mounts `view` on the CPU painter, whichever painters the build has, and
/// renders one frame.
fn render_on_cpu(view: impl Fn() -> AnyView + 'static, theme: MinimalTestTheme) {
    let mut runtime = HeadlessRuntime::new_for_tests_on_context(
        OffscreenPainter::Cpu(crate::engine::RasterEngineState::new()),
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
    hydrolysis_gpu,
    should_panic(
        expected = "hydrolysis: GpuContentView needs a GPU device, but this runtime renders on the \
                    Cherenkov CPU backend"
    )
)]
#[cfg_attr(
    not(hydrolysis_gpu),
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
    hydrolysis_gpu,
    should_panic(
        expected = "hydrolysis: ExternalFrameView needs a GPU device, but this runtime renders on \
                    the Cherenkov CPU backend"
    )
)]
#[cfg_attr(
    not(hydrolysis_gpu),
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
    expected = "a filtered view runs a shader effect, which the Cherenkov CPU backend does not \
                realize yet (water-rs/waterui#4065)"
)]
fn a_filtered_view_is_refused_until_cpu_shader_effects() {
    render_on_cpu(
        || {
            AnyView::new(
                ().size(32.0, 32.0)
                    .background(Color::srgb_hex("#36343B"))
                    .blur(2.0f32),
            )
        },
        MinimalTestTheme::default(),
    );
}

#[test]
#[should_panic(
    expected = "the theme registered the backdrop shaders MaterialShader(0), but the Cherenkov \
                CPU backend does not realize backdrop shaders yet (water-rs/waterui#4065)"
)]
fn a_theme_with_backdrop_shaders_is_refused_until_cpu_backdrop_shaders() {
    let chrome = ChromePlan {
        shaders: vec![(
            MaterialShader::new(0),
            BackdropShaderSource::wgsl(
                "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> { return backdrop_sample(px.p); }\n",
            ),
        )],
        ..ChromePlan::default()
    };
    render_on_cpu(
        || AnyView::new(Color::srgb_hex("#36343B")),
        MinimalTestTheme {
            chrome,
            ..MinimalTestTheme::default()
        },
    );
}
