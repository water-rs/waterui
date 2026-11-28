//! Drag and Drop Example - Demonstrates `WaterUI`'s drag and drop system
//!
//! This example shows:
//! - Making views draggable with `.draggable()`, carrying an app-defined
//!   `Transferable` type that travels within the process
//! - Creating stateful drop zones with `.state().drop_destination()`; the
//!   handler's first argument type is the type the zone accepts
//! - Using `.drop_hover()` for visual feedback when dragging over drop zone
//! - Spring animations on successful drop

use core::time::Duration;
use waterui::app::{App, Windows};
use waterui::drag_drop::{DropInfo, Transferable};
use waterui::prelude::font::Title;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::Binding;
use waterui::task::{sleep, spawn_local};

const FRUIT_CARD_WIDTH: f32 = 160.0;

/// The value a fruit card's drag carries. Only the basket accepts it.
#[derive(Debug, Clone, PartialEq, Transferable)]
struct Fruit {
    emoji: &'static str,
    label: &'static str,
}

/// A draggable fruit card
fn fruit_card(emoji: &'static str, label: &'static str, color: Color) -> impl View + use<> {
    hstack((text(emoji).font_size(28.0), text(label).font_size(16.0)))
        .spacing(8.0)
        .padding()
        .width(FRUIT_CARD_WIDTH)
        .background(color.with_opacity(0.9))
        .draggable(Fruit { emoji, label })
}

/// Animated basket that collects dropped items
fn fruit_basket(
    is_hovering: &Binding<bool>,
    collected: &Binding<Vec<String>>,
    bounce: &Binding<f32>,
) -> impl View + use<> {
    // Scale up when hovering (SignalExt methods take &self and clone internally)
    let hover_scale = is_hovering
        .select(1.05, 1.0)
        .with(Animation::spring(400.0, 15.0));

    // Bounce animation on drop
    let drop_bounce = bounce.with(Animation::spring(500.0, 10.0));

    // Combined scale for uniform scaling
    let combined_scale = hover_scale.zip(&drop_bounce).map(|(a, b)| a * b);

    // Display collected emojis only (no text)
    let emojis_display = collected.map(|list| {
        if list.is_empty() {
            "🧺".to_string()
        } else {
            // Extract just the emoji from each item
            list.iter()
                .filter_map(|s| s.chars().next())
                .collect::<String>()
        }
    });

    let count_display = collected.map(|list| {
        if list.is_empty() {
            "Drop fruits here!".to_string()
        } else {
            format!(
                "{} fruit{} collected!",
                list.len(),
                if list.len() == 1 { "" } else { "s" }
            )
        }
    });

    vstack((
        text!("{emojis_display}").font_size(40.0),
        text!("{count_display}").font_size(14.0),
    ))
    .spacing(12.0)
    .padding_with(24.0)
    .min_width(280.0)
    .min_height(120.0)
    .background(Color::srgb_hex("#10B981").with_opacity(0.2))
    .scale(combined_scale)
    .border(Color::srgb_hex("#10B981"), 3.0)
    .drop_destination(
        |fruit: Fruit,
         info: DropInfo,
         State(collected): State<Binding<Vec<String>>>,
         State(bounce): State<Binding<f32>>| {
            let point = info.location();
            waterui::log::debug!(
                x = point.x,
                y = point.y,
                fruit = fruit.label,
                "Dropped fruit into basket"
            );
            // Add to collection
            let dropped_item = format!("{} {}", fruit.emoji, fruit.label);
            let mut current_items = collected.snapshot();
            if !current_items.iter().any(|x| x == &dropped_item) {
                current_items.push(dropped_item);
                collected.set(current_items);
            }
            // Trigger bounce animation
            let current = bounce.snapshot();
            let target = if (current - 1.2).abs() < 0.01 {
                1.25
            } else {
                1.2
            };
            bounce.set(target);
            spawn_local(async move {
                sleep(Duration::from_millis(200)).await;
                bounce.set(1.0);
            })
            .detach();
        },
    )
    .on_enter(|info: DropInfo, State(hover): State<Binding<bool>>| {
        let point = info.location();
        waterui::log::debug!(x = point.x, y = point.y, "Drag entered basket");
        hover.set(true);
    })
    .on_exit(|State(hover): State<Binding<bool>>| {
        hover.set(false);
    })
    .state(is_hovering)
    .state(collected)
    .state(bounce)
}

/// The drag-and-drop demo: draggable fruit cards and a basket that collects them.
#[preview]
pub fn demo() -> impl View {
    let is_hovering = Binding::new(false);
    let collected: Binding<Vec<String>> = Binding::new(Vec::new());
    let bounce = Binding::new(1.0_f32);

    scroll(
        vstack((
            text("Fruit Basket").font(Title),
            "Drag fruits into the basket!",
            Divider,
            // Fruits to drag
            vstack((
                text("Drag these fruits").font_size(14.0),
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
            fruit_basket(&is_hovering, &collected, &bounce),
            spacer().height(16.0),
            // Reset button
            button("Clear Basket")
                .action(|State(c): State<Binding<Vec<String>>>| c.set(Vec::new()))
                .state(&collected),
            spacer(),
        ))
        .padding(),
    )
}

/// Builds the drag-and-drop app around [`demo`].
pub fn app(env: Environment) -> App<impl Windows> {
    App::new(env).window(Window::new("", demo))
}
