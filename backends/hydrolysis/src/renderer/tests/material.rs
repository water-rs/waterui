//! A within-window `Material` background through the whole frame path: the
//! tree builds a material wrapper, its node's frame layer commits under the
//! view's ancestry, and `LayerTarget::mount_material` makes the frame a member of a backdrop group the engine captures at a
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

use super::{MinimalTestTheme, pumped_test_environment, test_environment, test_renderer};
use crate::HeadlessRuntime;
use std::rc::Rc;

use crate::renderer::material::MaterialRuntime;
use crate::renderer::mount::layers::{LayerVisitor, NodeLayers, visit};
use crate::renderer::mount::target::GpuMaterial;

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

/// The frame's only material frame: the opacities it is presented under,
/// its runtime, and the display scale its backdrop group was built for.
fn material_frame(runtime: &HeadlessRuntime) -> (Vec<f32>, Rc<MaterialRuntime>, Option<f64>) {
    struct Frames(Vec<(Vec<f32>, Rc<MaterialRuntime>, Option<f64>)>);
    impl LayerVisitor for Frames {
        fn node(&mut self, layers: &NodeLayers, _world: kurbo::Affine, alphas: &[f32]) {
            if let Some(runtime) = layers.material_runtime() {
                self.0.push((
                    alphas.to_vec(),
                    Rc::clone(runtime),
                    layers
                        .material::<GpuMaterial>()
                        .map(GpuMaterial::display_scale),
                ));
            }
        }
    }
    let mut frames = Frames(Vec::new());
    visit(&runtime.renderer().mount_roots(), &mut frames);
    assert_eq!(frames.0.len(), 1, "the frame presents one material layer");
    frames.0.remove(0)
}

/// The opacity scopes the frame's material layer is presented under.
fn ancestry_alphas(runtime: &HeadlessRuntime) -> Vec<f32> {
    material_frame(runtime).0
}

/// What the install left behind: the display scale the material mount's
/// backdrop group was built for, and the engine's backdrop capture bytes
/// and format.
fn installed(runtime: &HeadlessRuntime) -> (Option<f64>, u64, Option<&'static str>) {
    let scale = material_frame(runtime).2;
    let window = runtime
        .renderer()
        .cherenkov_window
        .as_ref()
        .expect("the frame installed into a window");
    let memory = window.state.engine.memory();
    (
        scale,
        memory.backdrop_captures.0,
        memory.backdrop_capture_format,
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
    // two passes, all in encoded sRGB.
    let chain = material_frame(&runtime).1.chain(DISPLAY_SCALE);
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

/// The material frame's backdrop membership as the mirror target records it:
/// a member at the mount's display scale while shown, none under a fully
/// transparent ancestry.
#[test]
fn a_material_frame_is_a_backdrop_member_only_while_visible() {
    let members = |opacity: f32| {
        let mut renderer = test_renderer();
        let window = kurbo::Rect::new(0.0, 0.0, f64::from(WIDTH_PT), f64::from(HEIGHT_PT));
        renderer.begin_rebuild_frame();
        renderer.capture_window_tree(
            AnyView::new(
                ().size(WIDTH_PT, HEIGHT_PT)
                    .background(Material::Regular)
                    .opacity(opacity),
            ),
            &test_environment(),
            window,
            kurbo::Affine::IDENTITY,
            kurbo::Affine::IDENTITY,
        );
        renderer.finish_rebuild_frame();
        renderer.commit_mirror();
        renderer
            .mirror()
            .backdrops()
            .into_iter()
            .map(|(_, scale)| scale)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        members(1.0),
        [1.0],
        "a shown material joins one backdrop group"
    );
    assert!(
        members(0.0).is_empty(),
        "a hidden material holds no backdrop group"
    );
}
