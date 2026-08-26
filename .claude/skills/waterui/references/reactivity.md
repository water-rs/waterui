# Reactivity

## Contents

- Signal vocabulary
- Creating state
- Transforming signals
- Combining signals
- Constants as signals
- Projecting struct fields
- Feeding signals to views
- Reading state in handlers
- Animation
- Reactive collections
- Conditionals
- `Dynamic` and `watch` — the escape hatch
- Async, tasks, and lifecycle

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
let status = Binding::container("Waiting…");    // Binding<&'static str> — fine for status text

let sel: Binding<Option<Selected>> = Binding::default();   // empty optional selection
let pane: Binding<Pane> = binding(Pane::Inbox);            // general form
let ws = binding::<WindowState>(WindowState::default());   // …or pin T with a turbofish
```

`Binding::new` does not exist. `binding(v)` is declared as `binding<T>(value: impl
Into<T>)`, so `T` cannot be inferred from the argument alone — it works when something
downstream pins the type: a `let` annotation, a turbofish, a struct field, a control such
as `toggle(.., &b)`, or a helper parameter typed `&Binding<i32>`. When a helper with a
typed parameter consumes the binding, plain `binding(0)` / `binding(false)` is the
idiomatic spelling — no typed constructor needed. A `#[form]` struct has the cleanest
form of all: `Settings::binding()` (from `FormBuilder`), which needs no annotation.

Writing:

```rust
count.set(5);
*count.get_mut() += 1;          // guard: writes back on drop
count.with_mut(|v| v.push(x));  // in-place mutation of a container
flag.toggle();                  // bool convenience
```

## Transforming signals

These are methods on any signal (from `SignalExt`, which the prelude re-exports). They
take `&self` and do not consume or clone the source — `count.not()` and
`hovered.select(..)` read naturally with no `.clone()` first. Clone only when a finished
signal is *moved* into two places (e.g. passed to `.scale(x, y)` twice).

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

`.select(a, b)` needs both arms to be one concrete type — convert theme tokens or `Srgb`
values to `Color` first (`let on: Color = Accent.into();`).

`Binding<T>` additionally has binding-specific helpers that produce *writable* results:
`.range(0..=10)`, `.clamp(..)`, `.filter(..)`, `.bidirectional_select(a, b)`,
`.unwrap_or(d)`, `.reverse()`. The general two-way transform is an **associated
function**, not a method — `Binding::mapping(&source, getter, setter)` — and its setter
receives the *source binding* to write back through, not a `&mut` slot.

## Combining signals

```rust
let total = price.zip(&quantity).map(|(p, q)| p * q);
let ready = loaded.and(&authorized);
let label = count.zip(&unit).map(|(n, u)| format!("{n} {u}"));
```

`.zip` takes the other signal by reference; chain it for three or more. Chaining
left-associates, so the closure destructures **nested** pairs, not a flat tuple:

```rust
let config = a.zip(&b).zip(&c).zip(&d)
    .map(|(((a, b), c), d)| build_config(a, b, c, d))
    .computed();
```

Call `.computed()` at the end when you need a nameable `Computed<T>` to store in a struct.

## Constants as signals

`impl IntoComputed<T>` accepts a plain `T`, and nami pre-declares the primitives,
`String`, `Duration`, `Vec<T>`, `BTreeMap`, and `BTreeSet` as constant signals. Two
pieces close the gaps:

```rust
use waterui::reactive::impl_constant;

impl_constant!(ChartMode);              // your own Clone type as a constant signal
let dates = Computed::constant(decorated_dates());   // a nameable, shareable constant Computed<T>
```

Without `impl_constant!`, passing a custom enum where `impl IntoComputed<T>` is expected
fails to compile; without `Computed::constant`, there is no way to *name* a constant one.

## Projecting struct fields

A `Binding<Struct>` whose type derives `Project` (which `#[form]` does automatically)
gives you a per-field binding through `.project()`. Each projected field is a real
two-way `Binding`, so a control bound to it writes back into the parent struct — and a
change to one field does not invalidate readers of the others.

```rust
#[form]
struct Settings { name: Str, volume: f64, dark: bool }   // text fields bind Str, not String

let settings = Settings::binding();

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

`.clone()` on a `Binding` clones a handle, not the value — it is cheap and expected when
a signal is moved into a modifier and used again afterwards.

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
  `ListMove`, `State<SnackbarManager>`, `WebViewProxy`, `DragData`.

```rust
fn delete_row(ListDelete(index): ListDelete, State(state): State<Editor>) {
    let _ = state.rows.remove(index);
}
```

To accept a caller-supplied action in your *own* reusable view function, take the handler
generically and pass it through:

```rust
use waterui::Handler;

