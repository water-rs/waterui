//! water-rs/waterui#2002 — group opacity: a translucent ancestor's
//! opacity applies once, to the composited result of its whole subtree.
//! The pieces one subtree mounts as — scene segments and keyed mounts —
//! all hang under the ancestor's own layer, so two overlapping pieces of
//! a faded subtree composite opaquely against each other and the alpha
//! reaches only the final blend against what lies beneath the subtree.
//! The defect this pins: each piece carrying the ancestor's alpha of its
//! own, which lets a lower piece show through an upper one.
//!
//! The tests assert against a reference render — one opaque piece under
//! `.opacity(0.5)` over the same field — rather than a hand-computed
//! blend, so the pin holds whatever working space the engine composites
//! in.

use waterui::ViewExt as _;
use waterui::background::Material;
use waterui::gesture::TapGesture;
use waterui::graphics::Color;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment};
use waterui_graphics::draw::{
    BackdropShaderSource, CaptureClass, CaptureLevels, CaptureScale, Live, MaterialCapture,
    MaterialEffect, MaterialGrouping, MaterialShader, Paint,
};
use waterui_layout::stack::zstack;

use super::{ChromeDraw, ChromePlan, MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;
use crate::theme::{ButtonMetrics, InteractionStyle};

/// The window, in points; `SCALE` device pixels per point.
const WIDTH: u32 = 160;
const HEIGHT: u32 = 120;
const SCALE: f64 = 2.0;
/// `SCALE` as a pixel count.
const SCALE_PX: usize = 2;

/// The field beneath the faded subtree: pure green.
const BASE: [u8; 3] = [0, 255, 0];
/// The opaque piece drawn over the keyed mount: the material test red.
const LABEL: [u8; 3] = [230, 38, 38];
/// The gesture body's colour, a third tone distinct from both above.
const BODY: &str = "#3F3F46";

const GLASS_SHADER: MaterialShader = MaterialShader::new(0);
const GLASS: CaptureClass = CaptureClass::new(0);
/// `backdrop_sample + params[0]`: the member's output differs from the
/// backdrop it captures, so a doubled fade cannot hide behind it.
const TINT_WGSL: &str = "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> { return backdrop_sample(px.p) + params[0]; }\n";

/// The rect the chrome member and the segment above it both cover, in
/// points — small enough to stay inside the gesture's bounds.
fn chrome_rect() -> kurbo::RoundedRect {
    kurbo::RoundedRect::from_rect(kurbo::Rect::new(2.0, 3.0, 10.0, 11.0), 4.0)
}

/// Counts the chrome member layers under the mount roots.
struct Members(usize);

impl<E: crate::text::TextEngine> crate::renderer::mount::layers::LayerVisitor<E> for Members {
    fn node(
        &mut self,
        layers: &crate::renderer::mount::layers::NodeLayers<E>,
        _world: kurbo::Affine,
        _alphas: &[f32],
    ) {
        self.0 += layers.chrome_members().len();
    }
}

/// The chrome member layers the runtime's windows mounted.
fn chrome_members(runtime: &HeadlessRuntime) -> usize {
    let mut members = Members(0);
    crate::renderer::mount::layers::visit(&runtime.renderer().mount_roots(), &mut members);
    members.0
}

/// `view` after one rendered frame.
fn pumped(
    view: impl Fn() -> AnyView + 'static,
    env: Environment,
    theme: MinimalTestTheme,
) -> HeadlessRuntime {
    let builder = AnyViewBuilder::<AnyView>::new(view);
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, builder, WIDTH, HEIGHT, theme).with_scale_factor(SCALE);
    let _ = runtime.pump_snapshot();
    runtime
}

/// The pixel at `(x, y)` points of the runtime's next captured frame.
fn pixel(runtime: &mut HeadlessRuntime, x: usize, y: usize) -> [u8; 4] {
    let snapshot = runtime
        .pump_snapshot()
        .into_snapshot()
        .expect("a captured frame carries pixels");
    let (x, y) = (x * SCALE_PX, y * SCALE_PX);
    let offset = (y * snapshot.width() as usize + x) * 4;
    snapshot.rgba8()[offset..offset + 4]
        .try_into()
        .expect("a pixel is four bytes")
}

/// One opaque `colour` under `.opacity(0.5)` over `BASE`, sampled at the
/// window's centre: what one fade of that colour over that field reads.
fn faded(color: [u8; 3]) -> [u8; 4] {
    let mut runtime = pumped(
        move || {
            AnyView::new(zstack((
                Color::srgb(BASE[0], BASE[1], BASE[2]),
                ().size(40.0, 40.0)
                    .background(Color::srgb(color[0], color[1], color[2]))
                    .opacity(0.5),
            )))
        },
        pumped_test_environment(),
        MinimalTestTheme::default(),
    );
    pixel(&mut runtime, 80, 60)
}

