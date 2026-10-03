//! Layout regressions for the native navigation containers.
//!
//! <https://github.com/water-rs/hydrolysis/issues/153>: the split measured a
//! dynamically materialized column at intrinsic — `builder.build(selected)`
//! under a `ProposalSize` it never read — and the window's minimum-size probe
//! then resized the frame around an overflowing detail. Each column must
//! answer the proposal the split hands it, and measurement must read the
//! retained, mounted column rather than rebuilding it. `docs/layout-spec.md`
//! §7 is the contract: a column's rect is the proposal for its content.

use hydrolysis_m3::Material3;
use waterui::component::list::{List, ListItem};
use waterui::component::{button, text, vstack};
use waterui::id::SelfId;
use waterui::navigation::{
    NavigationLink, NavigationSplitView, NavigationStack, NavigationToolbar, NavigationToolbarItem,
    NavigationToolbarPlacement, NavigationView, Tab, Tabs,
};
use waterui::{Binding, Str, View, ViewExt};
use waterui_testing::{NodeBounds, OffscreenApp, Role, ui};

const WINDOW_WIDTH: u32 = 1400;
const WINDOW_HEIGHT: u32 = 900;
const BAR_HEIGHT: f32 = 48.0;

/// A lazy `List` of `count` text rows — the chat-log shape from the issue.
fn rows(count: usize, prefix: &'static str) -> impl View {
    let data: Vec<SelfId<usize>> = (0..count).map(SelfId::new).collect();
    List::for_each(data, move |item| {
        ListItem::new(text(format!("{prefix} {}", *item)))
    })
}

/// The `vstack` content of the detail pane: a lazy 60-row list and a
/// fixed-height trailing bar, with no spacing so the pane fills as
/// `list.height + bar.height` exactly.
fn detail_content() -> impl View {
    vstack((
        rows(60, "message").a11y_label("messages"),
        text("composer").height(BAR_HEIGHT).a11y_label("composer"),
    ))
    .spacing(0.0)
}

fn mount_split(selection: Binding<Option<i32>>) -> OffscreenApp {
    ui().viewport(WINDOW_WIDTH, WINDOW_HEIGHT)
        .theme(Material3::defaults())
        .mount_offscreen(move || {
            let selection = selection.clone();
            NavigationSplitView::new(
                &selection,
                move || rows(20, "chat").a11y_label("chats"),
                move |id| NavigationView::new(format!("Chat {id}"), detail_content()),
            )
        })
}

fn list_bounds(app: &mut OffscreenApp, label: &'static str) -> NodeBounds {
    app.query().role(Role::LIST).label(label).single().bounds()
}

fn label_bounds(app: &mut OffscreenApp, label: &'static str) -> NodeBounds {
    app.query().role(Role::LABEL).label(label).single().bounds()
}

fn assert_close(actual: f64, expected: f64, epsilon: f64, context: &str) {
    assert!(
        (actual - expected).abs() <= epsilon,
        "{context}: expected {expected}, measured {actual}"
    );
}

/// Asserts the geometry the issue fixes: the trailing bar inside the window,
/// the lazy list filling the pane minus the bar, and the sidebar's list height
/// unchanged by the detail materializing.
fn assert_split_geometry(app: &mut OffscreenApp, context: &str) {
    let messages = list_bounds(app, "messages");
    let composer = label_bounds(app, "composer");
    assert!(
        composer.y() + composer.height() <= (WINDOW_HEIGHT as f32) + 0.5,
        "{context}: composer escaped the window: {composer:?}"
    );
    // The pane is the detail column's content area: it starts where the list
    // starts and runs to the window bottom, and the bar closes it — so the
    // list's height is the pane minus the bar.
    let pane_height = (WINDOW_HEIGHT as f32) - messages.y();
    assert_close(
        f64::from(messages.height()),
        f64::from(pane_height - BAR_HEIGHT),
        1.0,
        context,
    );
}

