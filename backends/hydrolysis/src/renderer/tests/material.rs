//! A within-window `Material` background through the whole frame path: the
//! tree builds a material wrapper, its node's frame layer commits under the
//! view's ancestry, and `LayerTarget::mount_material` makes the frame a member of a backdrop group the engine captures at a
//! quarter of device resolution and runs through the level's colour stage
//! and then its blur. Under a fully transparent ancestry the mount holds no
//! group, so the engine captures nothing for it.
//!
//! A window whose background is a material realizes it at the window: a
//! within-window level mounts the root over the same backdrop group, and a
//! behind-window level clears the transparent window to the level's tint
//! under the content.

use nami::Signal as _;
use waterui::ViewExt as _;
use waterui::background::Material;
use waterui::graphics::Color;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::filtrate::{
    ColorStage, Filter as _, OperatingSpace, Placed, SpatialStage, StageCollector,
};
use waterui_layout::stack::{vstack, zstack};

use super::{
    MinimalTestTheme, mounts, pumped_test_environment, test_environment, test_renderer,
    window_mount,
};
use crate::HeadlessRuntime;

use crate::renderer::material::BehindWindowLevel;
use crate::renderer::mount::layers::{LayerVisitor, NodeLayers, visit};
use cherenkov::LayerId;

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

/// The frame's only material member: the opacities it is presented under
/// and the member layer id its backdrop membership hangs on.
fn material_frame(runtime: &HeadlessRuntime) -> (Vec<f32>, LayerId) {
    struct Frames(Vec<(Vec<f32>, LayerId)>);
    impl LayerVisitor for Frames {
        fn node(&mut self, layers: &NodeLayers, _world: kurbo::Affine, alphas: &[f32]) {
            if let Some(member) = layers.material_member() {
                self.0.push((alphas.to_vec(), member));
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

/// The alphas under which the frame's material node presents — the node's
/// own ancestry — whether or not it currently holds a membership (a hidden
/// material holds none; its last lowered request still names it).
fn material_frame_alphas(runtime: &HeadlessRuntime) -> Vec<Vec<f32>> {
    struct Alphas(Vec<Vec<f32>>);
    impl LayerVisitor for Alphas {
        fn node(&mut self, layers: &NodeLayers, _world: kurbo::Affine, alphas: &[f32]) {
            if layers.material_request().is_some() {
                self.0.push(alphas.to_vec());
            }
        }
    }
    let mut found = Alphas(Vec::new());
    visit(&runtime.renderer().mount_roots(), &mut found);
    found.0
}

/// What the install left behind: the display scale `member`'s backdrop
/// group was built for, and the engine's backdrop capture bytes and format.
fn installed(
    runtime: &HeadlessRuntime,
    member: LayerId,
) -> (Option<f64>, u64, Option<&'static str>) {
    let window = runtime
        .renderer()
        .cherenkov_window
        .as_ref()
        .expect("the frame installed into a window");
    let memory = window.state.engine.memory();
    (
        mounts(runtime).backdrop_display_scale(member),
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
    // two passes, all in encoded sRGB — read off the group's own runtime.
    let member = material_frame(&runtime).1;
    let chain = mounts(&runtime)
        .backdrop_chain(member)
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
    let (scale, bytes, format) = installed(&runtime, member);
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
        material_frame_alphas(&runtime),
        [[0.0]],
        "the material is presented, fully transparent"
    );
    assert!(
        mounts(&runtime).member_scales().is_empty(),
        "a hidden material holds no backdrop group"
    );
    assert_eq!(
        super::capture_bytes(&runtime),
        0,
        "a hidden material costs no capture"
    );
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

/// A window of `WIDTH`×`HEIGHT` points with `background`, whose only content
/// is an opaque 40×40 pt red box, after one rendered frame.
fn window_with_background(background: Material) -> HeadlessRuntime {
    let content = AnyViewBuilder::<AnyView>::new(|| {
        AnyView::new(vstack((
            ().size(40.0, 40.0).background(Color::srgb(255, 0, 0)),
        )))
    });
    let window = waterui::window::Window::new(
        "",
        waterui_core::binding(waterui::window::WindowState::Normal),
        move || content.build(),
    )
    .background(background);
    let mut runtime = HeadlessRuntime::new_for_tests_with_window(
        pumped_test_environment(),
        window,
        WIDTH,
        HEIGHT,
        MinimalTestTheme::default(),
    )
    .with_scale_factor(DISPLAY_SCALE);
    let _ = runtime.pump_snapshot();
    runtime
}

#[test]
fn a_within_window_material_window_mounts_its_root_over_the_backdrop_group() {
    let runtime = window_with_background(Material::Regular);

    // The window layer itself is the group's only member: its content
    // mounts over the backdrop.
    assert_eq!(
        mounts(&runtime).member_scales(),
        [(window_mount(&runtime).window().id(), DISPLAY_SCALE)],
        "the window's backdrop is the frame's only material member"
    );

    // The mount holds the level's group, captured at a quarter of the
    // 320×240 device pixels.
    let (scale, bytes, _) = installed(&runtime, window_mount(&runtime).window().id());
    assert_eq!(
        scale,
        Some(DISPLAY_SCALE),
        "the mount holds a backdrop group"
    );
    assert_eq!(bytes, 80 * 60 * 8, "the capture is a quarter of the window");
}

#[test]
fn a_behind_window_material_window_clears_transparent_to_its_tint_under_the_content() {
    let mut runtime = window_with_background(Material::UltraThin);
    // The tint the frame cleared to is the one in the runtime's own colour
    // scheme.
    let scheme = waterui::theme::current_color_scheme(runtime.env()).snapshot();
    let tint = BehindWindowLevel::UltraThin.tint(scheme);
    assert!(
        mounts(&runtime).member_scales().is_empty(),
        "a behind-window material builds no backdrop of the window's content"
    );

    let snapshot = runtime
        .pump_snapshot()
        .snapshot
        .expect("a captured frame must carry pixels");
    // Offscreen targets store straight alpha: every pixel the box leaves
    // uncovered reads as the tint's grey at the tint's coverage, and the box
    // draws opaquely over it.
    let level = |value: f32| (value * 255.0).round();
    let (mut tinted, mut content) = (0_u32, 0_u32);
    for pixel in snapshot.rgba8.as_chunks::<4>().0 {
        let [r, g, b, a] = *pixel;
        if a == 255 {
            assert!(
                r >= 200 && g <= 60 && b <= 60,
                "an opaque pixel is the box's red, got {pixel:?}"
            );
            content += 1;
        } else {
            assert!(
                (f32::from(a) - level(tint.alpha)).abs() <= 1.0
                    && [r, g, b]
                        .iter()
                        .all(|&channel| (f32::from(channel) - level(tint.color)).abs() <= 2.0),
                "an uncovered pixel {pixel:?} is the tint {tint:?}"
            );
            tinted += 1;
        }
    }
    assert!(content > 0, "the content was drawn");
    assert!(tinted > content, "the tint shows around the content");
}
