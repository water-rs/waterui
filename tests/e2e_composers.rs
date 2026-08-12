//! End-to-end semantic coverage for the Rust-side composer views.
//!
//! Card, Badge, Accordion, Divider and the `#[derive(FormBuilder)]` flow are
//! deliberate Rust-side compositions (Framework Design Principle #2): they
//! reuse stacks, padding, and theme tokens and ship no FFI types, so their
//! contract is exactly what the accessibility tree exposes.

use hydrolysis_m3::install as install_m3;
use waterui::Binding;
use waterui::component::badge::Badge;
use waterui::widget::accordion::Accordion;
use waterui::widget::card::Card;
use waterui_testing::{Role, SemanticApp, UiBuilder};

fn titled_card() -> impl waterui::View {
    Card::new(waterui::component::text("Card body"))
        .title("Receipts")
        .subtitle("Last 30 days")
}

#[waterui::test(titled_card, theme = install_m3)]
fn card_exposes_title_subtitle_and_content(app: &mut SemanticApp) {
    app.query()
        .role(Role::LABEL)
        .label("Receipts")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Last 30 days")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Card body")
        .assert_exists();
}

#[waterui::test(theme = install_m3)]
fn badge_value_tracks_its_signal(ui: UiBuilder) {
    let unread = Binding::i32(3);
    let unread_for_view = unread.clone();
    let mut app =
        ui.mount(move || Badge::new(unread_for_view.clone(), waterui::component::text("Inbox")));
    app.query().role(Role::LABEL).label("Inbox").assert_exists();
    app.query().role(Role::LABEL).label("3").assert_exists();

    unread.set(12);
    assert!(
        app.query()
            .role(Role::LABEL)
            .label("12")
            .wait_for_existence(core::time::Duration::from_secs(2)),
        "badge count must re-render when its signal changes"
    );
    app.query().role(Role::LABEL).label("3").assert_not_exists();
}

#[waterui::test(theme = install_m3)]
fn accordion_tap_expands_and_collapses_content(ui: UiBuilder) {
    let expanded = Binding::bool(false);
    let expanded_for_view = expanded.clone();
    let mut app = ui.mount(move || {
        Accordion::with_toggle(
            &expanded_for_view,
            waterui::component::text("Details"),
            || waterui::component::text("Hidden specifics"),
        )
    });

    app.query()
        .role(Role::LABEL)
        .label("Details")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Hidden specifics")
        .assert_not_exists();

    let header = app.query().role(Role::LABEL).label("Details").single();
    header.tap_at(&mut app, 0.5, 0.5);
    assert!(
        expanded.get(),
        "tapping the header must expand the accordion"
    );
    app.query()
        .role(Role::LABEL)
        .label("Hidden specifics")
        .assert_exists();

    let header = app.query().role(Role::LABEL).label("Details").single();
    header.tap_at(&mut app, 0.5, 0.5);
    assert!(
        !expanded.get(),
        "tapping the header again must collapse the accordion"
    );
    app.query()
        .role(Role::LABEL)
        .label("Hidden specifics")
        .assert_not_exists();
}

fn divided_stack() -> impl waterui::View {
    use waterui::widget::Divider;
    waterui::component::vstack((
        waterui::component::text("Above"),
        Divider,
        waterui::component::text("Below"),
    ))
}

#[waterui::test(divided_stack, theme = install_m3)]
fn divider_separates_content_without_hijacking_semantics(app: &mut SemanticApp) {
    app.query().role(Role::LABEL).label("Above").assert_exists();
    app.query().role(Role::LABEL).label("Below").assert_exists();
}

#[waterui::form]
struct Profile {
    /// Display name.
    name: String,
    /// Whether the account is active.
    active: bool,
}

#[waterui::test(theme = install_m3)]
fn derived_form_edits_flow_back_into_the_struct_binding(ui: UiBuilder) {
    let profile = Binding::container(Profile::default());
    let profile_for_view = profile.clone();
    let mut app = ui.mount(move || waterui::form::form(&profile_for_view));

    app.query().role(Role::TEXT_INPUT).set_text("Ada");
    assert_eq!(
        profile.get().name,
        "Ada",
        "text input edits must reach the derived struct binding"
    );

    app.query().role(Role::SWITCH).tap();
    assert!(
        profile.get().active,
        "toggle flips must reach the derived struct binding"
    );
}
