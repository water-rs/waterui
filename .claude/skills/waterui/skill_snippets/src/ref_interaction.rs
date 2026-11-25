//! Snippets from `.claude/skills/waterui/references/interaction.md`, in file
//! order. Transcription conventions are documented in the crate README.

use core::num::NonZeroU32;

use waterui::prelude::*;

// ---------------------------------------------------------------------------
// interaction.md § "## Handlers, everywhere" — rust block 1/12
//
// At module scope: the section presents these as the imports the rest of the
// file relies on.
// ---------------------------------------------------------------------------
use waterui::cursor::CursorStyle;
use waterui::drag_drop::{Files, Transferable};
use waterui::gesture::{DragGesture, LongPressGesture, TapGesture};

// ---------------------------------------------------------------------------
// interaction.md § "## Tap shortcuts" — rust block 2/12
// ---------------------------------------------------------------------------
pub fn interaction_block_02() -> impl View {
    let taps = Binding::i32(0);

    text("Simple Tap")
        .padding()
        .on_tap(|State(count): State<Binding<i32>>| *count.get_mut() += 1)
        .state(&taps)
}

// ---------------------------------------------------------------------------
// interaction.md § "## Tap shortcuts" (prose):
// `.on_long_press(h)`,
// `.sensory_feedback(Feedback::Success, &trigger)`, and imperative
// `waterui::haptic::Haptic::impact(Intensity::LOW)`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_tap_siblings_prose() {
    use waterui::haptic::Haptic as HapticEngine;
    use waterui::task::spawn_local;

    let handler = || ();
    let saved_count = Binding::i32(0);
    let _ = Divider.on_long_press(handler);
    let _ = Divider.sensory_feedback(Feedback::Success, &saved_count);
    let _ = Divider.on_tap(|| {
        spawn_local(async {
            let _ = HapticEngine::impact(Intensity::LOW).await;
        })
        .detach();
    });
}

// ---------------------------------------------------------------------------
// interaction.md § "## Gesture recognizers" — rust block 3/12
// Listing: four independent recognizers.
// ---------------------------------------------------------------------------
pub fn interaction_block_03() {
    use core::time::Duration;
    use waterui::gesture::{DragEvent, LongPressEvent, TapEvent};

    let view = Divider;
    let _ = {
        view.gesture(TapGesture::new(), |_: TapEvent| {}) // single tap
    };
    let view = Divider;
    let _ = {
        view.gesture(
            TapGesture::repeat(NonZeroU32::new(2).unwrap()),
            |_: TapEvent| {},
        ) // double tap — a count, not a new type
    };
    let view = Divider;
    let _ = {
        view.gesture(
            LongPressGesture::new().minimum_duration(Duration::from_millis(500)),
            |_: LongPressEvent| {},
        )
    };
    let view = Divider;
    let _ = {
        view.gesture(DragGesture::new(5.0), |_: DragEvent| {}) // minimum pointer travel, f32 layout units
    };
}

// ---------------------------------------------------------------------------
// interaction.md § "## Gesture recognizers" (prose): `MagnificationGesture::new`
// and `RotationGesture::new` "complete the set". Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_remaining_gestures_prose() {
    use waterui::gesture::{MagnificationGesture, RotationGesture};

    let _ = MagnificationGesture::new(1.0);
    let _ = RotationGesture::new(0.0);
}

// ---------------------------------------------------------------------------
// interaction.md § "## Gesture recognizers" (prose): the gesture payloads read
// directly, and `HoverEvent` in an `.event(HoverMove, ..)`
// handler. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_event_payloads_prose() {
    use waterui::event::{HoverEvent, HoverMove};
    use waterui::gesture::{
        DragEvent, LongPressEvent, MagnificationEvent, MagnificationGesture, RotationEvent,
        RotationGesture, TapEvent,
    };

    let _ = Divider.gesture(TapGesture::new(), |tap: TapEvent| {
        let _ = tap.location();
    });
    let _ = Divider.gesture(LongPressGesture::new(), |_: LongPressEvent| ());
    let _ = Divider.gesture(DragGesture::new(5.0), |drag: DragEvent| {
        let _ = (
            drag.location(),
            drag.translation(),
            drag.velocity(),
            drag.phase(),
        );
    });
    let _ = Divider.gesture(MagnificationGesture::new(1.0), |_: MagnificationEvent| ());
    let _ = Divider.gesture(RotationGesture::new(0.0), |_: RotationEvent| ());
    let _ = Divider.event(HoverMove, |hover: HoverEvent| {
        let _ = hover.location();
    });
}

