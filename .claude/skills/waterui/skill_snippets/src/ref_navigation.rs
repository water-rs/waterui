//! Snippets from `.claude/skills/waterui/references/navigation.md`, in file
//! order. Transcription conventions are documented in the crate README.

use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui_icons_material_icon as mdi;

/// Glue: the app's model navigation.md's snippets thread through their views.
/// `#[state]` marks it an extractor, which `send_draft`'s bare `mail: Mail`
/// parameter relies on.
#[state]
#[derive(Clone)]
pub struct Mail;

impl Mail {
    fn send_draft(&self) {}
    fn mark_read(&self, _id: u64) {}
}

/// Glue: the settings route navigation.md refers to.
#[derive(Clone, PartialEq, Eq)]
pub enum SettingsRoute {
    Appearance,
}

/// Glue: blocks 3, 4, 5 and 8 name `MailRoute` outside the module that declares
/// it, so they get an identical file-scope copy rather than a `pub` added to
/// the snippet's own `enum MailRoute { .. }` line.
#[derive(Clone, PartialEq, Eq)]
pub enum MailRoute {
    Message(u64),
    Compose,
}

// `NavigationStack::with_path` and `.destination(..)` both take a
// `NavigationView`, so these glue helpers apply a navigation title the way
// navigation.md's own "Bar chrome on a destination" section prescribes.
fn inbox_root(_mail: Mail) -> NavigationView {
    vstack((text("inbox"), Divider)).title("Inbox")
}
fn message_detail(_mail: Mail, _id: u64) -> NavigationView {
    vstack((text("message"), Divider)).title("Message")
}
fn compose_page(_mail: Mail) -> NavigationView {
    vstack((text("compose"), Divider)).title("Compose")
}
fn message_list(_mail: Mail) -> impl View {
    vstack((text("a"), text("b")))
}
fn library_split() -> impl View {
    text("library")
}
fn settings_stack() -> impl View {
    text("settings")
}
fn search_stack() -> impl View {
    text("search")
}
fn now_playing_bar() -> impl View {
    text("now playing")
}

// ---------------------------------------------------------------------------
// navigation.md § "## Tabs" — rust block 1/16
// ---------------------------------------------------------------------------
// navigation.md writes `mdi::` qualified so the icon set stays visible.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn navigation_block_01() -> impl View {
    use waterui::navigation::tab_style;

    fn inbox_stack() -> impl View {
        text("inbox stack")
    }
    let unread_count = Binding::i32(3);

    use waterui::{id::SelfId, reactive::collection::List, views::ForEach};

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Pane {
        Inbox,
        Library,
        Settings,
        Search,
    }

    let pane = binding(Pane::Inbox);
    let panes = [Pane::Inbox, Pane::Library, Pane::Settings, Pane::Search].map(SelfId::new);

    Tabs::new(
        &pane,
        ForEach::new(
            List::from(panes.to_vec()),
            move |pane: SelfId<Pane>| match *pane {
                Pane::Inbox => Tab::container(label("Inbox").icon(mdi::inbox()), inbox_stack)
                    .badge(unread_count.clone()),
                Pane::Library => {
                    Tab::container(label("Library").icon(mdi::image_album()), library_split)
                }
                Pane::Settings => {
                    Tab::container(label("Settings").icon(mdi::cog()), settings_stack)
                }
                Pane::Search => Tab::container(label("Search").icon(mdi::magnify()), search_stack)
                    .role(TabRole::Search), // iOS: the trailing system search tab; elsewhere a regular tab
            },
        ),
    )
    .style(tab_style::automatic())
    .minimize_behavior(TabBarMinimizeBehavior::OnScrollDown) // iOS 26 collapses the bar while scrolling; ignored elsewhere
    .bottom_accessory(now_playing_bar()) // iOS 26 glass bar above the tab bar (mini-player slot); not shown elsewhere
}

