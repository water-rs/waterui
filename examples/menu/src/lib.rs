//! Menu Example - Demonstrates WaterUI's menu components
//!
//! This example showcases:
//! - `Menu` as a semantic popup menu surface
//! - Nested menus built with normal `Menu::new(...)`
//! - `ContextMenu` and popup rows built from ordinary buttons

use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::reactive::Binding;
use waterui::window::Window;

const BLUE: Srgb = Srgb::from_hex("#2196F3");
const ORANGE_BG: Srgb = Srgb::from_hex("#FFF3E0");
const ORANGE_FG: Srgb = Srgb::from_hex("#E65100");
const RED: Srgb = Srgb::from_hex("#F44336");
const GREEN: Srgb = Srgb::from_hex("#4CAF50");
const WHITE: Srgb = Srgb::from_hex("#FFFFFF");

fn toolbar_action(
    title: &'static str,
    icon_tag: &'static str,
    message: &'static str,
    status: &Binding<String>,
) -> impl View {
    button(label(title).icon(text(icon_tag).caption().bold()))
        .borderless()
        .with_state(status)
        .action(move |status| status.set(message.to_string()))
}

fn toolbar_actions(status: &Binding<String>) -> impl View {
    hstack((
        toolbar_action("Search", "[S]", "Toolbar search", status),
        toolbar_action("Compose", "[+]", "Toolbar compose", status),
        toolbar_action("Settings", "[=]", "Toolbar settings", status),
    ))
    .spacing(8.0)
}

fn toolbar_scene_section(status: &Binding<String>) -> impl View {
    vstack((
        text("Toolbar Labels").sub_headline(),
        text("The same semantic labels can render with text in content and icon-only in compact chrome. Screen readers keep reading the semantic title.")
            .body()
            .foreground(MutedForeground),
        spacer().height(12.0),
        text("Regular content").caption().foreground(MutedForeground),
        toolbar_actions(status),
        spacer().height(8.0),
        text("Compact toolbar").caption().foreground(MutedForeground),
        toolbar_actions(status).install(LabelDisplayMode::IconOnly),
        spacer().height(12.0),
        text!("Toolbar action: {status}")
            .font(font::Caption)
            .foreground(MutedForeground),
    ))
    .padding()
}

fn window_toolbar(status: &Binding<String>) -> impl View {
    toolbar_actions(status).padding_with(EdgeInsets::symmetric(6.0, 8.0))
}

fn menu_section(selected: &Binding<String>) -> impl View {
    vstack((
        text("Menu Component").sub_headline(),
        text("Tap the menu button to see nested buttons, submenus, and separators")
            .body()
            .foreground(MutedForeground),
        spacer().height(12.0),
        Menu::new(
            "Choose an Option",
            (
                button("Option A")
                    .with_state(selected)
                    .action(|selected| selected.set("Option A".to_string())),
                button("Option B")
                    .with_state(selected)
                    .action(|selected| selected.set("Option B".to_string())),
                Divider,
                Menu::new(
                    "More Options",
                    (
                        button("Option C")
                            .with_state(selected)
                            .action(|selected| selected.set("Option C".to_string())),
                        button("Reset")
                            .with_state(selected)
                            .action(|selected| selected.set("None".to_string())),
                    ),
                ),
            ),
        ),
        spacer().height(12.0),
        hstack((
            text("Selected: ").caption().foreground(MutedForeground),
            text!("{selected}").body(),
        )),
    ))
    .padding()
}

fn styled_menu_section(action_log: &Binding<String>) -> impl View {
    vstack((
        text("Styled Menu").sub_headline(),
        text("The popup label stays a normal label, and menu rows can now be plain buttons")
            .body()
            .foreground(MutedForeground),
        spacer().height(12.0),
        Menu::new(
            text("Actions").bold(),
            (
                button("Edit")
                    .with_state(action_log)
                    .action(|log| log.set("Edit action triggered".to_string())),
                button("Duplicate")
                    .with_state(action_log)
                    .action(|log| log.set("Duplicate action triggered".to_string())),
                Divider,
                Menu::new(
                    "Danger Zone",
                    (button("Delete")
                        .with_state(action_log)
                        .action(|log| log.set("Delete action triggered".to_string())),),
                ),
            ),
        ),
        spacer().height(12.0),
        text!("{action_log}")
            .font(font::Caption)
            .foreground(MutedForeground),
    ))
    .padding()
}

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
            .context_menu((
                button("Copy")
                    .with_state(context_action)
                    .action(|action| action.set("Copied!".to_string())),
                button("Cut")
                    .with_state(context_action)
                    .action(|action| action.set("Cut!".to_string())),
                Divider,
                button("Paste")
                    .with_state(context_action)
                    .action(|action| action.set("Pasted!".to_string())),
                button("Select All")
                    .with_state(context_action)
                    .action(|action| action.set("Selected all!".to_string())),
            )),
        spacer().height(12.0),
        text!("{context_action}")
            .font(font::Caption)
            .foreground(MutedForeground),
    ))
    .padding()
}

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
                .context_menu((
                    button("Red Action 1")
                        .with_state(view_action)
                        .action(|action| action.set("Red Action 1".to_string())),
                    button("Red Action 2")
                        .with_state(view_action)
                        .action(|action| action.set("Red Action 2".to_string())),
                )),
            spacer().width(12.0),
            text("Green")
                .foreground(WHITE)
                .padding()
                .background(GREEN)
                .context_menu((
                    button("Green Action 1")
                        .with_state(view_action)
                        .action(|action| action.set("Green Action 1".to_string())),
                    button("Green Action 2")
                        .with_state(view_action)
                        .action(|action| action.set("Green Action 2".to_string())),
                )),
            spacer().width(12.0),
            text("Blue")
                .foreground(WHITE)
                .padding()
                .background(BLUE)
                .context_menu((
                    button("Blue Action 1")
                        .with_state(view_action)
                        .action(|action| action.set("Blue Action 1".to_string())),
                    button("Blue Action 2")
                        .with_state(view_action)
                        .action(|action| action.set("Blue Action 2".to_string())),
                )),
        )),
        spacer().height(12.0),
        text!("{view_action}")
            .font(font::Caption)
            .foreground(MutedForeground),
    ))
    .padding()
}

fn main(toolbar_status: Binding<String>) -> impl View {
    let menu_selected = Binding::container(String::from("None"));
    let styled_action = Binding::container(String::from("No action yet"));
    let context_action = Binding::container(String::from("No action yet"));
    let view_action = Binding::container(String::from("No action yet"));

    scroll(
        vstack((
            text("WaterUI Menu Examples").headline(),
            text("Demonstrating popup menus, nested menus, and context menus")
                .body()
                .foreground(MutedForeground),
            Divider,
            spacer().height(8.0),
            menu_section(&menu_selected),
            Divider,
            styled_menu_section(&styled_action),
            Divider,
            context_menu_section(&context_action),
            Divider,
            context_menu_views_section(&view_action),
            Divider,
            toolbar_scene_section(&toolbar_status),
            spacer().height(40.0),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    let toolbar_status = Binding::container(String::from("No toolbar action yet"));
    let content_toolbar_status = toolbar_status.clone();

    App::new_with_windows(
        [Window::new("WaterUI Menu Examples", move || {
            main(content_toolbar_status.clone())
        })
        .toolbar(window_toolbar(&toolbar_status))],
        env,
    )
}

waterui_ffi::export!();
