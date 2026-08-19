//! Section headers and footers on `List`.
//!
//! A section title is semantic text, not a string frozen when the list was
//! built: it localizes, and an app can drive it from a signal ("3 unread").
//! These tests read the chrome back out of the accessibility tree, so they
//! assert both that a title reaches a screen reader as its own heading and
//! that changing its signal repaints it without rebuilding the list.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use waterui::component::list::{List, ListItem, Section, row};
use waterui::prelude::*;
use waterui::reactive::{SignalExt, binding};
use waterui::text::Text;
use waterui_testing::{Role, UiBuilder};

/// How long a chrome update may take to reach the tree. Pumps are virtual, so
/// this costs frames rather than wall-clock time.
const CHROME_TIMEOUT: Duration = Duration::from_millis(500);

/// Counts how many times the view body that owns the list is evaluated, so a
/// test can tell a precise signal update apart from a structural rebuild.
#[derive(Clone)]
struct Fixture {
    unread: Binding<usize>,
    footer: Binding<Str>,
    bodies: Rc<Cell<usize>>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            unread: binding(3usize),
            footer: binding(Str::from("Updated just now")),
            bodies: Rc::new(Cell::new(0)),
        }
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the fixture is cloned into the mounted body, which must own it"
)]
fn sectioned_list(fixture: Fixture) -> impl View {
    fixture.bodies.set(fixture.bodies.get() + 1);
    let unread = fixture.unread.map(|unread| format!("{unread} unread"));
    List::content((
        Section::new(Text::computed(unread)).content((row("Alpha", "one"), row("Beta", "two"))),
        // A plain literal is localized text, so this header also proves the
        // header goes through the i18n pipeline rather than around it.
        Section::new("Archive")
            .footer(Text::computed(fixture.footer))
            .content((row("Gamma", "three"),)),
    ))
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 520))]
fn section_chrome_is_its_own_accessibility_node(ui: UiBuilder) {
    let fixture = Fixture::new();
    let mut app = ui.mount(move || sectioned_list(fixture.clone()));

    app.query()
        .role(Role::HEADER)
        .label("3 unread")
        .assert_exists();
    app.query()
        .role(Role::HEADER)
        .label("Archive")
        .assert_exists();
    app.query()
        .role(Role::FOOTER)
        .label("Updated just now")
        .assert_exists();
    // The header is a node of its own, not folded into the row it opens.
    app.query()
        .role(Role::LIST_ITEM)
        .label("Alpha one")
        .assert_exists();
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 520))]
fn a_section_header_follows_its_signal(ui: UiBuilder) {
    let fixture = Fixture::new();
    let probe = fixture.clone();
    let mut app = ui.mount(move || sectioned_list(fixture.clone()));

    app.query()
        .role(Role::HEADER)
        .label("3 unread")
        .assert_exists();
    let bodies_after_mount = probe.bodies.get();

    probe.unread.set(5);

    assert!(
        app.query()
            .role(Role::HEADER)
            .label("5 unread")
            .wait_for_existence(CHROME_TIMEOUT),
        "the header should follow its signal"
    );
    app.query()
        .role(Role::HEADER)
        .label("3 unread")
        .assert_not_exists();
    assert_eq!(
        probe.bodies.get(),
        bodies_after_mount,
        "a section header update must not rebuild the view that owns the list"
    );
    // The rows the section opens are untouched by a header change.
    app.query()
        .role(Role::LIST_ITEM)
        .label("Alpha one")
        .assert_exists();
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 520))]
fn a_section_footer_follows_its_signal(ui: UiBuilder) {
    let fixture = Fixture::new();
    let probe = fixture.clone();
    let mut app = ui.mount(move || sectioned_list(fixture.clone()));

    app.query()
        .role(Role::FOOTER)
        .label("Updated just now")
        .assert_exists();

    probe.footer.set(Str::from("Updated 5 minutes ago"));

    assert!(
        app.query()
            .role(Role::FOOTER)
            .label("Updated 5 minutes ago")
            .wait_for_existence(CHROME_TIMEOUT),
        "the footer should follow its signal"
    );
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 520))]
fn an_unlabeled_section_adds_no_chrome_nodes(ui: UiBuilder) {
    let mut app = ui.mount(|| {
        List::content((
            Section::unlabeled().content((row("Alpha", "one"),)),
            Section::unlabeled().content((row("Beta", "two"),)),
        ))
    });

    app.query()
        .role(Role::LIST_ITEM)
        .label("Alpha one")
        .assert_exists();
    app.query()
        .role(Role::LIST_ITEM)
        .label("Beta two")
        .assert_exists();
    app.query().role(Role::HEADER).assert_not_exists();
    app.query().role(Role::FOOTER).assert_not_exists();
}

/// The section marker rides the *first* item its content produces, so the
/// header has to land above that row rather than anywhere else in the list.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 520))]
fn a_section_opens_on_the_first_row_of_its_content(ui: UiBuilder) {
    let mut app = ui.mount(|| {
        List::content((Section::new("Only").content((
            || ListItem::new(text("First").padding()),
            || ListItem::new(text("Second").padding()),
        )),))
    });

    let header = app.query().role(Role::HEADER).label("Only").single();
    let first = app.query().role(Role::LIST_ITEM).label("First").single();
    assert!(
        header.bounds().y() + header.bounds().height() <= first.bounds().y() + 0.5,
        "the header must sit above the first row it opens: header {:?}, row {:?}",
        header.bounds(),
        first.bounds()
    );
}