// ---------------------------------------------------------------------------
// navigation.md § "## Tabs" (prose): a fixed `vec![Tab..]` selected by a
// `Binding<usize>`, `.enabled(..)`, `Tab::new`, the other tab styles and the
// `TabBarMinimizeBehavior` variants. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn navigation_tabs_prose() -> impl View {
    use waterui::navigation::tab_style;

    let selected = binding(0_usize);
    let _ = TabBarMinimizeBehavior::Automatic;
    let _ = TabBarMinimizeBehavior::Never;
    let _ = TabBarMinimizeBehavior::OnScrollUp;
    let _ = Tabs::new(
        &selected,
        vec![Tab::new("Raw", || NavigationView::new("Raw", text("raw")))],
    )
    .style(tab_style::sidebar());
    Tabs::new(
        &selected,
        vec![Tab::container(label("Library"), library_split).enabled(true)],
    )
    .style(tab_style::tab_bar())
}

// ---------------------------------------------------------------------------
// navigation.md § "## Navigation stacks and routes" — rust block 2/16
// ---------------------------------------------------------------------------
pub mod navigation_block_02 {
    use super::{Mail, compose_page, inbox_root, message_detail};
    use waterui::prelude::*;

    #[derive(Clone, PartialEq)]
    enum MailRoute {
        Message(u64),
        Compose,
    }

    fn inbox_stack(mail: Mail) -> impl View {
        NavigationStack::with_path(NavigationPath::<MailRoute>::new(), inbox_root(mail.clone()))
            .destination(move |route| match route {
                MailRoute::Message(id) => message_detail(mail.clone(), id),
                MailRoute::Compose => compose_page(mail.clone()),
            })
    }

    /// Glue: the snippet only ever matches on the route enum, so the gate
    /// constructs both variants here to prove they exist.
    pub fn use_it(mail: Mail) -> impl View {
        let _ = (MailRoute::Message(1), MailRoute::Compose);
        inbox_stack(mail)
    }
}

// ---------------------------------------------------------------------------
// navigation.md § "## Navigation stacks and routes" — rust block 3/16
// Listing: a declarative link, then an imperative handler closure.
// ---------------------------------------------------------------------------
pub fn navigation_block_03() {
    let _ = {
        NavigationLink::value("Appearance", SettingsRoute::Appearance) // a tappable row/tile
    };

    // …or imperatively, from any handler, via the Navigator extractor:
    let _ = { |navigator: Navigator<MailRoute>| navigator.push(MailRoute::Compose) };
}

// ---------------------------------------------------------------------------
// navigation.md § "## Navigation stacks and routes" (prose): both extractor
// spellings work, and a link's first argument is `impl IntoLabel`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn navigation_extractor_spellings_prose() {
    fn row_content(_m: Str) -> impl View {
        text("row")
    }
    let message = Str::from("body");
    let sender = Binding::container(Str::from("Ada"));
    let subject = Binding::container(Str::from("Hi"));

    let _ = |State(navigator): State<Navigator<MailRoute>>| navigator.push(MailRoute::Compose);
    let _ = NavigationLink::value(
        Label::new(text!("{sender}, {subject}"), move || {
            row_content(message.clone())
        }),
        SettingsRoute::Appearance,
    );
}

// ---------------------------------------------------------------------------
// navigation.md § "## Navigation stacks and routes" (prose): the other
// `Navigator` and `NavigationPath` operations, and a route-free
// `NavigationLink::new` inside `NavigationStack::new`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn navigation_path_operations_prose(navigator: Navigator<MailRoute>) -> impl View {
    navigator.pop_n(2);
    navigator.clear();
    navigator.pop_to_root();
    navigator.replace([MailRoute::Compose, MailRoute::Message(1)]);

    let path = NavigationPath::from(vec![MailRoute::Compose]);
    path.push(MailRoute::Message(2));
    let _ = path.pop();
    path.pop_n(1);
    path.clear();
    path.replace([MailRoute::Compose]);
    let _ = (path.len(), path.is_empty(), path.snapshot());
    path.truncate(0);

    NavigationStack::new(inbox_root(Mail)).transition(navigation_transition::automatic())
}

pub fn navigation_route_free_link_prose() -> impl View {
    let _ = NavigationLink::new("Details", || message_detail(Mail, 1));
    NavigationStack::new(inbox_root(Mail))
}

