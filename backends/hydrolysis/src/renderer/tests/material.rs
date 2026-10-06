//! A within-window `Material` background through the whole frame path: the
//! tree builds a material wrapper, its flush presents a
//! [`RenderLayer::Material`] under the view's ancestry, and the install pass
//! makes the mount a member of a backdrop group the engine captures at a
//! quarter of device resolution and runs through the level's colour stage
//! and then its blur. Under a fully transparent ancestry the mount holds no
//! group, so the engine captures nothing for it.

use waterui::ViewExt as _;
use waterui::background::Material;
use waterui::graphics::Color;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::filtrate::{
    ColorStage, Filter as _, OperatingSpace, Placed, SpatialStage, StageCollector,
};
use waterui_layout::stack::zstack;

use super::{MinimalTestTheme, capture_bytes, material_layers, mounts, pumped_test_environment};
use crate::HeadlessRuntime;
use crate::renderer::MaterialLayer;

/// The window, in points; the material fills it.
const WIDTH: u32 = 160;
const HEIGHT: u32 = 120;
const WIDTH_PT: f32 = 160.0;
const HEIGHT_PT: f32 = 120.0;
/// Device pixels per point.
const DISPLAY_SCALE: f64 = 2.0;

/// The stages of a filter chain in the order they run, with the space each
/// operates in.
#[derive(Default)]
struct Stages(Vec<(&'static str, OperatingSpace)>);

impl StageCollector for Stages {
    fn color(&mut self, stage: Placed<ColorStage>) {
        self.0.push((stage.stage.name, stage.stage.space));
    }
    fn spatial(&mut self, stage: Placed<SpatialStage>) {
        self.0.push((stage.stage.name, stage.stage.space));
    }
}

/// A window filled by a `Regular` material over red, under `opacity`,
/// after one rendered frame.
fn rendered(opacity: f32) -> HeadlessRuntime {
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(zstack((
            Color::srgb(230, 38, 38),
            ().size(WIDTH_PT, HEIGHT_PT)
                .background(Material::Regular)
                .opacity(opacity),
        )))
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WIDTH,
        HEIGHT,
        MinimalTestTheme::default(),
    )
    .with_scale_factor(DISPLAY_SCALE);
    let _ = runtime.pump_snapshot();
    runtime
}

/// The frame's only material layer.
fn material_layer(runtime: &HeadlessRuntime) -> &MaterialLayer {
    let layers = material_layers(runtime);
    assert_eq!(layers.len(), 1, "the frame presents one material layer");
    layers[0]
}

/// The opacity scopes the frame's material layer is presented under.
fn ancestry_alphas(runtime: &HeadlessRuntime) -> Vec<f32> {
    material_layer(runtime)
        .active_layers
        .iter()
        .map(|scope| scope.alpha)
        .collect()
}

/// What the install left behind: the display scale the material mount's
/// backdrop group was built for, and the engine's backdrop capture bytes
/// and format.
fn installed(runtime: &HeadlessRuntime) -> (Option<f64>, u64, Option<&'static str>) {
    let key = material_layer(runtime).key;
    let format = runtime
        .renderer()
        .cherenkov_windows
        .values()
        .next()
        .expect("the frame installed into a window")
        .state
        .engine
        .memory()
        .backdrop_capture_format;
    (
        mounts(runtime).backdrop_display_scale(key),
        capture_bytes(runtime),
        format,
    )
}

#[test]
fn a_material_installs_a_quarter_scale_colour_then_blur_backdrop_group() {
    let runtime = rendered(0.5);

    // Flush: the material presents under its opacity scope.
    assert_eq!(
        ancestry_alphas(&runtime),
        [0.5],
        "the material is shown under its opacity scope"
    );

    // The chain the install builds runs the colour stage, then the blur's
    // two passes, all in encoded sRGB — read off the group's own runtime.
    let key = material_layer(&runtime).key;
    let chain = mounts(&runtime)
        .backdrop_chain(key)
        .expect("the member holds a backdrop group");
    let mut stages = Stages::default();
    chain.collect_stages(&mut stages);
    assert_eq!(
        stages.0,
        [
            ("LumaCurve", OperatingSpace::Srgb),
            ("gaussian_blur_horizontal_srgb", OperatingSpace::Srgb),
            ("gaussian_blur_vertical_srgb", OperatingSpace::Srgb),
        ]
    );
    // 29.5 pt at 2 px/pt, captured at a quarter: 14.75 texels.
    assert!((chain.second.sigma - 14.75).abs() <= f32::EPSILON);

    // Install: the mount holds a group built for this display scale, and the
    // engine captures it at a quarter of the 320×240 device pixels: 80×60
    // half-float texels.
    let (scale, bytes, format) = installed(&runtime);
    assert_eq!(
        scale,
        Some(DISPLAY_SCALE),
        "the mount holds a backdrop group"
    );
    assert_eq!(format, Some("rgba16float"));
    assert_eq!(bytes, 80 * 60 * 8, "the capture is a quarter of the window");
}

#[test]
fn a_material_under_a_transparent_ancestry_captures_nothing() {
    let runtime = rendered(0.0);
    assert_eq!(
        ancestry_alphas(&runtime),
        [0.0],
        "the material is presented, fully transparent"
    );
    let (scale, bytes, _) = installed(&runtime);
    assert_eq!(scale, None, "a hidden material holds no backdrop group");
    assert_eq!(bytes, 0, "a hidden material costs no capture");
}
