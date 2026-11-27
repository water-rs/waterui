# Interaction: taps, gestures, hover, cursor, drag & drop

## Contents

- Handlers, everywhere
- Tap shortcuts
- Gesture recognizers
- Combining gestures
- Which view receives a press
- Hover
- Pointer cursor
- Drag and drop
- Key handling
- Reactive pressed/hover visuals
- Reporting a control's state
- Selected controls

The compiled examples for this file are `examples/gesture`, `examples/hover`, and
`examples/drag_drop` in the WaterUI repository.

## Handlers, everywhere

Every interaction callback in WaterUI is a *handler* — the same extractor machinery as
`Button::action` (SKILL.md rule 3). Parameters are extractors (`State<T>` for foreign
values, a bare `#[state]` type for owned ones, `Environment`, or framework
extractors), state is injected with `.state(&binding)`, and repeated
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

Sibling: `.on_long_press(handler)` (using the platform's
long-press timeout). Behind
the opt-in `haptics` cargo feature, attach `.sensory_feedback(Feedback::Success, &trigger)`
to play feedback whenever a `PartialEq + Clone` signal changes, or call
`waterui::haptic::Haptic::impact(Intensity::LOW)` inside a gesture handler for an
imperative trigger (`Haptic`, `Feedback`, and `Intensity` are also re-exported at the facade
root and in `prelude`). Use `.on_tap` for tappable *content*; use `button(..)` when the thing
is semantically a button — the button brings platform chrome and the `BUTTON` accessibility
role.

## Gesture recognizers

`.gesture(gesture, handler)` attaches a recognizer to any view. The first argument is
anything `Into<Gesture>`:

```rust
use core::{num::NonZeroU32, time::Duration};
use waterui::gesture::{DragEvent, LongPressEvent, TapEvent};

view.gesture(TapGesture::new(), |_: TapEvent| {})                                          // single tap
view.gesture(TapGesture::repeat(NonZeroU32::new(2).unwrap()), |_: TapEvent| {})             // double tap — a count, not a new type
view.gesture(LongPressGesture::new().minimum_duration(Duration::from_millis(500)), |_: LongPressEvent| {})
view.gesture(DragGesture::new(5.0), |_: DragEvent| {})                                      // minimum pointer travel, f32 layout units
```

Two details to keep straight:

- `LongPressGesture::new()` uses the platform's default long-press timeout; call
  `.minimum_duration(Duration::from_millis(500))` to override it. `LongPressEvent::duration()`
  reports the `Duration` actually held.
- `DragGesture::new` takes an `f32` distance threshold. This recognizer only *detects* a
  drag — it moves no data. Moving data between views is the separate drag-and-drop system
  below; do not conflate them.

`MagnificationGesture::new(initial_scale)` and `RotationGesture::new(initial_angle)`
complete the set. All gesture structs are `#[non_exhaustive]` — construct them only
through these constructors.

A recognized gesture's payload is passed as the handler's typed first argument — `TapEvent`,
`LongPressEvent`, `DragEvent` (`location()`, `translation()`, `velocity()`,
`phase()`), `MagnificationEvent` or `RotationEvent`, all from
`waterui::gesture` (with `GestureEvent` for composite `Gesture` combinators). Pointer motion inside a view is `.on_hover_move(handler)` (or `.event(HoverMove, handler)`),
whose handler takes `HoverEvent` (`location()`) (`waterui::event`);
`.on_hover_enter` / `.on_hover_exit` carry no `HoverEvent`.

`.buttons(..)` restricts tap, long-press, and drag recognizers to a `PointerButtons`
mask (default `PRIMARY`). A recognizer activates only when the pressing
`PointerButton` is in the mask; a pointer sequence belongs to one button, so a
second button pressed mid-sequence does not join it. `TapEvent`, `LongPressEvent`,
and `DragEvent` carry the pressing `button`.

```rust
use waterui::gesture::{PointerButtons, TapEvent};

view.gesture(TapGesture::new().buttons(PointerButtons::MIDDLE), |_: TapEvent| {}) // middle click
```

## Combining gestures

`.then(..)` sequences gestures; the handler fires only after the whole sequence succeeds,
receiving [`GestureEvent`]:

```rust
use core::time::Duration;
use waterui::gesture::GestureEvent;

view.gesture(
    TapGesture::new().then(LongPressGesture::new().minimum_duration(Duration::from_millis(300))), // tap, then long-press
    |_: GestureEvent, State(status): State<Binding<&'static str>>| status.set("Done!"),
)
.state(&status)
```

`.then` returns the erased `Gesture` type. Siblings with the same shape:
`simultaneously_with`, `exclusively_before`.

Note in passing: `Binding::new("Waiting…")` infers `Binding<&'static str>`, which is
a perfectly good status-string binding — `text!("{status}")` accepts it directly.

