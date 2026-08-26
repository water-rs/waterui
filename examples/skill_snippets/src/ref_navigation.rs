//! Snippets from `.claude/skills/waterui/references/navigation.md`, in file
//! order. Transcription conventions are documented in the crate README.

use waterui::prelude::*;
use waterui::reactive::binding;
use waterui_icons_material_icon as mdi;

/// Glue: the app model navigation.md's snippets thread through their views.
#[derive(Clone)]
pub struct Mail;

impl Mail {
    fn send_draft(&self) {}
    fn mark_read(&self, _id: u64) {}
}
waterui::impl_extractor!(Mail);

/// Glue: the settings route navigation.md refers to.
#[derive(Clone, PartialEq, Eq)]
pub enum SettingsRoute {
    Appearance,
}

/// Glue: blocks 3, 4 and 7 name `MailRoute` outside the module that declares
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

// ---------------------------------------------------------------------------
// navigation.md § "## Tabs" — rust block 1/13
// ---------------------------------------------------------------------------
pub fn navigation_block_01() -> impl View {
    use waterui::navigation::tab_style;

    fn inbox_stack() -> impl View {
        text("inbox stack")
    }
    let unread_count = Binding::i32(3);

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Pane {
        Inbox,
        Library,
        Settings,
    }

    let pane = binding(Pane::Inbox);

    Tabs::new(
        &pane,
        vec![
            Tab::container(Pane::Inbox, label("Inbox").icon(mdi::inbox()), inbox_stack)
                .badge(unread_count),
            Tab::container(
                Pane::Library,
                label("Library").icon(mdi::image_album()),
                library_split,
            ),
            Tab::container(
                Pane::Settings,
                label("Settings").icon(mdi::cog()),
                settings_stack,
            ),
        ],
    )
    .style(tab_style::automatic())
}

// ---------------------------------------------------------------------------
// navigation.md § "## Navigation stacks and routes" — rust block 2/13
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
// navigation.md § "## Navigation stacks and routes" — rust block 3/13
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
// navigation.md § "## Going back, and destination lifecycle" — rust block 4/13
// ---------------------------------------------------------------------------
pub fn send_draft(State(mail): State<Mail>, navigator: Navigator<MailRoute>) {
    mail.send_draft();
    let _ = navigator.pop(); // returns Option<T> and is #[must_use] — bind it
}

// ---------------------------------------------------------------------------
// navigation.md § "## Going back, and destination lifecycle" — rust block 5/13
// ---------------------------------------------------------------------------
pub fn navigation_block_05() -> impl View {
    let mail = Mail;
    let id = 7_u64;
    let subject = Binding::container(Str::from("Re: hello"));

    message_detail(mail.clone(), id)
        .title(text!("{subject}"))
        .on_navigation_appear({
            let mail = mail.clone();
            move || mail.mark_read(id)
        })
}

// ---------------------------------------------------------------------------
// navigation.md § "## Bar chrome on a destination" — rust block 6/13
// ---------------------------------------------------------------------------
pub fn navigation_block_06() -> NavigationView {
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
            .on_navigation_pop_attempted(|State(m): State<SnackbarManager>| {
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
}

// ---------------------------------------------------------------------------
// navigation.md § "## Toolbars" — rust block 7/13
// A bare method fragment; applied to a navigation destination receiver.
// ---------------------------------------------------------------------------
pub fn navigation_block_07() -> NavigationView {
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
// navigation.md § "## Transitions" — rust block 8/13
// Method-fragment listing applied to a navigation destination receiver.
// ---------------------------------------------------------------------------
pub fn navigation_block_08() {
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
// navigation.md § "## Transitions" — rust block 9/13
// Listing: the two halves of a matched zoom.
// ---------------------------------------------------------------------------
pub fn navigation_block_09() {
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
// navigation.md § "## Split views" — rust block 10/13
// ---------------------------------------------------------------------------
pub fn navigation_block_10() -> impl View {
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
// navigation.md § "## Windows" — rust block 11/13
// ---------------------------------------------------------------------------
#[expect(
    clippy::redundant_closure,
    reason = "the snippet is transcribed verbatim from the skill; rewriting it to satisfy the lint would defeat this crate's purpose"
)]
pub fn navigation_block_11(env: Environment) -> waterui::app::App {
    use waterui::app::App;

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
// navigation.md § "## Windows" (prose): the `Window` builder — `.style(..)`,
// `.background(..)` with a Color or a Material, `.resizable(bool)`,
// `.min_size(..)` / `.max_size(..)` (one `impl IntoComputed<Size>` each), and
// the `WindowState` variants. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn navigation_window_builder_prose() {
    use waterui::background::Material;
    use waterui::window::{Window, WindowState, WindowStyle};

    let state = binding::<WindowState>(WindowState::default());

    let _ = Window::new("W", state.clone(), || text("c"))
        .style(WindowStyle::Titled)
        .background(Color::transparent())
        .resizable(true)
        .min_size(Size::new(320.0, 240.0))
        .max_size(Size::new(1920.0, 1080.0));
    let _ = Window::new("W", state, || text("c"))
        .style(WindowStyle::Borderless)
        .background(Material::Regular);

    let _ = WindowStyle::FullSizeContentView;
    let _ = WindowState::Normal;
    let _ = WindowState::Closed;
    let _ = WindowState::Minimized;
    let _ = WindowState::Fullscreen;
}

// ---------------------------------------------------------------------------
// navigation.md § "## Windows" — rust block 12/13
// ---------------------------------------------------------------------------
pub fn navigation_block_12() -> impl View {
    use waterui::window::WindowState;

    let window_state = binding::<WindowState>(WindowState::default());

    button("Open Window")
        .action(|State(s): State<Binding<WindowState>>| s.set(WindowState::Normal))
        .state(&window_state)
}

// ---------------------------------------------------------------------------
// navigation.md § "## Windows that open and close" — rust block 13/13
// ---------------------------------------------------------------------------
pub fn navigation_block_13() -> impl View {
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