// ---------------------------------------------------------------------------
// interaction.md § "## Gesture recognizers" — rust block 4/12
// ---------------------------------------------------------------------------
pub fn interaction_block_04() {
    let view = Divider;

    use waterui::gesture::{PointerButtons, TapEvent};

    let _ = view.gesture(
        TapGesture::new().buttons(PointerButtons::MIDDLE),
        |_: TapEvent| {},
    ); // middle click
}

// ---------------------------------------------------------------------------
// interaction.md § "## Combining gestures" — rust block 5/12
// ---------------------------------------------------------------------------
pub fn interaction_block_05() -> impl View {
    use core::time::Duration;
    use waterui::gesture::GestureEvent;

    let status = Binding::container("Waiting…");

    let view = Divider;
    view.gesture(
        TapGesture::new()
            .then(LongPressGesture::new().minimum_duration(Duration::from_millis(300))), // tap, then long-press
        |_: GestureEvent, State(status): State<Binding<&'static str>>| status.set("Done!"),
    )
    .state(&status)
}

// ---------------------------------------------------------------------------
// interaction.md § "## Combining gestures" (prose): `simultaneously_with`,
// `exclusively_before`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_combinators_prose() {
    use core::time::Duration;

    let hold = LongPressGesture::new().minimum_duration(Duration::from_millis(300));
    let _ = TapGesture::new().simultaneously_with(hold.clone());
    let _ = TapGesture::new().exclusively_before(hold);
}

// ---------------------------------------------------------------------------
// interaction.md § "## Which view receives a press" (prose): `.hittable(..)`
// with a plain bool or a signal, and `.disabled(..)`. Not counted as a rust
// block.
// ---------------------------------------------------------------------------
pub fn interaction_hittable_prose() {
    let can_interact = Binding::bool(true);
    let _ = Divider.hittable(false);
    let _ = Divider.hittable(can_interact);
    let _ = Divider.disabled(false);
}

// ---------------------------------------------------------------------------
// interaction.md § "## Hover" — rust block 6/12
// ---------------------------------------------------------------------------
pub fn interaction_block_06() -> impl View {
    fn card() -> impl View {
        text("card")
    }
    let is_hovered = Binding::bool(false);

    card()
        .on_hover_enter(|State(hovered): State<Binding<bool>>| hovered.set(true))
        .on_hover_exit(|State(hovered): State<Binding<bool>>| hovered.set(false))
        .state(&is_hovered)
}

// ---------------------------------------------------------------------------
// interaction.md § "## Pointer cursor" — rust block 7/12
// Listing: a plain style, then a derived one.
// ---------------------------------------------------------------------------
pub fn interaction_block_07() {
    fn link_row() -> impl View {
        text("link")
    }
    let dragging = Binding::bool(false);

    let _ = { link_row().cursor(CursorStyle::PointingHand) };

    // Reactive: derive the style from state.
    let view = Divider;
    let _ = {
        view.cursor(
            dragging
                .select(CursorStyle::ClosedHand, CursorStyle::OpenHand)
                .computed(),
        )
    };
}

// ---------------------------------------------------------------------------
// interaction.md § "## Pointer cursor" (prose): the full variant list.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_cursor_variants_prose() {
    let _ = CursorStyle::Arrow;
    let _ = CursorStyle::PointingHand;
    let _ = CursorStyle::IBeam;
    let _ = CursorStyle::Crosshair;
    let _ = CursorStyle::OpenHand;
    let _ = CursorStyle::ClosedHand;
    let _ = CursorStyle::NotAllowed;
    let _ = CursorStyle::ResizeLeft;
    let _ = CursorStyle::ResizeRight;
    let _ = CursorStyle::ResizeUp;
    let _ = CursorStyle::ResizeDown;
    let _ = CursorStyle::ResizeLeftRight;
    let _ = CursorStyle::ResizeUpDown;
    let _ = CursorStyle::Move;
    let _ = CursorStyle::Wait;
    let _ = CursorStyle::Copy;
}

// ---------------------------------------------------------------------------
// interaction.md § "## Drag and drop" — rust block 8/12
// ---------------------------------------------------------------------------
pub mod interaction_block_08 {
    use waterui::prelude::*;

    use waterui::drag_drop::Transferable;
    use waterui::reactive::impl_constant;

