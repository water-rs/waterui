//! Reply — a Material Design 3 list-detail mail client.
//!
//! A three-zone expanded-width layout: a navigation rail with a compose FAB,
//! a thread list with a search bar, and a reading pane for the selected
//! thread. The theme is the sample's own palette — warm surfaces, an amber
//! selection container, and a green tertiary FAB — installed into a scoped
//! environment so managed backends can apply their defaults first without
//! losing these colors.

use core::num::NonZeroUsize;

use waterui::accessibility::AccessibilityRole;
use waterui::app::App;
use waterui::color::signal_color;
use waterui::component::vstack;
use waterui::layout::ContentMode;
use waterui::media::Photo;
use waterui::metadata::Metadata;
use waterui::prelude::*;
use waterui::shape::{Capsule, FixedRoundedRectangle, ShapeExt};
use waterui::text::font::{Body, Caption, Font, FontWeight, Subheadline};
use waterui::widget::avatar;
use waterui::widget::condition::when;
use waterui_icons_material_icon as mdi;

use hydrolysis_m3::color::{
    self, InverseOnSurface, OnSurface, OnSurfaceVariant, Outline, SecondaryContainer,
    SurfaceBright, SurfaceContainer, SurfaceContainerHigh, SurfaceContainerLowest, SurfaceVariant,
};
use hydrolysis_m3::navigation_rail::NavigationRailLayout;
use hydrolysis_m3::{
    Argb, MaterialColorMode, MaterialColorScheme, MaterialColorSchemes, MaterialColorSource,
    MaterialRoleColor, fab, icon_button, material_badge, navigation_rail, navigation_rail_item,
};
use mdi::dots_vertical;
use mdi::email_outline;
use mdi::magnify;
use mdi::menu;
use mdi::message_outline;
use mdi::note_outline;
use mdi::pencil;
use mdi::star as mdi_star;
use mdi::star_outline;
use mdi::trash_can_outline;
use mdi::video_outline;

const RAIL_WIDTH: f32 = 80.0;
/// Minimum list-pane width; the pane splits the remaining width evenly with
/// the detail pane (`splitFraction = 0.5`).
const LIST_MIN_WIDTH: f32 = 360.0;
/// `CornerMedium` — list and thread cards.
const CARD_RADIUS: f32 = 12.0;
const AVATAR: f32 = 40.0;
const SEARCH_AVATAR: f32 = 32.0;

/// `labelMedium` (Roboto Medium 12/16) — sender names, timestamps, counts.
fn label_medium() -> Font {
    Font::new(Caption)
        .weight(FontWeight::Medium)
        .line_height(16.0)
}

/// `bodyMedium` (Roboto 14/20) — list snippets and the recipients line.
fn body_medium() -> Font {
    Font::new(Body).size(14.0).line_height(20.0)
}

/// `labelLarge` (Roboto Medium 14/20) — button labels.
fn label_large() -> Font {
    Font::new(Body)
        .size(14.0)
        .weight(FontWeight::Medium)
        .line_height(20.0)
}

const ASSETS: &str = "https://raw.githubusercontent.com/android/compose-samples/main/Reply/app/src/main/res/drawable";

fn asset(name: &str) -> Url {
    format!("{ASSETS}/{name}")
        .parse()
        .expect("sample asset URL is valid")
}

