//! Text field looks (water-rs/waterui#4042): a field whose theme hands out a
//! [`TextFieldLook`] records its container once, and a hover or a focus
//! change afterwards moves only live operands. Measurement and drawing read
//! the one look, a secure field wears the same look, and the spec carries the
//! layout direction.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use cherenkov_record::ShapeData;
use kurbo::Rect;
use nami::Binding;
use waterui_controls::TextField;
use waterui_core::Str;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::interaction::{InteractionState, StateValue};
use waterui_core::layout::LayoutDirection;
use waterui_core::{AnyView, Environment};
use waterui_form::secure::{Secure, secure};
use waterui_layout::stack::vstack;

use super::chrome_primitives::Frames;
use super::{MinimalTestTheme, invisible_chrome, pumped_test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerKind};
use crate::renderer::chrome::LiveChrome;
use crate::renderer::mount::layers::{LayerVisitor, visit};
use crate::renderer::recording::Recording;
use crate::testing::plain_text_field_look;
use crate::theme::{Border, Driven, InputFieldMetrics, TextFieldLook, TextFieldSpec};

const POINTER: u64 = 1;
const WINDOW: u32 = 320;
const METRICS: InputFieldMetrics = InputFieldMetrics {
    label_height: 14.0,
    min_width: 150.0,
    min_height: 40.0,
    horizontal_inset: 8.0,
    vertical_inset: 6.0,
};

/// Each run that draws glyphs: the world transform of its first glyph run and
/// how far its glyphs span along the run.
#[derive(Default)]
struct GlyphRuns(Vec<(kurbo::Affine, f64)>);

impl<E: crate::text::TextEngine> LayerVisitor<E> for GlyphRuns {
    fn run(&mut self, world: kurbo::Affine, recording: &Recording) {
        if let Some((transform, glyphs)) = recording.glyph_runs().next() {
            let first = glyphs.first().map_or(0.0, |glyph| f64::from(glyph.x));
            let last = glyphs.last().map_or(0.0, |glyph| f64::from(glyph.x));
            self.0.push((world * transform, last - first));
        }
    }
}

/// Each run that draws glyphs: its glyph runs' live paints and the colours
/// of their fixed brushes, in mount order.
#[derive(Default)]
struct GlyphColours(Vec<RunColours>);

/// One run's glyph-run live paints and fixed brush colours.
type RunColours = (Vec<Option<cherenkov_record::Paint>>, Vec<[u8; 4]>);

impl<E: crate::text::TextEngine> LayerVisitor<E> for GlyphColours {
    fn run(&mut self, _world: kurbo::Affine, recording: &Recording) {
        if recording.glyph_runs().next().is_some() {
            self.0
                .push((recording.glyph_paints(), recording.glyph_brush_colours()));
        }
    }
}

/// The plain look with an active indicator that thickens on focus.
fn look() -> TextFieldLook {
    let indicator = StateValue::new(Border::new(
        1.0,
        cherenkov_record::WorkingColor::new([0.0; 4]),
    ))
    .when(
        InteractionState::FOCUSED,
        Border::new(
            2.0,
            cherenkov_record::WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
        ),
    );
    plain_text_field_look(invisible_chrome(), METRICS).active_indicator(Driven::from(indicator))
}

struct Fixture {
    runtime: HeadlessRuntime,
    frames: Frames,
    specs: Rc<RefCell<Vec<TextFieldSpec>>>,
}

impl Fixture {
    fn new(view: AnyView, direction: LayoutDirection) -> Self {
        Self::mount(view, direction, look(), 4)
    }

    /// Mounts `view` under a theme whose fields wear `look`, then pumps
    /// `pumps` frames.
    fn mount(view: AnyView, direction: LayoutDirection, look: TextFieldLook, pumps: usize) -> Self {
        Self::themed(view, direction, move |_, _| look.clone(), pumps)
    }

