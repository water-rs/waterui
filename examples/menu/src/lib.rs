//! Menu Example - Demonstrates WaterUI's menu components
//!
//! This example showcases:
//! - `Menu` component - A button-like dropdown menu
//! - `ContextMenu` modifier - Long-press context menus
//! - Menu items with actions

use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::Binding;

/// Section demonstrating the Menu component
fn menu_section(selected: Binding<String>) -> impl View {
    let selected_display = selected.clone();
    vstack((
        text("Menu Component").font(font::Subheadline),
        text("Tap the menu button to see options")
            .font(font::Body)
            .foreground(theme_color::MutedForeground),
        spacer().height(12.0),
        {
            let selected = selected.clone();
            Menu::new(
                hstack((
                    text("Choose an Option"),
                    text(" ▼").foreground(theme_color::MutedForeground),
                )),
                vec![
                    MenuItem::new("Option A", {
                        let selected = selected.clone();
                        move || selected.set("Option A".to_string())
                    }),
                    MenuItem::new("Option B", {
                        let selected = selected.clone();
                        move || selected.set("Option B".to_string())
                    }),
                    MenuItem::new("Option C", {
                        let selected = selected.clone();
                        move || selected.set("Option C".to_string())
                    }),
                ],
            )
        },
        spacer().height(12.0),
        hstack((
            text("Selected: ")
                .font(font::Caption)
                .foreground(theme_color::MutedForeground),
            Text::new(selected_display).font(font::Body),
        )),
    ))
    .padding()
}

/// Section demonstrating the Menu with styled label
fn styled_menu_section(action_log: Binding<String>) -> impl View {
    let action_display = action_log.clone();
    vstack((
        text("Styled Menu").font(font::Subheadline),
        text("Menu with custom styled label")
            .font(font::Body)
            .foreground(theme_color::MutedForeground),
        spacer().height(12.0),
        {
            let action_log = action_log.clone();
            Menu::new(
                hstack((
                    text("Actions")
                        .bold()
                        .foreground(Color::srgb_hex("#2196F3")),
                    spacer().width(8.0),
                    text("▼").font(font::Caption),
                ))
                .padding_with(EdgeInsets::symmetric(8.0, 12.0))
                .background(Color::srgb_hex("#E3F2FD")),
                vec![
                    MenuItem::new("Edit", {
                        let action_log = action_log.clone();
                        move || action_log.set("Edit action triggered".to_string())
                    }),
                    MenuItem::new("Duplicate", {
                        let action_log = action_log.clone();
                        move || action_log.set("Duplicate action triggered".to_string())
                    }),
                    MenuItem::new("Delete", {
                        let action_log = action_log.clone();
                        move || action_log.set("Delete action triggered".to_string())
                    }),
                ],
            )
        },
        spacer().height(12.0),
        Text::display(action_display)
            .font(font::Caption)
            .foreground(theme_color::MutedForeground),
    ))
    .padding()
}

/// Section demonstrating the ContextMenu modifier
fn context_menu_section(context_action: Binding<String>) -> impl View {
    let action_display = context_action.clone();
    vstack((
        text("Context Menu").font(font::Subheadline),
        text("Long press the box below to see context menu")
            .font(font::Body)
            .foreground(theme_color::MutedForeground),
        spacer().height(12.0),
        {
            let context_action = context_action.clone();
            text("Long Press Me")
                .padding_with(EdgeInsets::all(24.0))
                .background(Color::srgb_hex("#FFF3E0"))
                .foreground(Color::srgb_hex("#E65100"))
                .context_menu(vec![
                    MenuItem::new("Copy", {
                        let context_action = context_action.clone();
                        move || context_action.set("Copied!".to_string())
                    }),
                    MenuItem::new("Cut", {
                        let context_action = context_action.clone();
                        move || context_action.set("Cut!".to_string())
                    }),
                    MenuItem::new("Paste", {
                        let context_action = context_action.clone();
                        move || context_action.set("Pasted!".to_string())
                    }),
                    MenuItem::new("Select All", {
                        let context_action = context_action.clone();
                        move || context_action.set("Selected all!".to_string())
                    }),
                ])
        },
        spacer().height(12.0),
        Text::display(action_display)
            .font(font::Caption)
            .foreground(theme_color::MutedForeground),
    ))
    .padding()
}

/// Section demonstrating context menu on different views
fn context_menu_views_section(view_action: Binding<String>) -> impl View {
    let action_display = view_action.clone();
    vstack((
        text("Context Menu on Views").font(font::Subheadline),
        text("Long press any colored box")
            .font(font::Body)
            .foreground(theme_color::MutedForeground),
        spacer().height(12.0),
        hstack((
            {
                let view_action = view_action.clone();
                text("Red")
                    .foreground(Color::srgb_hex("#FFFFFF"))
                    .padding()
                    .background(Color::srgb_hex("#F44336"))
                    .context_menu(vec![
                        MenuItem::new("Red Action 1", {
                            let view_action = view_action.clone();
                            move || view_action.set("Red Action 1".to_string())
                        }),
                        MenuItem::new("Red Action 2", {
                            let view_action = view_action.clone();
                            move || view_action.set("Red Action 2".to_string())
                        }),
                    ])
            },
            spacer().width(12.0),
            {
                let view_action = view_action.clone();
                text("Green")
                    .foreground(Color::srgb_hex("#FFFFFF"))
                    .padding()
                    .background(Color::srgb_hex("#4CAF50"))
                    .context_menu(vec![
                        MenuItem::new("Green Action 1", {
                            let view_action = view_action.clone();
                            move || view_action.set("Green Action 1".to_string())
                        }),
                        MenuItem::new("Green Action 2", {
                            let view_action = view_action.clone();
                            move || view_action.set("Green Action 2".to_string())
                        }),
                    ])
            },
            spacer().width(12.0),
            {
                let view_action = view_action.clone();
                text("Blue")
                    .foreground(Color::srgb_hex("#FFFFFF"))
                    .padding()
                    .background(Color::srgb_hex("#2196F3"))
                    .context_menu(vec![
                        MenuItem::new("Blue Action 1", {
                            let view_action = view_action.clone();
                            move || view_action.set("Blue Action 1".to_string())
                        }),
                        MenuItem::new("Blue Action 2", {
                            let view_action = view_action.clone();
                            move || view_action.set("Blue Action 2".to_string())
                        }),
                    ])
            },
        )),
        spacer().height(12.0),
        Text::display(action_display)
            .font(font::Caption)
            .foreground(theme_color::MutedForeground),
    ))
    .padding()
}

#[hot_reload]
fn main() -> impl View {
    let menu_selected = Binding::container(String::from("None"));
    let styled_action = Binding::container(String::from("No action yet"));
    let context_action = Binding::container(String::from("No action yet"));
    let view_action = Binding::container(String::from("No action yet"));

    scroll(
        vstack((
            // Header
            text("WaterUI Menu Examples").font(font::Headline),
            text("Demonstrating Menu and Context Menu components")
                .font(font::Body)
                .foreground(theme_color::MutedForeground),
            Divider,
            spacer().height(8.0),
            // Menu component section
            menu_section(menu_selected),
            Divider,
            // Styled menu section
            styled_menu_section(styled_action),
            Divider,
            // Context menu section
            context_menu_section(context_action),
            Divider,
            // Context menu on different views
            context_menu_views_section(view_action),
            spacer().height(40.0),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
