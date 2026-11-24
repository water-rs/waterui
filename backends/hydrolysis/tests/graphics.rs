//! Rendered-flush twin of `waterui-graphics`'s accessibility coverage: the
//! graphics leaves' nodes carry real bounds on the rendered runtime, and
//! decorative leaves stay out of the published tree until the application
//! names them. The semantic legs — role, label, naming precedence — live in
//! the component crate's `e2e_semantics.rs`.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation).

mod support;

use hydrolysis::draw::{Draw, Paint, WorkingColor};
use kurbo::{Rect, Shape as _};
use support::{mount_rendered, node_bounds_with_role, nodes};
use waterui::Str;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::component::text;
use waterui::graphics::color::Color;
use waterui::layout::{Size, UnitPoint};
use waterui::reactive::constant;
use waterui::shape::{Circle, ShapeExt as _};
use waterui_core::Environment;
use waterui_graphics::backend::Picture;
use waterui_graphics::{FlowingGradient, Gradient, ShaderPaintView};

fn linear_gradient_view() -> impl waterui::View {
    Gradient::linear(
        [Color::srgb_hex("#0F172A"), Color::srgb_hex("#38BDF8")],
        UnitPoint::TOP_LEADING,
        UnitPoint::BOTTOM_TRAILING,
    )
    .size(180.0, 120.0)
    .a11y_role(AccessibilityRole::Image)
    .a11y_label("Linear gradient")
}

fn animated_mesh_gradient_view() -> impl waterui::View {
    FlowingGradient::new()
        .size(180.0, 120.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Animated mesh gradient")
}

fn shader_surface_view() -> impl waterui::View {
    ShaderPaintView::new(include_str!("fixtures/two_tone.wgsl"))
        .size(180.0, 120.0)
        .a11y_role(AccessibilityRole::Image)
        .a11y_label("Shader surface")
}

/// A drawing that names itself, the way an SVG with a `<title>` does.
fn labeled_picture_view() -> impl waterui::View {
    let recording = Picture::record(|recorder| {
        recorder.fill(
            Rect::new(0.0, 0.0, 24.0, 24.0).to_path(0.1),
            Paint::Solid(WorkingColor::new([0.0, 0.0, 0.0, 1.0])),
        );
    });
    Picture::new(Size::new(24.0, 24.0), constant(recording))
        .offered_label(Str::from("Warning sign"))
}

/// The same drawing, named by the application instead.
fn renamed_picture_view() -> impl waterui::View {
    labeled_picture_view().a11y_label("Severe weather")
}

#[track_caller]
fn assert_image_node<V: waterui::View>(
    view: impl Fn() -> V + Clone + 'static,
    case: &str,
    label: &str,
) {
    let mut app = mount_rendered(Environment::new(), view, 320, 240);
    app.settle();
    let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Image, label);
    assert!(bounds.width() > 0.0, "{case}: width must be positive");
    assert!(bounds.height() > 0.0, "{case}: height must be positive");
}

#[test]
fn linear_gradient_exposes_accessibility_image() {
    assert_image_node(
        linear_gradient_view,
        "linear-gradient-exposes-accessibility-image",
        "Linear gradient",
    );
}

#[test]
fn animated_mesh_gradient_exposes_accessibility_image() {
    let mut app = mount_rendered(Environment::new(), animated_mesh_gradient_view, 320, 240);
    for _ in 0..30 {
        app.pump();
        if app.runtime_mut().is_settled_except_perpetual() {
            break;
        }
    }
    assert!(
        !app.runtime_mut().is_settled() && app.runtime_mut().is_settled_except_perpetual(),
        "animated-mesh-gradient-exposes-accessibility-image: FlowingGradient is perpetual work"
    );
    let bounds =
        node_bounds_with_role(app.tree(), accesskit::Role::Image, "Animated mesh gradient");
    assert!(
        bounds.width() > 0.0,
        "animated-mesh-gradient: width must be positive"
    );
    assert!(
        bounds.height() > 0.0,
        "animated-mesh-gradient: height must be positive"
    );
}

#[test]
fn shader_surface_exposes_accessibility_image() {
    assert_image_node(
        shader_surface_view,
        "shader-surface-exposes-accessibility-image",
        "Shader surface",
    );
}

#[test]
fn a_picture_offers_its_own_name() {
    assert_image_node(
        labeled_picture_view,
        "a-picture-offers-its-own-name",
        "Warning sign",
    );
}

#[test]
fn the_application_label_wins_over_the_pictures_own() {
    let mut app = mount_rendered(Environment::new(), renamed_picture_view, 320, 240);
    app.settle();
    let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Image, "Severe weather");
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "the renamed picture's node carries bounds"
    );
    assert!(
        !support::nodes(app.tree()).any(|(_, node)| node.label() == Some("Warning sign")),
        "the picture's own name must not reach the tree once the application named it"
    );
}

/// Decorative graphics leaves must stay out of the accessibility tree until
/// the application names them (water-rs/hydrolysis#148).
fn decorative_background_view() -> impl waterui::View {
    text("Content").background(Gradient::linear(
        [Color::srgb_hex("#0F172A"), Color::srgb_hex("#38BDF8")],
        UnitPoint::TOP_LEADING,
        UnitPoint::BOTTOM_TRAILING,
    ))
}

#[test]
fn an_unlabelled_background_fill_emits_no_accessibility_node() {
    let mut app = mount_rendered(Environment::new(), decorative_background_view, 320, 240);
    app.settle();
    assert!(
        !nodes(app.tree()).any(|(_, node)| node.role() == accesskit::Role::Image),
        "a decorative background fill must not emit an unnamed Image node"
    );
}

