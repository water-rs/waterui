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
use waterui_math::{latex, mathml};
use waterui_str::Str;
use waterui_testing::{Role, SemanticApp, UiBuilder};

const FRACTION: &str = r"\frac{a}{b}";
const ROOT: &str = r"\sqrt{x}";

/// The markup the converter produces for `source`.
///
/// Computed through the same public converter the view publishes from rather
/// than pasted as a literal, so the expectation tracks the converter instead of
/// rotting into a stale string the next `MathML` change silently invalidates.
fn markup(source: &str, style: MathStyle) -> String {
    let item = latex::parse(source).unwrap_or_else(|error| panic!("`{source}`: {error}"));
    mathml::to_mathml(&item, style)
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

/// The formula reaches the tree as an image node carrying its `MathML`.
#[waterui::test(unlabelled_formula)]
fn an_unnamed_formula_publishes_its_mathml(app: &mut SemanticApp) {
    let node = app
        .query()
        .role(Role::IMAGE)
        .label(markup(FRACTION, MathStyle::Text))
        .single();

    let bounds = node.bounds();
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "the formula's node must occupy the box it draws into, got {}x{}",
        bounds.width(),
        bounds.height()
    );
}

/// The style the formula is set in reaches the markup, because a formula
/// announced as set on its own line is not the same announcement as an inline
/// one.
#[waterui::test(display_formula)]
fn display_style_reaches_the_published_markup(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label(markup(FRACTION, MathStyle::Display))
        .assert_exists();
}

/// What the application named the formula wins: it knows what the formula is
/// for, and the markup is only the payload of last resort.
#[waterui::test(labelled_formula)]
fn an_application_label_wins_over_the_markup(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Ratio of a to b")
        .assert_exists();

    assert!(
        !app.query()
            .role(Role::IMAGE)
            .label(markup(FRACTION, MathStyle::Text))
            .exists(),
        "a formula the application named must not also be announced as raw markup"
    );
}

/// The markup follows the source signal, so a formula bound to state does not
/// freeze at the markup it had when its subtree was built.
#[waterui::test]
fn the_published_markup_follows_the_source_signal(ui: UiBuilder) {
    let source = Binding::container(Str::from_static(FRACTION));
    let mounted = source.clone();
    let mut app = ui.mount(move || Math::new(mounted.clone()));

    app.query()
        .role(Role::IMAGE)
        .label(markup(FRACTION, MathStyle::Text))
        .assert_exists();

    source.set(Str::from_static(ROOT));

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(markup(ROOT, MathStyle::Text))
            .wait_for_existence(Duration::from_secs(2)),
        "the published markup must follow the formula's signal"
    );
}