    /// Mounts `view` under a theme that hands out the look `look` makes for
    /// each spec, then pumps `pumps` frames.
    fn themed(
        view: AnyView,
        direction: LayoutDirection,
        look: impl Fn(&TextFieldSpec, &Environment) -> TextFieldLook + 'static,
        pumps: usize,
    ) -> Self {
        let theme = MinimalTestTheme::default().with_text_field(look);
        let specs = Rc::clone(&theme.text_field_specs);
        let view = RefCell::new(Some(view));
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            view.borrow_mut()
                .take()
                .expect("the fixture view is built once")
        });
        let mut env = pumped_test_environment();
        env.insert(direction);
        let mut runtime = HeadlessRuntime::new_for_tests(env, builder, WINDOW, WINDOW, theme);
        let mut frames = Frames {
            start: Instant::now(),
            next: 0,
        };
        for _ in 0..pumps {
            frames.pump(&mut runtime);
        }
        Self {
            runtime,
            frames,
            specs,
        }
    }

    fn text(direction: LayoutDirection) -> Self {
        let value = Binding::container(Str::default());
        Self::new(
            AnyView::new(vstack((TextField::new("Name", &value),))),
            direction,
        )
    }

    /// A labelled field holding `content`, pumped `pumps` frames.
    fn filled(content: &str, look: TextFieldLook, pumps: usize) -> Self {
        let value = Binding::container(Str::from(content.to_owned()));
        Self::mount(
            AnyView::new(vstack((TextField::new("Name", &value),))),
            LayoutDirection::LeftToRight,
            look,
            pumps,
        )
    }

    /// The glyph runs the mount draws, each with its first glyph run's
    /// world transform and the pixel width the run's glyphs span.
    fn glyph_runs(&self) -> Vec<(kurbo::Affine, f64)> {
        let mut runs = GlyphRuns::default();
        visit(&self.runtime.renderer().mount_roots(), &mut runs);
        runs.0
    }

    fn secure() -> Self {
        let value = Binding::container(Secure::new(String::new()));
        Self::new(
            AnyView::new(vstack((secure("Password", &value),))),
            LayoutDirection::LeftToRight,
        )
    }

    /// The field's live chrome.
    fn chrome(&self) -> Rc<LiveChrome> {
        let chromes = self.runtime.renderer().hit_test.interaction.live_chromes();
        <[_; 1]>::try_from(chromes).expect("a field binds one live chrome")[0].clone()
    }

    fn rect(&self) -> Rect {
        match self.chrome().shape() {
            ShapeData::RoundedRect(shape) => shape.rect(),
            other => {
                panic!("the plain container's silhouette is a rounded rectangle, got {other:?}")
            }
        }
    }

    fn input(&mut self, event: InputEvent) {
        self.runtime.push_input_event(event);
    }

    fn pointer(
        &mut self,
        at: kurbo::Point,
        event: impl FnOnce(u64, PointerKind, f32, f32) -> InputEvent,
    ) {
        let (x, y) = (
            crate::num_cast::f64_as_f32(at.x),
            crate::num_cast::f64_as_f32(at.y),
        );
        self.input(event(POINTER, PointerKind::Mouse, x, y));
    }

    /// Moves keyboard focus onto the field's text input, as a key does.
    fn focus_field(&mut self) {
        let renderer = self.runtime.renderer_mut();
        let key = renderer.text_editing.text_input_targets[0]
            .interaction_key
            .clone();
        assert!(renderer.set_keyboard_focus(Some(key), true));
    }

    fn settle(&mut self) {
        for _ in 0..48 {
            self.frames.pump(&mut self.runtime);
        }
    }
}

#[test]
fn hover_and_focus_record_nothing() {
    let mut fixture = Fixture::text(LayoutDirection::LeftToRight);
    let chrome = fixture.chrome();
    let recordings = chrome.recordings();
    assert!(recordings > 0, "the mount records the container");
    let center = fixture.rect().center();

    fixture.pointer(center, |id, kind, x, y| InputEvent::PointerMove {
        id,
        kind,
        x,
        y,
    });
    assert_eq!(
        fixture.frames.work(&mut fixture.runtime, 8),
        [0, 0, 0],
        "hovering the field records nothing and walks no tree"
    );

    fixture.focus_field();
    fixture.settle();
    assert!(
        chrome.state().contains(InteractionState::FOCUSED),
        "the field took keyboard focus"
    );
    assert!(
        Rc::ptr_eq(&chrome, &fixture.chrome()),
        "focusing keeps the mounted chrome"
    );
    assert_eq!(
        chrome.recordings(),
        recordings,
        "the focus change, the active indicator and the label floating record no chrome"
    );
}

