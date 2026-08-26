---
name: waterui
description: Build cross-platform native apps with the WaterUI Rust framework. Use this skill whenever writing, reviewing, or debugging WaterUI code — views, reactive state (Binding/Computed/signals), layout, styling, navigation, lists, forms, gestures, drag and drop, menus, windows, media, charts, maps, localization/i18n/RTL, animation, accessibility, tests — or when running the `water` CLI (`water create`, `water run`, `water preview`, `water build`, `water bench`). Trigger on any Rust file that imports `waterui`, any project containing a `Water.toml`, and any mention of WaterUI, `text!`, `vstack`/`hstack`, `Binding<T>`, `impl View`, or a `water` command, even when the framework is not named explicitly.
---

# Building apps with WaterUI

WaterUI is a Rust UI framework that renders to real native widgets (UIKit/AppKit,
Android View, GTK4) or to its own GPU renderer, from one view tree. It is
**fine-grained reactive**: a value change updates exactly the widget that reads it,
without rebuilding the surrounding tree.

Almost every mistake in WaterUI code comes from writing it as if it were React or
SwiftUI. The five rules below are what actually differ. Read them before writing code.

## Reference map

Read the file that matches the task. Each is self-contained; none reference each other
beyond pointers.

| Topic | File |
|---|---|
| Signals, `Binding`, `Computed`, collections, async tasks, animation, conditionals | [references/reactivity.md](references/reactivity.md) |
| Component catalog with real signatures: layout, controls, menus, text, lists, forms, overlays | [references/components.md](references/components.md) |
| Photos, video, media picking, web views + JS bridge, shaders, particles, charts, maps | [references/media.md](references/media.md) |
| Gestures, taps, hover, cursor, drag & drop | [references/interaction.md](references/interaction.md) |
| Tabs, navigation stacks, toolbars, transitions, split views, windows | [references/navigation.md](references/navigation.md) |
| Colors, theme tokens, dark mode, icons, shapes, gradients, Material 3 | [references/styling.md](references/styling.md) |
| Translations, plurals, locale switching, formatting, RTL | [references/i18n.md](references/i18n.md) |
| `#[waterui::test]`, `#[waterui::bench]`, `#[preview]`, snapshots | [references/testing.md](references/testing.md) |
| `water` CLI, `Water.toml`, Cargo features, assets, permissions, platforms, embedded | [references/project.md](references/project.md) |
| Compile errors, silent bugs, and their fixes | [references/troubleshooting.md](references/troubleshooting.md) |

When an API is still unclear, the compiled examples in `examples/*/src/lib.rs` of the
WaterUI repository are ground truth — they are built in CI, so they are never stale.
The prose companion to this skill is the book at <https://book.waterui.dev>, which goes
deeper on the topics here and covers ones this skill does not (plugins, resolvers and
hooks, shaders, error handling, library authoring). Each book release is pinned to an
exact WaterUI commit, shown in the book itself — check that pin against the version you
depend on before copying from it.

## The five rules

### 1. Pass the signal, never a snapshot of it

Reactive APIs take `impl IntoComputed<T>`, `impl IntoSignalF32`, or `&Binding<T>`.
Handing them `.get()` reads the value once and freezes it — the UI then never updates,
and nothing fails at compile time, so this bug is silent.

```rust
view.opacity(fade.clone())              // reacts
view.opacity(fade.get())                // frozen forever — a plain f32
Photo::new(url).blur(blur.clone())      // reacts
text!("Count: {count}")                 // reacts
```

`.get()` belongs inside event handlers and `.map()` closures, where you genuinely want
the value at that instant. It does not belong in a view body.

### 2. `watch` is not the reactive primitive — it is the escape hatch

`watch(signal, |v| ...)` tears down and rebuilds its entire subtree on every change, so
any state living inside that subtree is destroyed. Three things replace nearly every use:

```rust
text!("{status}")                               // reactive text — not watch + format!
Photo::new(url).blur(blur.clone())              // reactive value — pass the signal
Lazy::for_each(rows.clone(), row_view)          // dynamic set of views — a collection
```

Reach for `watch` only for a genuinely one-off structural swap where no signal-aware API
and no collection applies. Check those three first, every time.

### 3. Inject handler state with `.state()`, do not capture clones

`.action()` takes a *handler*: a function whose parameters are extractors resolved from
the environment. `.state(&value)` puts a value in that environment; `State<T>` pulls it
out. This keeps handlers as plain named functions instead of a thicket of `move` closures.
The same machinery drives every callback in the framework — `.on_tap`, gestures, drops,
menu commands, list edits — not just buttons.

