//! Drag and Drop Example - Demonstrates WaterUI's native drag and drop capabilities
//!
//! This example showcases:
//! - Making views draggable with `.draggable()`
//! - Creating drop destinations with `.drop_destination()`
//! - Handling drop events with data extraction

use waterui::app::App;
use waterui::drag_drop::DragData;
use waterui::prelude::font::{Headline, Title};
use waterui::prelude::*;
use waterui::reactive::Binding;
use waterui_core::extract::Use;

/// A draggable colored card
fn draggable_card(label: &'static str, color: Color, data: DragData) -> impl View {
    text(label)
        .padding()
        .width(120.0)
        .height(60.0)
        .background(color.with_opacity(0.8))
        .draggable(data)
}

/// Drop zone that shows what was dropped
fn drop_zone(dropped_text: Binding<String>) -> impl View {
    vstack((
        text("Drop Zone").font(Title),
        {
            let dropped_text = dropped_text.clone();
            text(dropped_text.clone())
                .padding()
                .width(250.0)
                .height(100.0)
                .background(Color::srgb_hex("#E0E0E0").with_opacity(0.3))
                .drop_destination(move |Use(data): Use<DragData>| {
                    let display_text = match &data {
                        DragData::Text(s) => format!("Text: {}", s),
                        DragData::Url(u) => format!("URL: {}", u),
                        _ => "Unknown data type".to_string(),
                    };
                    dropped_text.set(display_text);
                })
        },
        "Drag items above and drop here",
    ))
    .spacing(8.0)
}

/// Section with draggable text items
fn draggable_text_section() -> impl View {
    vstack((
        text("Text Items").font(Title),
        "Drag these items to the drop zone",
        hstack((
            draggable_card(
                "Hello",
                Color::srgb_hex("#2196F3"),
                DragData::text("Hello, World!"),
            ),
            draggable_card(
                "WaterUI",
                Color::srgb_hex("#9C27B0"),
                DragData::text("Welcome to WaterUI!"),
            ),
            draggable_card(
                "Rust 🦀",
                Color::srgb_hex("#FF9800"),
                DragData::text("Built with Rust"),
            ),
        ))
        .spacing(12.0),
    ))
    .spacing(8.0)
    .padding()
}

/// Section with draggable URL items
fn draggable_url_section() -> impl View {
    vstack((
        text("URL Items").font(Title),
        "Drag URLs to the drop zone",
        hstack((
            draggable_card(
                "GitHub",
                Color::srgb_hex("#333333"),
                DragData::url("https://github.com"),
            ),
            draggable_card(
                "Rust Lang",
                Color::srgb_hex("#DEA584"),
                DragData::url("https://rust-lang.org"),
            ),
        ))
        .spacing(12.0),
    ))
    .spacing(8.0)
    .padding()
}

#[hot_reload]
fn main() -> impl View {
    let dropped_text = Binding::container(String::from("Drop something here..."));

    scroll(
        vstack((
            // Header
            text("WaterUI Drag & Drop").font(Headline),
            "Native drag and drop for Apple and Android",
            Divider,
            spacer(),
            // Draggable sections
            draggable_text_section(),
            Divider,
            draggable_url_section(),
            Divider,
            // Drop zone
            drop_zone(dropped_text),
            spacer(),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
