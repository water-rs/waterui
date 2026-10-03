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
- Reporting a control's state
- Selected controls

The compiled examples for this file are `examples/gesture`, `examples/hover`, and
`examples/drag_drop` in the WaterUI repository.

## Handlers, everywhere

Every interaction callback in WaterUI is a *handler* — the same extractor machinery as
`Button::action` (SKILL.md rule 3). Parameters are extractors (`State<T>` for foreign
values, a bare `#[state]` type for owned ones, `Environment`, `impl_extractor!` types
installed as environment values), state is injected with `.state(&binding)`, and repeated
parameters of the same state type bind positionally to the `.state()` call order.
`.state()` wraps the view it is applied to, so it may come *after* the handler-bearing
modifier and the handler still sees it.

The gesture/drag/hover *types* are not in the prelude — the modules are re-exported at the
crate root, so import the types explicitly:

```rust
use waterui::cursor::CursorStyle;
use waterui::drag_drop::{Files, Transferable};
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

`.buttons(..)` restricts tap, long-press, and drag recognizers to a `PointerButtons`
mask (default `PRIMARY`). A recognizer activates only when the pressing
`PointerButton` is in the mask; a pointer sequence belongs to one button, so a
second button pressed mid-sequence does not join it. `TapEvent`, `LongPressEvent`,
and `DragEvent` carry the pressing `button`.

```rust
use waterui::gesture::PointerButtons;

view.gesture(TapGesture::new().buttons(PointerButtons::MIDDLE), handler) // middle click
```

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
        .select(CursorStyle::ClosedHand, CursorStyle::OpenHand)
        .computed(),
)
```

Variants: `Arrow` (default), `PointingHand`, `IBeam`, `Crosshair`, `OpenHand`,
`ClosedHand`, `NotAllowed`, `ResizeLeft`/`Right`/`Up`/`Down`, `ResizeLeftRight`,
`ResizeUpDown`, `Move`, `Wait`, `Copy`. Buttons styled `ButtonStyle::Link` show the
pointing hand by default. Like hover, cursors exist only on pointer platforms.

## Drag and drop

Three modifiers; a drag carries one typed value. `.draggable(value)` takes any
`Transferable` type, and a destination accepts exactly the type of its handler's first
argument. `Str`, `Url` and `Files` also cross to other applications; an app type marked
`Transferable` stays in the process:

```rust
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
    vstack((text("Basket"), text!("{count} items", count = collected.map(|v| v.len()))))
        .padding()
        .drop_destination(
            |fruit: Fruit, State(collected): State<Binding<Vec<String>>>| {
                collected.with_mut(|v| v.push(fruit.0.to_string()));
            },
        )
        .drop_hover(hovering)
        .state(collected)
}
```

The parts an agent cannot guess:

- **The dropped value is the handler's first parameter, and its type is the filter.**
  `|text: Str| ..` accepts text drags only, `|files: Files| ..` file drags only,
  `|fruit: Fruit| ..` in-process `Fruit` drags only; any other drag is neither
  highlighted nor delivered. Extractors such as `State<T>` follow it.
- **`.drop_hover(&binding)` must chain directly on `.drop_destination(..)`.** It exists
  only on the value that call returns; inserting another modifier between them is a
  compile error. It sets the binding `true` on drag-enter, `false` on exit — feed it to
  a background or scale signal for a highlight. `.on_enter(f)` / `.on_exit(f)` chain in
  the same position and *add* handlers rather than replacing them.
- `.draggable(..)` takes a plain transferable value or any signal of one (`Binding<T>`,
  `Computed<T>`); the payload type is inferred from it. A plain value of your own type
  needs `impl_constant!` (as above). Text is `.draggable(Str::from(..))`.
- The initiating gesture is platform-defined: click-drag on macOS, long-press-drag on
  iOS and Android. Do not add your own long-press recognizer on top.

## Key handling

`.on_key_press(handler)` attaches a key handler to a view. The focused view sees each
key first; a key it does not consume bubbles to the nearest ancestor with an
`OnKeyPress` handler, then the next, stopping at the first `KeyHandling::Handled`. The
handler runs with `KeyPress` in its environment — read it with `Use<KeyPress>` — and
returns `Handled` or `Ignored`. The backend is what decides "consumed": a single-line
`field` eats text-editing keys and submits Return through `.on_submit(..)` when one is
set; Escape, Up/Down, PageUp/PageDown and a Return with no `on_submit` bubble out.

```rust
use waterui::key::{Key, KeyHandling, KeyPress, NamedKey};

search_panel()
    .on_key_press(|Use(press): Use<KeyPress>, State(open): State<Binding<bool>>| {
        if press.key == Key::Named(NamedKey::Escape) {
            open.set(false);
            KeyHandling::Handled
        } else {
            KeyHandling::Ignored
        }
    })
    .state(&open)
```

- Return `Ignored`, never a missing `Handled` — `Ignored` is what keeps the key bubbling.
- `press.modifiers` is a `keyboard_types::Modifiers` bitset (`Modifiers::SHIFT`,
  `Modifiers::CONTROL`, `Modifiers::ALT`, `Modifiers::META`); `press.code` is the
  physical `Code`, `press.repeat` marks auto-repeat.
- `field("Search", &query).on_submit(handler)` fires on Return in a line-limited
  field; with no line limit Return inserts a newline and never submits.

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

## Reporting a control's state

`.on_hover_enter`/`.on_hover_exit` report the pointer position of *that* modifier's
view. When chrome lives around a control it does not own — a floating surface's
shadow, a chip's outline, a split button's half — ask the backend for the whole
interaction state instead: `.interaction_state(&binding)` writes the
`InteractionState` of the outermost interactive control at or inside the view it
is applied to, every time it changes. The binding sits at `InteractionState::empty()`
while no interactive control is there.

```rust
use waterui::interaction::InteractionState;
use waterui::reactive::{Binding, binding};

let state: Binding<InteractionState> = binding(InteractionState::empty());
let lift = state.map(|s| if s.contains(InteractionState::HOVERED) { -6.0 } else { 0.0 });

vstack((
    text!("Chip"),
    button("Action").action(|| {}),
))
.offset(0.0, lift)            // chrome follows the control's state
.interaction_state(&state)
```

`InteractionState` is a bitflags set — `HOVERED`, `FOCUSED` (focus-visible only,
like `:focus-visible`: keyboard focus, never a click), `PRESSED`, `DRAGGED`,
`SELECTED`, `DISABLED` — so test it with `.contains(...)`, never `==`.

## Selected controls

`.selected(..)` takes `impl IntoComputed<bool>` and marks the interactive control
it modifies as selected: the control gains `InteractionState::SELECTED` (so its
style's selected values apply) and assistive technology announces it as selected.
It applies to the control it modifies — unlike `.disabled(..)` it is *not*
inherited by nested controls.

```rust
let selection: Binding<i32> = binding(0);
text!("Inbox").selected(selection.map(|s| s == 0))
```

Navigation destinations, tabs, and list items are the usual carriers; combine
with `.interaction_state` when the row's own chrome (a selection indicator,
say) must also follow hover or press.