#[test]
fn text_and_secure_fields_lay_out_by_the_look() {
    let rects = [
        Fixture::text(LayoutDirection::LeftToRight).rect(),
        Fixture::secure().rect(),
    ];
    for rect in rects {
        assert!(
            rect.height() >= METRICS.min_height - 1e-6,
            "the container is at least as tall as the look's metrics ask: {rect:?}"
        );
        assert!(
            rect.width() >= METRICS.min_width - 1e-6,
            "the container is at least as wide as the look's metrics ask: {rect:?}"
        );
    }
    assert_eq!(
        rects[0].size(),
        rects[1].size(),
        "a secure field measures as a text field does: they share the look"
    );
}

#[test]
fn the_spec_carries_the_layout_direction() {
    for direction in [LayoutDirection::LeftToRight, LayoutDirection::RightToLeft] {
        let fixture = Fixture::text(direction);
        let specs = fixture.specs.borrow();
        assert!(!specs.is_empty(), "the field asked for its look");
        assert!(
            specs.iter().all(|spec| spec.direction() == direction),
            "{direction:?}: {specs:?}"
        );
    }
}

#[test]
fn a_field_requests_its_look_once_per_spec() {
    let mut fixture = Fixture::text(LayoutDirection::LeftToRight);
    let asked = fixture.specs.borrow().len();
    fixture.settle();
    assert_eq!(
        fixture.specs.borrow().len(),
        asked,
        "steady frames keep the look the field holds"
    );
}

/// The scale a run's world transform draws at.
fn scale_of(world: kurbo::Affine) -> f64 {
    world.as_coeffs()[0]
}

/// A field that holds text when it mounts floats its label on the first
/// frame, with no focus change: the label draws at the look's floating scale
/// and the value at full size.
#[test]
fn a_field_mounted_with_text_has_its_label_floated() {
    let fixture = Fixture::filled("Hello", look(), 1);
    let scales: Vec<f64> = fixture
        .glyph_runs()
        .into_iter()
        .map(|(world, _)| scale_of(world))
        .collect();
    let floated = look().floating_label().get_scale();
    assert!(
        scales.iter().any(|scale| (scale - floated).abs() < 1e-6),
        "a label floats at {floated}; the runs draw at {scales:?}"
    );
}

/// The plain look, its floating label in a font of `size` points when given.
fn sized_look(size: Option<f32>) -> TextFieldLook {
    use crate::theme::{FloatingLabel, Label as LookLabel};
    use waterui_text::font::Font;

    let plain = look();
    let Some(size) = size else {
        return plain;
    };
    let floating = plain.floating_label();
    let styled = FloatingLabel::new(
        LookLabel::new().font(Font::default().size(size)),
        floating.get_float().clone(),
    )
    .scale(floating.get_scale())
    .top(floating.get_top());
    TextFieldLook::new(
        invisible_chrome(),
        METRICS,
        styled,
        plain.caret().clone(),
        plain.selection().clone(),
    )
}

/// How far the one run an empty field draws, its label, spans, and the scale
/// it draws at.
fn label_of(fixture: &Fixture) -> (f64, f64) {
    let [(world, span)] =
        <[_; 1]>::try_from(fixture.glyph_runs()).expect("an empty field draws only its label");
    (scale_of(world), span)
}

/// The label draws with the look's font, at rest and floated: a larger font
/// spans more than the look's default one, and floating scales it by the
/// look's scale.
#[test]
fn the_label_draws_with_the_looks_font() {
    let (_, plain_span) = label_of(&Fixture::filled("", sized_look(None), 4));
    let mut fixture = Fixture::filled("", sized_look(Some(40.0)), 4);
    let (rest_scale, rest_span) = label_of(&fixture);
    assert!(
        rest_span > plain_span * 1.5,
        "the label rests in the look's font: {rest_span} against {plain_span}"
    );
    assert!((rest_scale - 1.0).abs() < 1e-6);

    fixture.focus_field();
    fixture.settle();
    let (floated_scale, floated_span) = label_of(&fixture);
    assert!(
        (floated_scale - 0.75).abs() < 1e-6,
        "the label floats at the look's scale"
    );
    assert!(
        (floated_span - rest_span).abs() < 1e-3,
        "the floated label keeps the look's font: {floated_span} against {rest_span}"
    );
}

