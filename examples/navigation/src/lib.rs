//! A tour of the navigation chrome each platform draws for itself.
//!
//! Nothing here paints a bar, a tab strip, or a sidebar. Every piece of chrome
//! in this app is a *semantic* declaration that each backend projects into its
//! own platform object:
//!
//! | Declared here | Apple | Android | GTK | Hydrolysis |
//! |---|---|---|---|---|
//! | `Tabs` | `UITabBarController` / `NSTabViewController` | bottom navigation or navigation rail | notebook | retained tab bar |
//! | `NavigationStack` | navigation controller, interactive pop | fragment back stack, predictive back | stack + header bars | retained GPU pages |
//! | `NavigationSplitView` | `UISplitViewController` | `SlidingPaneLayout`, fold-aware | paned | adaptive columns |
//! | `.navigation_toolbar(…)` | bar button items and bottom bar | app-bar actions and overflow | header-bar packing | themed bar slots |
//! | `.searchable(…)` | `UISearchController` in the bar | Material search bar | header-bar search | bar search field |
//! | `List` with sections | grouped `UITableView`, swipe-to-delete | Material list, drag handles | list box | themed rows |
//!
//! The style axis works the same way: `.large_title()` collapses on scroll on
//! iOS and becomes a large app bar on Android, and `tab_style::automatic()`
//! resolves to a tab bar on a phone and a sidebar on a desktop-class window.
//! None of that is decided by this file.
//!
//! Each tab owns its own navigation container (`Tab::container`), which is what
//! makes a tab remember where you left it: push twice in Inbox, visit Settings,
//! come back, and the two pushed pages are still there.

use waterui::Identifiable;
use waterui::app::App;
use waterui::component::list::{List, ListDelete, ListItem, ListMove, Section, row};
use waterui::id::Id;
use waterui::navigation::{NavigationSplitView, NavigationView, Navigator};
use waterui::prelude::theme_color::{Accent, Foreground, MutedForeground, SurfaceVariant};
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::reactive::collection::SignalCollection;
use waterui::shape::{Circle, RoundedRectangle};
use waterui_icons_material_icon as mdi;

// ---------------------------------------------------------------------------
// Tab identity and stack routes
// ---------------------------------------------------------------------------

/// Tab identity. `Tabs` is generic over it, so no `Id` reaches app code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pane {
    Inbox,
    Library,
    Gallery,
    Settings,
}

/// Everything the Inbox stack can push. Routes are values, so the whole stack
/// is inspectable and restorable without touching views.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MailRoute {
    Message(u64),
    Compose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PhotoRoute(usize);

#[derive(Debug, Clone, PartialEq, Eq)]
enum SettingsRoute {
    Appearance,
    About,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Album {
    Recents,
    Favorites,
    Shared,
}

impl Album {
    const fn title(self) -> &'static str {
        match self {
            Self::Recents => "Recents",
            Self::Favorites => "Favorites",
            Self::Shared => "Shared with You",
        }
    }

    const fn count(self) -> i32 {
        match self {
            Self::Recents => 128,
            Self::Favorites => 12,
            Self::Shared => 41,
        }
    }
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Identifiable)]
struct Message {
    #[id]
    id: u64,
    sender: Str,
    subject: Str,
    preview: Str,
    unread: bool,
    flagged: bool,
}

fn seed_messages() -> Vec<Message> {
    [
        (
            "Ada Lovelace",
            "Analytical engine notes",
            "The engine weaves algebraic patterns.",
        ),
        (
            "Grace Hopper",
            "Compiler timings",
            "Shaved another pass off the linker.",
        ),
        (
            "Alan Kay",
            "On messaging",
            "The big idea is messaging, not objects.",
        ),
        (
            "Barbara Liskov",
            "Substitution review",
            "Subtypes must not surprise their callers.",
        ),
        (
            "Ken Thompson",
            "Pipes",
            "One tool, one job, composed by the shell.",
        ),
        (
            "Margaret Hamilton",
            "Priority displays",
            "Overload handling saved the landing.",
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (sender, subject, preview))| Message {
        id: index as u64,
        sender: Str::from(sender),
        subject: Str::from(subject),
        preview: Str::from(preview),
        unread: index % 2 == 0,
        flagged: false,
    })
    .collect()
}