#[test]
fn navigation_split_detail_fits_pane_when_selected_after_mount() {
    let selection = Binding::container(None::<i32>);
    let mut app = mount_split(selection.clone());
    app.settle();
    let sidebar_height_before = list_bounds(&mut app, "chats").height();

    selection.set(Some(7));
    app.settle();
    // A second selection forces the re-flush that, on the broken build,
    // replays the layout onto the surface the window just grew to — the
    // geometry assertions then observe the issue's overflow rather than a
    // stale pre-resize frame.
    selection.set(Some(8));
    app.settle();

    // The window itself must never have grown: the surface is the window.
    let snap = app.snapshot();
    assert_eq!(
        snap.height, WINDOW_HEIGHT,
        "the window grew to fit an overflowing detail column"
    );
    assert_split_geometry(&mut app, "selection set after mount");
    assert_close(
        f64::from(list_bounds(&mut app, "chats").height()),
        f64::from(sidebar_height_before),
        0.5,
        "the sidebar list's height changed when the detail appeared",
    );

    // The same geometry must result when the selection was already set at
    // mount — issue #153's before/after asymmetry.
    let mut before = mount_split(Binding::container(Some(7)));
    before.settle();
    let messages_after = list_bounds(&mut app, "messages");
    let composer_after = label_bounds(&mut app, "composer");
    let messages_before = list_bounds(&mut before, "messages");
    let composer_before = label_bounds(&mut before, "composer");
    for (after, before, name) in [
        (messages_after, messages_before, "message list"),
        (composer_after, composer_before, "composer"),
    ] {
        for (actual, expected, axis) in [
            (f64::from(after.x()), f64::from(before.x()), "x"),
            (f64::from(after.y()), f64::from(before.y()), "y"),
            (f64::from(after.width()), f64::from(before.width()), "width"),
            (
                f64::from(after.height()),
                f64::from(before.height()),
                "height",
            ),
        ] {
            assert_close(
                actual,
                expected,
                0.5,
                &format!(
                    "{name} differs between selection-after-mount and selection-before-mount on {axis}"
                ),
            );
        }
    }
}

#[test]
fn navigation_split_detail_fits_pane_when_selected_before_mount() {
    let mut app = mount_split(Binding::container(Some(7)));
    app.settle();
    assert_split_geometry(&mut app, "selection set before mount");
}

/// The same ignored-proposal defect, in `Tabs`: a lazily measured tab content
/// reported its intrinsic height to its container's probe instead of the
/// content rect the tabs hands it, so the trailing sibling is pushed past the
/// window.
#[test]
fn tabs_content_fits_pane() {
    let selection = Binding::container(0i32);
    let mut app = ui()
        .viewport(WINDOW_WIDTH, WINDOW_HEIGHT)
        .theme(Material3::defaults())
        .mount_offscreen(move || {
            let selection = selection.clone();
            vstack((
                Tabs::new(
                    &selection,
                    vec![
                        Tab::new(0i32, "Messages", move || {
                            NavigationView::new(
                                "Messages",
                                rows(60, "message").a11y_label("messages"),
                            )
                        }),
                        Tab::new(1i32, "Settings", move || {
                            NavigationView::new("Settings", text("settings"))
                        }),
                    ],
                ),
                text("dock").height(BAR_HEIGHT).a11y_label("dock"),
            ))
            .spacing(0.0)
        });
    app.settle();

    let dock = label_bounds(&mut app, "dock");
    assert!(
        dock.y() + dock.height() <= (WINDOW_HEIGHT as f32) + 0.5,
        "the vstack's trailing bar escaped the window: {dock:?}"
    );
    let messages = list_bounds(&mut app, "messages");
    // The tab content rect ends where the tab bar begins; the list inside
    // must fill it exactly.
    let tab_bar = app.query().role(Role::TAB_LIST).single();
    let content_bottom = tab_bar.bounds().y();
    assert_close(
        f64::from(messages.y() + messages.height()),
        f64::from(content_bottom),
        1.0,
        "the tab content's list must fill the pane minus the dock",
    );
}

// ── Navigation chrome semantics — water-rs/hydrolysis#161 ──
//
// Whatever the navigation bar draws must exist as a node on the semantic
// runtime and on the rendered runtime's accessibility tree: the title's
// Header, the subtitle's own static text beside it, every toolbar group, the
// back affordance, and the search field.

/// The chat-detail shape from the issue: a title, a subtitle under it, and
/// content.
fn chat_detail_view() -> NavigationView {
    NavigationView::new("dogfood crew", text("chat body")).navigation_subtitle(text("5 members"))
}

#[test]
fn navigation_subtitle_emits_on_semantic_mount() {
    let mut app = ui().viewport(390, 844).mount(chat_detail_view);
    app.settle();
    let bar = app.query().role(Role::NAVIGATION).single();
    assert!(
        app.query()
            .role(Role::LABEL)
            .label("5 members")
            .children_of(&bar)
            .exists(),
        "the navigation subtitle must emit as a Label beside the Header under the bar node"
    );
}

#[test]
fn navigation_subtitle_emits_on_offscreen_mount() {
    let mut app = ui()
        .viewport(390, 844)
        .theme(Material3::defaults())
        .mount_offscreen(chat_detail_view);
    app.settle();
    let bar = app.query().role(Role::NAVIGATION).single();
    let subtitle = app
        .query()
        .role(Role::LABEL)
        .label("5 members")
        .children_of(&bar)
        .single();
    assert!(
        subtitle.bounds().height() > 0.0,
        "the rendered runtime's subtitle node must carry real bounds"
    );
}

