//! End-to-end accessibility-semantics tests for the `qr` component.
//!
//! A QR code reaches the screen as an anonymous field of squares, and unlike a
//! decorative drawing it is content: the whole point of the picture is a string
//! that only a camera can get at. The node the leaf publishes is where anyone
//! who cannot point a camera at it reads that string, so these tests pin what
//! it carries.

use core::time::Duration;

use nami::Binding;
use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::text::text;
use waterui_qr::qr_code;
use waterui_str::Str;
use waterui_testing::{Role, SemanticApp, UiBuilder};

const PAYLOAD: &str = "https://waterui.dev";
const OTHER_PAYLOAD: &str = "https://waterui.dev/docs/qr";

fn unlabelled_code() -> impl waterui::View {
    qr_code(PAYLOAD)
}

fn labelled_code() -> impl waterui::View {
    qr_code(PAYLOAD).a11y_label("Ticket")
}

/// A code in a stack, which proposes its own width to every child.
fn code_in_a_stack() -> impl waterui::View {
    vstack((qr_code(PAYLOAD), text("Scan to open")))
}

/// The code reaches the tree as an image node carrying what it encodes.
#[waterui::test(unlabelled_code)]
fn an_unnamed_code_publishes_its_payload(app: &mut SemanticApp) {
    let node = app.query().role(Role::IMAGE).label(PAYLOAD).single();

    let bounds = node.bounds();
    assert!(
        bounds.width() > 0.0 && bounds.height() > 0.0,
        "the code's node must occupy the box it draws into, got {}x{}",
        bounds.width(),
        bounds.height()
    );
}

/// What the application named the code wins: it knows what the code is for, and
/// the payload is only the announcement of last resort.
#[waterui::test(labelled_code)]
fn an_application_label_wins_over_the_payload(app: &mut SemanticApp) {
    app.query()
        .role(Role::IMAGE)
        .label("Ticket")
        .assert_exists();

    assert!(
        !app.query().role(Role::IMAGE).label(PAYLOAD).exists(),
        "a code the application named must not also be announced as its raw payload"
    );
}

/// A code stays square wherever it is placed.
///
/// A stack names its children's cross axis, and scene content given one axis
/// derives the other from its natural size — so what has to hold here is the
/// ratio, not the extent: a QR code stretched into a rectangle is a code no
/// decoder will look at twice, whatever size it is.
#[waterui::test(code_in_a_stack)]
fn a_code_placed_in_a_stack_stays_square(app: &mut SemanticApp) {
    let bounds = app
        .query()
        .role(Role::IMAGE)
        .label(PAYLOAD)
        .single()
        .bounds();

    assert!(
        (bounds.width() - bounds.height()).abs() < 1.0,
        "the code must keep the square aspect of its module grid, got {}x{}",
        bounds.width(),
        bounds.height()
    );
    assert!(
        bounds.width() > 0.0,
        "the code must occupy the box it draws into"
    );
}

/// The payload follows the signal, so a code bound to state does not announce
/// what it encoded when its subtree was built.
#[waterui::test]
fn the_published_payload_follows_the_signal(ui: UiBuilder) {
    let payload = Binding::container(Str::from_static(PAYLOAD));
    let shown = payload.clone();
    let mut app = ui.mount(move || qr_code(shown.clone()));

    app.query().role(Role::IMAGE).label(PAYLOAD).assert_exists();

    payload.set(Str::from_static(OTHER_PAYLOAD));

    assert!(
        app.query()
            .role(Role::IMAGE)
            .label(OTHER_PAYLOAD)
            .wait_for_existence(Duration::from_secs(2)),
        "the published payload must follow the code's signal"
    );
}