```rust
button("Increment")
    .action(|State(count): State<Binding<i32>>| *count.get_mut() += 1)
    .state(&count)
```

Repeated `State<T>` of the **same type** bind positionally: the first `.state()` call
feeds the first `State<T>` parameter.

```rust
button("Search")
    .action(|State(q): State<Binding<Str>>, State(hist): State<Binding<Vec<Str>>>| {
        hist.get_mut().push(q.get());
    })
    .state(&query)      // -> first parameter
    .state(&history)    // -> second parameter
```

**Beyond two or three pieces of state, stop threading them individually.** Put them in
one `Clone` struct, inject it once on a container, and write handlers as free functions.
This is the idiomatic shape for a real screen:

```rust
#[derive(Clone)]
struct Editor {
    rows: ReactiveList<Row>,
    editing: Binding<bool>,
}

fn toggle_editing(State(state): State<Editor>) {
    state.editing.set(!state.editing.get());
}

fn content(state: Editor) -> impl View {
    vstack((
        button("Edit").action(toggle_editing),
        List::for_each(state.rows.clone(), row_view),
    ))
    .state(&state)          // injected once, visible to every handler below
}
```

Async is the same shape: `.action_async(|State(x): State<Binding<Str>>| async move { … })`.

### 4. A changing set of views is a collection, not a `watch`

`ForEach`/`List` diff by `Identifiable` id, so inserting one row touches one row.
`watch` over a `Vec` rebuilds everything and can escalate to a full-window rebuild.

```rust
use waterui::Identifiable;                             // the derive is NOT in the prelude
use waterui::component::lazy::Lazy;
use waterui::reactive::collection::List as ReactiveList;

#[derive(Clone, Identifiable)]
struct Row { #[id] id: u64, title: Str }

let rows = ReactiveList::from(seed_rows);              // bulk-seed; .push/.insert/.remove diff by id

Lazy::for_each(rows.clone(), |row| text(row.title))        // reactive sequence in a stack
List::for_each(rows.clone(), |row| ListItem::new(...))     // platform list: lazy, editable
```

Note the shape: **`ForEach` is a *collection of views*, not a view.** A container consumes
it — `Lazy::for_each(data, f)` is the shorthand for `Lazy::vstack(ForEach::new(data, f))`,
and `List::for_each` is the list-control equivalent. Writing `ForEach::new(..)` where a
view is expected is a trait-bound error, not a runtime surprise.

`List` realizes only the visible window — it handles 100,000 rows, including after a
programmatic jump. Use it whenever the data is a list of rows; use the `Lazy` stacks when
you just need a reactive sequence inside your own layout. A **derived** row set (filtered
or sorted from other state) is still a collection: wrap the derived signal in
`SignalCollection` rather than watching a `Vec`.

### 5. Rebuild-driven state loss is correct behavior, not a bug to patch

A component's `body` may be expensive and may do one-time setup. When a parent's control
flow (`when`, `watch`, a route change) reconstructs a component, that instance is *gone*
and a new one initializes — losing its internal state is the intended semantics.

If some state must survive, that state was owned at the wrong level: lift it into a
`Binding` held by the parent and pass it down. Never try to preserve it with hidden
caches, hook-like slots, or position-keyed storage. Those do not exist in WaterUI and
adding them is an architectural error.

## Quick start

```rust
use waterui::app::App;
use waterui::prelude::*;

fn counter() -> impl View {
    let count = Binding::i32(0);

    vstack((
        text!("Count: {count}").headline(),
        button("+1")
            .action(|State(count): State<Binding<i32>>| *count.get_mut() += 1)
            .state(&count),
    ))
    .spacing(8.0)
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(counter, env)
}
```

`use waterui::prelude::*;` brings in views, layout, colors, text, controls, navigation,
menus, `State`, `Str`, `AnyView`, `SignalExt`, and `AnimationExt`. Several everyday names
live outside it:

```rust
use waterui::Identifiable;                             // the derive macro
use waterui::reactive::binding;                        // the general Binding constructor
use waterui::reactive::collection::List as ReactiveList;
use waterui::widget::condition::when;                  // conditionals
use waterui::animation::Animation;                     // animation curves
use waterui::component::lazy::Lazy;                    // reactive stacks over a collection
use waterui::views::ForEach;                           // the collection itself
use waterui::gesture::{DragGesture, LongPressGesture, TapGesture};
use waterui::cursor::CursorStyle;
use waterui::drag_drop::DragData;
use waterui::env::with;                                // scope a value to a subtree
use waterui::task::{sleep, spawn_local};               // async utilities
```

