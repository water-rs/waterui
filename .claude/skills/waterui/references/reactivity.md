# Reactivity

## Contents

- Signal vocabulary
- Creating state
- Transforming signals
- Combining signals
- Projecting struct fields
- Feeding signals to views
- Reading state in handlers
- Animation
- Reactive collections
- Conditionals
- `Dynamic` and `watch` — the escape hatch
- Async and lifecycle

## Signal vocabulary

Three types cover everything:

- **`Binding<T>`** — mutable state you own. Read with `.get()`, write with `.set()` /
  `.get_mut()`.
- **`Computed<T>`** — a derived, read-only value. Produced by `.computed()` on any signal.
- **`impl Signal<Output = T>`** — the trait both implement, plus every transform.
  Transforms return anonymous types (`Map<..>`, `Zip<..>`); you rarely name them.

APIs take the loosest thing that works, in three flavors:

| Parameter type | Accepts |
|---|---|
| `impl IntoComputed<T>` | a plain `T`, a `Binding<T>`, a `Computed<T>`, any signal of `T` |
| `impl IntoSignalF32` | `f32`, `f64`, and signals of either |
| `&Binding<T>` | a binding specifically — the API writes back to it |

That third one is the tell for two-way controls: `toggle`, `slider`, `stepper`, `field`,
and `Picker` all take `&Binding<..>` because they mutate it.

## Creating state

```rust
use waterui::reactive::binding;

let count = Binding::i32(0);                    // typed constructors, primitives:
let ratio = Binding::f64(1.5);                  // bool f32 f64 i32 i64 isize u32 u64 usize
let flag  = Binding::bool(false);
let name  = Binding::container(Str::from("Ada"));  // any Clone type
let items = Binding::container(Vec::<Row>::new());

let pane: Binding<Pane> = binding(Pane::Inbox); // general form
```

`Binding::new` does not exist. The typed constructors are the reliable default:
`binding(v)` is declared as `binding<T>(value: impl Into<T>)`, so `T` cannot be inferred
from the argument alone. It works when something downstream pins the type — an
annotation, a struct field of known type, or a control such as `toggle(.., &b)` /
`slider(.., &b)` — and otherwise produces "type annotations needed".

Writing:

```rust
count.set(5);
*count.get_mut() += 1;          // guard: writes back on drop
count.with_mut(|v| v.push(x));  // in-place mutation of a container
flag.toggle();                  // bool convenience
```

## Transforming signals

These are methods on any signal (from `SignalExt`). They are cheap and do not clone the
source — `count.not()` reads naturally, no `.clone()` needed.

```rust
// core
.map(|v| v * 2)          .computed()        .cached()      .distinct()
.map_into::<U>()         .inspect(..)       .with(metadata)

// bool
.not()   .and(&other)   .or(&other)   .select(if_true, if_false)   .then_some(v)

// comparison -> Signal<bool>
.equal_to(5)   .gt(0)   .lt(9)   .ge(1)   .le(8)   .condition(|v| v.is_ascii())

// numeric
.negate()   .abs()   .sign()   .is_positive()   .is_negative()   .is_zero()

// Option<T>
.is_some()   .is_none()   .unwrap_or(d)   .unwrap_or_default()
.map_some(|v| ..)   .and_then_some(|v| ..)   .flatten()   .some_equal_to(v)

// Result<T, E>
.is_ok()   .is_err()   .ok()   .err()   .map_ok(..)   .map_err(..)

// strings — note the str_ prefix; plain .is_empty()/.contains() are NOT signal methods
.str_is_empty()   .str_len()   .str_contains("query")

// time
.debounce(Duration::from_millis(300))   .throttle(Duration::from_millis(16))
```

`Binding<T>` additionally has binding-specific helpers that produce *writable* results:
`.range(0..=10)`, `.clamp(..)`, `.filter(..)`, `.mapping(getter, setter)`,
`.bidirectional_select(a, b)`, `.unwrap_or(d)`, `.reverse()`.

## Combining signals

```rust
let total = price.zip(&quantity).map(|(p, q)| p * q);
let ready = loaded.and(&authorized);
let label = count.zip(&unit).map(|(n, u)| format!("{n} {u}"));
```

`.zip` pairs two signals; chain it for three or more. Call `.computed()` at the end when
you need a nameable `Computed<T>` to store in a struct.

## Projecting struct fields

A `Binding<Struct>` whose type derives `Project` (which `#[form]` does automatically)
gives you a per-field binding through `.project()`. Each projected field is a real
two-way `Binding`, so a control bound to it writes back into the parent struct — and a
change to one field does not invalidate readers of the others.

```rust
#[form]
struct Settings { name: String, volume: f64, dark: bool }

let settings: Binding<Settings> = binding(Settings::default());

vstack((
    field("Name", &settings.project().name),
    slider("Volume", &settings.project().volume),
    toggle("Dark mode", &settings.project().dark),
    text!("Volume is {volume}", volume = settings.project().volume),
))
```

Use `#[derive(Project)]` directly when you want projection without form generation.

## Feeding signals to views

The whole point of the framework: hand the signal to the API, do not resolve it.

```rust
view.opacity(fade.clone())
view.visible(has_items.clone())
view.disabled(is_loading.clone())
view.scale(zoom.clone(), zoom.clone())
Photo::new(url).blur(radius.clone()).saturation(sat.clone())
text!("{status}")
```

`.clone()` on a `Binding` clones a handle, not the value — it is cheap and expected.

## Reading state in handlers

Inside an `.action()` handler you are outside the reactive graph, so `.get()` is correct
there. What you must not do is `.get()` while *building* a view.