// ---------------------------------------------------------------------------
// navigation.md § "## Navigation stacks and routes" — rust block 4/16
// ---------------------------------------------------------------------------
pub fn navigation_block_04() {
    use waterui::Url;

    let path = NavigationPath::<MailRoute>::new();
    let router = NavigationRouter::new(path.clone())
        .route(|url| (url.path() == "/compose").then(|| vec![MailRoute::Compose]));

    let opened = router.open(&Url::new("waterui://app/compose"));
    let _ = opened;
}

// ---------------------------------------------------------------------------
// navigation.md § "## Going back, and destination lifecycle" — rust block 5/16
// ---------------------------------------------------------------------------
pub fn send_draft(mail: Mail, navigator: Navigator<MailRoute>) {
    mail.send_draft();
    navigator.pop(); // returns the popped route as Option<T>, `None` at the root
}

// ---------------------------------------------------------------------------
// navigation.md § "## Going back, and destination lifecycle" — rust block 6/16
// ---------------------------------------------------------------------------
pub fn navigation_block_06() -> impl View {
    // Glue: the page body before `.title(..)` makes it a `NavigationView`;
    // block 2's `message_detail` is the titled destination built from it.
    fn message_body(_mail: Mail, _id: u64) -> impl View {
        vstack((text("message"), Divider))
    }
    let mail = Mail;
    let id = 7_u64;
    let subject = Binding::container(Str::from("Re: hello"));

    message_body(mail.clone(), id)
        .title(text!("{subject}"))
        .on_navigation_appear({
            let mail = mail.clone();
            move || mail.mark_read(id)
        })
}

// ---------------------------------------------------------------------------
// navigation.md § "## Bar chrome on a destination" — rust block 7/16
// ---------------------------------------------------------------------------
pub fn navigation_block_07() -> NavigationView {
    fn message_list_page(mail: Mail) -> NavigationView {
        let query = Binding::container(Str::from(""));
        let can_leave = Binding::bool(true);
        let unread = Binding::i32(2);

        message_list(mail)
            .title("Inbox") // or .title(text!("{subject}"))
            .large_title() // or .inline_title()
            .navigation_subtitle(text!("{unread} unread"))
            .searchable(&query, "Search mail") // a field inside the bar, not above the content
            .navigation_pop_enabled(can_leave) // refuse a back gesture reactively
            .on_navigation_pop_attempted(|m: SnackbarManager| {
                m.show(Snackbar::new("Finish the draft first"));
            })
    }

    message_list_page(Mail)
}

// ---------------------------------------------------------------------------
// navigation.md § "## Bar chrome on a destination" (prose):
// `NavigationView::new(title, content)` and `.inline_title()`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn navigation_view_new_prose() {
    let _ = NavigationView::new("Inbox", vstack((text("a"), Divider)));
    let _ = vstack((text("a"), Divider)).title("Inbox").inline_title();
    let _ = navigation("Inbox", vstack((text("a"), Divider)));
}

// ---------------------------------------------------------------------------
// navigation.md § "## Bar chrome on a destination" (prose): the rest of the
// destination's chrome. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn navigation_bar_chrome_prose() -> NavigationView {
    let visible = Binding::bool(true);

    message_list(Mail)
        .title("Inbox")
        .navigation_title_display_mode(NavigationTitleDisplayMode::Medium)
        .navigation_bar_visibility(visible)
        .navigation_bar_color(Color::srgb(0, 0, 0))
        .on_navigation_pop(|| ())
}

// ---------------------------------------------------------------------------
// navigation.md § "## Toolbars" — rust block 8/16
// A bare method fragment; applied to a navigation destination receiver.
// ---------------------------------------------------------------------------
// navigation.md writes `mdi::` qualified so the icon set stays visible.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn navigation_block_08() -> NavigationView {
    use waterui::navigation::{
        NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement,
    };

    fn toggle_editing() {}
    let edit_title = Binding::container(Str::from("Edit"));

    message_list(Mail).title("Inbox").navigation_toolbar(
        NavigationToolbar::default()
            .item(NavigationToolbarItem::new(
                NavigationToolbarPlacement::TopBarLeading,
                button(text!("{edit_title}"))
                    .style(ButtonStyle::Plain)
                    .action(toggle_editing),
            ))
            .item(NavigationToolbarItem::action(
                NavigationToolbarPlacement::PrimaryAction,
                label("Compose").icon(mdi::pencil()),
                |navigator: Navigator<MailRoute>| navigator.push(MailRoute::Compose),
            )),
    )
}

