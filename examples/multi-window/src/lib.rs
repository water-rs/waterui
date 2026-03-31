//! Multi-Window Example - Demonstrates WaterUI's multi-window capabilities
//!
//! This example showcases:
//! - Creating and managing multiple windows
//! - Different window styles (Titled, Borderless, FullSizeContentView)
//! - Window backgrounds with Color and Material blur effects
//! - Window state management and control
//! - Reactive window handles

use waterui::app::App;
use waterui::background::Material;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::window::{Window, WindowState, WindowStyle, conditional_window};

fn main() -> impl View {
    // Reactive state to track window states
    let standard_state = binding(WindowState::Closed);
    let borderless_state = binding(WindowState::Closed);
    let frosted_state = binding(WindowState::Closed);
    let transparent_state = binding(WindowState::Closed);
    let ultra_thin_state = binding(WindowState::Closed);

    // Use zstack so the invisible window triggers don't affect scroll layout
    zstack((
        scroll(
            vstack((
                // Header
                text("Multi-Window Gallery").title().bold(),
                text("Explore different window styles and backgrounds").body(),
                spacer().height(20.0),
                Divider,
                spacer().height(20.0),
                // Main content
                vstack((
                    // Section 1: Standard Window
                    window_section(
                        "Standard Titled Window",
                        "Classic window with title bar and opaque background",
                        &standard_state,
                    ),
                    spacer().height(16.0),
                    // Section 2: Borderless Window
                    window_section(
                        "Borderless Window",
                        "Frameless window with colored semi-transparent background",
                        &borderless_state,
                    ),
                    spacer().height(16.0),
                    // Section 3: Frosted Glass Window
                    window_section(
                        "Frosted Glass Window",
                        "Window with material blur effect (Regular thickness)",
                        &frosted_state,
                    ),
                    spacer().height(16.0),
                    // Section 4: Transparent Window
                    window_section(
                        "Transparent Overlay",
                        "Fully transparent window with FullSizeContentView style",
                        &transparent_state,
                    ),
                    spacer().height(16.0),
                    // Section 5: Ultra-Thin Material Window
                    window_section(
                        "Ultra-Thin Material Window",
                        "Subtle frosted effect with UltraThin material",
                        &ultra_thin_state,
                    ),
                ))
                .padding_with(EdgeInsets::all(12.0)),
                spacer(),
                Divider,
                spacer().height(12.0),
                text("Built with WaterUI Multi-Window Support").caption(),
                spacer().height(12.0),
            ))
            .padding_with(EdgeInsets::all(20.0)),
        ),
        // Conditionally render windows based on state (invisible triggers)
        conditional_window(standard_state.clone(), |state| {
            create_standard_window(state)
        }),
        conditional_window(borderless_state.clone(), |state| {
            create_borderless_window(state)
        }),
        conditional_window(frosted_state.clone(), |state| create_frosted_window(state)),
        conditional_window(transparent_state.clone(), |state| {
            create_transparent_window(state)
        }),
        conditional_window(ultra_thin_state.clone(), |state| {
            create_ultra_thin_window(state)
        }),
    ))
}

/// Helper function to create a window section with open and close buttons
fn window_section(
    title: &'static str,
    description: &'static str,
    state: &Binding<WindowState>,
) -> impl View {
    vstack((
        text(title).headline().bold(),
        text(description).body(),
        spacer().height(8.0),
        hstack((
            button("Open Window")
                .action(|State(s): State<Binding<WindowState>>| s.set(WindowState::Normal))
                .state(state),
            spacer().width(12.0),
            button("Close Window")
                .action(|State(s): State<Binding<WindowState>>| s.set(WindowState::Closed))
                .state(state),
        )),
    ))
    .padding_with(EdgeInsets::all(16.0))
    .background(Color::srgb_f32(0.25, 0.27, 0.30))
}