/// State owned above every navigation container, so pushing a page, switching
/// tabs, or rebuilding a destination never loses it.
#[derive(Clone)]
struct Mail {
    messages: Binding<Vec<Message>>,
    query: Binding<Str>,
    editing: Binding<bool>,
    draft_subject: Binding<Str>,
    draft_body: Binding<Str>,
}

impl Mail {
    fn new() -> Self {
        Self {
            messages: binding(seed_messages()),
            query: binding(Str::default()),
            editing: binding(false),
            draft_subject: binding(Str::default()),
            draft_body: binding(Str::default()),
        }
    }

    /// The rows the list shows: a signal-backed collection, so typing in the
    /// search field diffs rows by id instead of rebuilding the list.
    fn visible(&self) -> SignalCollection<Computed<Vec<Message>>> {
        SignalCollection::new(
            self.messages
                .clone()
                .zip(&self.query)
                .map(|(messages, query)| filter_messages(&messages, &query))
                .computed(),
        )
    }

    fn unread_count(&self) -> Computed<i32> {
        self.messages
            .clone()
            .map(|messages| messages.iter().filter(|message| message.unread).count() as i32)
            .computed()
    }

    fn message(&self, id: u64) -> Computed<Option<Message>> {
        self.messages
            .clone()
            .map(move |messages| messages.iter().find(|message| message.id == id).cloned())
            .computed()
    }

    fn update(&self, id: u64, edit: impl Fn(&mut Message)) {
        let mut messages = self.messages.get_mut();
        if let Some(message) = messages.iter_mut().find(|message| message.id == id) {
            edit(message);
        }
    }

    fn draft_is_empty(&self) -> Computed<bool> {
        self.draft_subject
            .clone()
            .zip(&self.draft_body)
            .map(|(subject, body)| subject.is_empty() && body.is_empty())
            .computed()
    }

    fn discard_draft(&self) {
        self.draft_subject.set(Str::default());
        self.draft_body.set(Str::default());
    }
}

