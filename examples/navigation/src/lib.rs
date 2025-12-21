//! Navigation Example - Demonstrates WaterUI's navigation capabilities
//!
//! This example showcases:
//! - NavigationStack for managing a stack of views
//! - NavigationLink for declarative navigation
//! - Nested navigation (multiple levels deep)
//! - Reactive state across navigation

use waterui::app::App;
use waterui::navigation::{NavigationLink, NavigationStack};
use waterui::prelude::*;
use waterui::reactive::binding;

/// A simple item to display in lists
#[derive(Clone)]
struct Item {
    name: &'static str,
    description: &'static str,
}

/// Sample data for the example
fn sample_items() -> Vec<Item> {
    vec![
        Item {
            name: "Getting Started",
            description: "Learn the basics of WaterUI navigation",
        },
        Item {
            name: "Advanced Patterns",
            description: "Deep dive into complex navigation flows",
        },
        Item {
            name: "Best Practices",
            description: "Tips for building great navigation experiences",
        },
    ]
}

/// Root view containing the navigation stack
fn main_view() -> impl View {
    let counter = binding(0);
    NavigationStack::new(home_view(counter).title("Navigation Demo"))
}

/// Home screen with navigation links
fn home_view(counter: Binding<i32>) -> impl View {
    let items = sample_items();

    scroll(
        vstack((
            header_section(),
            counter_section(counter.clone()),
            topics_section(items, counter),
            settings_section(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

/// Header section of home view
fn header_section() -> impl View {
    vstack((
        text("Welcome to Navigation Demo").font(font::Title),
        text("Tap any item below to navigate")
            .font(font::Body)
            .foreground(theme_color::MutedForeground),
    ))
}

/// Counter section demonstrating state persistence
fn counter_section(counter: Binding<i32>) -> impl View {
    vstack((
        spacer_min(16.0),
        Divider,
        spacer_min(16.0),
        vstack((
            text("Shared Counter").font(font::Subheadline),
            hstack((
                button("-").action({
                    let counter = counter.clone();
                    move || counter.set(counter.get() - 1)
                }),
                spacer_min(16.0),
                text!("Count: {counter}").font(font::Body),
                spacer_min(16.0),
                button("+").action({
                    let counter = counter.clone();
                    move || counter.set(counter.get() + 1)
                }),
            )),
            text("This counter persists across navigation")
                .font(font::Caption)
                .foreground(theme_color::MutedForeground),
        ))
        .padding_with(EdgeInsets::all(12.0))
        .background(theme_color::SurfaceVariant),
    ))
}

/// Topics navigation links section
fn topics_section(items: Vec<Item>, counter: Binding<i32>) -> impl View {
    vstack((
        spacer_min(16.0),
        Divider,
        spacer_min(16.0),
        text("Topics").font(font::Subheadline),
        spacer_min(8.0),
        vstack(
            items
                .into_iter()
                .map(|item| item_row(item, counter.clone()))
                .collect::<Vec<_>>(),
        ),
    ))
}

/// Settings navigation link section
fn settings_section() -> impl View {
    vstack((
        spacer_min(16.0),
        Divider,
        spacer_min(16.0),
        NavigationLink::new(
            hstack((
                text("Settings").font(font::Subheadline),
                spacer(),
                text(">").foreground(theme_color::MutedForeground),
            ))
            .padding_with(EdgeInsets::symmetric(12.0, 0.0)),
            || settings_view().title("Settings"),
        ),
    ))
}

/// A row that navigates to item detail
fn item_row(item: Item, counter: Binding<i32>) -> impl View {
    let name = item.name;
    let description = item.description;

    NavigationLink::new(
        vstack((
            hstack((
                text(name).font(font::Subheadline),
                spacer(),
                text(">").foreground(theme_color::MutedForeground),
            )),
            text(description)
                .font(font::Caption)
                .foreground(theme_color::MutedForeground),
        ))
        .padding_with(EdgeInsets::symmetric(8.0, 0.0)),
        move || detail_view(item.clone(), counter.clone()).title(name),
    )
}

/// Detail view for an item
fn detail_view(item: Item, counter: Binding<i32>) -> impl View {
    scroll(
        vstack((
            detail_header(&item),
            counter_display(counter),
            detail_navigation(item.clone()),
            detail_content(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

/// Header section of detail view
fn detail_header(item: &Item) -> impl View {
    vstack((
        text(item.name).font(font::Headline),
        spacer_min(8.0),
        text(item.description)
            .font(font::Body)
            .foreground(theme_color::MutedForeground),
        spacer_min(24.0),
        Divider,
    ))
}

/// Counter display in detail view
fn counter_display(counter: Binding<i32>) -> impl View {
    vstack((
        spacer_min(24.0),
        vstack((
            text("Shared Counter").font(font::Subheadline),
            text!("Current value: {counter}").font(font::Body),
            text("Modified from any screen")
                .font(font::Caption)
                .foreground(theme_color::MutedForeground),
        ))
        .padding_with(EdgeInsets::all(12.0))
        .background(theme_color::SurfaceVariant),
    ))
}

/// Navigation link to nested view
fn detail_navigation(item: Item) -> impl View {
    vstack((
        spacer_min(24.0),
        NavigationLink::new(
            hstack((
                text("Go Deeper").font(font::Subheadline),
                spacer(),
                text(">").foreground(theme_color::MutedForeground),
            ))
            .padding_with(EdgeInsets::symmetric(12.0, 0.0)),
            move || nested_detail_view(item.clone()).title("Nested View"),
        ),
    ))
}

/// Content placeholder in detail view
fn detail_content() -> impl View {
    vstack((
        spacer_min(16.0),
        vstack((
            text("Content Area").font(font::Subheadline),
            spacer_min(8.0),
            text("This is where the main content would appear.").font(font::Body),
            text("Try using the back button to go back.")
                .font(font::Body)
                .foreground(theme_color::MutedForeground),
        ))
        .padding_with(EdgeInsets::all(12.0)),
    ))
}

/// A nested detail view to demonstrate deep navigation
fn nested_detail_view(item: Item) -> impl View {
    vstack((
        text("Nested Navigation").font(font::Title),
        spacer_min(16.0),
        text!("You navigated from: {name}", name = item.name).font(font::Body),
        spacer_min(24.0),
        text("This demonstrates multi-level navigation.")
            .font(font::Body)
            .foreground(theme_color::MutedForeground),
        spacer_min(24.0),
        NavigationLink::new(
            hstack((
                text("Go Even Deeper").font(font::Subheadline),
                spacer(),
                text(">").foreground(theme_color::MutedForeground),
            ))
            .padding_with(EdgeInsets::symmetric(12.0, 0.0)),
            || deepest_view().title("Level 3"),
        ),
    ))
    .padding_with(EdgeInsets::all(16.0))
}

/// The deepest level view
fn deepest_view() -> impl View {
    vstack((
        text("You're Deep!").font(font::Headline),
        spacer_min(24.0),
        text("This is 3 levels deep in the navigation stack.").font(font::Body),
        spacer_min(16.0),
        text("Use the back button to return.")
            .font(font::Body)
            .foreground(theme_color::MutedForeground),
    ))
    .padding_with(EdgeInsets::all(16.0))
}

/// Settings view with sub-navigation
fn settings_view() -> impl View {
    let notifications = binding(true);
    let dark_mode = binding(false);

    scroll(
        vstack((
            text("Settings").font(font::Title),
            spacer_min(16.0),
            toggle_row(
                "Notifications",
                "Receive push notifications",
                &notifications,
            ),
            Divider,
            toggle_row("Dark Mode", "Use dark color scheme", &dark_mode),
            Divider,
            settings_links(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

/// A toggle row in settings
fn toggle_row(title: &'static str, subtitle: &'static str, value: &Binding<bool>) -> impl View {
    hstack((
        vstack((
            text(title).font(font::Subheadline),
            text(subtitle)
                .font(font::Caption)
                .foreground(theme_color::MutedForeground),
        )),
        spacer(),
        Toggle::new(value),
    ))
    .padding_with(EdgeInsets::symmetric(8.0, 0.0))
}

/// Navigation links in settings
fn settings_links() -> impl View {
    vstack((
        NavigationLink::new(
            hstack((
                text("About").font(font::Subheadline),
                spacer(),
                text(">").foreground(theme_color::MutedForeground),
            ))
            .padding_with(EdgeInsets::symmetric(12.0, 0.0)),
            || about_view().title("About"),
        ),
        Divider,
        NavigationLink::new(
            hstack((
                text("Privacy Policy").font(font::Subheadline),
                spacer(),
                text(">").foreground(theme_color::MutedForeground),
            ))
            .padding_with(EdgeInsets::symmetric(12.0, 0.0)),
            || privacy_view().title("Privacy"),
        ),
    ))
}

/// About view
fn about_view() -> impl View {
    vstack((
        text("Navigation Example").font(font::Title),
        spacer_min(8.0),
        text("Version 1.0.0")
            .font(font::Caption)
            .foreground(theme_color::MutedForeground),
        spacer_min(24.0),
        text("Demonstrates WaterUI navigation capabilities.").font(font::Body),
        spacer_min(16.0),
        features_list(),
    ))
    .padding_with(EdgeInsets::all(16.0))
}

/// Features list in about view
fn features_list() -> impl View {
    vstack((
        text("Features showcased:"),
        spacer_min(8.0),
        "- NavigationStack for view management",
        "- NavigationLink for declarative navigation",
        "- Nested navigation (multiple levels)",
        "- State persistence across navigation",
    ))
}

/// Privacy view
fn privacy_view() -> impl View {
    scroll(
        vstack((
            text("Privacy Policy").font(font::Title),
            spacer_min(16.0),
            text("This is a sample privacy policy page.").font(font::Body),
            spacer_min(16.0),
            text("Navigation allows easy access to information.")
                .font(font::Body)
                .foreground(theme_color::MutedForeground),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

/// Application entry point
pub fn app(env: Environment) -> App {
    App::new(main_view(), env)
}

waterui_ffi::export!();
