//! Multi-Window Example - Demonstrates WaterUI's multi-window capabilities
//!
//! This example showcases:
//! - Creating and managing multiple windows
//! - Different window styles (Titled, Borderless, FullSizeContentView)
//! - Window backgrounds with Color and Material blur effects
//! - Window state management and control
//! - Reactive window handles
//! - Maximize, always-on-top level, attention requests, and resize increments

use std::time::Duration;

use waterui::app::App;
use waterui::background::Material;
use waterui::prelude::theme_color::SurfaceVariant;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::task::{sleep, spawn_local};
use waterui::window::{
    UserAttention, Window, WindowHandle, WindowLevel, WindowPresentation, WindowState, WindowStyle,
    conditional_window,
};

#[preview]
pub fn demo() -> impl View {
    // Reactive state to track window states. `WindowState::default()` is
    // `Closed`, so a fresh binding represents a window that has not yet been
    // shown.
    let standard_state = binding::<WindowState>(WindowState::default());
    let borderless_state = binding::<WindowState>(WindowState::default());
    let frosted_state = binding::<WindowState>(WindowState::default());
    let transparent_state = binding::<WindowState>(WindowState::default());
    let ultra_thin_state = binding::<WindowState>(WindowState::default());
    let controls_state = binding::<WindowState>(WindowState::default());
    let standard_window = WindowPresentation::new(&standard_state);
    let borderless_window = WindowPresentation::new(&borderless_state);
    let frosted_window = WindowPresentation::new(&frosted_state);
    let transparent_window = WindowPresentation::new(&transparent_state);
    let ultra_thin_window = WindowPresentation::new(&ultra_thin_state);
    let controls_window = WindowPresentation::new(&controls_state);

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
                    spacer().height(16.0),
                    // Section 6: Window Controls
                    window_section(
                        "Window Controls",
                        "Maximize, always-on-top, attention requests, resize increments",
                        &controls_state,
                    ),
                ))
                .padding_with(12.0),
                spacer(),
                Divider,
                spacer().height(12.0),
                text("Built with WaterUI Multi-Window Support").caption(),
                spacer().height(12.0),
            ))
            .padding_with(20.0),
        ),
        // Conditionally render windows based on state (invisible triggers)
        conditional_window(&standard_window, create_standard_window),
        conditional_window(&borderless_window, create_borderless_window),
        conditional_window(&frosted_window, create_frosted_window),
        conditional_window(&transparent_window, create_transparent_window),
        conditional_window(&ultra_thin_window, create_ultra_thin_window),
        conditional_window(&controls_window, create_controls_window),
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
    .padding_with(16.0)
    .background(SurfaceVariant)
}

/// Create a standard titled window with opaque background
fn create_standard_window(state: Binding<WindowState>) -> Window {
    Window::new("Standard Window", state, move || {
        window_content(
            "Standard Titled Window",
            "This window uses the default Titled style with an Opaque background.\n\nFeatures:\n• Title bar with controls\n• Opaque system background\n• Resizable and closable",
        )
    })
    .style(WindowStyle::Titled)
    // Default is opaque, no need to set background
    .resizable(true)
}

/// Create a borderless window with colored background
fn create_borderless_window(state: Binding<WindowState>) -> Window {
    let tinted_color = Color::srgb_f32(0.2, 0.4, 0.8).with_opacity(0.85);

    Window::new("Borderless Window", state, move || {
        window_content(
            "Borderless Window",
            "This window has no title bar and uses a semi-transparent blue background.\n\nFeatures:\n• No title bar\n• Custom colored background\n• Semi-transparent (85% opacity)",
        )
    })
    .style(WindowStyle::Borderless)
    .background(tinted_color)
    .resizable(true)
}

/// Create a frosted glass window with material blur
fn create_frosted_window(state: Binding<WindowState>) -> Window {
    Window::new("Frosted Glass", state, move || {
        window_content(
            "Frosted Glass Window",
            "This window's background is the Regular material, blended within the window.\n\nFeatures:\n• Titled style\n• Within-window material background\n• Frosted over the window's own background",
        )
    })
    .style(WindowStyle::Titled)
    .background(Material::Regular)
    .resizable(true)
}

/// Create a transparent overlay window
fn create_transparent_window(state: Binding<WindowState>) -> Window {
    // Use a semi-transparent color for the overlay effect
    let overlay_color = Color::srgb_f32(0.1, 0.1, 0.1).with_opacity(0.3);

    Window::new("Transparent Overlay", state, transparent_window_content)
        .style(WindowStyle::FullSizeContentView)
        .background(overlay_color)
        .resizable(true)
}

/// Create an ultra-thin material window
fn create_ultra_thin_window(state: Binding<WindowState>) -> Window {
    Window::new("Ultra-Thin Material", state, move || {
        window_content(
            "Ultra-Thin Material Window",
            "This window's background is the UltraThin material, blended behind the window.\n\nFeatures:\n• Borderless style\n• The desktop shows through the window\n• Most transparent material",
        )
    })
    .style(WindowStyle::Borderless)
    .background(Material::UltraThin)
    .resizable(true)
}

/// Create a window showcasing the window-control API: maximizing, an
/// always-on-top toggle, an attention request after a delay, and resize
/// increments.
fn create_controls_window(state: Binding<WindowState>) -> Window {
    let always_on_top = binding(false);
    let mut window = Window::new("Window Controls", state, || ())
        .style(WindowStyle::Titled)
        .resizable(true)
        .level(always_on_top.map(|on| {
            if on {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            }
        }))
        .resize_increments(Size::new(80.0, 24.0));
    // The content drives the window through its handle, which is only
    // available once the `Window` exists, so the builder is installed after.
    let handle = window.handle();
    window.content = waterui::handler::AnyViewBuilder::new(move || {
        AnyView::new(controls_window_content(&handle, &always_on_top))
    });
    window
}

/// Content of the controls window: buttons driving its `WindowHandle`.
fn controls_window_content(handle: &WindowHandle, always_on_top: &Binding<bool>) -> impl View {
    vstack((
        text("Window Controls").title().bold(),
        spacer().height(16.0),
        text("Drive this window through its `WindowHandle`. The attention request fires three seconds after the button is pressed, and resizing snaps to 80x24 steps.")
            .body(),
        spacer().height(24.0),
        toggle("Always on Top", always_on_top),
        spacer().height(12.0),
        hstack((
            button("Maximize").action({
                let handle = handle.clone();
                move || handle.maximize()
            }),
            spacer().width(12.0),
            button("Restore").action({
                let handle = handle.clone();
                move || handle.restore()
            }),
        )),
        spacer().height(12.0),
        hstack((
            button("Request Attention in 3s").action({
                let handle = handle.clone();
                move || {
                    let handle = handle.clone();
                    spawn_local(async move {
                        sleep(Duration::from_secs(3)).await;
                        handle.request_attention(UserAttention::Informational);
                    });
                }
            }),
            spacer().width(12.0),
            button("Cancel Attention").action({
                let handle = handle.clone();
                move || handle.cancel_attention()
            }),
        )),
    ))
    .padding_with(24.0)
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
    .padding_with(24.0)
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
    .padding_with(24.0)
}

/// Helper to create a colored box
fn colored_box(color: Color, label: &'static str) -> impl View {
    vstack((spacer(), text(label).bold().body(), spacer()))
        .padding_with(32.0)
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
    .padding_vertical(4.0)
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
