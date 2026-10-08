//! Layout regressions for the native navigation containers.
//!
//! <https://github.com/water-rs/hydrolysis/issues/153>: the split measured a
//! dynamically materialized column at intrinsic — `builder.build(selected)`
//! under a `ProposalSize` it never read — and the window's minimum-size probe
//! then resized the frame around an overflowing detail. Each column must
//! answer the proposal the split hands it, and measurement must read the
//! retained, mounted column rather than rebuilding it. `docs/layout-spec.md`
//! §7 is the contract: a column's rect is the proposal for its content.

mod support {
    /// Widens a `u32` extent to `f32`, rounding to nearest when the value exceeds
    /// the 24-bit mantissa; the extents involved are small.
    #[expect(
        clippy::cast_precision_loss,
        reason = "the widened values are pixel extents, far below 2^24"
    )]
    pub const fn u32_as_f32(v: u32) -> f32 {
        v as f32
    }
}

use hydrolysis_m3::Material3;
use waterui::component::list::{List, ListItem};
use waterui::component::{button, hstack, text, vstack};
use waterui::id::SelfId;
use waterui::navigation::{
    NavigationLink, NavigationSplitColumnVisibility, NavigationSplitView, NavigationStack,
    NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement, NavigationView, Tab,
    Tabs,
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
        composer.y() + composer.height() <= support::u32_as_f32(WINDOW_HEIGHT) + 0.5,
        "{context}: composer escaped the window: {composer:?}"
    );
    // The pane is the detail column's content area: it starts where the list
    // starts and runs to the window bottom, and the bar closes it — so the
    // list's height is the pane minus the bar.
    let pane_height = (support::u32_as_f32(WINDOW_HEIGHT)) - messages.y();
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
        dock.y() + dock.height() <= support::u32_as_f32(WINDOW_HEIGHT) + 0.5,
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