/// The band a labelled field reserves is the look's `label_height` whatever
/// the label's font (water-rs/waterui#2387): the label's own measure only
/// decides that the band exists, so a label font larger than the metrics
/// assume leaves the field its height on the first settled frame, with the
/// label drawn at that font inside it.
#[test]
fn the_label_font_leaves_the_reserved_band() {
    let plain = Fixture::filled("", sized_look(None), 2);
    let large = Fixture::filled("", sized_look(Some(40.0)), 2);
    assert_eq!(
        large.rect().height(),
        plain.rect().height(),
        "the field keeps the band the look's metrics reserve"
    );
    assert!(label_of(&large).1 > label_of(&plain).1 * 1.5);
}

/// A spec change takes a new look, and the label follows its font: one that
/// names a different font rebuilds the label in it, and one that names none
/// restores the label's own.
///
/// A field's spec changes only when the environment it renders in does, which
/// no mounted view arranges without rebuilding the field, so the field's
/// realization is driven directly.
#[test]
fn a_spec_change_follows_the_looks_label_font() {
    use crate::renderer::RetainedSubview;
    use crate::theme::WidgetTheme;
    use crate::widgets::controls::field_look::FieldChrome;
    use waterui::Color;
    use waterui::theme::Tint;
    use waterui_controls::label::IntoLabel as _;

    let size = Rc::new(std::cell::Cell::new(Some(40.0_f32)));
    let theme: Rc<dyn WidgetTheme> = Rc::new({
        let size = Rc::clone(&size);
        MinimalTestTheme::default().with_text_field(move |_, _| sized_look(size.get()))
    });
    let mut fixture = Fixture::filled("", sized_look(None), 1);
    let renderer = fixture.runtime.renderer_mut();
    let env_with = |tint: Color| {
        let mut env = crate::renderer::tests::test_environment();
        env.insert(Tint::new(tint));
        env
    };
    let label = "Name".into_label();
    let mut field = FieldChrome::new();
    let mut view = RetainedSubview::new(field.label_view(label.clone()));
    let width = |view: &mut RetainedSubview<_>, renderer: &mut _, env: &Environment| {
        f64::from(view.measure_intrinsic(renderer, env).width)
    };
    let own = width(&mut view, renderer, &env_with(Color::srgb(0, 0, 0)));

    let red = env_with(Color::srgb(200, 0, 0));
    field.look(&theme, &red);
    field.apply_label_font(&mut view, &label, renderer, &red);
    let large = width(&mut view, renderer, &red);
    assert!(
        large > own * 1.5,
        "the look's font reaches the label: {large} against {own}"
    );

    size.set(Some(20.0));
    let blue = env_with(Color::srgb(0, 0, 200));
    field.look(&theme, &blue);
    field.apply_label_font(&mut view, &label, renderer, &blue);
    let smaller = width(&mut view, renderer, &blue);
    assert!(
        smaller < large * 0.75 && smaller > own,
        "the new look's font reaches the label: {smaller} against {large} and {own}"
    );

    size.set(None);
    let green = env_with(Color::srgb(0, 200, 0));
    field.look(&theme, &green);
    field.apply_label_font(&mut view, &label, renderer, &green);
    let restored = width(&mut view, renderer, &green);
    assert!(
        (restored - own).abs() < 1e-3,
        "a look with no font restores the label's own: {restored} against {own}"
    );
}

