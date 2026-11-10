//! Accessibility-tree coverage for selecting the rows of `List::content` by
//! the app's own values (water-rs/waterui#4138): a static row is selected by
//! the value it is tagged with, a `ForEach` row by its item's id, and an
//! untagged row not at all — with no row index anywhere in the app's code.

use waterui::component::list::{List, ListItem, Section, row};
use waterui::component::text;
use waterui::id::SelfId;
use waterui::reactive::collection::SignalCollection;
use waterui::views::ForEach;
use waterui::{Binding, Computed, Signal, SignalExt as _, Str, ViewExt as _};
use waterui_testing::{Role, RuntimeDriver, SemanticApp, ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mailbox {
    Inbox,
    Archive,
    Drafts,
}

/// A sidebar of tagged mailboxes plus an untagged storage row.
fn mailboxes(selection: &Binding<Option<Mailbox>>) -> impl waterui::View + use<> {
    List::content((
        Section::new("Mailboxes").content((
            (|| ListItem::new(text("Inbox"))).tag(Mailbox::Inbox),
            row("Archive", "12").tag(Mailbox::Archive),
            (|| ListItem::new(text("下書き"))).tag(Mailbox::Drafts),
        )),
        row("Storage", "3 GB"),
    ))
    .selection(selection)
}

fn assert_selected<R: RuntimeDriver>(app: &mut SemanticApp<R>, label: &str, selected: bool) {
    app.settle();
    app.query()
        .role(Role::LIST_ITEM)
        .label(label)
        .selected(selected)
        .assert_exists();
}

/// Tapping a tagged static row writes its tag, a programmatic write shows
/// the matching row, and an untagged row is not selectable. Rendered, so the
/// untagged row meets a real pointer press.
#[test]
fn selecting_a_tagged_static_row_writes_its_value() {
    let selection = Binding::container(None::<Mailbox>);
    let mut app = ui()
        .theme(hydrolysis_m3::Material3::defaults())
        .viewport(360, 320)
        .mount_offscreen({
            let selection = selection.clone();
            move || mailboxes(&selection)
        });

    app.query().role(Role::LIST_ITEM).label("Archive 12").tap();
    assert_eq!(selection.snapshot(), Some(Mailbox::Archive));
    assert_selected(&mut app, "Archive 12", true);
    assert_selected(&mut app, "Inbox", false);

    selection.set(Some(Mailbox::Drafts));
    assert_selected(&mut app, "下書き", true);
    assert_selected(&mut app, "Archive 12", false);

    // A pointer press: the row registers no selection target for it.
    app.query()
        .role(Role::LIST_ITEM)
        .label("Storage 3 GB")
        .tap_at(0.5, 0.5);
    assert_eq!(
        selection.snapshot(),
        Some(Mailbox::Drafts),
        "a row without a tag is not selectable: tapping it leaves the selection alone",
    );
    assert_selected(&mut app, "Storage 3 GB", false);
}

const TODAY: &[&str] = &["Buy milk", "Call Ana", "Water plants", "牛乳を買う"];
const UPCOMING: &[&str] = &["Book flights", "Renew passport", "여권 갱신"];

type Rows = SignalCollection<Computed<Vec<SelfId<&'static str>>>>;

/// The reminders in `all` whose title contains `query`.
fn filtered(query: &Binding<Str>, all: &'static [&'static str]) -> Rows {
    SignalCollection::new(
        query
            .clone()
            .map(move |query: Str| {
                all.iter()
                    .copied()
                    .filter(|title| title.contains(query.as_str()))
                    .map(SelfId::new)
                    .collect::<Vec<_>>()
            })
            .computed(),
    )
}

/// Every reminder in `all`, unfiltered.
fn unfiltered(all: &'static [&'static str]) -> Rows {
    SignalCollection::new(Computed::constant(
        all.iter().copied().map(SelfId::new).collect::<Vec<_>>(),
    ))
}

fn reminder(title: SelfId<&'static str>) -> ListItem {
    ListItem::new(text(title.into_inner()))
}

/// A reminder list selected by title: an untagged summary row, a Today
/// section the search filters, and an Upcoming section it leaves alone.
fn reminders(
    query: &Binding<Str>,
    selection: &Binding<Option<&'static str>>,
) -> impl waterui::View + use<> {
    List::content((
        row("Lists", "2"),
        Section::new("Today").content(ForEach::new(filtered(query, TODAY), reminder)),
        Section::new("Upcoming").content(ForEach::new(unfiltered(UPCOMING), reminder)),
    ))
    .selection(selection)
}

fn mount_reminders(query: &Binding<Str>, selection: &Binding<Option<&'static str>>) -> SemanticApp {
    ui().mount({
        let query = query.clone();
        let selection = selection.clone();
        move || reminders(&query, &selection)
    })
}

/// Tapping a row of a `ForEach` inside a section writes the item's id.
#[test]
fn selecting_a_foreach_row_in_a_reactive_section_writes_the_item_id() {
    let query = Binding::container(Str::from(""));
    let selection = Binding::container(None::<&'static str>);
    let mut app = mount_reminders(&query, &selection);

    app.query().role(Role::LIST_ITEM).label("Call Ana").tap();
    assert_eq!(selection.snapshot(), Some("Call Ana"));
    assert_selected(&mut app, "Call Ana", true);
    assert_selected(&mut app, "Buy milk", false);

    app.query().role(Role::LIST_ITEM).label("여권 갱신").tap();
    assert_eq!(selection.snapshot(), Some("여권 갱신"));
    assert_selected(&mut app, "여권 갱신", true);
    assert_selected(&mut app, "Call Ana", false);
}

/// A selected row keeps its selection while filtering another section moves
/// it up the list, and the rows that move with it stay selectable by their
/// own ids.
#[test]
fn selection_survives_its_row_moving_when_another_section_is_filtered() {
    let query = Binding::container(Str::from(""));
    let selection = Binding::container(None::<&'static str>);
    let mut app = mount_reminders(&query, &selection);

    app.query()
        .role(Role::LIST_ITEM)
        .label("Renew passport")
        .tap();
    assert_eq!(selection.snapshot(), Some("Renew passport"));

    query.set(Str::from("milk"));
    assert_eq!(
        selection.snapshot(),
        Some("Renew passport"),
        "filtering Today does not write the selection",
    );
    assert_selected(&mut app, "Renew passport", true);
    assert_selected(&mut app, "Book flights", false);
    assert_selected(&mut app, "Buy milk", false);

    app.query()
        .role(Role::LIST_ITEM)
        .label("Book flights")
        .tap();
    assert_eq!(selection.snapshot(), Some("Book flights"));
    assert_selected(&mut app, "Book flights", true);
    assert_selected(&mut app, "Renew passport", false);

    query.set(Str::from(""));
    assert_selected(&mut app, "Book flights", true);
    selection.set(Some("Water plants"));
    assert_selected(&mut app, "Water plants", true);
    assert_selected(&mut app, "Book flights", false);
}