/// The sample's light scheme: warm neutral surfaces, amber secondary
/// container, green tertiary container.
fn reply_scheme() -> MaterialColorScheme {
    const fn role(r: u8, g: u8, b: u8) -> MaterialRoleColor {
        MaterialRoleColor::new(Argb::from_rgb(r, g, b))
    }
    let mut scheme = MaterialColorScheme::baseline_light();
    scheme.mode = MaterialColorMode::Light;
    scheme.primary = role(0x80, 0x56, 0x10);
    scheme.on_primary = role(0xFF, 0xFF, 0xFF);
    scheme.primary_container = role(0xFF, 0xDD, 0xB3);
    scheme.on_primary_container = role(0x29, 0x18, 0x00);
    scheme.secondary = role(0x6F, 0x5B, 0x40);
    scheme.on_secondary = role(0xFF, 0xFF, 0xFF);
    scheme.secondary_container = role(0xFB, 0xDE, 0xBC);
    scheme.on_secondary_container = role(0x27, 0x19, 0x04);
    scheme.tertiary = role(0x51, 0x64, 0x3F);
    scheme.on_tertiary = role(0xFF, 0xFF, 0xFF);
    scheme.tertiary_container = role(0xD4, 0xEA, 0xBB);
    scheme.on_tertiary_container = role(0x10, 0x20, 0x04);
    scheme.error = role(0xBA, 0x1A, 0x1A);
    scheme.on_error = role(0xFF, 0xFF, 0xFF);
    scheme.error_container = role(0xFF, 0xDA, 0xD6);
    scheme.on_error_container = role(0x41, 0x00, 0x02);
    scheme.background = role(0xFF, 0xF8, 0xF4);
    scheme.on_background = role(0x20, 0x1B, 0x13);
    scheme.surface = role(0xFF, 0xF8, 0xF4);
    scheme.on_surface = role(0x20, 0x1B, 0x13);
    scheme.surface_variant = role(0xF0, 0xE0, 0xCF);
    scheme.on_surface_variant = role(0x4F, 0x45, 0x39);
    scheme.outline = role(0x81, 0x75, 0x67);
    scheme.outline_variant = role(0xD3, 0xC4, 0xB4);
    scheme.scrim = role(0x00, 0x00, 0x00);
    scheme.inverse_surface = role(0x36, 0x2F, 0x27);
    scheme.inverse_on_surface = role(0xFC, 0xEF, 0xE2);
    scheme.inverse_primary = role(0xF4, 0xBD, 0x6F);
    scheme.surface_dim = role(0xE4, 0xD8, 0xCC);
    scheme.surface_bright = role(0xFF, 0xF8, 0xF4);
    scheme.surface_container_lowest = role(0xFF, 0xFF, 0xFF);
    scheme.surface_container_low = role(0xFF, 0xF1, 0xE5);
    scheme.surface_container = role(0xF9, 0xEC, 0xDF);
    scheme.surface_container_high = role(0xF3, 0xE6, 0xDA);
    scheme.surface_container_highest = role(0xED, 0xE0, 0xD4);
    scheme.shadow = role(0x00, 0x00, 0x00);
    scheme.surface_tint = scheme.primary;
    scheme
}

/// Installs the sample's color scheme into `content`'s scoped environment.
struct ReplyTheme<V> {
    content: V,
}

impl<V: View> View for ReplyTheme<V> {
    fn body(self, env: &Environment) -> impl View {
        let mut scoped = Environment::new().layered_on(env);
        let scheme = reply_scheme();
        hydrolysis_m3::install_with_colors(&mut scoped, scheme);
        // The app env carries a `MaterialColorSchemes` pair from
        // `install_defaults`, which role resolution consults before the
        // singular scheme — shadow it or the sample renders baseline colors.
        scoped.insert(MaterialColorSchemes::new(
            MaterialColorSource::default(),
            scheme,
            scheme,
        ));
        Metadata::new(self.content, scoped)
    }
}

fn themed(content: impl View) -> impl View {
    ReplyTheme { content }
}

struct Message {
    sender: &'static str,
    time: &'static str,
    recipients: &'static str,
    body: &'static str,
    signature: Option<&'static str>,
    avatar_asset: &'static str,
}

struct Thread {
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
        sender: "老强",
        time: "10 min ago",
        subject: "豆花鱼",
        snippet: "最近忙吗？昨晚我去了你最爱的那家饭馆，点了他们的特色豆花鱼，吃着吃着就想你了。",
        avatar_asset: "avatar_8.jpg",
        photo_asset: None,
        messages: DOUHUA_MESSAGES,
    },
    Thread {
        sender: "So Duri",
        time: "20 min ago",
        subject: "Dinner Club",
        snippet: "I think it's time for us to finally try that new noodle shop downtown that doesn't use me\u{2026}",
        avatar_asset: "avatar_3.jpg",
        photo_asset: None,
        messages: DINNER_CLUB_MESSAGES,
    },
    Thread {
        sender: "Lily MacDonald",
        time: "2 hours ago",
        subject: "This food show is made for you",
        snippet: "Ping– you'd love this new food show I started watching. It's produced by a Thai drummer\u{2026}",
        avatar_asset: "avatar_1.jpg",
        photo_asset: Some("paris_3.jpg"),
        messages: FOOD_SHOW_MESSAGES,
    },
];

