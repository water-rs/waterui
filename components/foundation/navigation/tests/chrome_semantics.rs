//! Navigation chrome semantics — water-rs/hydrolysis#161.
//!
//! Whatever the navigation bar draws must exist as a node on the semantic
//! runtime and on the rendered runtime's accessibility tree: the title's
//! Header, the subtitle's own static text beside it, every toolbar group,
//! the back affordance, and the search field.
//!
//! Moved from `backends/hydrolysis/tests/navigation.rs` under
//! water-rs/waterui#1673: the chrome emission is the component's contract.
//! The offscreen twins mount under `TestStyle` — the minimal style —
//! because every assertion reads the tree, never a style package's chrome.

use waterui::ViewExt as _;
use waterui::component::list::{List, ListItem};
use waterui::component::{button, text, vstack};
use waterui::id::SelfId;
use waterui::{Binding, Str, View};
use waterui_navigation::{
    NavigationLink, NavigationSplitColumnVisibility, NavigationSplitView, NavigationStack,
    NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement, NavigationView,
};
use waterui_testing::{Role, RuntimeDriver, SemanticApp, Styled, TestStyle, UiBuilder};

/// A lazy `List` of `count` text rows.
fn rows(count: usize, prefix: &'static str) -> impl View {
    let data: Vec<SelfId<usize>> = (0..count).map(SelfId::new).collect();
    List::for_each(data, move |item| {
        ListItem::new(text(format!("{prefix} {}", *item)))
    })
}

/// The `vstack` content of a detail pane: a lazy list and a fixed-height
/// trailing bar.
fn detail_content() -> impl View {
    vstack((
        rows(60, "message").a11y_label("messages"),
        text("composer").height(48.0).a11y_label("composer"),
    ))
    .spacing(0.0)
}

/// The chat-detail shape from the issue: a title, a subtitle under it, and
/// content.
fn chat_detail_view() -> NavigationView {
    NavigationView::new("dogfood crew", text("chat body")).navigation_subtitle(text("5 members"))
}

#[waterui::test(viewport = (390, 844))]
fn navigation_subtitle_emits_on_semantic_mount(ui: UiBuilder) {
    let mut app = ui.mount(chat_detail_view);
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

#[waterui::test(theme = TestStyle::new(), viewport = (390, 844))]
fn navigation_subtitle_emits_on_offscreen_mount(ui: UiBuilder<Styled<TestStyle>>) {
    let mut app = ui.mount_offscreen(chat_detail_view);
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

fn assert_toolbar_items<R: RuntimeDriver>(app: &mut SemanticApp<R>) {
    app.settle();
    for label in ["Edit", "Compose", "Add", "Mark All"] {
        assert!(
            app.query().role(Role::BUTTON).label(label).exists(),
            "the toolbar item {label:?} must emit a Button node"
        );
    }
}

#[waterui::test(viewport = (390, 844))]
fn navigation_toolbar_items_emit_on_semantic_mount(ui: UiBuilder) {
    let mut app = ui.mount(toolbar_view);
    assert_toolbar_items(&mut app);
}

#[waterui::test(theme = TestStyle::new(), viewport = (390, 844))]
fn navigation_toolbar_items_emit_on_offscreen_mount(ui: UiBuilder<Styled<TestStyle>>) {
    let mut app = ui.mount_offscreen(toolbar_view);
    assert_toolbar_items(&mut app);
}

#[waterui::test(viewport = (390, 844))]
fn navigation_search_field_emits_on_semantic_mount(ui: UiBuilder) {
    let query = Binding::container(Str::from(""));
    let mut app = ui.mount(move || {
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

#[waterui::test(theme = TestStyle::new(), viewport = (390, 844))]
fn navigation_search_field_emits_on_offscreen_mount(ui: UiBuilder<Styled<TestStyle>>) {
    let query = Binding::container(Str::from(""));
    let mut app = ui.mount_offscreen(move || {
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

#[waterui::test(viewport = (390, 844))]
fn navigation_large_title_emits_on_semantic_mount(ui: UiBuilder) {
    let mut app = ui.mount(|| NavigationView::new("Inbox", text("mail body")).large_title());
    app.settle();
    assert!(
        app.query().role(Role::HEADER).label("Inbox").exists(),
        "the large title must emit as a Header"
    );
}

#[waterui::test(theme = TestStyle::new(), viewport = (390, 844))]
fn navigation_large_title_emits_on_offscreen_mount(ui: UiBuilder<Styled<TestStyle>>) {
    let mut app =
        ui.mount_offscreen(|| NavigationView::new("Inbox", text("mail body")).large_title());
    app.settle();
    assert!(
        app.query().role(Role::HEADER).label("Inbox").exists(),
        "the large title must emit as a Header"
    );
}

#[waterui::test(viewport = (390, 844))]
fn navigation_back_button_emits_on_semantic_mount(ui: UiBuilder) {
    let mut app = ui.mount(|| {
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

#[waterui::test(theme = TestStyle::new(), viewport = (390, 844))]
fn navigation_back_button_emits_on_offscreen_mount(ui: UiBuilder<Styled<TestStyle>>) {
    let mut app = ui.mount_offscreen(|| {
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
#[waterui::test(viewport = (390, 844))]
fn collapsed_split_emits_back_button_and_only_the_front_pane(ui: UiBuilder) {
    let selection = Binding::container(None::<i32>);
    let mut app = ui.mount({
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
#[waterui::test(theme = TestStyle::new(), viewport = (390, 844))]
fn collapsed_split_back_button_navigates_on_offscreen_mount(ui: UiBuilder<Styled<TestStyle>>) {
    let selection = Binding::container(None::<i32>);
    let mut app = ui.mount_offscreen({
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
#[waterui::test(viewport = (1400, 900))]
fn expanded_split_emits_every_column_on_semantic_mount(ui: UiBuilder) {
    let selection = Binding::container(Some(7i32));
    let mut app = ui.mount(move || {
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
#[waterui::test(viewport = (1400, 900))]
fn double_column_split_omits_its_sidebar_on_semantic_mount(ui: UiBuilder) {
    let sidebar_selection = Binding::container(Some(1i32));
    let content_selection = Binding::container(Some(7i32));
    let mut app = ui.mount(move || {
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