```rust
button("Reset")
    .action(|State(form): State<Binding<Settings>>| form.set(Settings::default()))
    .state(&form)
```

Handler parameters are extractors. Besides `State<T>`:

- `Environment` — the whole environment.
- `Use<T>` — any `Clone` value installed in the environment (as opposed to `.state()`).
- `Option<E>` — makes any extractor optional instead of failing.
- Custom types: `#[derive(Clone)] struct ApiClient; impl_extractor!(ApiClient);` then take
  `client: ApiClient` as a parameter directly.
- Context extractors supplied by components, e.g. `Navigator<Route>`, `ListDelete`,
  `ListMove`, `State<SnackbarManager>`.

```rust
fn delete_row(ListDelete(index): ListDelete, State(state): State<Editor>) {
    let _ = state.rows.remove(index);
}
```

## Animation

Animation is metadata attached to a *signal*, not a separate view type. Attach it, then
pass the animated signal wherever the plain one would go.

```rust
use core::time::Duration;
use waterui::animation::Animation;   // not in the prelude

let scale = Binding::f32(1.0);
let animated = scale.with(Animation::spring(300.0, 15.0));

view.scale(animated.clone(), animated.clone())
```

Curves: `Animation::linear(d)`, `ease_in(d)`, `ease_out(d)`, `ease_in_out(d)`,
`spring(stiffness, damping)`, `bezier(d, x1, y1, x2, y2)`.
`.animated()` is shorthand for `ease_in_out(250ms)`.

Because the animation rides on the signal, any signal can be animated — including derived
ones: `active.select(1.2_f32, 1.0).with(Animation::spring(300.0, 15.0))`.

## Reactive collections

`nami::collection::List<T>` is a reactive sequence: mutating it emits a membership patch
rather than a whole-collection invalidation.

```rust
use waterui::component::lazy::Lazy;
use waterui::reactive::collection::List as ReactiveList;

let rows = ReactiveList::<Row>::new();
rows.push(Row { id: 1, title: "First".into() });
rows.remove(0);
let snapshot = rows.snapshot();      // Vec<Row>, for read-only work
let _ = rows.replace(new_vec);       // wholesale swap, still diffed by id

Lazy::for_each(rows.clone(), row_view)          // == Lazy::vstack(ForEach::new(..))
Lazy::hstack(ForEach::new(rows.clone(), row_view))
```

`ForEach` implements `Views` (a collection of views), not `View`, so a container has to
consume it. `Lazy::for_each` / `Lazy::vstack` / `Lazy::hstack` are those containers for
plain stacks, and `List::for_each` is the platform list control.

Items must derive `Identifiable` with an `#[id]` field — that id is the diffing key, so
it must be stable across updates of the same logical row.

A *fixed* set of items does not need any of this: an array or `Vec` works directly with
`List::for_each` and `ForEach::new`.

## Conditionals

```rust
use waterui::widget::condition::when;

// A plain bool: Option<impl View> is itself a View.
row.flagged.then(|| new_marker())

// A reactive bool: a *signal* of Option<View> is NOT a view — use when(..) or .visible(..).
new_marker().visible(is_new.clone())

// If / else.
when(logged_in.clone(), || dashboard()).otherwise(|| login_form())

// If / else-if / else.
when(state.equal_to(0), || loading())
    .or(state.equal_to(1), || ready())
    .or(state.equal_to(2), || failed())
    .otherwise(|| unknown())

// Negation works through the Not impl on Binding.
when(!is_loading.clone(), || content())
```

`when` reconstructs the branch that becomes active — state inside a branch does not
survive switching away and back. That is rule 5, and it is intended.

## `Dynamic` and `watch` — the escape hatch

`watch(signal, |value| view)` and `Dynamic` replace an entire subtree when the signal
fires. Everything inside is destroyed and rebuilt: focus, scroll position, and any state
owned below that point are lost, and on a large subtree it costs orders of magnitude more
than a precise update.

Before writing one, check all three replacements:

| Want | Use instead of `watch` |
|---|---|
| Text that changes | `text!("{value}")` |
| A property that changes | pass the signal to the modifier or component |
| A set of views that changes | `ForEach` / `List` |

`Dynamic` is legitimate when a handler must swap in a genuinely different view *object*
that no signal-aware API can express — for example replacing a media view when its source
URL changes:

```rust
let (handler, slot) = Dynamic::new();

button("Load")
    .action(
        |State(url): State<Binding<Str>>,
         State(blur): State<Binding<f64>>,
         State(h): State<DynamicHandler>| {
            // Photo takes impl IntoComputed<Url>; Url is From<&'static str>, so a
            // runtime string has to be parsed — and a bad URL is reported here rather
            // than silently reaching the image loader as a local path.
            let Ok(parsed) = url.get().as_str().parse::<Url>() else { return };
            h.set(Photo::new(parsed).blur(blur.clone()));
        },
    )
    .state(&url)
    .state(&blur)
    .state(&handler);

vstack((slot, /* … */))
```

Even there, keep the *reactive* properties reactive — the replacement above is built with
`.blur(blur.clone())`, not `.blur(blur.get())`, so the slider keeps working without
another swap.

## Async and lifecycle

```rust
button("Fetch")
    .action_async(|State(out): State<Binding<Str>>| async move {
        out.set(fetch().await);
    })
    .state(&result);

view.task(async { warm_cache().await });   // runs while the view is alive
view.on_appear(|| tracing::debug!("shown"));
view.on_disappear(|| ());
view.on_change(&query, |new_value| tracing::debug!(?new_value));
```

Futures run on the UI thread's local executor, so they may hold non-`Send` state — but
they must never block. Anything CPU-bound belongs on a worker, with the result delivered
back through a `Binding`.