/// An icon button sitting on a filled circle, the way the sample renders
/// secondary actions (star, delete, more) inside message surfaces. The circle
/// token differs by surface: list cards use `surfaceContainerHigh`, thread
/// cards `surfaceContainer`, the detail app bar `surface`.
fn circled_icon_button(
    label: &'static str,
    icon: impl View + 'static,
    circle: impl Into<Color>,
) -> impl View {
    icon_button(label, icon).background(Capsule.fill(circle))
}

fn star(selected: bool, circle: impl Into<Color>) -> impl View {
    circled_icon_button(
        if selected { "Unstar" } else { "Star" },
        when(selected, mdi_star).otherwise(star_outline),
        circle,
    )
}

fn sender_line(message: &'static Message, starred: bool) -> impl View {
    hstack((
        avatar(message.sender)
            .image(asset(message.avatar_asset))
            .size(AVATAR),
        vstack((
            text(message.sender).font(label_medium()),
            text(message.time).font(label_medium()).foreground(Outline),
        ))
        .leading()
        .spacing(2.0),
        spacer(),
        star(starred, SurfaceContainer),
    ))
    .spacing(12.0)
}

/// Sender avatar asset keyed by display name — the sample ships a fixed set.
fn avatar_of(name: &'static str) -> Url {
    asset(match name {
        "老强" => "avatar_8.jpg",
        "So Duri" => "avatar_3.jpg",
        "Lily MacDonald" => "avatar_1.jpg",
        "Me" => "avatar_10.jpg",
        other => panic!("no avatar asset for {other}"),
    })
}

fn thread_card(thread: &'static Thread, selected: Binding<usize>, index: usize) -> impl View {
    let container = signal_color(
        selected
            .clone()
            .equal_to(index)
            .select(Color::new(SecondaryContainer), Color::new(SurfaceVariant))
            .computed(),
    );
    vstack((
        hstack((
            avatar(thread.sender)
                .image(asset(thread.avatar_asset))
                .size(AVATAR),
            vstack((
                text(thread.sender).font(label_medium()),
                text(thread.time).font(label_medium()).foreground(Outline),
            ))
            .leading()
            .spacing(2.0),
            spacer(),
            star(false, SurfaceContainerHigh),
        ))
        .spacing(12.0),
        text(thread.subject).font(Body),
        text(thread.snippet)
            .font(body_medium())
            .line_limit(NonZeroUsize::new(2).expect("2 is non-zero"))
            .foreground(OnSurfaceVariant),
        thread.photo_asset.map(|name| {
            AnyView::new(
                Photo::new(asset(name))
                    .resizable()
                    .content_mode(ContentMode::Fill)
                    .max_height(160.0)
                    .clip(FixedRoundedRectangle::new(CARD_RADIUS)),
            )
        }),
    ))
    .leading()
    .spacing(8.0)
    .padding_with(20.0)
    .background(container)
    .clip(FixedRoundedRectangle::new(CARD_RADIUS))
    .on_tap(move |State(selected): State<Binding<usize>>| {
        selected.set(index);
    })
    .state(&selected)
    .a11y_label(thread.subject)
    .a11y_role(AccessibilityRole::Button)
}

fn search_bar() -> impl View {
    hstack((
        magnify().foreground(OnSurfaceVariant),
        text("Search replies")
            .font(Body)
            .foreground(OnSurfaceVariant),
        spacer(),
        avatar("Me").image(avatar_of("Me")).size(SEARCH_AVATAR),
    ))
    .spacing(12.0)
    .padding_with((12.0, 16.0))
    .background(Capsule.fill(SurfaceContainerHigh))
}

fn rail_item<Icon: Clone + View + 'static>(
    selected_rail: &Binding<usize>,
    index: usize,
    label: &'static str,
    icon: Icon,
) -> impl View {
    // The sample's rail items are icon-only; the a11y label carries the name.
    navigation_rail_item("", icon, &selected_rail.condition(move |now| *now == index))
        .action(move |State(selected_rail): State<Binding<usize>>| {
            selected_rail.set(index);
        })
        .state(selected_rail)
        .a11y_label(label)
}