Components behind cargo features are also absent until you enable them. `waterui`'s
defaults are `gpu`, `assets`, `media`, `inspector`, `snackbar`; `webview`, `chart`,
`barcode`, `map`, `particle`, and `flow-markdown` are opt-in in `Cargo.toml`. Keep the
generated `dev = ["waterui/dynamic_linking"]` feature — it is what makes `water preview`
and the fast dev loop link dynamically ([references/project.md](references/project.md)).

## Core building blocks

### Views

Any function returning `impl View` is a view — no wrapper type, no trait to implement.
`&'static str`, `String`, and `Str` are views too, so a bare literal is valid content.

```rust
fn card(title: &'static str) -> impl View {
    vstack((text(title).title(), Divider))
}

vstack((card("Hello"), card("World"), "a bare literal is a view"))
```

`Str` is WaterUI's string type: cheap to clone (a static reference or a ref-counted
buffer), derefs to `str`, and converts from both directions — `Str::from_static("lit")`
for literals, `format!(..).into()` for runtime text. It is in the prelude and is what
`Binding<Str>` text fields hold.

Views are `'static`, so a borrowed `&str` parameter will not compile. Take `&'static str`,
`Str`, or `impl IntoText` instead — the last is the friendliest and is what the built-in
constructors accept. (A helper that only *reads* a `&T` while building may keep the
reference parameter and return `impl View + use<>`.)

When a component needs to read the environment or be stored as a value, implement the
trait directly — `body` takes `self` by value:

```rust
struct ColorSwatch { color: Binding<Color> }

impl View for ColorSwatch {
    fn body(self, _env: &Environment) -> impl View {
        signal_color(self.color).size(64.0, 32.0)   // a Color is itself a view
    }
}
```

`AnyView` is the erased view type — `.anyview()` or `AnyView::new(v)` — for match arms,
struct fields, and heterogeneous `collect()`s.

### State

```rust
let count = Binding::i32(0);                    // bool f32 f64 i32 i64 isize u32 u64 usize
let flag  = Binding::bool(false);
let name  = Binding::container(String::new());  // any Clone type
let sel: Binding<Option<Selected>> = Binding::default();   // empty optional

let pane: Binding<Pane> = binding(Pane::Inbox); // general form, needs an inferable type
let settings = Settings::binding();             // #[form] types: inference-free
```

There is no `Binding::new`. Prefer the typed constructors: `binding(v)` takes
`impl Into<T>`, so `T` is frequently ambiguous and you get "type annotations needed"
unless something downstream pins it (an annotation, a turbofish, a struct field, or a
control like `toggle`/`slider` that takes `&Binding<bool>` / `&Binding<f64>`).

Pass bindings to child views by reference (`&Binding<T>`); clone only when a value must be
owned by a closure or a modifier.

### Text

`text()` for static strings, `text!` for anything reactive, interpolated, or localized.
`text!` is the i18n pipeline: the whole literal is a translation-catalog key, and its
placeholder names are slot keys — bare identifiers, aliased with `name = expr` when the
local has a different name ([references/i18n.md](references/i18n.md)).

```rust
text("Settings").title()                        // title/headline/sub_headline/body/caption/footnote
text!("Count: {count}")                         // updates automatically
text!("{unread} unread", unread = mail.count()) // aliasing an expression into a slot
text!("Blur: {blur:.1}")                        // format specs work
```

Import the macro and write bare `text!` — never `waterui::text!`. On a `Text`, `.size(..)`
is the *font* size (and shadows the two-argument frame `.size(w, h)`).

### Layout

```rust
hstack((a, b, c)).spacing(8.0)
vstack((a, b)).alignment(HorizontalAlignment::Leading).padding()
zstack((background, content))
scroll(content)
spacer()                    // flexible gap
spacer().height(16.0)       // fixed gap

let buttons: HStack<_> = items.iter().map(|i| button(i.label)).collect();
```

A fixed, known set of children is a tuple — `vstack((a, b, c))`. Reach for `vec!` only
when the length is genuinely runtime-dependent, and for a *changing* set use rule 4.
A layer that must fill the window (overlays, HUDs) uses `absolute((..))`, not a
content-sized `zstack` ([references/components.md](references/components.md)).

### Conditionals