/// A field disabled when it mounts draws disabled from its first frame
/// (water-rs/waterui#4553): its container and active indicator realize the
/// disabled state, and its label takes the look's disabled colour.
#[test]
fn a_field_mounted_disabled_draws_disabled_from_its_first_frame() {
    use crate::theme::{FloatingLabel, Label as LookLabel};
    use waterui::{Color, ViewExt as _};

    // Red at rest, green while disabled.
    let colour = StateValue::new(Color::srgb(200, 0, 0))
        .when(InteractionState::DISABLED, Color::srgb(0, 200, 0));
    let plain = look();
    let floating = plain.floating_label();
    let label = FloatingLabel::new(LookLabel::new().color(colour), floating.get_float().clone())
        .scale(floating.get_scale())
        .top(floating.get_top());
    let look = TextFieldLook::new(
        invisible_chrome(),
        METRICS,
        label,
        plain.caret().clone(),
        plain.selection().clone(),
    )
    .active_indicator(
        plain
            .get_active_indicator()
            .expect("the plain look has an active indicator")
            .clone(),
    );
    let value = Binding::container(Str::default());
    let fixture = Fixture::mount(
        AnyView::new(vstack((TextField::new("Name", &value).disabled(true),))),
        LayoutDirection::LeftToRight,
        look,
        1,
    );

    assert!(
        fixture
            .chrome()
            .state()
            .contains(InteractionState::DISABLED),
        "the container and its active indicator realize the disabled state"
    );
    let mut runs = GlyphColours::default();
    visit(&fixture.runtime.renderer().mount_roots(), &mut runs);
    let [(paints, brushes)] =
        <[_; 1]>::try_from(runs.0).expect("an empty field draws only its label");
    assert!(
        paints.iter().all(Option::is_none),
        "the label draws in its recorded brush: {paints:?}"
    );
    assert_eq!(
        brushes,
        [[0, 200, 0, 255]],
        "the label draws in the look's disabled green, not its resting red"
    );
}

/// The world-space extent along x of the glyphs of the one run a mount draws.
#[derive(Default)]
struct RunExtent(Vec<(f64, f64)>);

impl<E: crate::text::TextEngine> LayerVisitor<E> for RunExtent {
    fn run(&mut self, world: kurbo::Affine, recording: &Recording) {
        if let Some((transform, glyphs)) = recording.glyph_runs().next() {
            let at = |glyph: &crate::renderer::recording::Glyph| {
                (world * transform * kurbo::Point::new(f64::from(glyph.x), 0.0)).x
            };
            if let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) {
                self.0.push((at(first), at(last)));
            }
        }
    }
}

/// A label view rests and floats at the field's leading inset, mirrored in a
/// right-to-left layout, as the prompt standing in for a label does, rather
/// than centred in a box the field's width.
#[test]
fn a_label_view_sits_at_the_leading_inset() {
    // One glyph's advance bounds how far a run's last glyph origin lies from
    // the run's trailing edge.
    const GLYPH: f64 = 16.0;
    for direction in [LayoutDirection::LeftToRight, LayoutDirection::RightToLeft] {
        let value = Binding::container(Str::default());
        let mut fixture = Fixture::new(
            AnyView::new(vstack((TextField::new("Name", &value),))),
            direction,
        );
        for floated in [false, true] {
            if floated {
                fixture.focus_field();
                fixture.settle();
            }
            let field = fixture.rect();
            let mut extent = RunExtent::default();
            visit(&fixture.runtime.renderer().mount_roots(), &mut extent);
            let [(first, last)] =
                <[_; 1]>::try_from(extent.0).expect("an empty field draws only its label");
            let inset = METRICS.horizontal_inset;
            match direction {
                LayoutDirection::LeftToRight => assert!(
                    (first - (field.x0 + inset)).abs() < 3.0,
                    "{direction:?} floated={floated}: the label starts at the leading inset \
                     {} but its first glyph is at {first}",
                    field.x0 + inset
                ),
                LayoutDirection::RightToLeft => assert!(
                    (field.x1 - inset) - last < GLYPH && last < field.x1 - inset,
                    "{direction:?} floated={floated}: the label ends at the leading inset \
                     {} but its last glyph is at {last}",
                    field.x1 - inset
                ),
            }
        }
    }
}

/// The plain look, its value styled by `value`.
fn valued_look(value: crate::theme::Label) -> TextFieldLook {
    look().value(value)
}

/// The live paint of every glyph run that draws in one, in mount order: the
/// field's value, for a look that styles its colour.
fn value_paints(fixture: &Fixture) -> Vec<cherenkov_record::Paint> {
    let mut runs = GlyphColours::default();
    visit(&fixture.runtime.renderer().mount_roots(), &mut runs);
    runs.0
        .into_iter()
        .flat_map(|(paints, _)| paints.into_iter().flatten())
        .collect()
}