## Which view receives a press

Hit testing is pass-through: only views with a press handler (`.on_tap`, `.gesture`,
`.draggable`) or inherent interactivity (a button, slider, field) capture a press;
layout containers, `Spacer` and plain content do not. The topmost interactive view
under the pointer wins, so an `.on_tap` on a child shadows an `.on_tap` on its ancestor
and a row-level handler never fires where a tappable child covers the pointer.
Recognizers attached to the *same* view combine as above. `.hittable(false)` (or a
signal of `bool`) makes a view and everything inside it transparent to presses without
changing how it draws; `.disabled(..)` does the same and also gives every control in the
subtree its disabled look and accessibility state.

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

#[derive(Debug, Clone, PartialEq, Transferable)]
struct Fruit(&'static str);

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
  highlighted nor delivered. Extractors such as `DropInfo` (carrying `.location()`
  in the destination's local coordinate space) and `State<T>` follow it.
- **`.drop_hover(&binding)` must chain directly on `.drop_destination(..)`.** It exists
  only on the value that call returns; inserting another modifier between them is a
  compile error. It sets the binding `true` on drag-enter, `false` on exit — feed it to
  a background or scale signal for a highlight. `.on_enter(handler)` / `.on_exit(handler)`
  chain in the same position, accept extractor parameters (`DropInfo` on enter, `State<T>`
  on both), and *add* handlers rather than replacing them.
- `.draggable(..)` takes a plain transferable value or a reactive signal of one
  (`Binding<T>`, `Computed<T>`, or mapped signal); the payload type is inferred
  from it. Mark your own type with `#[derive(Clone, Transferable)]` (as above).
  Text is `.draggable(Str::from(..))`.
- The initiating gesture is platform-defined: click-drag on macOS, long-press-drag on
  iOS and Android. Do not add your own long-press recognizer on top.

## Key handling

`.on_key_press(handler)` attaches a key handler to a view. The focused view sees each
key first; a key it does not consume bubbles to the nearest ancestor with an
`OnKeyPress` handler, then the next, stopping at the first `KeyHandling::Handled`. The
handler receives `press: KeyPress` as its first argument and returns `Handled` or `Ignored`. The backend is what decides "consumed": a single-line
`field` eats text-editing keys and submits Return through `.on_submit(..)` when one is
set; Escape, Up/Down, PageUp/PageDown and a Return with no `on_submit` bubble out.

```rust
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
```

- Return `Ignored`, never a missing `Handled` — `Ignored` is what keeps the key bubbling.
- `KeyPress` exposes accessors: `press.key()` is the layout-aware `&Key`,
  `press.code()` the physical `Code` (layout-independent bindings match this;
  `Code` is in `waterui::key`), `press.modifiers()` a `Modifiers` bitset
  (`Modifiers::SHIFT`, `CONTROL`, `ALT`, `META`), `press.repeat()` marks
  auto-repeat. `Key`, `KeyHandling`, `KeyPress`, `Modifiers`, and `NamedKey`
  are in the prelude (`waterui::key` also exports them alongside `Code`).
- Match portable command shortcuts with `Shortcut::new('k').command().matches(&press)`
  (`Shortcut` maps `.command()` to `META` on Apple and `CONTROL` elsewhere, and ignores
  lock modifiers) rather than branching on `cfg!(target_vendor = "apple")`. For button
  actions, prefer `button(..).keyboard_shortcut(Shortcut::new(..).command())` so the
  backend arms the shortcut natively without needing focus.
- `field("Search", &query).on_submit(handler)` fires on Return in a line-limited
  field; with no line limit Return inserts a newline and never submits.

## Reactive pressed/hover visuals

Drive visuals from the interaction state — never rebuild the view to restyle it. Stack
both looks and cross-fade with complementary opacity signals, with the animation riding
on the signal:

```rust
let scale = is_hovered
    .select(1.05, 1.0)
    .with(Animation::spring(400.0, 15.0));

zstack((
    Blue.with_opacity(0.2).opacity(is_hovered.select(0.0, 1.0)),
    Blue.with_opacity(0.45).opacity(is_hovered.select(1.0, 0.0)),
    text("Hover me").padding(),
))
.scale(scale)
```

Note the two opacities: `Color::with_opacity(0.2)` bakes alpha into the color value,
while `.opacity(signal)` is the reactive view modifier doing the cross-fade. Signal
transforms (`.select`, `.map`, `.zip`) take `&self`, so no `.clone()` is needed before
them — clone only when a finished signal is consumed twice, as `.scale_xy(x, y)` does.

## Reporting a control's state

`.on_hover_enter`/`.on_hover_exit` report the pointer entering and leaving *that*
modifier's view. When chrome lives around a control it does not own — a floating surface's
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