fn rail(selected_rail: Binding<usize>) -> impl View {
    vstack((
        icon_button("Menu", menu()),
        fab("Compose", pencil()).tertiary(),
        navigation_rail((
            rail_item(
                &selected_rail,
                0,
                "Mail",
                material_badge(4, email_outline()),
            ),
            rail_item(&selected_rail, 1, "Notes", note_outline()),
            rail_item(&selected_rail, 2, "Chat", message_outline()),
            rail_item(&selected_rail, 3, "Meet", video_outline()),
        ))
        .layout(NavigationRailLayout::CollapsedNarrow),
        spacer(),
    ))
    .spacing(4.0)
    .min_width(RAIL_WIDTH)
    .max_width(RAIL_WIDTH)
    .max_height(f32::INFINITY)
    .background(color::Surface)
}

fn list_pane(selected: Binding<usize>) -> impl View {
    scroll(
        vstack((
            search_bar(),
            vstack(
                THREADS
                    .iter()
                    .enumerate()
                    .map(|(index, thread)| thread_card(thread, selected.clone(), index))
                    .collect::<Vec<_>>(),
            )
            .spacing(8.0),
        ))
        .spacing(16.0)
        .padding_with([16.0, 16.0, 4.0, 12.0]),
    )
    .min_width(LIST_MIN_WIDTH)
    .max_width(f32::INFINITY)
    .max_height(f32::INFINITY)
}

fn message_body(message: &'static Message) -> impl View {
    vstack((
        sender_line(message, false),
        text(message.recipients)
            .font(body_medium())
            .foreground(Outline),
        text(message.body).font(Body).foreground(OnSurfaceVariant),
        message
            .signature
            .map(|signature| AnyView::new(text(signature).font(Body))),
        reply_actions(),
    ))
    .leading()
    .spacing(16.0)
}

/// The sample's reply actions are filled pills (`surfaceBright` container,
/// `onSurface` label) that split the card width evenly — the native `Button`
/// reports no stretch axis, so the pill is built from a capsule surface.
fn reply_pill(label: &'static str) -> impl View {
    text(label)
        .font(label_large())
        .foreground(OnSurface)
        .padding_with([10.0, 10.0, 24.0, 24.0])
        .max_width(f32::INFINITY)
        .background(Capsule.fill(SurfaceBright))
        .on_tap(|| {})
        .a11y_role(accessibility::AccessibilityRole::Button)
        .a11y_label(label)
}

fn reply_actions() -> impl View {
    hstack((reply_pill("Reply"), reply_pill("Reply all")))
        .spacing(12.0)
        .padding_with([20.0, 8.0, 0.0, 0.0])
}

/// The detail column's app bar: subject, message count, and overflow actions,
/// laid directly on the `inverseOnSurface` column background.
fn detail_header(thread: &'static Thread) -> impl View {
    hstack((
        vstack((
            text(thread.subject).font(Subheadline),
            text(text!("{#count} Messages", count = thread.messages.len()))
                .font(label_medium())
                .foreground(Outline),
        ))
        .leading()
        .spacing(4.0),
        spacer(),
        circled_icon_button("Delete", trash_can_outline(), color::Surface),
        circled_icon_button("More", dots_vertical(), color::Surface),
    ))
    .spacing(8.0)
    .padding_with([16.0, 16.0, 20.0, 16.0])
}

fn detail_pane(thread: &'static Thread) -> impl View {
    let mut rows = vec![AnyView::new(detail_header(thread))];
    for message in thread.messages {
        rows.push(AnyView::new(
            message_body(message)
                .padding_with(20.0)
                .background(SurfaceContainerLowest)
                .clip(FixedRoundedRectangle::new(CARD_RADIUS)),
        ));
    }
    scroll(
        vstack(rows)
            .leading()
            .spacing(8.0)
            .padding_with([0.0, 12.0, 16.0, 16.0]),
    )
    .background(InverseOnSurface)
    .max_width(f32::INFINITY)
    .max_height(f32::INFINITY)
}

fn reply(selected: Binding<usize>, selected_rail: Binding<usize>) -> impl View {
    themed(
        hstack((
            rail(selected_rail),
            list_pane(selected.clone()),
            watch(selected, move |index| detail_pane(&THREADS[index]))
                .max_width(f32::INFINITY)
                .max_height(f32::INFINITY),
        ))
        .spacing(0.0)
        .max_height(f32::INFINITY)
        .background(color::Background),
    )
}

/// Self-contained entry for previews and embedding.
#[preview]
pub fn demo() -> impl View {
    reply(binding(1usize), binding(0usize))
}

pub fn app(env: Environment) -> App {
    let selected = binding(1usize);
    let selected_rail = binding(0usize);
    App::new(move || reply(selected.clone(), selected_rail.clone()), env)
}
