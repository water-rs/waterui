//! Gesture Example - Demonstrates WaterUI's gesture recognition capabilities
//!
//! This example showcases:
//! - Tap gestures (single and multi-tap)
//! - Long press gestures
//! - Drag gestures
//! - Gesture chaining with `.then()`
//! - Using `on_tap` convenience method
//! - Non-primary pointer buttons (middle-click closes a tab)

use waterui::Identifiable;
use waterui::app::App;
use waterui::gesture::{DragGesture, LongPressGesture, PointerButtons, TapGesture};
use waterui::graphics::color::Srgb;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::Binding;
use waterui::reactive::collection::List as ReactiveList;

const TAP_COLOR: Srgb = Srgb::from_hex("#2196F3");
const DOUBLE_TAP_COLOR: Srgb = Srgb::from_hex("#4CAF50");
const LONG_PRESS_COLOR: Srgb = Srgb::from_hex("#FF9800");
const DRAG_COLOR: Srgb = Srgb::from_hex("#9C27B0");
const CHAINED_COLOR: Srgb = Srgb::from_hex("#F44336");
const ON_TAP_COLOR: Srgb = Srgb::from_hex("#00BCD4");

/// Section displaying tap gesture demos
fn tap_section(tap_count: &Binding<i32>) -> impl View {
    vstack((
        text("Tap Gesture").headline(),
        "Tap the box below to increment the counter",
        text!("Tap count: {count}", count = tap_count.clone()),
        text("Tap Me!")
            .padding()
            .background(TAP_COLOR.with_opacity(0.3))
            .gesture(TapGesture::new(), |State(c): State<Binding<i32>>| {
                *c.get_mut() += 1
            })
            .state(tap_count),
    ))
    .padding()
}

/// Section displaying double-tap gesture demo
fn double_tap_section(double_tap_count: &Binding<i32>) -> impl View {
    vstack((
        text("Double Tap Gesture").headline(),
        "Double-tap the box to increment",
        text!(
            "Double tap count: {count}",
            count = double_tap_count.clone()
        ),
        text("Double Tap Me!")
            .padding()
            .background(DOUBLE_TAP_COLOR.with_opacity(0.3))
            .gesture(TapGesture::repeat(2), |State(c): State<Binding<i32>>| {
                *c.get_mut() += 1
            })
            .state(double_tap_count),
    ))
    .padding()
}

/// Section displaying long press gesture demo
fn long_press_section(long_press_count: &Binding<i32>) -> impl View {
    vstack((
        text("Long Press Gesture").headline(),
        "Press and hold for 500ms",
        text!(
            "Long press count: {count}",
            count = long_press_count.clone()
        ),
        text("Long Press Me!")
            .padding()
            .background(LONG_PRESS_COLOR.with_opacity(0.3))
            .gesture(
                LongPressGesture::new(500),
                |State(c): State<Binding<i32>>| *c.get_mut() += 1,
            )
            .state(long_press_count),
    ))
    .padding()
}

/// Section displaying drag gesture demo
fn drag_section(drag_count: &Binding<i32>) -> impl View {
    vstack((
        text("Drag Gesture").headline(),
        "Drag within the box (min 5pt)",
        text!("Drag events: {count}", count = drag_count.clone()),
        text("Drag Here")
            .padding()
            .width(200.0)
            .height(100.0)
            .background(DRAG_COLOR.with_opacity(0.3))
            .gesture(DragGesture::new(5.0), |State(c): State<Binding<i32>>| {
                *c.get_mut() += 1
            })
            .state(drag_count),
    ))
    .padding()
}

/// Section displaying chained gesture demo
fn chained_section(chained_status: &Binding<&'static str>) -> impl View {
    let chained_status_display = chained_status.clone();
    vstack((
        text("Chained Gesture").headline(),
        "Tap first, then long press to complete",
        text!("{chained_status_display}"),
        text("Tap then Long Press")
            .padding()
            .background(CHAINED_COLOR.with_opacity(0.3))
            .gesture(
                TapGesture::new().then(LongPressGesture::new(300)),
                |State(s): State<Binding<&'static str>>| s.set("Chained gesture completed!"),
            )
            .state(chained_status),
    ))
    .padding()
}

/// One tab chip in the middle-click demo strip.
#[derive(Clone, Identifiable)]
struct TabChip {
    #[id]
    id: u32,
    title: &'static str,
}

/// A tab strip where a middle click closes a tab: each chip's
/// `MIDDLE`-only tap runs alongside its primary selection tap without
/// competing for the same press.
fn middle_click_tab_section(tabs: &ReactiveList<TabChip>, open_tabs: &Binding<i32>) -> impl View {
    let tabs_for_rows = tabs.clone();
    let open_tabs_for_handler = open_tabs.clone();
    vstack((
        text("Middle Click").headline(),
        "Middle-click a tab to close it",
        text!("{count} open tabs", count = open_tabs.clone()),
        HStack::for_each(tabs.clone(), move |tab| {
            let tab_id = tab.id;
            text(tab.title)
                .padding()
                .background(Srgb::from_hex("#607D8B").with_opacity(0.3))
                .gesture(
                    TapGesture::new().buttons(PointerButtons::MIDDLE),
                    move |State(tabs): State<ReactiveList<TabChip>>,
                          State(open_tabs): State<Binding<i32>>| {
                        if let Some(index) = tabs.snapshot().iter().position(|tab| tab.id == tab_id)
                        {
                            let _ = tabs.remove(index);
                            *open_tabs.get_mut() -= 1;
                        }
                    },
                )
                .state(&tabs_for_rows)
                .state(&open_tabs_for_handler)
        })
        .spacing(8.0),
    ))
    .padding()
}

/// Section demonstrating on_tap shorthand
fn on_tap_section(tap_count: &Binding<i32>) -> impl View {
    vstack((
        text("on_tap Shorthand").headline(),
        "Convenient method for simple tap handlers",
        "This uses the same counter as Section 1",
        text("Simple Tap")
            .padding()
            .background(ON_TAP_COLOR.with_opacity(0.3))
            .on_tap(|State(c): State<Binding<i32>>| *c.get_mut() += 1)
            .state(tap_count),
    ))
    .padding()
}

#[preview]
pub fn demo() -> impl View {
    let tap_count = Binding::i32(0);
    let double_tap_count = Binding::i32(0);
    let long_press_count = Binding::i32(0);
    let drag_count = Binding::i32(0);
    let chained_status = Binding::container("Waiting for tap...");
    let open_tabs = Binding::i32(4);
    let tabs = ReactiveList::from(
        ["Overview", "Details", "Activity", "Settings"]
            .iter()
            .enumerate()
            .map(|(id, title)| TabChip {
                id: id as u32,
                title,
            })
            .collect::<Vec<_>>(),
    );

    scroll(
        vstack((
            // Header
            text("WaterUI Gesture Examples").title(),
            "Demonstrating gesture recognition and handling",
            Divider,
            spacer(),
            // Gesture sections
            tap_section(&tap_count),
            Divider,
            double_tap_section(&double_tap_count),
            Divider,
            long_press_section(&long_press_count),
            Divider,
            drag_section(&drag_count),
            Divider,
            chained_section(&chained_status),
            Divider,
            // Grouped to stay under the tuple arity limit.
            vstack((
                middle_click_tab_section(&tabs, &open_tabs),
                on_tap_section(&tap_count),
            )),
        ))
        .padding_with(16.0),
    )
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
