//! Multi-Window Example - Demonstrates WaterUI's multi-window capabilities
//!
//! This example showcases:
//! - Creating and managing multiple windows
//! - Different window styles (Titled, Borderless, FullSizeContentView)
//! - Different window backgrounds (Opaque, Transparent, Color, Material)
//! - Window state management and control
//! - Reactive window handles

use waterui::app::App;
use waterui::background::Material;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::window::{Window, WindowBackground, WindowStyle};

fn main() -> impl View {
    // Reactive state to track which windows are shown
    let show_standard = binding(false);
    let show_borderless = binding(false);
    let show_frosted = binding(false);
    let show_transparent = binding(false);
    let show_ultra_thin = binding(false);

    vstack((
        scroll(
            vstack((
                // Header
                text("Multi-Window Gallery").size(32.0).bold(),
                text("Explore different window styles and backgrounds"),
                spacer().height(20.0),
                Divider,
                spacer().height(20.0),
                // Main content
                vstack((
                    // Section 1: Standard Window
                    window_section(
                        "Standard Titled Window",
                        "Classic window with title bar and opaque background",
                        &show_standard,
                    ),
                    spacer().height(16.0),
                    // Section 2: Borderless Window
                    window_section(
                        "Borderless Window",
                        "Frameless window with colored semi-transparent background",
                        &show_borderless,
                    ),
                    spacer().height(16.0),
                    // Section 3: Frosted Glass Window
                    window_section(
                        "Frosted Glass Window",
                        "Window with material blur effect (Regular thickness)",
                        &show_frosted,
                    ),
                    spacer().height(16.0),
                    // Section 4: Transparent Window
                    window_section(
                        "Transparent Overlay",
                        "Fully transparent window with FullSizeContentView style",
                        &show_transparent,
                    ),
                    spacer().height(16.0),
                    // Section 5: Ultra-Thin Material Window
                    window_section(
                        "Ultra-Thin Material Window",
                        "Subtle frosted effect with UltraThin material",
                        &show_ultra_thin,
                    ),
                ))
                .padding_with(EdgeInsets::all(12.0)),
                spacer(),
                Divider,
                spacer().height(12.0),
                text("Built with WaterUI Multi-Window Support").size(12.0),
                spacer().height(12.0),
            ))
            .padding_with(EdgeInsets::all(20.0)),
        ),
        // Conditionally render windows
        conditional_window(show_standard.clone(), create_standard_window),
        conditional_window(show_borderless.clone(), create_borderless_window),
        conditional_window(show_frosted.clone(), create_frosted_window),
        conditional_window(show_transparent.clone(), create_transparent_window),
        conditional_window(show_ultra_thin.clone(), create_ultra_thin_window),
    ))
}

/// Helper to conditionally render a window based on a boolean binding
fn conditional_window(
    show: Binding<bool>,
    creator: fn() -> Window,
) -> impl View {
    watch(show, move |shown| {
        if shown {
            AnyView::new(creator())
        } else {
            AnyView::new(())
        }
    })
}

/// Helper function to create a window section with open and toggle buttons
fn window_section(
    title: &'static str,
    description: &'static str,
    show: &Binding<bool>,
) -> impl View {
    let show_clone = show.clone();
    let show_for_close = show.clone();

    vstack((
        text(title).size(20.0).bold(),
        text(description),
        spacer().height(8.0),
        hstack((
            button("Open Window").action(move || {
                show_clone.set(true);
            })
            .padding_with(EdgeInsets::symmetric(12.0, 8.0)),
            spacer().width(12.0),
            button("Close Window").action(move || {
                show_for_close.set(false);
            })
            .padding_with(EdgeInsets::symmetric(12.0, 8.0)),
        )),
    ))
    .padding_with(EdgeInsets::all(16.0))
    .background(Color::srgb_f32(0.95, 0.95, 0.95))
}

/// Create a standard titled window with opaque background
fn create_standard_window() -> Window {
    Window::new("Standard Window", window_content("Standard Titled Window", "This window uses the default Titled style with an Opaque background.\n\nFeatures:\n• Title bar with controls\n• Opaque system background\n• Resizable and closable"))
        .style(WindowStyle::Titled)
        .background(WindowBackground::Opaque)
        .resizable(true)
}

