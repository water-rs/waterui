//! Snippets from `.claude/skills/waterui/references/interaction.md`, in file
//! order. Transcription conventions are documented in the crate README.

use waterui::prelude::*;

// ---------------------------------------------------------------------------
// interaction.md § "## Handlers, everywhere" — rust block 1/8
//
// At module scope: the section presents these as the imports the rest of the
// file relies on.
// ---------------------------------------------------------------------------
use waterui::cursor::CursorStyle;
use waterui::drag_drop::DragData;
use waterui::gesture::{DragGesture, LongPressGesture, TapGesture};

// ---------------------------------------------------------------------------
// interaction.md § "## Tap shortcuts" — rust block 2/8
// ---------------------------------------------------------------------------
pub fn interaction_block_02() -> impl View {
    let taps = Binding::i32(0);

    text("Simple Tap")
        .padding()
        .on_tap(|State(count): State<Binding<i32>>| *count.get_mut() += 1)
        .state(&taps)
}

// ---------------------------------------------------------------------------
// interaction.md § "## Tap shortcuts" (prose): `.on_tap_gesture_count(2, h)`
// plus the haptic siblings. The prose names the gate on the haptic pair: they
// and the `Intensity` they take live behind the `std` cargo feature, "which is
// not among the defaults", so they are deliberately not compiled here.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_tap_siblings_prose() {
    let handler = || ();
    let _ = Divider.on_tap_gesture_count(2, handler);

    // Behind `feature = "std"`, which this crate does not enable:
    //   let _ = Divider.on_tap_haptic(intensity, handler);
    //   let _ = Divider.on_tap_haptic_default(handler);
}

// ---------------------------------------------------------------------------
// interaction.md § "## Gesture recognizers" — rust block 3/8
// Listing: four independent recognizers.
// ---------------------------------------------------------------------------
pub fn interaction_block_03() {
    let handler = || ();

    let view = Divider;
    let _ = {
        view.gesture(TapGesture::new(), handler) // single tap
    };
    let view = Divider;
    let _ = {
        view.gesture(TapGesture::repeat(2), handler) // double tap — a count, not a new type
    };
    let view = Divider;
    let _ = {
        // duration is a u32, NOT a core::time::Duration
        view.gesture(LongPressGesture::new(500), handler)
    };
    let view = Divider;
    let _ = {
        view.gesture(DragGesture::new(5.0), handler) // minimum pointer travel, f32 layout units
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
// interaction.md § "## Combining gestures" — rust block 4/8
// ---------------------------------------------------------------------------
pub fn interaction_block_04() -> impl View {
    let status = Binding::container("Waiting…");

    let view = Divider;
    view.gesture(
        TapGesture::new().then(LongPressGesture::new(300)), // tap, then long-press
        |State(status): State<Binding<&'static str>>| status.set("Done!"),
    )
    .state(&status)
}

// ---------------------------------------------------------------------------
// interaction.md § "## Combining gestures" (prose): `sequenced_before` (alias of
// `then`), `simultaneously_with`, `exclusively_before`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_combinators_prose() {
    let _ = TapGesture::new().sequenced_before(LongPressGesture::new(300));
    let _ = TapGesture::new().simultaneously_with(LongPressGesture::new(300));
    let _ = TapGesture::new().exclusively_before(LongPressGesture::new(300));
}

// ---------------------------------------------------------------------------
// interaction.md § "## Hover" — rust block 5/8
// ---------------------------------------------------------------------------
pub fn interaction_block_05() -> impl View {
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
// interaction.md § "## Pointer cursor" — rust block 6/8
// Listing: a plain style, then a derived one.
// ---------------------------------------------------------------------------
pub fn interaction_block_06() {
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
                .map(|d| {
                    if d {
                        CursorStyle::ClosedHand
                    } else {
                        CursorStyle::OpenHand
                    }
                })
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
// interaction.md § "## Drag and drop" — rust block 7/8
// ---------------------------------------------------------------------------
pub mod interaction_block_07 {
    use waterui::prelude::*;

    use waterui::drag_drop::DragData;

    fn fruit_card(name: &'static str) -> impl View {
        text(name).padding().draggable(DragData::text(name))
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
            |State(collected): State<Binding<Vec<String>>>, data: DragData| {
                collected.with_mut(|v| v.push(data.as_str().to_string()));
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
// interaction.md § "## Drag and drop" (prose): `DragData::url(..)`, and
// `.on_enter(f)` / `.on_exit(f)` chaining in the same position as
// `.drop_hover(..)`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn interaction_drop_extras_prose() {
    let _ = DragData::url("https://waterui.dev");
    let _ = Divider
        .drop_destination(|_data: DragData| ())
        .on_enter(|| ())
        .on_exit(|| ());
}

// ---------------------------------------------------------------------------
// interaction.md § "## Reactive pressed/hover visuals" — rust block 8/8
// ---------------------------------------------------------------------------
pub fn interaction_block_08() -> impl View {
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
    .scale(scale.clone(), scale)
}
