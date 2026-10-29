//! Reply — a list-detail mail client built only from standard WaterUI
//! components.
//!
//! `Tabs` carries the destinations, a `NavigationSplitView` pairs the inbox
//! `List` with the reading pane, and every bar, search field, card and button
//! is a semantic component. Nothing here picks a Material color, shape, font
//! or padding: the style package installed on the backend decides how the
//! components look.

use core::num::NonZeroUsize;

use waterui::Identifiable;
use waterui::app::App;
use waterui::component::list::{List, ListItem};
use waterui::layout::ContentMode;
use waterui::media::Photo;
use waterui::navigation::{NavigationSplitView, NavigationStack, NavigationView};
use waterui::prelude::*;
use waterui::reactive::collection::SignalCollection;
use waterui::widget::avatar;
use waterui::widget::card;
use waterui_icons_material_icon as mdi;

use mdi::dots_vertical;
use mdi::email_outline;
use mdi::message_outline;
use mdi::note_outline;
use mdi::pencil;
use mdi::star_outline;
use mdi::trash_can_outline;
use mdi::video_outline;

const AVATAR: f32 = 40.0;
const TOOLBAR_AVATAR: f32 = 32.0;
/// The tallest a thread photo grows inside its list row.
const PHOTO_HEIGHT: f32 = 160.0;
/// Unread mail shown on the Mail destination's badge.
const UNREAD: i32 = 4;

const ASSETS: &str = "https://raw.githubusercontent.com/android/compose-samples/main/Reply/app/src/main/res/drawable";

fn asset(name: &str) -> Url {
    format!("{ASSETS}/{name}")
        .parse()
        .expect("sample asset URL is valid")
}

#[derive(PartialEq)]
struct Message {
    sender: &'static str,
    time: &'static str,
    recipients: &'static str,
    body: &'static str,
    signature: Option<&'static str>,
    avatar_asset: &'static str,
}

#[derive(Clone, PartialEq, Identifiable)]
struct Thread {
    #[id]
    id: u64,
    sender: &'static str,
    time: &'static str,
    subject: &'static str,
    snippet: &'static str,
    avatar_asset: &'static str,
    photo_asset: Option<&'static str>,
    messages: &'static [Message],
}

const DINNER_CLUB_MESSAGES: &[Message] = &[
    Message {
        sender: "So Duri",
        time: "20 min ago",
        recipients: "To me, Ziad, and Lily",
        body: "I think it's time for us to finally try that new noodle shop downtown that doesn't use menus. Anyone else have other suggestions for dinner club this week? I'm so intrigued by this idea of a noodle restaurant where no one gets to order for themselves – could be fun, or terrible, or both :)",
        signature: Some("So"),
        avatar_asset: "avatar_3.jpg",
    },
    Message {
        sender: "Me",
        time: "4 min ago",
        recipients: "To me, Ziad, and Lily",
        body: "Yes! I forgot about that place! I'm definitely up for taking a risk this week and handing control over to someone else. Let's do it.",
        signature: None,
        avatar_asset: "avatar_10.jpg",
    },
    Message {
        sender: "Lily MacDonald",
        time: "1 hour ago",
        recipients: "To me, Ziad, and So",
        body: "Count me in! I've been wanting to try that place since it opened. Thursday works best for me.",
        signature: Some("Lily"),
        avatar_asset: "avatar_1.jpg",
    },
];

const DOUHUA_MESSAGES: &[Message] = &[Message {
    sender: "老强",
    time: "10 min ago",
    recipients: "To me",
    body: "最近忙吗？昨晚我去了你最爱的那家饭馆，点了他们的特色豆花鱼，吃着吃着就想你了。有空过来，我请你吃。",
    signature: Some("老强"),
    avatar_asset: "avatar_8.jpg",
}];

const FOOD_SHOW_MESSAGES: &[Message] = &[Message {
    sender: "Lily MacDonald",
    time: "2 hours ago",
    recipients: "To me and Karthik",
    body: "Ping– you'd love this new food show I started watching. It's produced by a Thai drummer who started a noodle cart during lockdowns, and every episode ends with a cook-along. Attached a still from last night's episode.",
    signature: Some("Lily"),
    avatar_asset: "avatar_1.jpg",
}];

const THREADS: &[Thread] = &[
    Thread {
        id: 1,
        sender: "老强",
        time: "10 min ago",
        subject: "豆花鱼",
        snippet: "最近忙吗？昨晚我去了你最爱的那家饭馆，点了他们的特色豆花鱼，吃着吃着就想你了。",
        avatar_asset: "avatar_8.jpg",
        photo_asset: None,
        messages: DOUHUA_MESSAGES,
    },
    Thread {
        id: 2,
        sender: "So Duri",
        time: "20 min ago",
        subject: "Dinner Club",
        snippet: "I think it's time for us to finally try that new noodle shop downtown that doesn't use me\u{2026}",
        avatar_asset: "avatar_3.jpg",
        photo_asset: None,
        messages: DINNER_CLUB_MESSAGES,
    },
    Thread {
        id: 3,
        sender: "Lily MacDonald",
        time: "2 hours ago",
        subject: "This food show is made for you",
        snippet: "Ping– you'd love this new food show I started watching. It's produced by a Thai drummer\u{2026}",
        avatar_asset: "avatar_1.jpg",
        photo_asset: Some("paris_3.jpg"),
        messages: FOOD_SHOW_MESSAGES,
    },
];

