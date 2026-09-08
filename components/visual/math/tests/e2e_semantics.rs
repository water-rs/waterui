//! End-to-end accessibility-semantics tests for the `math` component.
//!
//! A formula is drawn as anonymous filled paths and glyph runs, so the node the
//! leaf publishes is the only place a screen reader can learn what the formula
//! says. These tests pin what that node carries.

use core::time::Duration;

use nami::Binding;
use waterui::ViewExt as _;
use waterui_math::ast::MathStyle;
use waterui_math::view::Math;
use waterui_math::{latex, mathml, speech};
use waterui_str::Str;
use waterui_testing::{Role, SemanticApp, UiBuilder};

const FRACTION: &str = r"\frac{a}{b}";
const ROOT: &str = r"\sqrt{x}";
/// The formula the whole feature exists for.
const QUADRATIC: &str = r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}";

/// What `source` sounds like when it is read out.
///
/// Computed through the same public converter and speech engine the view
/// publishes through rather than pasted as a literal, so the expectation tracks
/// them instead of rotting into a stale string the next rules update silently
/// invalidates.
fn spoken(source: &str, style: MathStyle) -> String {
    let item = latex::parse(source).unwrap_or_else(|error| panic!("`{source}`: {error}"));
    speech::speak(&mathml::to_mathml(&item, style))
        .unwrap_or_else(|error| panic!("`{source}`: {error}"))
}

fn unlabelled_formula() -> impl waterui::View {
    Math::new(FRACTION)
}

fn labelled_formula() -> impl waterui::View {
    Math::new(FRACTION).a11y_label("Ratio of a to b")
}

fn display_formula() -> impl waterui::View {
    Math::new(FRACTION).display()
}

fn quadratic_formula() -> impl waterui::View {
    Math::new(QUADRATIC).display()
}

/// The formula reaches the tree as an image node that says the formula out
/// loud.
#[waterui::test(unlabelled_formula)]
fn an_unnamed_formula_is_announced_as_speech(app: &mut SemanticApp) {
    let node = app
        .query()
        .role(Role::IMAGE)
        .label(spoken(FRACTION, MathStyle::Text))
        .single();

    let bounds = node.bounds();
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "the formula's node must occupy the box it draws into, got {}x{}",
        bounds.width(),
        bounds.height()
    );
}

/// The quadratic formula is carried as a sentence a listener can follow, not as
/// the markup that describes it and not as the LaTeX that produced it.
///
/// This is the assertion the whole feature is for, so it checks the substance
/// rather than mere presence: every landmark of the formula — the fraction, the
/// radical, the sign, the exponent — has to survive into what the node says, and
/// neither `MathML` tags nor LaTeX control sequences may appear in it. A node
/// that reverted to publishing markup, that published the source, or that
/// published a generic announcement all fail here.
#[waterui::test(quadratic_formula)]
fn the_quadratic_formula_is_published_as_a_sentence(app: &mut SemanticApp) {
    let said = spoken(QUADRATIC, MathStyle::Display);

    assert!(
        !said.trim().is_empty(),
        "the quadratic formula must be spoken, not left silent"
    );
    assert!(
        !said.contains('<') && !said.contains('\\'),
        "the node carries a sentence, not markup or LaTeX, got `{said}`"
    );

    let lowered = said.to_lowercase();
    for landmark in ["fraction", "square root", "plus or minus", "squared"] {
        assert!(
            lowered.contains(landmark),
            "the quadratic formula must be spoken with `{landmark}`, got `{said}`"
        );
    }

    app.query().role(Role::IMAGE).label(said).assert_exists();
}

/// The style the formula is set in reaches the speech, because a formula
/// announced as set on its own line is not the same announcement as an inline
/// one.
#[waterui::test(display_formula)]
fn display_style_reaches_the_published_speech(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label(spoken(FRACTION, MathStyle::Display))
        .assert_exists();
}

/// What the application named the formula wins: it knows what the formula is
/// for, and the spoken formula is only the announcement of last resort.
#[waterui::test(labelled_formula)]
fn an_application_label_wins_over_the_speech(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Ratio of a to b")
        .assert_exists();

    assert!(
        !app.query()
            .role(Role::IMAGE)
            .label(spoken(FRACTION, MathStyle::Text))
            .exists(),
        "a formula the application named must not also be announced as its own speech"
    );
}

/// The speech follows the source signal, so a formula bound to state does not
/// freeze at the sentence it had when its subtree was built.
#[waterui::test]
fn the_published_speech_follows_the_source_signal(ui: UiBuilder) {
    let source = Binding::container(Str::from_static(FRACTION));
    let mounted = source.clone();
    let mut app = ui.mount(move || Math::new(mounted.clone()));

    app.query()
        .role(Role::IMAGE)
        .label(spoken(FRACTION, MathStyle::Text))
        .assert_exists();

    source.set(Str::from_static(ROOT));

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(spoken(ROOT, MathStyle::Text))
            .wait_for_existence(Duration::from_secs(2)),
        "the published speech must follow the formula's signal"
    );
}