/// `got` must be the same fade `want` — a single alpha on the composited
/// subtree — allowing a few quanta for the two draws' sampling positions.
fn assert_same_fade(got: [u8; 4], want: [u8; 4], what: &str) {
    assert_eq!(got[3], 255, "{what}: the pixel is opaque, got {got:?}");
    for channel in 0..3 {
        assert!(
            got[channel].abs_diff(want[channel]) <= 4,
            "{what}: got {got:?}, want {want:?} — the ancestor alpha must apply once to the \
             composited subtree, not once per piece"
        );
    }
}

/// The issue's reproduction: an opaque `Material::Regular` background —
/// a keyed mount — with an opaque label piece over it, both under
/// `.opacity(0.5)` on a green field. The label pixel must read as one
/// fade of the opaque label; a per-piece fade instead lets the halved
/// material show through the halved label.
#[test]
fn a_translucent_material_and_label_fade_together() {
    let mut runtime = pumped(
        || {
            AnyView::new(zstack((
                Color::srgb(BASE[0], BASE[1], BASE[2]),
                zstack((
                    ().size(80.0, 80.0).background(Material::Regular),
                    ().size(40.0, 40.0)
                        .background(Color::srgb(LABEL[0], LABEL[1], LABEL[2])),
                ))
                .opacity(0.5),
            )))
        },
        pumped_test_environment(),
        MinimalTestTheme::default(),
    );

    // The label is centred over the material: (80, 60) is inside both.
    assert_same_fade(
        pixel(&mut runtime, 80, 60),
        faded(LABEL),
        "label over material",
    );
}

/// A `draw_context` program holding a keyed mount between scene
/// segments: the interaction state layer records a backdrop material —
/// a chrome member — and an opaque fill lands in the segment above it.
/// Under `.opacity(0.5)` on a green field the fill pixel must read as
/// one fade of the fill; a per-piece fade lets the tinted member show
/// through it.
#[test]
fn a_translucent_segment_over_a_chrome_member_fades_once() {
    let env = {
        let mut env = pumped_test_environment();
        env.install(InteractionStyle::new(
            ButtonMetrics::new(16.0, 8.0, 0.0, 0.0),
            Color::srgb(0, 0, 0),
            8.0_f64,
        ));
        env
    };
    let theme = MinimalTestTheme {
        chrome: ChromePlan {
            shaders: vec![(GLASS_SHADER, BackdropShaderSource::wgsl(TINT_WGSL))],
            captures: vec![(
                GLASS,
                MaterialCapture {
                    scale: CaptureScale::new(0.5).expect("0.5 is a valid capture scale"),
                    levels: CaptureLevels::ONE,
                    grouping: MaterialGrouping::Solo,
                    blend_space: cherenkov::BlendSpace::Linear,
                },
            )],
            draws: vec![ChromeDraw {
                shape: Live::from(chrome_rect()).into_shared(),
                shader: GLASS_SHADER,
                capture: GLASS,
                // A blue shift on the captured green: the member's output
                // cannot pass for the field beneath it.
                effect: Live::from(MaterialEffect::new(vec![0.0, 0.0, 0.9, 0.0])).into_shared(),
            }],
            // An opaque red fill in the segment above the member, covering
            // the member's whole rect.
            fills: vec![(
                chrome_rect(),
                Paint::Solid(
                    waterui::color::Srgb::new(
                        f32::from(LABEL[0]) / 255.0,
                        f32::from(LABEL[1]) / 255.0,
                        f32::from(LABEL[2]) / 255.0,
                    )
                    .resolve(),
                ),
            )],
            stateful_draws: None,
        },
        ..Default::default()
    };
    let mut runtime = pumped(
        || {
            AnyView::new(zstack((
                Color::srgb(BASE[0], BASE[1], BASE[2]),
                Color::srgb_hex(BODY)
                    .gesture(TapGesture::new(), || {})
                    .opacity(0.5),
            )))
        },
        env,
        theme,
    );

    // The keyed mount this test is about must really be mounted.
    assert_eq!(
        chrome_members(&runtime),
        1,
        "the state layer mounted its chrome member"
    );

    // (6, 7) is inside the member's rect and the fill above it; (40, 40)
    // is the gesture's plain body, under the same ancestor.
    assert_same_fade(
        pixel(&mut runtime, 6, 7),
        faded(LABEL),
        "segment over member",
    );
}