/// Create a standard titled window with opaque background
fn create_standard_window(state: Binding<WindowState>) -> Window {
    Window::new(
        "Standard Window",
        move || {
            window_content(
                "Standard Titled Window",
                "This window uses the default Titled style with an Opaque background.\n\nFeatures:\n• Title bar with controls\n• Opaque system background\n• Resizable and closable",
            )
        },
    )
        .style(WindowStyle::Titled)
        // Default is opaque, no need to set background
        .resizable(true)
        .with_state(state)
}

/// Create a borderless window with colored background
fn create_borderless_window(state: Binding<WindowState>) -> Window {
    let tinted_color = Color::srgb_f32(0.2, 0.4, 0.8).with_opacity(0.85);

    Window::new(
        "Borderless Window",
        move || {
            window_content(
                "Borderless Window",
                "This window has no title bar and uses a semi-transparent blue background.\n\nFeatures:\n• No title bar\n• Custom colored background\n• Semi-transparent (85% opacity)",
            )
        },
    )
        .style(WindowStyle::Borderless)
        .background(tinted_color)
        .resizable(true)
        .with_state(state)
}

/// Create a frosted glass window with material blur
fn create_frosted_window(state: Binding<WindowState>) -> Window {
    Window::new(
        "Frosted Glass",
        move || {
            window_content(
                "Frosted Glass Window",
                "This window uses a Regular material blur for a frosted glass effect.\n\nFeatures:\n• Titled style\n• Material blur background\n• See-through with blur effect",
            )
        },
    )
        .style(WindowStyle::Titled)
        .background(Material::Regular)
        .resizable(true)
        .with_state(state)
}

/// Create a transparent overlay window
fn create_transparent_window(state: Binding<WindowState>) -> Window {
    // Use a semi-transparent color for the overlay effect
    let overlay_color = Color::srgb_f32(0.1, 0.1, 0.1).with_opacity(0.3);

    Window::new("Transparent Overlay", move || transparent_window_content())
        .style(WindowStyle::FullSizeContentView)
        .background(overlay_color)
        .resizable(true)
        .with_state(state)
}

/// Create an ultra-thin material window
fn create_ultra_thin_window(state: Binding<WindowState>) -> Window {
    Window::new(
        "Ultra-Thin Material",
        move || {
            window_content(
                "Ultra-Thin Material Window",
                "This window uses an UltraThin material for a subtle frosted effect.\n\nFeatures:\n• Borderless style\n• Ultra-thin blur\n• Most transparent material",
            )
        },
    )
        .style(WindowStyle::Borderless)
        .background(Material::UltraThin)
        .resizable(true)
        .with_state(state)
}

/// Helper function to create window content
fn window_content(title: &'static str, description: &'static str) -> impl View {
    vstack((
        text(title).title().bold(),
        spacer().height(16.0),
        text(description).body(),
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
        text("Transparent Overlay").title().bold(),
        spacer().height(16.0),
        text("This window has a fully transparent background with FullSizeContentView style.")
            .body(),
        text("Content extends into the title bar area on macOS.").body(),
        spacer().height(24.0),
        // Show some colored boxes to demonstrate transparency
        hstack((
            colored_box(Color::srgb_f32(1.0, 0.3, 0.3).with_opacity(0.8), "Red"),
            spacer().width(12.0),
            colored_box(Color::srgb_f32(0.3, 1.0, 0.3).with_opacity(0.8), "Green"),
            spacer().width(12.0),
            colored_box(Color::srgb_f32(0.3, 0.3, 1.0).with_opacity(0.8), "Blue"),
        )),
    ))
    .padding_with(EdgeInsets::all(24.0))
}

/// Helper to create a colored box
fn colored_box(color: Color, label: &'static str) -> impl View {
    vstack((spacer(), text(label).bold().body(), spacer()))
        .padding_with(EdgeInsets::all(32.0))
        .background(color)
}

/// Showcase all material types
fn material_showcase() -> impl View {
    vstack((
        text("Material Types").sub_headline().bold(),
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
        text(name).bold().body().width(100.0),
        text(description).caption(),
    ))
    .padding_with(EdgeInsets::symmetric(4.0, 0.0))
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
