//! Drag and Drop Example - Demonstrates WaterUI's drag and drop system
//!
//! This example shows:
//! - Making views draggable with `.draggable()`
//! - Creating drop zones with `.drop_destination_with_events()`
//! - Visual feedback when dragging over drop zone
//! - Spring animations on successful drop

use core::time::Duration;
use waterui::animation::Animation;
use waterui::app::App;
use waterui::drag_drop::DragData;
use waterui::prelude::font::Title;
use waterui::prelude::*;
use waterui::reactive::Binding;
use waterui::task::{sleep, spawn_local};
use waterui_core::extract::Use;

/// A draggable fruit card
fn fruit_card(emoji: &'static str, label: &'static str, color: Color) -> impl View {
    hstack((text(emoji).size(28.0), text(label).size(16.0)))
        .spacing(8.0)
        .padding()
        .background(color.with_opacity(0.9))
        .draggable(DragData::text(format!("{} {}", emoji, label)))
}

/// Animated basket that collects dropped items
fn fruit_basket(
    is_hovering: Binding<bool>,
    collected: Binding<Vec<String>>,
    bounce: Binding<f32>,
) -> impl View {
    // Scale up when hovering
    let hover_scale = is_hovering
        .clone()
        .map(|h| if h { 1.05_f32 } else { 1.0 })
        .with(Animation::spring(400.0, 15.0));

    // Bounce animation on drop
    let drop_bounce = bounce.clone().with(Animation::spring(500.0, 10.0));

    // Display collected emojis only (no text)
    let items_display = collected.clone().map(|items| {
        if items.is_empty() {
            "🧺".to_string()
        } else {
            // Extract just the emoji from each item
            items
                .iter()
                .filter_map(|s| s.chars().next())
                .collect::<String>()
        }
    });

    let count_display = collected.clone().map(|items| {
        if items.is_empty() {
            "Drop fruits here!".to_string()
        } else {
            format!("{} fruit{} collected!", items.len(), if items.len() == 1 { "" } else { "s" })
        }
    });

    let is_h = is_hovering.clone();
    let coll = collected.clone();
    let b = bounce.clone();

    vstack((
        text(items_display).size(40.0),
        text(count_display).size(14.0),
    ))
    .spacing(12.0)
    .padding_with(EdgeInsets::all(24.0))
    .min_width(280.0)
    .min_height(120.0)
    .background(Color::srgb_hex("#10B981").with_opacity(0.2))
    .scale(hover_scale.zip(drop_bounce).map(|(a, b)| a * b))
    .border(Color::srgb_hex("#10B981"), 3.0)
    .drop_destination_with_events(
        // On drop - add to collection with bounce animation
        move |Use(data): Use<DragData>| {
            let item = data.as_str().to_string();
            let mut items = coll.get();
            if !items.contains(&item) {
                items.push(item);
                coll.set(items);
            }
            // Trigger bounce - use a unique value each time to ensure animation triggers
            let current = b.get();
            let target = if (current - 1.2).abs() < 0.01 { 1.25 } else { 1.2 };
            b.set(target);
            let b2 = b.clone();
            spawn_local(async move {
                sleep(Duration::from_millis(200)).await;
                b2.set(1.0);
            });
        },
        // On enter
        {
            let h = is_h.clone();
            move || h.set(true)
        },
        // On exit
        {
            let h = is_h.clone();
            move || h.set(false)
        },
    )
}

#[hot_reload]
fn main() -> impl View {
    let is_hovering = Binding::bool(false);
    let collected: Binding<Vec<String>> = Binding::container(Vec::new());
    let bounce = Binding::container(1.0_f32);

    scroll(
        vstack((
            text("Fruit Basket").font(Title),
            "Drag fruits into the basket!",
            Divider,
            // Fruits to drag
            vstack((
                text("Drag these fruits").size(14.0),
                hstack((
                    fruit_card("🍎", "Apple", Color::srgb_hex("#EF4444")),
                    fruit_card("🍊", "Orange", Color::srgb_hex("#F97316")),
                ))
                .spacing(12.0),
                hstack((
                    fruit_card("🍋", "Lemon", Color::srgb_hex("#EAB308")),
                    fruit_card("🍇", "Grape", Color::srgb_hex("#8B5CF6")),
                ))
                .spacing(12.0),
                hstack((
                    fruit_card("🍓", "Strawberry", Color::srgb_hex("#EC4899")),
                    fruit_card("🥝", "Kiwi", Color::srgb_hex("#22C55E")),
                ))
                .spacing(12.0),
            ))
            .spacing(12.0)
            .padding(),
            spacer().height(32.0),
            // Drop basket
            fruit_basket(is_hovering, collected.clone(), bounce),
            spacer().height(16.0),
            // Reset button
            button("Clear Basket").action_with(&collected, |c: Binding<Vec<String>>| c.set(Vec::new())),
            spacer(),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