/// Create a borderless window with colored background
fn create_borderless_window() -> Window {
    let tinted_color = Color::srgb_f32(0.2, 0.4, 0.8).with_alpha(0.85);

    Window::new("Borderless Window", window_content("Borderless Window", "This window has no title bar and uses a semi-transparent blue background.\n\nFeatures:\n• No title bar\n• Custom colored background\n• Semi-transparent (85% opacity)"))
        .style(WindowStyle::Borderless)
        .background(WindowBackground::Color(tinted_color))
        .resizable(true)
}

/// Create a frosted glass window with material blur
fn create_frosted_window() -> Window {
    Window::new("Frosted Glass", window_content("Frosted Glass Window", "This window uses a Regular material blur for a frosted glass effect.\n\nFeatures:\n• Titled style\n• Material blur background\n• See-through with blur effect"))
        .style(WindowStyle::Titled)
        .background(WindowBackground::Material(Material::Regular))
        .resizable(true)
}

/// Create a transparent overlay window
fn create_transparent_window() -> Window {
    Window::new("Transparent Overlay", transparent_window_content())
        .style(WindowStyle::FullSizeContentView)
        .background(WindowBackground::Transparent)
        .resizable(true)
}

/// Create an ultra-thin material window
fn create_ultra_thin_window() -> Window {
    Window::new("Ultra-Thin Material", window_content("Ultra-Thin Material Window", "This window uses an UltraThin material for a subtle frosted effect.\n\nFeatures:\n• Borderless style\n• Ultra-thin blur\n• Most transparent material"))
        .style(WindowStyle::Borderless)
        .background(WindowBackground::Material(Material::UltraThin))
        .resizable(true)
}

/// Helper function to create window content
fn window_content(title: &'static str, description: &'static str) -> impl View {
    vstack((
        text(title).size(24.0).bold(),
        spacer().height(16.0),
        text(description).size(14.0),
        spacer().height(24.0),
        Divider,
        spacer().height(16.0),
        material_showcase(),
    ))
    .padding_with(EdgeInsets::all(24.0))
}

/// Content for transparent window with colored boxes
fn transparent_window_content() -> impl View {
    vstack((
        text("Transparent Overlay").size(24.0).bold(),
        spacer().height(16.0),
        text("This window has a fully transparent background with FullSizeContentView style.")
            .size(14.0),
        text("Content extends into the title bar area on macOS.").size(14.0),
        spacer().height(24.0),
        // Show some colored boxes to demonstrate transparency
        hstack((
            colored_box(Color::srgb_f32(1.0, 0.3, 0.3).with_alpha(0.8), "Red"),
            spacer().width(12.0),
            colored_box(Color::srgb_f32(0.3, 1.0, 0.3).with_alpha(0.8), "Green"),
            spacer().width(12.0),
            colored_box(Color::srgb_f32(0.3, 0.3, 1.0).with_alpha(0.8), "Blue"),
        )),
    ))
    .padding_with(EdgeInsets::all(24.0))
}

/// Helper to create a colored box
fn colored_box(color: Color, label: &'static str) -> impl View {
    vstack((
        spacer(),
        text(label).bold().size(16.0),
        spacer(),
    ))
    .padding_with(EdgeInsets::all(32.0))
    .background(color)
}

/// Showcase all material types
fn material_showcase() -> impl View {
    vstack((
        text("Material Types").size(18.0).bold(),
        spacer().height(12.0),
        material_item("UltraThin", "Most transparent, subtle blur"),
        material_item("Thin", "Light transparency with slight blur"),
        material_item("Regular", "Balanced transparency and blur"),
        material_item("Thick", "More opaque with stronger blur"),
        material_item("UltraThick", "Most opaque, heavy frosted effect"),
    ))
}

/// Helper to display a material type description
fn material_item(name: &'static str, description: &'static str) -> impl View {
    hstack((
        text(name).bold().size(14.0).width(100.0),
        text(description).size(12.0),
    ))
    .padding_with(EdgeInsets::symmetric(4.0, 0.0))
}

pub fn app(env: Environment) -> App {
    App::new(main(), env)
}

waterui_ffi::export!();