```rust
use waterui::widget::condition::when;   // not in the prelude

// A plain bool: Option<impl View> is itself a View.
row.flagged.then(|| flag_icon())

// A reactive bool: use when(...), or keep the view and drive .visible(..).
when(logged_in.clone(), || dashboard()).otherwise(|| login())
when(state.equal_to(0), || loading())
    .or(state.equal_to(1), || ready())
    .otherwise(|| error())

new_marker().visible(is_new.clone())        // keep the view, drive its visibility
```

The distinction matters: `Option<impl View>` is a view, but a *signal* of
`Option<impl View>` is not, so `flag.map(|b| b.then(|| view))` on a `Binding` does not
compile. Reach for `when` or `.visible` there.

For many branches over a plain (non-reactive) value, a `match` returning `.anyview()` is
clearer than a long `when` chain.

### Modifiers

```rust
.padding() / .padding_with(16.0) / .padding_with(EdgeInsets::all(16.0))
.background(color) / .foreground(color) / .overlay(view)
.size(w, h) / .width(w) / .height(h) / .min_width(w) / .max_width(w) / .min_size(..) / .max_size(..)
.scale(x, y) / .rotation(degrees) / .offset(x, y)     // two arguments, not one
.border(color, width) / .shadow(shadow) / .clip(shape)
.opacity(signal) / .visible(signal) / .disabled(signal)
.blur(signal) / .brightness(..) / .contrast(..) / .saturation(..) / .grayscale(..) / .hue_rotation(..)
.a11y_label(..) / .a11y_id("settings.wifi") / .a11y_role(..)
.on_appear(..) / .on_change(&signal, ..) / .on_tap(..) / .gesture(g, handler) / .context_menu(items)
.cursor(style) / .ignore_safe_area(EdgeSet::ALL) / .floating()
```

Visual modifiers take signals — pass bindings straight in; the filter row works on *any*
view, not just images. Plain-size frame modifiers (`.size`, `.width`, `.min_width`) take
`f32`; the `max_*` pair also accepts signals.

### The Environment

`Environment` is a type-indexed container that flows down the view tree: the type *is* the
key, so there is no registration step and no string names. It carries the theme, the
locale, and any service your app installs, which is how a deeply nested button reaches
shared configuration without every intermediate function taking it as a parameter.

```rust
use waterui::env::{use_env, with};

#[derive(Clone)]
struct ApiClient { base_url: Str }
waterui::impl_extractor!(ApiClient);          // makes it a handler/`use_env` parameter

// Seeding, usually in `app(env)`:
env.insert(client);                            // in place
env.with(client);                              // in place, chains
let scoped = env.extending(client);            // non-mutating overlay
with(subtree_view, LayoutDirection::RightToLeft)   // free fn: scope a value to one subtree

// Reading, from a view:
use_env(|client: ApiClient| text!("API: {url}", url = Binding::container(client.base_url)))

// Reading, from a handler — same extractors, no ceremony:
button("Send").action(|client: ApiClient| send(&client))

// Reading, where absence is legitimate — the non-panicking form:
let locale = env.get::<Locale>().cloned().unwrap_or_else(|| locales::EN.clone());
```

Inserting the same type twice replaces the earlier value; `Store<K, V>` pairs a value with
a marker type when one type genuinely needs several roles. `.install(plugin)` scopes a
value to a subtree rather than the whole app.

Extraction is fast-fail: a missing type panics with a message naming it. That is
deliberate — wrap the parameter in `Option<T>` (or use `env.get`) where absence is
legitimate. It is also why a hand-built `Environment::new()` needs a theme installed
before it can render themed views; theme tokens panic rather than falling back to a
guessed color.

`.state(&value)` from rule 3 is the same machinery with a narrower scope: it installs into
the environment of one view, and `State<T>` reads it back.

### Accessibility is part of construction

Every control takes a mandatory label: `button("Save")`, `toggle("Wi-Fi", &on)`,
`slider("Volume", &level)`, `Picker::new("Sort", items, &choice)`. (The one exception:
`progress(value).label("Downloading")` takes its label as a modifier.) This is not
optional decoration — it drives screen readers *and* it is what `waterui-testing`
queries. To hide a label visually while keeping it in the tree, use the display-mode API,
never drop the label: `.hide_label()` on any labeled control, `.label_style(
LabelDisplayMode::IconOnly)` for icon buttons, `.install(LabelDisplayMode::IconOnly)` for
a whole toolbar.

## Component index

Look up exact signatures in [references/components.md](references/components.md); media,
web, graphics, and data rows live in [references/media.md](references/media.md).