// ---------------------------------------------------------------------------
// navigation.md § "## Toolbars" (prose): `NavigationToolbar::new(vec![items…])`
// and the complete placement set. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn navigation_toolbar_prose() {
    use waterui::navigation::{
        NavigationToolbar, NavigationToolbarItem, NavigationToolbarPlacement,
    };

    let _ = NavigationToolbar::new(vec![NavigationToolbarItem::new(
        NavigationToolbarPlacement::Principal,
        button("Item").action(|| ()),
    )]);

    let _ = NavigationToolbarItem::action(
        NavigationToolbarPlacement::SecondaryAction,
        label("Share"),
        || (),
    );
    let _ = NavigationToolbarPlacement::PrimaryAction;
    let _ = NavigationToolbarPlacement::SecondaryAction;
    let _ = NavigationToolbarPlacement::Confirmation;
    let _ = NavigationToolbarPlacement::Cancellation;
    let _ = NavigationToolbarPlacement::BottomBar;
    let _ = NavigationToolbarPlacement::Status;
    let _ = NavigationToolbarPlacement::TopBarLeading;
    let _ = NavigationToolbarPlacement::TopBarTrailing;
}

// ---------------------------------------------------------------------------
// navigation.md § "## Transitions" — rust block 9/16
// Method-fragment listing applied to a navigation destination receiver.
// ---------------------------------------------------------------------------
pub fn navigation_block_09() {
    use waterui::id::Id;
    use waterui::navigation::navigation_transition;

    let id = Id::try_from(1_i32).expect("non-zero");

    let dest = message_list(Mail).title("Inbox");
    let _ = { dest.transition(navigation_transition::zoom(id)) };
    let dest = message_list(Mail).title("Inbox");
    let _ = { dest.transition(navigation_transition::fade()) };
    let dest = message_list(Mail).title("Inbox");
    let _ = { dest.transition(navigation_transition::none()) };
}

// ---------------------------------------------------------------------------
// navigation.md § "## Transitions" — rust block 10/16
// Listing: the two halves of a matched zoom.
// ---------------------------------------------------------------------------
pub fn navigation_block_10() {
    use waterui::id::Id;

    // The prose spells out the construction: `Id::try_from(index_i32 + 1)`.
    fn photo_transition(index: i32) -> Id {
        Id::try_from(index + 1).expect("non-zero")
    }
    let index = 3_i32;

    let tile = text("tile");
    let _ = { tile.navigation_transition_source(photo_transition(index)) };
    let hero = text("hero");
    let _ = { hero.navigation_transition_destination(photo_transition(index)) };
}

// ---------------------------------------------------------------------------
// navigation.md § "## Split views" — rust block 11/16
// ---------------------------------------------------------------------------
pub fn navigation_block_11() -> impl View {
    use waterui::navigation::{ColumnWidth, split_style};

    #[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
    enum Album {
        #[default]
        Recents,
    }
    fn album_sidebar(_sel: Binding<Option<Album>>) -> impl View {
        text("sidebar")
    }
    fn album_detail(_album: Album) -> NavigationView {
        vstack((text("detail"), Divider)).title("Detail")
    }

    use waterui::navigation::NavigationSplitView;

    let selection: Binding<Option<Album>> = binding(Some(Album::default()));
    let sidebar_selection = selection.clone();

    NavigationSplitView::new(
        &selection,
        move || album_sidebar(sidebar_selection.clone()),
        album_detail,
    )
    .sidebar_width(ColumnWidth::new(220.0, 280.0, 360.0)) // min, ideal, max
    .style(split_style::prominent_detail())
    .placeholder(|| text("No album selected"))
}