fn drawer_item<F, Args>(title: &'static str, action: F) -> impl View
where
    F: Handler<Args, ()> + 'static,
    Args: 'static,
{
    text(title).padding().on_tap(action)
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
`spring(stiffness, damping)`, `bezier(d, x1, y1, x2, y2)`, and `Animation::default()`
(the system default). `.with(animation)` is the one attachment spelling to use.
`.animated()` (prelude) attaches the *system-default* animation — do not import a second
`AnimationExt` from `waterui::animation`: it carries a same-named `animated()` with
different timing, and which one runs then depends on which trait is in scope.

Because the animation rides on the signal, any signal can be animated — including derived
ones — and independently-animated signals compose:

```rust
let hover_scale = hovered.select(1.05_f32, 1.0).with(Animation::spring(400.0, 15.0));
let drop_bounce = bounce.with(Animation::spring(500.0, 10.0));
let combined = hover_scale.zip(&drop_bounce).map(|(a, b)| a * b);   // still animated
```

## Reactive collections

`nami::collection::List<T>` is a reactive sequence: mutating it emits a membership patch
rather than a whole-collection invalidation.

```rust
use waterui::component::lazy::Lazy;
use waterui::reactive::collection::List as ReactiveList;

let rows = ReactiveList::from(seed_vec);   // bulk-seed in one move — not a push loop
rows.push(Row { id: 9, title: "Last".into() });
rows.insert(0, Row { id: 0, title: "First".into() });   // positional splice, id-diffed
let _ = rows.remove(0);              // #[must_use] — bind the removed value
let snapshot = rows.snapshot();      // Vec<Row>, for read-only work
let _ = rows.replace(new_vec);       // wholesale swap, still diffed by id

Lazy::for_each(rows.clone(), row_view)          // == Lazy::vstack(ForEach::new(..))
Lazy::hstack(ForEach::new(rows.clone(), row_view))
```

`ForEach` implements `Views` (a collection of views), not `View`, so a container has to
consume it. `Lazy::for_each` / `Lazy::vstack` / `Lazy::hstack` defer realization;
`VStack::for_each` / `HStack::for_each` build an ordinary stack over the same collection
when you want stack modifiers and eager layout (see components.md for
`collection_transition`, which animates membership changes).

Items must derive `Identifiable` (`use waterui::Identifiable;` — the derive is not in the
prelude) with an `#[id]` field that is stable across updates of the same logical row.

A **derived** row set — filtered, sorted, or joined from other state — is wrapped in
`SignalCollection`, which adapts any `Signal<Output = Vec<T>>` into a diffable
collection. This is the answer to "filter a list as the user types"; without it the only
road is `watch` over a `Vec`, which rule 4 forbids:

```rust
use waterui::reactive::collection::SignalCollection;

let visible = SignalCollection::new(
    messages.zip(&query).map(|(all, q)| {
        all.into_iter().filter(|m| m.subject.contains(q.as_str())).collect::<Vec<_>>()
    }),
);
List::for_each(visible, message_row)
```

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
survive switching away and back. That is rule 5, and it is intended. When tearing a view
down is itself costly or lossy (a `ParticleSystem` restarts its simulation, a GPU view
re-uploads), prefer keeping every layer mounted and cross-fading their `.opacity`
signals instead of switching branches.

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
| A set of views that changes | `ForEach` / `List` / `SignalCollection` |

The associated-function spelling `Dynamic::watch(signal, closure)` is the same thing as
the free `watch(..)`; the closure takes the value by move and must return one uniform
type, so heterogeneous arms erase with `AnyView::new(..)`:

```rust
Dynamic::watch(mode.clone(), |mode| match mode {
    ChartMode::Bar => AnyView::new(bar_chart()),
    ChartMode::Line => AnyView::new(line_chart()),
})
```

That usage is legitimate — the arms are genuinely different view types and this is
structural control flow, not a stand-in for a reactive property. The other legitimate
form is a handler-driven swap through `Dynamic::new()`:

```rust
let (handler, slot) = Dynamic::new();

button("Load")
    .action(
        |State(url): State<Binding<Str>>,
         State(blur): State<Binding<f64>>,
         State(h): State<DynamicHandler>| {
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

## Async, tasks, and lifecycle

```rust
button("Fetch")
    .action_async(|State(out): State<Binding<Str>>| async move {
        out.set(fetch().await);
    })
    .state(&result);

view.task(async { warm_cache().await });   // runs while the view is alive; dropped with it
view.on_appear(|| waterui::log::debug!("shown"));
view.on_disappear(|| ());
view.on_change(&query, |new_value| waterui::log::debug!(?new_value));
```

`.on_change(&signal, f)` takes the signal by reference and a plain `Fn(T)` closure — the
new value arrives **by value**, and this is an ordinary closure, not an extractor handler,
so it reaches state by capturing cloned bindings.

Free-standing async work — from a synchronous handler, or a background loop driving a
binding — goes through `waterui::task`:

```rust
use waterui::task::{sleep, spawn_local};

spawn_local(async move {
    sleep(Duration::from_millis(200)).await;   // the async sleep — never std::thread::sleep
    bounce.set(1.0);
})
.detach();
```

Two rules there: `spawn_local` is the right spawn for UI work (the future may hold
non-`Send` state), and the returned handle **cancels the future when dropped** — a
fire-and-forget task must be `.detach()`ed or it silently dies on the spot. For a
cancellable stream (an LLM feed, a poller), keep a revision counter in a binding and have
the loop return when the revision moves on.

Futures run on the UI thread's local executor, so they may hold non-`Send` state — but
they must never block. Anything CPU-bound belongs on a worker (`waterui::task::spawn`),
with the result delivered back through a `Binding`. Clippy's `future_not_send` fires on
async helpers holding UI state; that is the sanctioned case for a narrowly scoped
`#[expect(clippy::future_not_send, reason = "UI-thread state")]` on the item.