/// `color` as the paint the value draws in.
fn paint_of(color: waterui::Color) -> cherenkov_record::Paint {
    cherenkov_record::Paint::from(nami::Signal::snapshot(
        &color.resolve(&pumped_test_environment()),
    ))
}

/// Red at rest, green disabled and blue focused.
fn value_colours() -> StateValue<waterui::Color> {
    StateValue::new(waterui::Color::srgb(200, 0, 0))
        .when(InteractionState::DISABLED, waterui::Color::srgb(0, 200, 0))
        .when(InteractionState::FOCUSED, waterui::Color::srgb(0, 0, 200))
}

/// A field mounted disabled draws its value in the look's disabled colour
/// from its first frame (water-rs/waterui#4570), and a secure field's
/// masked text wears the same colour.
#[test]
fn a_field_mounted_disabled_draws_its_value_disabled_from_its_first_frame() {
    use crate::theme::Label as LookLabel;
    use waterui::ViewExt as _;

    let look = valued_look(LookLabel::new().color(value_colours()));
    let value = Binding::container(Str::from("hello"));
    let fixture = Fixture::mount(
        AnyView::new(vstack((TextField::new("Name", &value).disabled(true),))),
        LayoutDirection::LeftToRight,
        look.clone(),
        1,
    );
    assert_eq!(
        value_paints(&fixture),
        [paint_of(waterui::Color::srgb(0, 200, 0))],
        "the value draws in the look's disabled green, not its resting red"
    );

    let secret = Binding::container(Secure::new("hunter2".to_owned()));
    let fixture = Fixture::mount(
        AnyView::new(vstack((secure("Password", &secret).disabled(true),))),
        LayoutDirection::LeftToRight,
        look,
        1,
    );
    assert_eq!(
        value_paints(&fixture),
        [paint_of(waterui::Color::srgb(0, 200, 0))],
        "the masked text draws in the look's disabled green"
    );
}

/// The value follows the field's state: focusing the field and disabling it
/// each recolour the value, through the paint the field reports its state to.
#[test]
fn the_value_colour_follows_the_fields_state() {
    use crate::theme::Label as LookLabel;
    use waterui::ViewExt as _;

    let look = valued_look(LookLabel::new().color(value_colours()));
    let value = Binding::container(Str::from("hello"));
    let disabled = Binding::container(false);
    let mut fixture = Fixture::mount(
        AnyView::new(vstack((
            TextField::new("Name", &value).disabled(disabled.clone()),
        ))),
        LayoutDirection::LeftToRight,
        look,
        4,
    );
    assert_eq!(
        value_paints(&fixture),
        [paint_of(waterui::Color::srgb(200, 0, 0))],
        "the value rests in red"
    );

    fixture.focus_field();
    fixture.settle();
    assert_eq!(
        value_paints(&fixture),
        [paint_of(waterui::Color::srgb(0, 0, 200))],
        "the focus recolours the value live"
    );
    disabled.set(true);
    fixture.settle();
    assert_eq!(
        value_paints(&fixture),
        [paint_of(waterui::Color::srgb(0, 200, 0))],
        "disabling the field recolours the value"
    );
}

/// The value lays out in the look's font: a larger one widens the run it
/// draws and the field's measured height grows to hold it.
#[test]
fn the_value_lays_out_with_the_looks_font() {
    use crate::theme::Label as LookLabel;
    use waterui_text::font::Font;

    let widest = |fixture: &Fixture| {
        fixture
            .glyph_runs()
            .into_iter()
            .map(|(_, span)| span)
            .fold(0.0, f64::max)
    };
    let plain = Fixture::filled("hello", look(), 4);
    let large = Fixture::filled(
        "hello",
        valued_look(LookLabel::new().font(Font::default().size(40.0))),
        4,
    );
    assert!(
        widest(&large) > widest(&plain) * 1.5,
        "the value draws in the look's font: {} against {}",
        widest(&large),
        widest(&plain)
    );
    assert!(
        large.rect().height() > plain.rect().height(),
        "the field measures the value in the look's font: {} against {}",
        large.rect().height(),
        plain.rect().height()
    );
}
