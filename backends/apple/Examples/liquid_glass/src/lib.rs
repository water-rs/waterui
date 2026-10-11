//! Liquid Glass on Apple platforms.
//!
//! Every surface here is a semantic declaration that the Apple backend projects
//! onto the platform's own glass: `Glass` becomes `UIGlassEffect` on iOS and
//! `NSGlassEffectView` on macOS. Other backends approximate or ignore it, so
//! the same views render without glass where the platform has none.
//!
//! The backdrop is deliberately loud — glass is a lens, and a lens needs
//! something to bend.

use waterui::app::App;
use waterui::background::Glass;
use waterui::icon::SystemIcon;
use waterui::navigation::{TabBarMinimizeBehavior, TabRole};
use waterui::prelude::theme_color::Accent;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::shape::{Circle, RoundedRectangle, ShapeExt};

/// Tab identity. `Tabs` is generic over it, so no `Id` reaches app code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pane {
    Surfaces,
    Controls,
    About,
    Search,
}

#[preview]
pub fn main() -> impl View {
    let pane = binding(Pane::Surfaces);

    Tabs::new(
        &pane,
        vec![
            Tab::container(
                Pane::Surfaces,
                label("Surfaces").icon(SystemIcon::new("square.on.square")),
                surfaces_page,
            ),
            Tab::container(
                Pane::Controls,
                label("Controls").icon(SystemIcon::new("button.horizontal")),
                controls_page,
            ),
            Tab::container(
                Pane::About,
                label("About").icon(SystemIcon::new("info.circle")),
                about_page,
            ),
            // The role, not a different kind of tab: iOS places it trailing as
            // the system search tab with its own glass and presentation.
            Tab::container(
                Pane::Search,
                label("Search").icon(SystemIcon::new("magnifyingglass")),
                search_page,
            )
            .role(TabRole::Search),
        ],
    )
    .style(tab_style::automatic())
    // iOS collapses the bar into a glass pill as the surfaces scroll, and
    // brings it back on the way up.
    .minimize_behavior(TabBarMinimizeBehavior::OnScrollDown)
    // The slot iOS keeps above the tab bar for a now-playing bar; the
    // platform gives it the glass and collapses it inline with the bar.
    .bottom_accessory(now_playing())
}

/// A now-playing bar for the tab bar's bottom accessory slot.
///
/// The slot is one bar tall, so the content is a single row: the title and
/// its subtitle stacked at the sizes a now-playing bar uses.
fn now_playing() -> impl View {
    hstack((
        label("Play").icon(SystemIcon::new("play.fill")),
        vstack((
            text("Now Playing").bold().size(15.0),
            text("Liquid Glass — Surfaces").size(12.0),
        ))
        .spacing(1.0),
    ))
    .spacing(12.0)
    .padding_with(EdgeInsets::symmetric(6.0, 16.0))
}

/// Glass surfaces over a colorful backdrop: the four parameters glass has —
/// style, interactivity, tint, and outline — each shown on its own.
///
/// Each tab's root is a navigation stack, so the page has a bar for its
/// title and the scrolling content has chrome to run under.
fn surfaces_page() -> impl View {
    NavigationStack::new(surfaces_root())
}

fn surfaces_root() -> NavigationView {
    zstack((
        backdrop(),
        scroll(
            vstack((
                caption(
                    "Regular glass is the default: a capsule that stays legible over anything.",
                ),
                pill("Now Playing", Glass::regular()),
                caption("Clear glass diffuses less, for surfaces over media."),
                pill("Clear", Glass::clear()),
                caption(
                    "Interactive glass answers touch and hover with the platform's own effects.",
                ),
                pill("Tap me", Glass::regular().interactive(true)),
                caption("A tint washes the glass toward a color."),
                pill("Accent", Glass::regular().tint(Accent)),
                pill("Tomato", Glass::clear().tint(Color::srgb(255, 99, 71))),
                caption("The outline belongs to the glass, not to an outer clip."),
                card(),
                caption("The capsule follows the size of what it wraps."),
                text("Small")
                    .bold()
                    .size(13.0)
                    .padding_with(EdgeInsets::all(8.0))
                    .background(Glass::regular()),
                pill("Medium", Glass::regular()),
                text("Large")
                    .bold()
                    .size(24.0)
                    .padding_with(EdgeInsets::all(20.0))
                    .background(Glass::regular()),
            ))
            .spacing(12.0)
            .padding(),
        ),
    ))
    .title("Surfaces")
    .large_title()
}

/// Glass button styles beside the bordered ones they correspond to on
/// platforms without glass.
fn controls_page() -> impl View {
    NavigationStack::new(controls_root())
}

fn controls_root() -> NavigationView {
    zstack((
        backdrop(),
        scroll(
            vstack((
                caption("Glass: the capsule is the emphasis, the label keeps the primary color."),
                hstack((
                    button("Glass").style(ButtonStyle::Glass),
                    button("Bordered").style(ButtonStyle::Bordered),
                ))
                .spacing(12.0),
                caption("Prominent glass: the accent fills the capsule, for the primary action."),
                hstack((
                    button("Glass Prominent").style(ButtonStyle::GlassProminent),
                    button("Bordered Prominent").style(ButtonStyle::BorderedProminent),
                ))
                .spacing(12.0),
                caption("Labels with symbols get the same capsule."),
                hstack((
                    button(label("Share").icon(SystemIcon::new("square.and.arrow.up")))
                        .style(ButtonStyle::Glass),
                    button(label("Play").icon(SystemIcon::new("play.fill")))
                        .style(ButtonStyle::GlassProminent),
                ))
                .spacing(12.0),
            ))
            .spacing(12.0)
            .padding(),
        ),
    ))
    .title("Controls")
    .large_title()
}

fn about_page() -> impl View {
    NavigationStack::new(
        vstack((
            text("Liquid Glass").size(24.0),
            "Glass is the chrome-layer surface of iOS 26 and macOS 26. This app declares it; the Apple backend projects it.",
        ))
        .spacing(12.0)
        .padding()
        .title("About"),
    )
}

/// The search tab's root: a stack whose bar carries the search field, so the
/// system search tab has a field to present.
fn search_page() -> impl View {
    let query = binding(Str::default());
    NavigationStack::new(
        vstack((
            text!("Results for “{query}”"),
            caption("Glass terms: regular, clear, interactive, tint, capsule, card."),
        ))
        .spacing(12.0)
        .padding()
        .title("Search")
        .searchable(&query, "Search glass"),
    )
}

fn caption(body: &'static str) -> impl View {
    text(body).size(15.0)
}

fn pill(title: &'static str, glass: Glass) -> impl View {
    text(title).bold().padding().background(glass)
}

fn card() -> impl View {
    vstack((
        text("Rounded card").bold(),
        "Text inside glass keeps its full contrast; the glass adapts to what is behind it.",
    ))
    .spacing(6.0)
    .padding()
    .background(Glass::regular().shape(RoundedRectangle::new(0.2)))
}

/// Large colored discs, so the lensing at each glass edge has edges to bend.
fn backdrop() -> impl View {
    zstack((
        Circle
            .fill(Color::srgb(255, 149, 0))
            .size(320.0, 320.0)
            .offset(-120.0, -160.0),
        Circle
            .fill(Color::srgb(48, 176, 199))
            .size(280.0, 280.0)
            .offset(140.0, 40.0),
        Circle
            .fill(Color::srgb(175, 82, 222))
            .size(360.0, 360.0)
            .offset(-60.0, 320.0),
        Circle
            .fill(Color::srgb(52, 199, 89))
            .size(220.0, 220.0)
            .offset(150.0, 560.0),
    ))
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