    #[derive(Debug, Clone, PartialEq)]
    struct Fruit(&'static str);
    impl Transferable for Fruit {}
    impl_constant!(Fruit); // lets a plain `Fruit` value be passed to `.draggable(..)`

    fn fruit_card(name: &'static str) -> impl View {
        text(name).padding().draggable(Fruit(name))
    }

    // `+ use<>` keeps the borrowed parameters out of the returned view's lifetime (they are
    // only read during construction) — without it the caller cannot treat the view as 'static.
    fn basket(collected: &Binding<Vec<String>>, hovering: &Binding<bool>) -> impl View + use<> {
        vstack((
            text("Basket"),
            text!("{count} items", count = collected.map(|v| v.len())),
        ))
        .padding()
        .drop_destination(
            |fruit: Fruit, State(collected): State<Binding<Vec<String>>>| {
                collected.with_mut(|v| v.push(fruit.0.to_string()));
            },
        )
        .drop_hover(hovering)
        .state(collected)
    }

    /// `+ use<>` keeps the borrowed parameters out of the returned view's
    /// lifetime, so the view escapes the caller exactly as a real screen needs.
    pub fn use_it() -> impl View {
        let collected = Binding::container(Vec::<String>::new());
        let hovering = Binding::bool(false);
        vstack((fruit_card("Apple"), basket(&collected, &hovering)))
    }
}

// ---------------------------------------------------------------------------
// interaction.md § "## Drag and drop" (prose): typed destinations for
// `Str` and `Files`, `.draggable(Str::from(..))`, and `.on_enter(f)` /
// `.on_exit(f)` chaining in the same position as `.drop_hover(..)`. Not
// counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_drop_extras_prose() {
    let _ = text("copy me").draggable(Str::from("payload"));
    let _ = Divider.drop_destination(|_text: Str| ());
    let _ = Divider
        .drop_destination(|_files: Files| ())
        .on_enter(|| ())
        .on_exit(|| ());
}

/// Glue: keeps the module-scope `Transferable` import proven to resolve.
#[derive(Debug, Clone)]
pub struct GlueTransferable;
impl Transferable for GlueTransferable {}

// ---------------------------------------------------------------------------
// interaction.md § "## Key handling" — rust block 9/12
// ---------------------------------------------------------------------------
pub fn interaction_block_09() -> impl View {
    use waterui::key::{Key, KeyHandling, KeyPress, NamedKey};

    let open = Binding::bool(false);
    let search_panel = || text("Search panel");

    search_panel()
        .on_key_press(|press: KeyPress, State(open): State<Binding<bool>>| {
            if *press.key() == Key::Named(NamedKey::Escape) {
                open.set(false);
                KeyHandling::Handled
            } else {
                KeyHandling::Ignored
            }
        })
        .state(&open)
}

// ---------------------------------------------------------------------------
// interaction.md § "## Key handling" (prose): the `KeyPress` accessors and the
// `waterui::key` re-exports. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_key_accessors_prose() {
    use waterui::key::{Code, Key, KeyPress, Modifiers, NamedKey};

    let press = KeyPress::new(
        Key::Named(NamedKey::Escape),
        Code::Escape,
        Modifiers::SHIFT | Modifiers::CONTROL | Modifiers::ALT | Modifiers::META,
        false,
    );
    let _ = (press.key(), press.code(), press.modifiers(), press.repeat());
}

// ---------------------------------------------------------------------------
// interaction.md § "## Reactive pressed/hover visuals" — rust block 10/12
// ---------------------------------------------------------------------------
// The two layers cross-fade through complementary opacity signals — reaching
// 0.0 here is the documented technique, not a visibility toggle.
#[allow(unknown_lints, opacity_as_visibility)]
pub fn interaction_block_10() -> impl View {
    let is_hovered = Binding::bool(false);

    use waterui::animation::Animation;

    let scale = is_hovered
        .select(1.05, 1.0)
        .with(Animation::spring(400.0, 15.0));

    zstack((
        Blue.with_opacity(0.2).opacity(is_hovered.select(0.0, 1.0)),
        Blue.with_opacity(0.45).opacity(is_hovered.select(1.0, 0.0)),
        text("Hover me").padding(),
    ))
    .scale(scale)
}

// ---------------------------------------------------------------------------
// interaction.md § "## Reporting a control's state" — rust block 11/12
// ---------------------------------------------------------------------------
pub fn interaction_block_11() -> impl View {
    use waterui::interaction::InteractionState;
    use waterui::reactive::{Binding, binding};

    let state: Binding<InteractionState> = binding(InteractionState::empty());
    let lift = state.map(|s| {
        if s.contains(InteractionState::HOVERED) {
            -6.0
        } else {
            0.0
        }
    });

    AnyView::new(
        vstack((text!("Chip"), button("Action").action(|| {})))
            .offset(0.0, lift)
            .interaction_state(&state),
    )
}

// ---------------------------------------------------------------------------
// interaction.md § "## Selected controls" — rust block 12/12
// ---------------------------------------------------------------------------
pub fn interaction_block_12() -> impl View {
    use waterui::reactive::{Binding, binding};

    let selection: Binding<i32> = binding(0);
    text!("Inbox").selected(selection.map(|s| s == 0))
}