/// Threads whose sender, subject or snippet contain `query`, ignoring case.
fn matching_threads(query: &str) -> Vec<Thread> {
    let query = query.to_lowercase();
    THREADS
        .iter()
        .filter(|thread| {
            [thread.sender, thread.subject, thread.snippet]
                .iter()
                .any(|field| field.to_lowercase().contains(&query))
        })
        .cloned()
        .collect()
}

fn thread_by_id(id: u64) -> &'static Thread {
    THREADS
        .iter()
        .find(|thread| thread.id == id)
        .expect("the selection only holds ids of listed threads")
}

fn star() -> impl View {
    button(label("Star").icon(star_outline()))
        .borderless()
        .label_style(LabelDisplayMode::IconOnly)
}

fn thread_row(thread: Thread) -> ListItem {
    ListItem::new(Label::new(thread.subject, move || {
        vstack((
            hstack((
                avatar(thread.sender)
                    .image(asset(thread.avatar_asset))
                    .size(AVATAR),
                vstack((
                    text(thread.sender).sub_headline(),
                    text(thread.time).caption().muted(),
                ))
                .leading(),
                spacer(),
                star(),
            )),
            text(thread.subject).body(),
            text(thread.snippet)
                .caption()
                .line_limit(NonZeroUsize::new(2).expect("2 is non-zero"))
                .muted(),
            thread.photo_asset.map(|name| {
                Photo::new(asset(name))
                    .resizable()
                    .content_mode(ContentMode::Fill)
                    .max_height(PHOTO_HEIGHT)
            }),
        ))
        .leading()
    }))
}

/// The inbox column: the thread list under a titled bar that carries the
/// search field, the compose action and the account avatar.
fn inbox(selected: &Binding<Option<u64>>, query: &Binding<Str>) -> NavigationView {
    let visible = SignalCollection::new(
        query
            .clone()
            .map(|query| matching_threads(&query))
            .computed(),
    );
    List::for_each(visible, thread_row)
        .selection(selected)
        .title("Inbox")
        .searchable(query, "Search replies")
        .navigation_toolbar(
            NavigationToolbar::default()
                .item(NavigationToolbarItem::action(
                    NavigationToolbarPlacement::PrimaryAction,
                    label("Compose").icon(pencil()),
                    || {},
                ))
                .item(NavigationToolbarItem::new(
                    NavigationToolbarPlacement::TopBarTrailing,
                    avatar("Me")
                        .image(asset("avatar_10.jpg"))
                        .size(TOOLBAR_AVATAR),
                )),
        )
}

fn message_card(message: &'static Message) -> impl View {
    card(
        vstack((
            hstack((
                avatar(message.sender)
                    .image(asset(message.avatar_asset))
                    .size(AVATAR),
                vstack((
                    text(message.sender).sub_headline(),
                    text(message.time).caption().muted(),
                ))
                .leading(),
                spacer(),
                star(),
            )),
            text(message.recipients).caption().muted(),
            text(message.body).body(),
            message.signature.map(|signature| text(signature).body()),
            hstack((
                button("Reply").bordered().action(|| {}),
                button("Reply all").bordered().action(|| {}),
            )),
        ))
        .leading(),
    )
}

/// The reading pane: the subject as the navigation title, the message count
/// as its subtitle, and the thread's overflow actions in the bar.
fn detail(thread: &'static Thread) -> NavigationView {
    scroll(vstack(thread.messages.iter().map(message_card).collect::<Vec<_>>()).leading())
        .title(thread.subject)
        .navigation_subtitle(text!("{#count} Messages", count = thread.messages.len()))
        .navigation_toolbar(
            NavigationToolbar::default()
                .item(NavigationToolbarItem::action(
                    NavigationToolbarPlacement::SecondaryAction,
                    label("Delete").icon(trash_can_outline()),
                    || {},
                ))
                .item(NavigationToolbarItem::action(
                    NavigationToolbarPlacement::SecondaryAction,
                    label("More").icon(dots_vertical()),
                    || {},
                )),
        )
}

/// The Mail destination: the inbox beside the reading pane, collapsing to one
/// column at compact widths.
fn mail(selected: Binding<Option<u64>>) -> impl View {
    let query = binding(Str::default());
    NavigationSplitView::new(
        &selected.clone(),
        move || NavigationStack::new(inbox(&selected, &query)),
        |id| detail(thread_by_id(id)),
    )
    .placeholder(|| text("Select a thread"))
}

/// A destination with no content of its own in this sample.
fn empty_destination(name: &'static str) -> impl View {
    text(name).title()
}

fn reply(selected: Binding<Option<u64>>, destination: Binding<usize>) -> impl View {
    Tabs::new(
        &destination,
        vec![
            Tab::container(label("Mail").icon(email_outline()), move || {
                mail(selected.clone())
            })
            .badge(UNREAD),
            Tab::container(label("Notes").icon(note_outline()), || {
                empty_destination("Notes")
            }),
            Tab::container(label("Chat").icon(message_outline()), || {
                empty_destination("Chat")
            }),
            Tab::container(label("Meet").icon(video_outline()), || {
                empty_destination("Meet")
            }),
        ],
    )
    .style(tab_style::automatic())
}

/// Self-contained entry for previews and embedding.
#[preview]
pub fn demo() -> impl View {
    reply(binding(Some(2u64)), binding(0usize))
}

pub fn app(env: Environment) -> App {
    let selected = binding(Some(2u64));
    let destination = binding(0usize);
    App::new(move || reply(selected.clone(), destination.clone()), env)
}