/// One toolbar action in each group the bar draws: leading, principal (the
/// title area), trailing, and bottom.
fn toolbar_view() -> NavigationView {
    NavigationView::new("Inbox", text("mail body")).navigation_toolbar(
        NavigationToolbar::default()
            .item(NavigationToolbarItem::new(
                NavigationToolbarPlacement::TopBarLeading,
                button("Edit").action(|| {}),
            ))
            .item(NavigationToolbarItem::new(
                NavigationToolbarPlacement::Principal,
                button("Compose").action(|| {}),
            ))
            .item(NavigationToolbarItem::new(
                NavigationToolbarPlacement::TopBarTrailing,
                button("Add").action(|| {}),
            ))
            .item(NavigationToolbarItem::new(
                NavigationToolbarPlacement::BottomBar,
                button("Mark All").action(|| {}),
            )),
    )
}

fn assert_toolbar_items<R: waterui_testing::RuntimeDriver>(
    app: &mut waterui_testing::SemanticApp<R>,
) {
    app.settle();
    for label in ["Edit", "Compose", "Add", "Mark All"] {
        assert!(
            app.query().role(Role::BUTTON).label(label).exists(),
            "the toolbar item {label:?} must emit a Button node"
        );
    }
}

#[test]
fn navigation_toolbar_items_emit_on_semantic_mount() {
    let mut app = ui().viewport(390, 844).mount(toolbar_view);
    assert_toolbar_items(&mut app);
}

#[test]
fn navigation_toolbar_items_emit_on_offscreen_mount() {
    let mut app = ui()
        .viewport(390, 844)
        .theme(Material3::defaults())
        .mount_offscreen(toolbar_view);
    assert_toolbar_items(&mut app);
}

#[test]
fn navigation_search_field_emits_on_semantic_mount() {
    let query = Binding::container(Str::from(""));
    let mut app = ui().viewport(390, 844).mount(move || {
        NavigationView::new("Mail", text("mail body")).searchable(&query, "Search mail")
    });
    app.settle();
    assert!(
        app.query()
            .role(Role::TEXT_INPUT)
            .label("Search mail")
            .exists(),
        "the search field must emit a text input named by its prompt"
    );
}

#[test]
fn navigation_search_field_emits_on_offscreen_mount() {
    let query = Binding::container(Str::from(""));
    let mut app = ui()
        .viewport(390, 844)
        .theme(Material3::defaults())
        .mount_offscreen(move || {
            NavigationView::new("Mail", text("mail body")).searchable(&query, "Search mail")
        });
    app.settle();
    assert!(
        app.query()
            .role(Role::TEXT_INPUT)
            .label("Search mail")
            .exists(),
        "the search field must emit a text input named by its prompt"
    );
}

#[test]
fn navigation_large_title_emits_on_semantic_mount() {
    let mut app = ui()
        .viewport(390, 844)
        .mount(|| NavigationView::new("Inbox", text("mail body")).large_title());
    app.settle();
    assert!(
        app.query().role(Role::HEADER).label("Inbox").exists(),
        "the large title must emit as a Header"
    );
}

#[test]
fn navigation_large_title_emits_on_offscreen_mount() {
    let mut app = ui()
        .viewport(390, 844)
        .theme(Material3::defaults())
        .mount_offscreen(|| NavigationView::new("Inbox", text("mail body")).large_title());
    app.settle();
    assert!(
        app.query().role(Role::HEADER).label("Inbox").exists(),
        "the large title must emit as a Header"
    );
}

#[test]
fn navigation_back_button_emits_on_semantic_mount() {
    let mut app = ui().viewport(390, 844).mount(|| {
        NavigationStack::new(NavigationView::new(
            "Root",
            vstack((NavigationLink::new("Open Detail", || {
                NavigationView::new("Detail", text("detail"))
            }),)),
        ))
    });
    app.settle();
    let link = app.query().role(Role::BUTTON).label("Open Detail").single();
    link.tap(&mut app);
    app.settle();
    assert!(
        app.query().role(Role::BUTTON).label("Back").exists(),
        "the pushed stack must emit its back affordance"
    );
}

#[test]
fn navigation_back_button_emits_on_offscreen_mount() {
    let mut app = ui()
        .viewport(390, 844)
        .theme(Material3::defaults())
        .mount_offscreen(|| {
            NavigationStack::new(NavigationView::new(
                "Root",
                vstack((NavigationLink::new("Open Detail", || {
                    NavigationView::new("Detail", text("detail"))
                }),)),
            ))
        });
    app.settle();
    let link = app.query().role(Role::BUTTON).label("Open Detail").single();
    link.tap(&mut app);
    app.settle();
    assert!(
        app.query().role(Role::BUTTON).label("Back").exists(),
        "the pushed stack must emit its back affordance"
    );
}
