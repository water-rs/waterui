//! Menu Example - Demonstrates WaterUI's menu components
//!
//! This example showcases:
//! - `Menu` component - A button-like dropdown menu
//! - `ContextMenu` modifier - Long-press context menus
//! - Menu items with `.with_state()` for clean state capture

use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::reactive::Binding;

const BLUE: Srgb = Srgb::from_hex("#2196F3");
const LIGHT_BLUE: Srgb = Srgb::from_hex("#E3F2FD");
const ORANGE_BG: Srgb = Srgb::from_hex("#FFF3E0");
const ORANGE_FG: Srgb = Srgb::from_hex("#E65100");
const RED: Srgb = Srgb::from_hex("#F44336");
const GREEN: Srgb = Srgb::from_hex("#4CAF50");
const WHITE: Srgb = Srgb::from_hex("#FFFFFF");

/// Section demonstrating the Menu component
fn menu_section(selected: &Binding<String>) -> impl View {
    vstack((
        text("Menu Component").sub_headline(),
        text("Tap the menu button to see options")
            .body()
            .foreground(MutedForeground),
        spacer().height(12.0),
        Menu::new(
            hstack((
                text("Choose an Option"),
                text(" ▼").foreground(MutedForeground),
            )),
            vec![
                MenuItem::new("Option A")
                    .with_state(selected)
                    .action(|s| s.set("Option A".to_string())),
                MenuItem::new("Option B")
                    .with_state(selected)
                    .action(|s| s.set("Option B".to_string())),
                MenuItem::new("Option C")
                    .with_state(selected)
                    .action(|s| s.set("Option C".to_string())),
            ],
        ),
        spacer().height(12.0),
        hstack((
            text("Selected: ").caption().foreground(MutedForeground),
            text!("{selected}").body(),
        )),
    ))
    .padding()
}

/// Section demonstrating the Menu with styled label
fn styled_menu_section(action_log: &Binding<String>) -> impl View {
    vstack((
        text("Styled Menu").sub_headline(),
        text("Menu with custom styled label")
            .body()
            .foreground(MutedForeground),
        spacer().height(12.0),
        Menu::new(
            hstack((
                text("Actions").bold().foreground(BLUE),
                spacer().width(8.0),
                text("▼").caption(),
            ))
            .padding_with(EdgeInsets::symmetric(8.0, 12.0))
            .background(LIGHT_BLUE),
            vec![
                MenuItem::new("Edit")
                    .with_state(action_log)
                    .action(|a| a.set("Edit action triggered".to_string())),
                MenuItem::new("Duplicate")
                    .with_state(action_log)
                    .action(|a| a.set("Duplicate action triggered".to_string())),
                MenuItem::new("Delete")
                    .with_state(action_log)
                    .action(|a| a.set("Delete action triggered".to_string())),
            ],
        ),
        spacer().height(12.0),
        text!("{action_log}")
            .font(font::Caption)
            .foreground(MutedForeground),
    ))
    .padding()
}

/// Section demonstrating the ContextMenu modifier
fn context_menu_section(context_action: &Binding<String>) -> impl View {
    vstack((
        text("Context Menu").sub_headline(),
        text("Long press the box below to see context menu")
            .body()
            .foreground(MutedForeground),
        spacer().height(12.0),
        text("Long Press Me")
            .padding_with(EdgeInsets::all(24.0))
            .background(ORANGE_BG)
            .foreground(ORANGE_FG)
            .context_menu(vec![
                MenuItem::new("Copy")
                    .with_state(context_action)
                    .action(|a| a.set("Copied!".to_string())),
                MenuItem::new("Cut")
                    .with_state(context_action)
                    .action(|a| a.set("Cut!".to_string())),
                MenuItem::new("Paste")
                    .with_state(context_action)
                    .action(|a| a.set("Pasted!".to_string())),
                MenuItem::new("Select All")
                    .with_state(context_action)
                    .action(|a| a.set("Selected all!".to_string())),
            ]),
        spacer().height(12.0),
        text!("{context_action}")
            .font(font::Caption)
            .foreground(MutedForeground),
    ))
    .padding()
}

/// Section demonstrating context menu on different views
fn context_menu_views_section(view_action: &Binding<String>) -> impl View {
    vstack((
        text("Context Menu on Views").sub_headline(),
        text("Long press any colored box")
            .body()
            .foreground(MutedForeground),
        spacer().height(12.0),
        hstack((
            text("Red")
                .foreground(WHITE)
                .padding()
                .background(RED)
                .context_menu(vec![
                    MenuItem::new("Red Action 1")
                        .with_state(view_action)
                        .action(|a| a.set("Red Action 1".to_string())),
                    MenuItem::new("Red Action 2")
                        .with_state(view_action)
                        .action(|a| a.set("Red Action 2".to_string())),
                ]),
            spacer().width(12.0),
            text("Green")
                .foreground(WHITE)
                .padding()
                .background(GREEN)
                .context_menu(vec![
                    MenuItem::new("Green Action 1")
                        .with_state(view_action)
                        .action(|a| a.set("Green Action 1".to_string())),
                    MenuItem::new("Green Action 2")
                        .with_state(view_action)
                        .action(|a| a.set("Green Action 2".to_string())),
                ]),
            spacer().width(12.0),
            text("Blue")
                .foreground(WHITE)
                .padding()
                .background(BLUE)
                .context_menu(vec![
                    MenuItem::new("Blue Action 1")
                        .with_state(view_action)
                        .action(|a| a.set("Blue Action 1".to_string())),
                    MenuItem::new("Blue Action 2")
                        .with_state(view_action)
                        .action(|a| a.set("Blue Action 2".to_string())),
                ]),
        )),
        spacer().height(12.0),
        text!("{view_action}")
            .font(font::Caption)
            .foreground(MutedForeground),
    ))
    .padding()
}

fn main() -> impl View {
    let menu_selected = Binding::container(String::from("None"));
    let styled_action = Binding::container(String::from("No action yet"));
    let context_action = Binding::container(String::from("No action yet"));
    let view_action = Binding::container(String::from("No action yet"));

    scroll(
        vstack((
            // Header
            text("WaterUI Menu Examples").headline(),
            text("Demonstrating Menu and Context Menu components")
                .body()
                .foreground(MutedForeground),
            Divider,
            spacer().height(8.0),
            // Menu component section
            menu_section(&menu_selected),
            Divider,
            // Styled menu section
            styled_menu_section(&styled_action),
            Divider,
            // Context menu section
            context_menu_section(&context_action),
            Divider,
            // Context menu on different views
            context_menu_views_section(&view_action),
            spacer().height(40.0),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
