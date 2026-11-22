//! Accessibility-tree coverage for list sections over reactive collections
//! (water-rs/waterui#4138): a `Section` holding a `ForEach` follows its
//! collection row by row, its header opens whichever row currently leads the
//! section, and an empty section draws no chrome until rows come back.

use waterui::component::list::{List, ListItem, Section, row};
use waterui::component::text;
use waterui::id::SelfId;
use waterui::reactive::collection::SignalCollection;
use waterui::views::ForEach;
use waterui::{Binding, Computed, SignalExt as _, Str};
use waterui_testing::{Role, SemanticApp, ui};

const TODAY: &[&str] = &["Buy milk", "Call Ana", "Water plants", "牛乳を買う"];
const UPCOMING: &[&str] = &["Book flights", "Renew passport", "여권 갱신"];

type Rows = SignalCollection<Computed<Vec<SelfId<&'static str>>>>;

/// The reminders in `all` whose title contains `query` — the filtered
/// collection a search field drives.
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

fn reminder(title: SelfId<&'static str>) -> ListItem {
    ListItem::new(text(title.into_inner()))
}

/// A grouped, searchable reminder list: a static row, then two sections
/// whose rows come from the same search.
fn reminders(query: &Binding<Str>) -> impl waterui::View + use<> {
    List::content((
        row("Lists", "2"),
        Section::new("Today")
            .footer("Due today")
            .content(ForEach::new(filtered(query, TODAY), reminder)),
        Section::new("Upcoming").content(ForEach::new(filtered(query, UPCOMING), reminder)),
    ))
}

/// The list's direct accessibility children in order: rows as their label,
/// section chrome as `# label` (header) and `_ label` (footer).
fn outline(app: &mut SemanticApp) -> Vec<String> {
    app.settle();
    let list = app.query().role(Role::LIST).single();
    app.query()
        .children_of(&list)
        .all()
        .iter()
        .filter_map(|element| {
            let node = element.node();
            let label = node.label().unwrap_or_default();
            if node.role() == Role::HEADER {
                Some(format!("# {label}"))
            } else if node.role() == Role::FOOTER {
                Some(format!("_ {label}"))
            } else if node.role() == Role::LIST_ITEM {
                Some(label.to_owned())
            } else {
                None
            }
        })
        .collect()
}

/// Filtering removes the rows that stop matching and restores them, in
/// order, when the query widens again.
#[test]
fn filtering_removes_and_restores_section_rows() {
    let query = Binding::container(Str::from(""));
    let mut app = ui().mount({
        let query = query.clone();
        move || reminders(&query)
    });
    let full = vec![
        "Lists 2",
        "# Today",
        "Buy milk",
        "Call Ana",
        "Water plants",
        "牛乳を買う",
        "_ Due today",
        "# Upcoming",
        "Book flights",
        "Renew passport",
        "여권 갱신",
    ];
    assert_eq!(outline(&mut app), full);
    let call_ana = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Call Ana")
        .single()
        .id();

    query.set(Str::from("a"));
    assert_eq!(
        outline(&mut app),
        vec![
            "Lists 2",
            "# Today",
            "Call Ana",
            "Water plants",
            "_ Due today",
            "# Upcoming",
            "Renew passport",
        ],
    );
    assert_eq!(
        app.query()
            .role(Role::LIST_ITEM)
            .label("Call Ana")
            .single()
            .id(),
        call_ana,
        "a row that keeps matching keeps its identity: the list removed the \
         others around it instead of rebuilding",
    );

    query.set(Str::from(""));
    assert_eq!(outline(&mut app), full);
}

/// A section whose collection empties loses its header and footer, and gets
/// both back when rows return.
#[test]
fn emptied_section_drops_and_regains_its_chrome() {
    let query = Binding::container(Str::from(""));
    let mut app = ui().mount({
        let query = query.clone();
        move || reminders(&query)
    });

    query.set(Str::from("passport"));
    assert_eq!(
        outline(&mut app),
        vec!["Lists 2", "# Upcoming", "Renew passport"],
        "the emptied Today section draws neither header nor footer",
    );

    query.set(Str::from("Water"));
    assert_eq!(
        outline(&mut app),
        vec!["Lists 2", "# Today", "Water plants", "_ Due today"],
        "Today gets its chrome back and the emptied Upcoming loses its own",
    );
}

/// When the row that opens a section is filtered out, the header moves to
/// the first row that remains, and back when the row returns.
#[test]
fn section_header_follows_the_first_remaining_row() {
    let query = Binding::container(Str::from(""));
    let mut app = ui().mount({
        let query = query.clone();
        move || reminders(&query)
    });

    query.set(Str::from("l"));
    assert_eq!(
        outline(&mut app),
        vec![
            "Lists 2",
            "# Today",
            "Buy milk",
            "Call Ana",
            "Water plants",
            "_ Due today",
            "# Upcoming",
            "Book flights",
        ],
    );

    query.set(Str::from("a"));
    let filtered = outline(&mut app);
    assert_eq!(
        &filtered[1..3],
        ["# Today", "Call Ana"],
        "the Today header opens Call Ana once Buy milk is filtered out",
    );

    query.set(Str::from(""));
    assert_eq!(
        &outline(&mut app)[1..3],
        ["# Today", "Buy milk"],
        "the header returns to Buy milk when it matches again",
    );
}