fn filter_messages(messages: &[Message], query: &str) -> Vec<Message> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return messages.to_vec();
    }
    messages
        .iter()
        .filter(|message| {
            message.sender.to_ascii_lowercase().contains(&needle)
                || message.subject.to_ascii_lowercase().contains(&needle)
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Root: one tab per navigation container
// ---------------------------------------------------------------------------

/// The settings pane on its own.
///
/// Each tab gets a preview of its own so its chrome can be looked at without
/// driving the running application to that tab — the pane a bug is in is the
/// pane that has to be rendered to see it.
#[preview]
pub fn settings_pane() -> impl View {
    settings_stack()
}

/// The library pane on its own, for the same reason.
#[preview]
pub fn library_pane() -> impl View {
    library_split()
}

#[preview]
pub fn demo() -> impl View {
    let mail = Mail::new();
    let pane = binding(Pane::Inbox);

    Tabs::new(
        &pane,
        vec![
            Tab::container(Pane::Inbox, label("Inbox").icon(mdi::inbox()), {
                let mail = mail.clone();
                move || inbox_stack(mail.clone())
            })
            .badge(mail.unread_count()),
            Tab::container(
                Pane::Library,
                label("Library").icon(mdi::image_album()),
                library_split,
            ),
            Tab::container(
                Pane::Gallery,
                label("Gallery").icon(mdi::view_gallery()),
                gallery_stack,
            ),
            Tab::container(
                Pane::Settings,
                label("Settings").icon(mdi::cog()),
                settings_stack,
            ),
        ],
    )
    // A tab bar on a phone, a sidebar or navigation rail on a large window: the
    // backend decides from the platform and the current window size.
    .style(tab_style::automatic())
}

// ---------------------------------------------------------------------------
// Inbox: bar chrome, search, and a native list
// ---------------------------------------------------------------------------

fn inbox_stack(mail: Mail) -> impl View {
    NavigationStack::with_path(NavigationPath::<MailRoute>::new(), inbox_root(mail.clone()))
        .destination(move |route| match route {
            MailRoute::Message(id) => message_detail(mail.clone(), id),
            MailRoute::Compose => compose_page(mail.clone()),
        })
}

fn inbox_root(mail: Mail) -> NavigationView {
    let unread = mail.unread_count();
    let edit_title = mail
        .editing
        .clone()
        .select(Str::from("Done"), Str::from("Edit"));

    message_list(mail.clone())
        .title("Inbox")
        // Starts large and collapses as the list scrolls: the iOS large-title
        // bar, Android's large app bar.
        .large_title()
        .navigation_subtitle(text!("{unread} unread"))
        // A search field inside the bar chrome, not a text field drawn above
        // the content.
        .searchable(&mail.query, "Search mail")
        .navigation_toolbar(
            NavigationToolbar::default()
                // Leading: the edit affordance, where each platform puts it.
                .item(NavigationToolbarItem::new(
                    NavigationToolbarPlacement::TopBarLeading,
                    button(text!("{edit_title}"))
                        .style(ButtonStyle::Plain)
                        .action(|State(editing): State<Binding<bool>>| {
                            editing.set(!editing.get());
                        })
                        .state(&mail.editing),
                ))
                // Primary: the action the screen exists for.
                .item(NavigationToolbarItem::action(
                    NavigationToolbarPlacement::PrimaryAction,
                    label("Compose").icon(mdi::pencil()),
                    |navigator: Navigator<MailRoute>| navigator.push(MailRoute::Compose),
                ))
                // Bottom bar and status: iOS uses the bottom toolbar, Android
                // the app bar's overflow and subtitle areas.
                .item(NavigationToolbarItem::new(
                    NavigationToolbarPlacement::BottomBar,
                    button("Mark All Read")
                        .style(ButtonStyle::Plain)
                        .action(mark_all_read)
                        .state(&mail),
                ))
                .item(NavigationToolbarItem::new(
                    NavigationToolbarPlacement::Status,
                    text!("{unread} unread")
                        .caption()
                        .foreground(MutedForeground),
                )),
        )
}

fn mark_all_read(State(mail): State<Mail>) {
    for message in mail.messages.get_mut().iter_mut() {
        message.unread = false;
    }
}

/// A native list rather than a stack of rows: lazily realized, swipe-to-delete,
/// and drag-to-reorder in edit mode, all drawn by the platform's own list.
fn message_list(mail: Mail) -> impl View {
    List::for_each(mail.visible(), message_row)
        .editing(mail.editing.clone())
        .on_delete(delete_message)
        .on_move(move_message)
}

fn message_row(message: Message) -> ListItem {
    let id = message.id;
    let announced = Str::from(format!("{}: {}", message.sender, message.subject));

    ListItem::new(NavigationLink::value(
        // A row that reads as one thing to assistive technology and shows
        // three lines on screen: `Label::new` keeps both, and requires the
        // spoken text rather than guessing it from the layout.
        Label::new(announced, move || {
            let message = message.clone();
            // The unread dot hangs beside the text rather than sitting inside
            // it, so every row's text starts at the same place whether or not
            // it has one — a row that indents itself because it is unread is
            // what a mail list must not do.
            hstack((
                hstack((message.unread.then(|| Accent.size(8.0, 8.0).clip(Circle)),))
                    .size(8.0, 8.0),
                vstack((
                    hstack((
                        text(message.sender).sub_headline().foreground(Foreground),
                        spacer(),
                        message
                            .flagged
                            .then(|| mdi::flag().size(14.0, 14.0).foreground(Accent)),
                    ))
                    .spacing(6.0),
                    text(message.subject).body().foreground(Foreground),
                    text(message.preview).caption().foreground(MutedForeground),
                ))
                .alignment(HorizontalAlignment::Leading)
                .spacing(2.0),
            ))
            .alignment(VerticalAlignment::Top)
            .spacing(6.0)
            .padding_with(EdgeInsets::symmetric(8.0, 0.0))
        }),
        MailRoute::Message(id),
    ))
}

fn delete_message(ListDelete(index): ListDelete, State(mail): State<Mail>) {
    let Some(id) = visible_id(&mail, index) else {
        return;
    };
    mail.messages.get_mut().retain(|message| message.id != id);
}

fn move_message(ListMove(movement): ListMove, State(mail): State<Mail>) {
    let (Some(moved), Some(target)) = (
        visible_id(&mail, movement.from()),
        visible_id(&mail, movement.to()),
    ) else {
        return;
    };
    let mut messages = mail.messages.get_mut();
    let (Some(from), Some(to)) = (
        messages.iter().position(|message| message.id == moved),
        messages.iter().position(|message| message.id == target),
    ) else {
        return;
    };
    let message = messages.remove(from);
    messages.insert(to, message);
}

/// Rows are addressed by position in the *filtered* list, so an edit maps back
/// through the same filter the user is looking at.
fn visible_id(mail: &Mail, index: usize) -> Option<u64> {
    let messages = mail.messages.get();
    let query = mail.query.get();
    let visible = filter_messages(&messages, &query);
    visible.as_slice().get(index).map(|message| message.id)
}

fn message_detail(mail: Mail, id: u64) -> NavigationView {
    let message = mail.message(id);
    let subject = message
        .clone()
        .map(|message| message.map_or_else(Str::default, |message| message.subject));
    let sender = message
        .clone()
        .map(|message| message.map_or_else(Str::default, |message| message.sender));
    let body = message
        .clone()
        .map(|message| message.map_or_else(Str::default, |message| message.preview));
    let flagged = message.map(|message| message.is_some_and(|message| message.flagged));
    let flag_title = flagged.select(Str::from("Unflag"), Str::from("Flag"));

    scroll(
        vstack((
            text!("{sender}").sub_headline().foreground(MutedForeground),
            text!("{body}").body().foreground(Foreground),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(12.0)
        .padding_with(EdgeInsets::all(16.0)),
    )
    .title(text!("{subject}"))
    // A pushed page keeps its title inline, the way a platform back stack does.
    .inline_title()
    .navigation_toolbar(
        NavigationToolbar::default().item(NavigationToolbarItem::new(
            NavigationToolbarPlacement::SecondaryAction,
            button(text!("{flag_title}"))
                .style(ButtonStyle::Plain)
                .action(move |State(mail): State<Mail>| {
                    mail.update(id, |message| message.flagged = !message.flagged);
                })
                .state(&mail),
        )),
    )
    // Opening the message is what marks it read: a destination lifecycle event,
    // not something the row's tap handler has to guess at.
    .on_navigation_appear({
        let mail = mail.clone();
        move || mail.update(id, |message| message.unread = false)
    })
}

/// An editing flow: confirmation and cancellation are semantic placements, and
/// the platform's own back gesture is refused while the draft is dirty.
fn compose_page(mail: Mail) -> NavigationView {
    let is_empty = mail.draft_is_empty();

    vstack((
        field("Subject", &mail.draft_subject),
        field("Message", &mail.draft_body),
        text(
            "Leaving is blocked while this draft has content: try the back \
             button or the platform back gesture, then use Cancel.",
        )
        .caption()
        .foreground(MutedForeground),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(12.0)
    .padding_with(EdgeInsets::all(16.0))
    .title("New Message")
    .inline_title()
    .navigation_toolbar(
        NavigationToolbar::default()
            .item(NavigationToolbarItem::new(
                NavigationToolbarPlacement::Confirmation,
                button("Send")
                    .style(ButtonStyle::BorderedProminent)
                    .action(send_draft)
                    .state(&mail)
                    .disabled(is_empty.clone()),
            ))
            .item(NavigationToolbarItem::new(
                NavigationToolbarPlacement::Cancellation,
                button("Cancel")
                    .style(ButtonStyle::Plain)
                    .action(cancel_draft)
                    .state(&mail),
            )),
    )
    .navigation_pop_enabled(is_empty)
    .on_navigation_pop_attempted(|State(manager): State<SnackbarManager>| {
        manager.show(Snackbar::new("Discard the draft with Cancel first"));
    })
}

fn send_draft(State(mail): State<Mail>, State(navigator): State<Navigator<MailRoute>>) {
    let messages = mail.messages.get();
    let next_id = messages
        .as_slice()
        .iter()
        .map(|message| message.id)
        .max()
        .map_or(0, |id| id + 1);
    drop(messages);
    mail.messages.get_mut().insert(
        0,
        Message {
            id: next_id,
            sender: Str::from("Me"),
            subject: mail.draft_subject.get(),
            preview: mail.draft_body.get(),
            unread: false,
            flagged: false,
        },
    );
    mail.discard_draft();
    let _ = navigator.pop();
}

fn cancel_draft(State(mail): State<Mail>, State(navigator): State<Navigator<MailRoute>>) {
    mail.discard_draft();
    let _ = navigator.pop();
}

// ---------------------------------------------------------------------------
// Library: an adaptive split — two columns or one, per window size
// ---------------------------------------------------------------------------

fn library_split() -> impl View {
    let selection = binding(Some(Album::Recents));
    let sidebar_selection = selection.clone();

    NavigationSplitView::new(
        &selection,
        move || album_sidebar(sidebar_selection.clone()),
        album_detail,
    )
    .sidebar_width(ColumnWidth::new(220.0, 280.0, 360.0))
    .style(split_style::prominent_detail())
    .placeholder(placeholder)
}

fn placeholder() -> impl View {
    vstack((
        text("No album selected")
            .sub_headline()
            .foreground(Foreground),
        text("Pick one from the sidebar.")
            .caption()
            .foreground(MutedForeground),
    ))
    .spacing(6.0)
}

fn album_sidebar(selection: Binding<Option<Album>>) -> impl View {
    List::content((Section::new("Albums").content((
        album_row(Album::Recents, &selection),
        album_row(Album::Favorites, &selection),
        album_row(Album::Shared, &selection),
    )),))
}

/// Static list content is built from row *builders*, so the platform list can
/// rematerialize a row whenever it scrolls back into view.
fn album_row(album: Album, selection: &Binding<Option<Album>>) -> impl Fn() -> ListItem + 'static {
    let count = album.count();
    let selection = selection.clone();

    move || {
        let tap_selection = selection.clone();
        ListItem::new(
            hstack((
                mdi::album().size(20.0, 20.0).foreground(Accent),
                text(album.title()).body().foreground(Foreground),
                spacer(),
                text!("{count}").caption().foreground(MutedForeground),
            ))
            .spacing(10.0)
            .padding_with(EdgeInsets::symmetric(10.0, 12.0))
            .on_tap(move || tap_selection.set(Some(album))),
        )
        // The platform draws its own selection chrome; the row only derives
        // its flag from the state that owns selection.
        .selected(selection.clone().map(move |current| current == Some(album)))
    }
}

fn album_detail(album: Album) -> NavigationView {
    let count = album.count();

    scroll(
        vstack((
            text!("{count} photos").body().foreground(MutedForeground),
            text(
                "On a wide window this is the trailing column beside the \
                 sidebar; on a phone the same declaration collapses into a \
                 pushed page with a back button.",
            )
            .caption()
            .foreground(MutedForeground),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(10.0)
        .padding_with(EdgeInsets::all(16.0)),
    )
    .title(album.title())
}

// ---------------------------------------------------------------------------
// Gallery: a matched zoom transition between the grid and the photo
// ---------------------------------------------------------------------------

/// One stable id per photo ties a tapped tile to the page it opens.
///
/// A matched pair belongs to one destination, not to the stack: tile 3 and the
/// page it opens share an identity that tile 4 does not. So every photo gets its
/// own id, and each page names the one it arrives by.
fn photo_transition(index: usize) -> Id {
    Id::try_from(i32::try_from(index).expect("photo index must fit an id") + 1)
        .expect("transition id must be non-zero")
}

fn gallery_stack() -> impl View {
    NavigationStack::with_path(NavigationPath::<PhotoRoute>::new(), gallery_root())
        .destination(|PhotoRoute(index)| photo_page(index))
}

fn gallery_root() -> NavigationView {
    let tiles: VStack<_> = (0..6).map(photo_tile).collect();

    scroll(tiles.spacing(12.0).padding_with(EdgeInsets::all(16.0)))
        .title("Gallery")
        .large_title()
}

fn photo_tile(index: usize) -> AnyView {
    let tile = NavigationLink::value(
        Label::new(format!("Photo {index}"), move || {
            zstack((
                photo_color(index).size(160.0, 120.0),
                text!("Photo {index}").sub_headline().foreground(Foreground),
            ))
            .clip(RoundedRectangle::new(0.1))
        }),
        PhotoRoute(index),
    );

    tile.navigation_transition_source(photo_transition(index))
        .anyview()
}

fn photo_page(index: usize) -> NavigationView {
    vstack((
        photo_color(index)
            .height(280.0)
            // A `RoundedRectangle` radius is a fraction of the shorter side,
            // not a point value.
            .clip(RoundedRectangle::new(0.06))
            .navigation_transition_destination(photo_transition(index)),
        text(
            "Tapping a tile matches this hero to that tile: the backend runs \
             its native zoom transition, and the platform's interactive back \
             gesture drives the same transition in reverse.",
        )
        .caption()
        .foreground(MutedForeground),
    ))
    .spacing(12.0)
    .padding_with(EdgeInsets::all(16.0))
    .title(format!("Photo {index}"))
    .inline_title()
    // The declaration lives on the destination, because the pair it names does.
    .transition(navigation_transition::zoom(photo_transition(index)))
}

/// Six flat swatches standing in for photographs.
fn photo_color(index: usize) -> impl View {
    const SWATCHES: [Srgb; 6] = [
        Srgb::from_hex("#4A84F6"),
        Srgb::from_hex("#8B5CF6"),
        Srgb::from_hex("#06B6D4"),
        Srgb::from_hex("#F97316"),
        Srgb::from_hex("#EC4899"),
        Srgb::from_hex("#22C55E"),
    ];
    SWATCHES[index % SWATCHES.len()].with_opacity(0.35)
}

// ---------------------------------------------------------------------------
// Settings: grouped sections and pushed sub-pages
// ---------------------------------------------------------------------------

fn settings_stack() -> impl View {
    let notifications = binding(true);
    let compact = binding(false);

    NavigationStack::with_path(
        NavigationPath::<SettingsRoute>::new(),
        settings_root(&notifications, &compact),
    )
    .destination(move |route| match route {
        SettingsRoute::Appearance => appearance_page(compact.clone()),
        SettingsRoute::About => about_page(),
    })
}

fn settings_root(notifications: &Binding<bool>, compact: &Binding<bool>) -> NavigationView {
    // `List::content` with sections is the grouped list every platform already
    // has: inset-grouped on iOS, section headers on macOS, Material groups on
    // Android.
    let notifications = notifications.clone();
    let compact = compact.clone();

    List::content((
        Section::new("General").content((
            move || ListItem::new(Toggle::new("Notifications", &notifications)),
            move || ListItem::new(Toggle::new("Compact rows", &compact)),
        )),
        Section::new("More")
            .footer("Sub-pages push onto this tab's own stack.")
            .content((
                || settings_link("Appearance", SettingsRoute::Appearance),
                || settings_link("About", SettingsRoute::About),
            )),
    ))
    .title("Settings")
    .large_title()
}

fn settings_link(title: &'static str, route: SettingsRoute) -> ListItem {
    // No disclosure indicator here: whether a row that pushes shows one is the
    // platform's own convention — iOS draws a chevron, a Mac does not — so the
    // link decides, not the application.
    ListItem::new(NavigationLink::value(title, route))
}

fn appearance_page(compact: Binding<bool>) -> NavigationView {
    let density = compact
        .clone()
        .select(Str::from("Compact"), Str::from("Comfortable"));

    List::content((Section::new("Density")
        .footer("The row height the message list uses.")
        .content((
            move || ListItem::new(Toggle::new("Compact rows", &compact)),
            row("Current", text!("{density}")),
        )),))
    .title("Appearance")
    .inline_title()
}

fn about_page() -> NavigationView {
    vstack((
        text("Navigation chrome tour")
            .sub_headline()
            .foreground(Foreground),
        text(
            "Every bar, tab strip, sidebar, search field, and transition in \
             this app is declared once and drawn by each platform itself.",
        )
        .body()
        .foreground(MutedForeground),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(10.0)
    .padding_with(EdgeInsets::all(16.0))
    .background(SurfaceVariant)
    .title("About")
    .inline_title()
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