/// water-rs/hydrolysis#148.
fn decorative_shape_view() -> impl waterui::View {
    Circle
        .fill(waterui::Color::srgb_hex("#3B82F6"))
        .size(24.0, 24.0)
}

#[test]
fn an_unlabelled_shape_fill_emits_no_accessibility_node() {
    let mut app = mount_rendered(Environment::new(), decorative_shape_view, 320, 240);
    app.settle();
    assert!(
        !nodes(app.tree()).any(|(_, node)| node.role() == accesskit::Role::Image),
        "a decorative shape fill must not emit an unnamed Image node"
    );
}

/// water-rs/hydrolysis#148.
fn labeled_shape_view() -> impl waterui::View {
    Circle
        .fill(waterui::Color::srgb_hex("#3B82F6"))
        .size(24.0, 24.0)
        .a11y_label("Avatar")
}

#[test]
fn a_labelled_shape_emits_a_named_image_node() {
    assert_image_node(
        labeled_shape_view,
        "a-labelled-shape-emits-a-named-image-node",
        "Avatar",
    );
}

/// water-rs/hydrolysis#148.
fn image_role_shape_view() -> impl waterui::View {
    Circle
        .fill(waterui::Color::srgb_hex("#3B82F6"))
        .size(24.0, 24.0)
        .a11y_role(AccessibilityRole::Image)
}

#[test]
fn an_explicit_image_role_keeps_the_shape_in_the_tree() {
    let mut app = mount_rendered(Environment::new(), image_role_shape_view, 320, 240);
    app.settle();
    assert!(
        nodes(app.tree()).any(|(_, node)| node.role() == accesskit::Role::Image),
        "an explicit image role keeps the leaf in the tree"
    );
}

const fn pixel_at(snapshot: &hydrolysis::HeadlessSnapshot, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * snapshot.width() + x) * 4) as usize;
    let rgba = snapshot.rgba8();
    [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
}

#[test]
fn theme_token_gradient_recolours_in_place_on_scheme_change() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use waterui::Binding;
    use waterui::SignalExt as _;
    use waterui::theme::color::{Accent, Surface};
    use waterui::theme::{ColorDerivation, ColorScheme, Theme, install_color_derivation};

    let scheme = Binding::container(ColorScheme::Light);
    let mut env = Environment::new();
    env.install(Theme::new().color_scheme(scheme.computed()));

    let style = hydrolysis::testing::TestStyle::with_token_installer(|env| {
        install_color_derivation::<Accent>(
            env,
            ColorDerivation::new(|mode| match mode {
                ColorScheme::Light => WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
                ColorScheme::Dark => WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            }),
        );
        install_color_derivation::<Surface>(
            env,
            ColorDerivation::new(|mode| match mode {
                ColorScheme::Light => WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
                ColorScheme::Dark => WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            }),
        );
    });

    let builds = Arc::new(AtomicUsize::new(0));
    let builds_in_view = Arc::clone(&builds);
    let mut app = support::mount_rendered_with(
        env,
        move || {
            builds_in_view.fetch_add(1, Ordering::Relaxed);
            Gradient::linear(
                [Accent.into(), Surface.into()],
                UnitPoint::TOP,
                UnitPoint::BOTTOM,
            )
            .size(64.0, 64.0)
        },
        64,
        64,
        style,
    );
    app.settle();
    let light_px = pixel_at(app.snapshot(), 32, 32);
    assert_eq!(builds.load(Ordering::Relaxed), 1);
    assert!(
        light_px[0] > 200 && light_px[2] < 50,
        "expected red in light mode, got {light_px:?}"
    );

    scheme.set(ColorScheme::Dark);
    app.settle();
    let dark_px = pixel_at(app.snapshot(), 32, 32);
    assert_eq!(
        builds.load(Ordering::Relaxed),
        1,
        "scheme change must update the gradient in place without rebuilding the view"
    );
    assert!(
        dark_px[2] > 200 && dark_px[0] < 50,
        "expected blue in dark mode, got {dark_px:?}"
    );
}

#[test]
fn animated_gradient_stop_updates_in_place_without_rebuild() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use waterui::Binding;
    use waterui::SignalExt as _;
    use waterui::graphics::color::Working;

    let stop_pos = Binding::container(0.1_f32);
    let red = Color::new(Working(WorkingColor::new([1.0, 0.0, 0.0, 1.0])));
    let blue = Color::new(Working(WorkingColor::new([0.0, 0.0, 1.0, 1.0])));
    let stops = stop_pos.map(move |p| vec![(0.0, red.clone()), (p, blue.clone())]);

    let builds = Arc::new(AtomicUsize::new(0));
    let builds_in_view = Arc::clone(&builds);
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            builds_in_view.fetch_add(1, Ordering::Relaxed);
            Gradient::linear(stops.clone(), UnitPoint::LEADING, UnitPoint::TRAILING)
                .size(64.0, 64.0)
        },
        64,
        64,
    );
    app.settle();
    // At x = 0.5 with blue starting at p = 0.1, pad extension fills the center with solid blue.
    let before_px = pixel_at(app.snapshot(), 32, 32);
    assert_eq!(builds.load(Ordering::Relaxed), 1);
    assert!(
        before_px[2] > 200 && before_px[0] < 50,
        "expected blue at center when second stop is at 0.1, got {before_px:?}"
    );

    stop_pos.set(0.9);
    app.settle();
    // Moving the second stop to 0.9 puts the center (0.5) halfway between red and blue.
    let after_px = pixel_at(app.snapshot(), 32, 32);
    assert_eq!(
        builds.load(Ordering::Relaxed),
        1,
        "stop signal change must update the gradient in place without rebuilding the view"
    );
    assert!(
        after_px[0] > 100,
        "expected red contribution at center when second stop moves to 0.9, got {after_px:?}"
    );
}