| Category | Components |
|---|---|
| Layout | `hstack` `vstack` `zstack` `scroll` `spacer` `grid` `absolute` `overlay` `Divider` |
| Controls | `button` `toggle` `slider` `stepper` `field`/`TextField` `progress` `Picker` |
| Menus | `Menu` `Command` `.context_menu` `Shortcut` |
| Interaction | `.on_tap` `.gesture` `TapGesture` `LongPressGesture` `DragGesture` `.draggable` `.drop_destination` `.on_hover_enter` `.cursor` |
| Text | `text` `text!` `styled` `Code` `RichText` `include_markdown!` `FlowMarkdown` |
| Collections | `List` `ListItem` `ForEach` `SignalCollection` `ScrollController` |
| Navigation | `Tabs` `Tab` `NavigationStack` `NavigationLink` `NavigationSplitView` `Window` |
| Forms | `#[form]` `form()` `DatePicker` `Calendar` `ColorPicker` `FilePicker` |
| Overlays | `Snackbar` `SnackbarManager` `FullScreenOverlayManager` `Card` `suspense` |
| Media | `Photo` `Image` `VideoPlayer` `PlaybackSession` `MediaPicker` |
| Data | `Chart` (12 kinds) `Map` |
| Graphics | `Canvas` `Barcode::qr()` `Svg` `shader!` `ParticleSystem` `GpuSurface` icon sets |
| Platform | `WebView` `#[js_api]` `waterui-chromium` |

## Verify before declaring done

WaterUI has a fast headless feedback loop; use it instead of reasoning about whether a
view renders. Neither of these needs a device.

```bash
water preview my_view --backend hydrolysis --theme material3 --output preview.png
cargo nextest run -p my-app
```

`#[preview] fn my_view() -> impl View` renders a view to PNG. `#[waterui::test]` drives
the real accessibility tree with taps and assertions — it is simultaneously an
interaction test and an accessibility check, which is why a component that cannot be
tested this way is a bug rather than a gap. Details in
[references/testing.md](references/testing.md).

## Gotchas worth memorizing

| Symptom | Cause | Fix |
|---|---|---|
| UI never updates | `.get()` in a view body | pass the binding |
| State resets on every keystroke | `watch` rebuilding the subtree | `text!` / signal-taking API / `Lazy::for_each` |
| `no function or associated item named 'new'` on `Binding` | `Binding::new` does not exist | `Binding::i32(v)` / `Binding::container(v)` / `Binding::default()` |
| `cannot find function 'when'` / derive `Identifiable` | not in the prelude | `use waterui::widget::condition::when;` / `use waterui::Identifiable;` |
| Rounded corners come out fully round | `RoundedRectangle::new(r)` takes a *fraction* of the shorter side, not points | `new(0.1)`-scale values, or `Capsule` |
| A background task dies instantly | the `spawn_local` handle cancels on drop | `.detach()` it |
| `LongPressGesture::new(Duration…)` rejected | duration is a `u32` in backend time units | `LongPressGesture::new(500)` |
| `.is_empty()` missing on a string signal | different name | `.str_is_empty()`, `.str_len()`, `.str_contains(..)` |
| Wrong binding arrives in a handler | positional `State<T>` | first `.state()` → first parameter |
| Scrolling or list updates are janky | `watch` over a `Vec` | `List::for_each` / `Lazy::for_each` / `SignalCollection` |
| `ForEach<..>: View is not satisfied` | `ForEach` is a collection, not a view | `Lazy::for_each(..)`, or hand it to a container |
| `.title("Inbox")` rejects its argument | `Text::title()` (font size) shadows the navigation title | title the container, or `NavigationView::new(title, content)` |
| A test's wait can never fail | query `.wait_for_existence(..)` returns `bool` | wrap it in `assert!` |
| `use of undeclared crate 'tracing'` | logging is re-exported | `waterui::log::debug!(..)` |
| `borrowed data escapes outside of function` | views are `'static` | `&'static str` / `Str` / `impl IntoText`, or `impl View + use<>` |
| type annotations needed after `binding(v)` | `binding` takes `impl Into<T>` | `Binding::i32(0)` etc., turbofish, or annotate |

Rust rules still apply on top of these: no `println!` (use `waterui::log::debug!`,
surfaced by `water run --logs debug`), and nothing blocking on the UI thread — use
`.action_async`, `.task(..)`, or `waterui::task::{spawn_local, sleep}` with `.detach()`.
