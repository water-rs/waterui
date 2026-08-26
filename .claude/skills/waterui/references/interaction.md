# Interaction: taps, gestures, hover, cursor, drag & drop

## Contents

- Handlers, everywhere
- Tap shortcuts
- Gesture recognizers
- Combining gestures
- Hover
- Pointer cursor
- Drag and drop
- Reactive pressed/hover visuals

The compiled examples for this file are `examples/gesture`, `examples/hover`, and
`examples/drag_drop` in the WaterUI repository.

## Handlers, everywhere

Every interaction callback in WaterUI is a *handler* — the same extractor machinery as
`Button::action` (SKILL.md rule 3). Parameters are extractors (`State<T>`, `Environment`,
custom `impl_extractor!` types), state is injected with `.state(&binding)`, and repeated
`State<T>` of the same type bind positionally to the `.state()` call order. `.state()`
wraps the view it is applied to, so it may come *after* the handler-bearing modifier and
the handler still sees it.

The gesture/drag/hover *types* are not in the prelude — the modules are re-exported at the
crate root, so import the types explicitly:

```rust
use waterui::cursor::CursorStyle;
use waterui::drag_drop::DragData;
use waterui::gesture::{DragGesture, LongPressGesture, TapGesture};
```

## Tap shortcuts

`.on_tap(handler)` makes any view tappable. It takes a handler, not a plain closure — so
reach state through `State<T>` + `.state()`, exactly as with a button:

```rust
text("Simple Tap")
    .padding()
    .on_tap(|State(count): State<Binding<i32>>| *count.get_mut() += 1)
    .state(&taps)
```

Siblings: `.on_tap_gesture_count(2, handler)` for a fixed tap count, and — behind the
`std` cargo feature, which is not among the defaults — `.on_tap_haptic(intensity,
handler)` / `.on_tap_haptic_default(handler)` to pair the tap with haptic feedback. Use
`.on_tap` for tappable *content*; use `button(..)` when the
thing is semantically a button — the button brings platform chrome and the `BUTTON`
accessibility role.

## Gesture recognizers

`.gesture(gesture, handler)` attaches a recognizer to any view. The first argument is
anything `Into<Gesture>`:

```rust
view.gesture(TapGesture::new(), handler)              // single tap
view.gesture(TapGesture::repeat(2), handler)          // double tap — a count, not a new type
view.gesture(LongPressGesture::new(500), handler)     // duration is a u32, NOT a core::time::Duration
view.gesture(DragGesture::new(5.0), handler)          // minimum pointer travel, f32 layout units
```

Two argument types are traps:

- `LongPressGesture::new` takes a bare `u32` in backend-interpreted time units (typically
  milliseconds). `Duration::from_millis(500)` does not compile there.
- `DragGesture::new` takes an `f32` distance threshold. This recognizer only *detects* a
  drag — it moves no data. Moving data between views is the separate drag-and-drop system
  below; do not conflate them.

`MagnificationGesture::new(initial_scale)` and `RotationGesture::new(initial_angle)`
complete the set. All gesture structs are `#[non_exhaustive]` — construct them only
through these constructors.

## Combining gestures

`.then(..)` sequences gestures; the handler fires only after the whole sequence succeeds:

```rust
view.gesture(
    TapGesture::new().then(LongPressGesture::new(300)),   // tap, then long-press
    |State(status): State<Binding<&'static str>>| status.set("Done!"),
)
.state(&status)
```

`.then` returns the erased `Gesture` type. Siblings with the same shape:
`sequenced_before` (alias of `then`), `simultaneously_with`, `exclusively_before`.

Note in passing: `Binding::container("Waiting…")` infers `Binding<&'static str>`, which is
a perfectly good status-string binding — `text!("{status}")` accepts it directly.

## Hover

`.on_hover_enter(handler)` / `.on_hover_exit(handler)` fire where a pointer exists —
macOS, iPadOS with a trackpad, Android with a pointer. On a phone they simply never fire,
so hover may *enhance* an interaction but must never be the only way to reach it.

```rust
card()
    .on_hover_enter(|State(hovered): State<Binding<bool>>| hovered.set(true))
    .on_hover_exit(|State(hovered): State<Binding<bool>>| hovered.set(false))
    .state(&is_hovered)
```

## Pointer cursor

`.cursor(style)` takes `impl IntoComputed<CursorStyle>` — a plain style or a signal of
one. The style applies within the view's bounds and reverts automatically on exit.

```rust
link_row().cursor(CursorStyle::PointingHand)

// Reactive: derive the style from state.
view.cursor(
    dragging
        .map(|d| if d { CursorStyle::ClosedHand } else { CursorStyle::OpenHand })
        .computed(),
)
```

Variants: `Arrow` (default), `PointingHand`, `IBeam`, `Crosshair`, `OpenHand`,
`ClosedHand`, `NotAllowed`, `ResizeLeft`/`Right`/`Up`/`Down`, `ResizeLeftRight`,
`ResizeUpDown`, `Move`, `Wait`, `Copy`. Buttons styled `ButtonStyle::Link` show the
pointing hand by default. Like hover, cursors exist only on pointer platforms.

## Drag and drop

Three modifiers, one payload type. `DragData::text(..)` / `DragData::url(..)` accept
`impl Into<Str>`, so runtime `String`s are fine:

```rust
use waterui::drag_drop::DragData;

fn fruit_card(name: &'static str) -> impl View {
    text(name).padding().draggable(DragData::text(name))
}

// `+ use<>` keeps the borrowed parameters out of the returned view's lifetime (they are
// only read during construction) — without it the caller cannot treat the view as 'static.
fn basket(collected: &Binding<Vec<String>>, hovering: &Binding<bool>) -> impl View + use<> {
    vstack((text("Basket"), text!("{count} items", count = collected.map(|v| v.len()))))
        .padding()
        .drop_destination(
            |State(collected): State<Binding<Vec<String>>>, data: DragData| {
                collected.with_mut(|v| v.push(data.as_str().to_string()));
            },
        )
        .drop_hover(hovering)
        .state(collected)
}
```

The parts an agent cannot guess:

- **The dropped payload arrives as a handler parameter.** `DragData` is an extractor, so
  `data: DragData` sits alongside `State<T>` parameters in any order. There is no
  `|data| ...` callback form.
- **`.drop_hover(&binding)` must chain directly on `.drop_destination(..)`.** It exists
  only on the value that call returns; inserting another modifier between them is a
  compile error. It sets the binding `true` on drag-enter, `false` on exit — feed it to
  a background or scale signal for a highlight. `.on_enter(f)` / `.on_exit(f)` chain in
  the same position and *add* handlers rather than replacing them.
- `.draggable(..)` takes `impl IntoComputed<DragData>`, so the payload may itself be
  reactive.
- The initiating gesture is platform-defined: click-drag on macOS, long-press-drag on
  iOS and Android. Do not add your own long-press recognizer on top.

## Reactive pressed/hover visuals

Drive visuals from the interaction state — never rebuild the view to restyle it. Stack
both looks and cross-fade with complementary opacity signals, with the animation riding
on the signal:

```rust
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
```

Note the two opacities: `Color::with_opacity(0.2)` bakes alpha into the color value,
while `.opacity(signal)` is the reactive view modifier doing the cross-fade. Signal
transforms (`.select`, `.map`, `.zip`) take `&self`, so no `.clone()` is needed before
them — clone only when a finished signal is consumed twice, as `.scale(x, y)` does.