/// water-rs/waterui#1915, the measure-time half: a retained sub-view's
/// layout pass also reads text metrics — a `text!` binding inside
/// navigation content — through the `&mut HydroState` measure path rather
/// than `read_signal`, so no watch registered it: growing the bound text
/// must still re-run the sub-view's layout and shift the sibling beside it.
#[test]
fn navigation_content_relayouts_when_a_measure_read_text_grows() {
    let value = Binding::container(String::from("9"));
    let value_for_view = value.clone();
    let mut app = ui()
        .viewport(320, 240)
        .theme(Material3::defaults())
        .mount_offscreen(move || {
            let value = value_for_view.clone();
            NavigationStack::new(NavigationView::new(
                "Inbox",
                hstack((
                    waterui::text!("{value}").a11y_label("counter"),
                    text("next").a11y_label("next"),
                ))
                .spacing(0.0),
            ))
        });
    app.settle();
    let counter_before = label_bounds(&mut app, "counter");
    let next_before = label_bounds(&mut app, "next");

    value.set(String::from("10000"));
    app.settle();

    let counter_after = label_bounds(&mut app, "counter");
    let next_after = label_bounds(&mut app, "next");
    assert!(
        f64::from(counter_after.width()) > f64::from(counter_before.width()),
        "the counter's laid-out frame did not grow with its text: \
         {counter_before:?} -> {counter_after:?}"
    );
    assert!(
        f64::from(next_after.x()) > f64::from(next_before.x()),
        "the sibling did not move when the text beside it grew: \
         {next_before:?} -> {next_after:?}"
    );
    // Zero spacing places the sibling exactly at the text's trailing edge,
    // wherever the row itself sits inside the page.
    assert_close(
        f64::from(next_after.x()),
        f64::from(counter_after.x() + counter_after.width()),
        1.0,
        "the sibling must start at the grown text's trailing edge",
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

/// water-rs/waterui#2239: the collapsed split's semantic tree must carry an
/// actionable back affordance — it was pointer-only — and must drop the
/// panes it does not present, the same shape the rendered path produces.
#[test]
fn collapsed_split_emits_back_button_and_only_the_front_pane() {
    let selection = Binding::container(None::<i32>);
    let mut app = ui().viewport(390, 844).mount({
        let selection = selection.clone();
        move || {
            NavigationSplitView::new(
                &selection,
                move || rows(20, "chat").a11y_label("chats"),
                move |id| NavigationView::new(format!("Chat {id}"), detail_content()),
            )
        }
    });
    app.settle();
    assert!(
        app.query().role(Role::LIST).label("chats").exists(),
        "with no selection the sidebar is the front pane"
    );
    assert!(
        !app.query().role(Role::LIST).label("messages").exists(),
        "the detail pane stays out of the tree while the sidebar shows"
    );
    assert!(
        !app.query().role(Role::BUTTON).label("Back").exists(),
        "the sidebar front emits no back button"
    );

    selection.set(Some(7));
    app.settle();
    let back = app.query().role(Role::BUTTON).label("Back").single();
    assert!(
        !app.query().role(Role::LIST).label("chats").exists(),
        "the hidden sidebar must leave the accessibility tree"
    );
    assert!(
        app.query().role(Role::LIST).label("messages").exists(),
        "the front detail pane emits"
    );

    back.tap(&mut app);
    app.settle();
    assert!(
        app.query().role(Role::LIST).label("chats").exists(),
        "activating Back must present the sidebar again"
    );
    assert!(
        !app.query().role(Role::BUTTON).label("Back").exists(),
        "the back button leaves the tree with the detail pane"
    );
}

/// The rendered path of the same fix: the collapsed split draws the back
/// chevron itself, so the emitted node must carry the chevron's real bounds
/// and its activation must navigate back to the sidebar.
#[test]
fn collapsed_split_back_button_navigates_on_offscreen_mount() {
    let selection = Binding::container(None::<i32>);
    let mut app = ui()
        .viewport(390, 844)
        .theme(Material3::defaults())
        .mount_offscreen({
            let selection = selection.clone();
            move || {
                NavigationSplitView::new(
                    &selection,
                    move || rows(20, "chat").a11y_label("chats"),
                    move |id| NavigationView::new(format!("Chat {id}"), detail_content()),
                )
            }
        });
    app.settle();
    selection.set(Some(7));
    app.settle();
    let back = app.query().role(Role::BUTTON).label("Back").single();
    assert!(
        back.bounds().width() > 0.0,
        "the rendered back node must carry the drawn chevron's bounds"
    );
    assert!(
        !app.query().role(Role::LIST).label("chats").exists(),
        "the hidden sidebar must leave the accessibility tree"
    );
    back.tap(&mut app);
    app.settle();
    assert!(
        app.query().role(Role::LIST).label("chats").exists(),
        "activating Back must present the sidebar again"
    );
}

/// The same split at desktop width stays expanded: every column emits and no
/// back affordance appears — the collapsed tree is a narrow-viewport shape.
#[test]
fn expanded_split_emits_every_column_on_semantic_mount() {
    let selection = Binding::container(Some(7i32));
    let mut app = ui().viewport(WINDOW_WIDTH, WINDOW_HEIGHT).mount(move || {
        NavigationSplitView::new(
            &selection,
            move || rows(20, "chat").a11y_label("chats"),
            move |id| NavigationView::new(format!("Chat {id}"), detail_content()),
        )
    });
    app.settle();
    assert!(
        app.query().role(Role::LIST).label("chats").exists(),
        "the expanded split emits its sidebar"
    );
    assert!(
        app.query().role(Role::LIST).label("messages").exists(),
        "the expanded split emits the selected detail"
    );
    assert!(
        !app.query().role(Role::BUTTON).label("Back").exists(),
        "an expanded split has no back affordance"
    );
}

/// An expanded three-column split that prefers its two trailing columns
/// leaves the sidebar unplaced, so the semantic walk must not emit it either —
/// both presentation paths share one column decision.
#[test]
fn double_column_split_omits_its_sidebar_on_semantic_mount() {
    let sidebar_selection = Binding::container(Some(1i32));
    let content_selection = Binding::container(Some(7i32));
    let mut app = ui().viewport(WINDOW_WIDTH, WINDOW_HEIGHT).mount(move || {
        NavigationSplitView::three_column(
            &sidebar_selection,
            &content_selection,
            move || rows(20, "folder").a11y_label("folders"),
            move |id| {
                NavigationView::new(format!("Folder {id}"), rows(20, "chat").a11y_label("chats"))
            },
            move |id| NavigationView::new(format!("Chat {id}"), detail_content()),
        )
        .column_visibility(NavigationSplitColumnVisibility::DoubleColumn)
    });
    app.settle();
    assert!(
        !app.query().role(Role::LIST).label("folders").exists(),
        "a double-column split hides its sidebar"
    );
    assert!(
        app.query().role(Role::LIST).label("chats").exists(),
        "the content column emits"
    );
    assert!(
        app.query().role(Role::LIST).label("messages").exists(),
        "the detail column emits"
    );
}