// ---------------------------------------------------------------------------
// navigation.md § "## Split views" (prose): `three_column`,
// `.column_visibility(..)` and the other split styles. Not counted as a rust
// block.
// ---------------------------------------------------------------------------
pub fn navigation_split_prose() -> impl View {
    use waterui::navigation::{NavigationSplitColumnVisibility, NavigationSplitView, split_style};

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Section {
        Recents,
    }
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Item {
        First,
    }

    let sidebar_selection: Binding<Option<Section>> = binding(Some(Section::Recents));
    let content_selection: Binding<Option<Item>> = binding(Some(Item::First));

    let _ = NavigationSplitView::new(&sidebar_selection, text("sidebar"), |_: Section| {
        vstack((text("detail"), Divider)).title("Detail")
    })
    .style(split_style::automatic())
    .style(split_style::balanced());

    NavigationSplitView::three_column(
        &sidebar_selection,
        &content_selection,
        text("sidebar"),
        |_: Section| vstack((text("content"), Divider)).title("Content"),
        |_: Item| vstack((text("detail"), Divider)).title("Detail"),
    )
    .column_visibility(NavigationSplitColumnVisibility::DoubleColumn)
}

// ---------------------------------------------------------------------------
// navigation.md § "## Windows" — rust block 12/16
// ---------------------------------------------------------------------------
#[expect(
    clippy::redundant_closure,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn navigation_block_12(env: Environment) -> App {
    fn scene() -> impl View {
        text("scene")
    }
    fn window_toolbar(_status: &Binding<Str>) -> impl View {
        hstack((button("Refresh"), Divider))
    }
    let status = Binding::container(Str::from("ready"));

    use waterui::window::{Window, WindowState, WindowStyle};

    let _: Option<WindowStyle> = None;

    App::new_with_windows(
        [Window::new(
            "WaterUI Menu Examples",
            binding(WindowState::Normal),
            move || scene(),
        )
        .toolbar(window_toolbar(&status))],
        env,
    )
}

// ---------------------------------------------------------------------------
// navigation.md § "## Windows" (prose): the `Window` builder — `.style(..)`
// with a `WindowStyle` or a `Binding<WindowStyle>`, `handle().set_style(..)`,
// `.background(..)` with a `Binding<WindowBackground>`, `set_background(..)`
// with a colour and a material, `.background(..)` with a Color or a Material,
// the three `WindowBackground` variants, `.icon(..)` with a `WindowIcon` or a
// `Binding<Option<WindowIcon>>`, `set_icon(..)`, `.resizable(bool)`,
// `.min_size(..)` / `.max_size(..)` (one `impl IntoComputed<Size>` each),
// `.level(..)` / `.resize_increments(..)`, `.title(..)`, `.placement(..)`,
// `.activation(..)`, `.app_id(..)` / `.instance_name(..)`, the `closable` and
// `attention` fields, the `WindowState` variants, and the `WindowHandle`
// controls. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn navigation_window_builder_prose() {
    use waterui::background::Material;
    use waterui::window::{
        Activation, Monitor, MonitorSelector, UserAttention, Window, WindowBackground, WindowIcon,
        WindowLevel, WindowState, WindowStyle,
    };

    let state = binding::<WindowState>(WindowState::default());

    let window = Window::new("W", state.clone(), || text("c"))
        .style(WindowStyle::Titled)
        .background(Color::transparent())
        .resizable(true)
        .min_size(Size::new(320.0, 240.0))
        .max_size(Size::new(1920.0, 1080.0))
        .level(WindowLevel::AlwaysOnTop)
        .resize_increments(Size::new(80.0, 24.0));
    let handle = window.handle();
    handle.maximize();
    handle.request_close();
    handle.request_attention(UserAttention::Informational);
    handle.cancel_attention();
    let _ = Window::new("W", state.clone(), || text("c"))
        .style(WindowStyle::Borderless)
        .background(Material::Regular);
    let style = binding(WindowStyle::Titled);
    let background = binding(WindowBackground::Opaque);
    let window = Window::new("W", state, || text("c"))
        .style(style.clone())
        .background(background.clone());
    window.handle().set_style(WindowStyle::Borderless);
    window
        .handle()
        .set_background(Color::srgb(0, 0, 0).with_opacity(0.8));
    window.handle().set_background(Material::UltraThin);
    background.set(WindowBackground::Material(Material::Thin));
    background.set(WindowBackground::Color(Color::transparent()));
    let icon = WindowIcon::new(1, 1, vec![255_u8, 0, 0, 255]).expect("a one-pixel RGBA icon");
    let themed_icon = binding(None::<WindowIcon>);
    let _ = Window::new("W", binding(WindowState::Closed), || text("c")).icon(icon.clone());
    let window =
        Window::new("W", binding(WindowState::Closed), || text("c")).icon(themed_icon.clone());
    window.handle().set_icon(icon);
    window.handle().set_icon(None);

    let mut window = Window::new("W", binding(WindowState::Closed), || text("c"))
        .title("Renamed")
        .icon(None::<WindowIcon>)
        .placement(MonitorSelector::Pointer, |monitor: &Monitor| {
            Rect::new(
                monitor.visible_frame().origin(),
                Size::new(monitor.visible_frame().width(), 320.0),
            )
        })
        .activation(Activation::OnClick)
        .app_id("cool.lexo.terminal")
        .instance_name("dropdown");
    window.closable = false;
    window.attention.set(Some(UserAttention::Critical));
    window.handle().focus();
    let _ = (
        MonitorSelector::Primary,
        MonitorSelector::Focused,
        Activation::OnShow,
        Activation::Never,
    );

    let _ = WindowStyle::FullSizeContentView;
    let _ = WindowState::Normal;
    let _ = WindowState::Closed;
    let _ = WindowState::Minimized;
    let _ = WindowState::Maximized;
    let _ = WindowState::Fullscreen;
}

