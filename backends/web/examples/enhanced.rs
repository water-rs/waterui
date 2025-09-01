use wasm_bindgen::prelude::*;
use waterui::{Environment, component::text};
use waterui_web::{WebApp, init};
use waterui_web::widgets::{
    button::{render_primary_button, render_text_button, ButtonConfig, ButtonVariant},
    layout::{render_hstack, render_vstack, render_spacer},
    media::{render_image, ImageConfig, ImageFit},
    navigation::{render_tab_view, render_tab_item, render_navigation_link},
    general::render_divider,
};

#[wasm_bindgen(start)]
pub fn main() {
    init();
    
    let app = WebApp::new("enhanced-app");
    let env = Environment::new();
    
    let content = text("Enhanced WaterUI Web Backend Demo");
    
    if let Err(e) = app.environment(env).render(content) {
        web_sys::console::log_1(&format!("Error rendering app: {:?}", e).into());
    }
}

#[wasm_bindgen]
pub fn demo_buttons() {
    web_sys::console::log_1(&"Demonstrating button components".into());
    
    // Create primary button with click handler
    let primary_btn = render_primary_button("Click me!", Some(Box::new(|| {
        web_sys::console::log_1(&"Primary button clicked!".into());
    })));
    
    // Create text button
    let text_btn = render_text_button("Cancel", None);
    
    // Create custom button
    let custom_btn = waterui_web::widgets::button::render_button(
        ButtonConfig {
            text: "Custom".to_string(),
            disabled: false,
            variant: ButtonVariant::Outline,
        },
        Some(Box::new(|| {
            web_sys::console::log_1(&"Custom button clicked!".into());
        }))
    );
    
    web_sys::console::log_1(&"Button demo complete".into());
}

#[wasm_bindgen]
pub fn demo_layout() {
    web_sys::console::log_1(&"Demonstrating layout components".into());
    
    // Create horizontal stack
    let hstack = render_hstack();
    
    // Create vertical stack
    let vstack = render_vstack();
    
    // Create spacer
    let spacer = render_spacer();
    
    web_sys::console::log_1(&"Layout demo complete".into());
}

#[wasm_bindgen]
pub fn demo_media() {
    web_sys::console::log_1(&"Demonstrating media components".into());
    
    // Create image
    let image = render_image(ImageConfig {
        src: "https://example.com/image.jpg".to_string(),
        alt: Some("Demo image".to_string()),
        width: Some(300.0),
        height: Some(200.0),
        fit: ImageFit::Cover,
    });
    
    // Create video
    let video = waterui_web::widgets::media::render_video(
        "https://example.com/video.mp4",
        true,  // controls
        false, // autoplay
        true   // muted
    );
    
    // Create canvas
    let canvas = waterui_web::widgets::media::render_canvas(400.0, 300.0);
    
    web_sys::console::log_1(&"Media demo complete".into());
}

#[wasm_bindgen]
pub fn demo_navigation() {
    web_sys::console::log_1(&"Demonstrating navigation components".into());
    
    // Create navigation view
    let nav_view = waterui_web::widgets::navigation::render_navigation_view();
    
    // Create navigation link
    let nav_link = render_navigation_link("Home", Some("/home"));
    
    // Create tab view
    let tab_view = render_tab_view();
    
    // Create tab items
    let active_tab = render_tab_item("Tab 1", true);
    let inactive_tab = render_tab_item("Tab 2", false);
    
    web_sys::console::log_1(&"Navigation demo complete".into());
}

#[wasm_bindgen]
pub fn demo_complete_ui() {
    web_sys::console::log_1(&"Demonstrating complete UI composition".into());
    
    // This would show how to compose multiple components together
    // In a real implementation, we'd need proper view composition
    
    let divider = render_divider();
    
    web_sys::console::log_1(&"Complete UI demo ready".into());
}