// ---------------------------------------------------------------------------
// navigation.md § "## Windows" — rust block 13/16
// ---------------------------------------------------------------------------
pub fn navigation_block_13() -> impl View {
    use waterui::window::WindowState;

    let window_state = binding::<WindowState>(WindowState::default());

    button("Open Window")
        .action(|State(s): State<Binding<WindowState>>| s.set(WindowState::Normal))
        .state(&window_state)
}

// ---------------------------------------------------------------------------
// navigation.md § "## Windows" — rust block 14/16
// ---------------------------------------------------------------------------
pub fn navigation_block_14() -> waterui::window::Window {
    fn editor_content() -> impl View {
        text("editor")
    }

    use waterui::window::CloseReply;
    use waterui::window::{Window, WindowState};

    let unsaved = binding(true);
    // While set, the editor shows a "Discard changes?" prompt whose Discard button
    // writes `WindowState::Closed` — a close that does not ask again.
    let confirm_discard = binding(false);
    let window_state = binding(WindowState::Normal);

    Window::new("Editor", window_state.clone(), editor_content).on_close_request(move || {
        let ask = unsaved.snapshot();
        confirm_discard.set(ask);
        async move {
            if ask {
                CloseReply::Cancel
            } else {
                CloseReply::Close
            }
        }
    })
}

// ---------------------------------------------------------------------------
// navigation.md § "## Windows that open and close" — rust block 15/16
// ---------------------------------------------------------------------------
pub fn navigation_block_15() -> impl View {
    fn inspector_content() -> impl View {
        text("inspector")
    }

    use waterui::window::{
        Window, WindowPresentation, WindowState, WindowStyle, conditional_window,
    };

    let state = binding::<WindowState>(WindowState::default());
    let presentation = WindowPresentation::new(&state); // guards against duplicate opens

    // An INVISIBLE view — it must still be placed in the tree (e.g. a later zstack child).
    conditional_window(&presentation, |state| {
        Window::new("Inspector", state, inspector_content)
            .style(WindowStyle::Titled)
            .resizable(true)
    })
}

// ---------------------------------------------------------------------------
// navigation.md § "## Resident apps and windows on demand" — rust block 16/16
// ---------------------------------------------------------------------------
pub fn navigation_block_16(env: Environment) -> App {
    fn scratch_content() -> impl View {
        text("scratch")
    }

    use waterui::app::{App, LastWindowPolicy, QuitReply};
    use waterui::window::{Window, WindowState};

    fn app(env: Environment) -> App {
        App::new_with_windows(Vec::<Window>::new(), env)
            .on_last_window_closed(LastWindowPolicy::StayResident)
            .on_quit_request(|| async { QuitReply::Quit })
            .on_terminate(|| async {})
            .menu_bar(Menu::new(
                "App",
                "New Window".action(|env: Environment| {
                    Window::new("Scratch", binding(WindowState::Normal), scratch_content)
                        .show(&env);
                }),
            ))
    }

    app(env)
}
