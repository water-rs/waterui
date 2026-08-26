# WaterUI API Style Guide

This document captures the design conventions that govern WaterUI's public
API. Treat each rule as durable; if you have a reason to break one, please
discuss it before merging.

These conventions came out of a broad ergonomics review of every example in
`examples/` and a multi-round design pass. They are not prescriptive opinions
- each one resolves a real source of papercut friction we saw repeatedly.

---

## 1. Free function vs `Type::new(...)`

For every public component, **two** entry points coexist by design:

- A **free function** with a lowercase name: `vstack(…)`, `hstack(…)`,
  `binding(…)`, `text(…)`, `button(…)`, `link(…)`, `slider(…)`, `progress(…)`.
  This is the **simple/default** form; it accepts the most common argument
  shape and infers what it can.

- An **inherent constructor** `Type::new(…)`: `VStack::new(…)`,
  `Binding::f32(…)`, `Text::new(…)`, `Button::new(…)`, `Slider::new(…)`.
  This is the **general/typed** form; it accepts the widest possible argument
  shape and shows up by name in type signatures (`let grid: VStack<_> = …`).

Do **not** collapse one into the other. Free functions are sugar; `::new` is
the primitive. Both have audiences.

For `Type::new` variants targeted at a specific value type, use a noun-suffix
- `Picker::new(...)` for the general case, then specialized constructors via
trait dispatch where the trait describes the value (e.g.
`DatePicker::new<T: DatePickable>(...)`). Avoid parallel
`Type::custom(...) / Type::variant(...)` ladders unless they really
materialize *different* objects.

---

## 2. Sugar and raw forms both stay

When a domain has a sensible default, expose three layers:

- A **sugar method** with no arguments: `.animated()` for "default animation",
  `.padding()` for "default padding".
- A **named explicit method** that takes the relevant value: `.padding_with(value)`.
  Reads better at the call site than the generic primitive when the metadata
  kind is the point.
- A **generic primitive** that accepts any payload: `.with(metadata)`. Stays
  underneath as the universal escape hatch.

Do not delete sugar in the name of "one way to do it". Sugar is what makes
call sites readable; minimalism of API surface is not the goal.

The middle layer has to earn its place, though. A named explicit method that is
a straight rename of the generic primitive — same receiver, same argument, same
return — buys nothing and costs a second name that can be imported separately
and resolve differently. `AnimationExt::with_animation(animation)` was exactly
that alias for `SignalExt::with(animation)` and was removed; animations attach
with `.with(animation)`.

---

## 3. Typography: three orthogonal layers

See [`waterui-text::font`](../components/text/src/font.rs) for the full
discussion. In short:

- Prefer **semantic categories**: `.title()`, `.headline()`,
  `.sub_headline()`, `.body()`, `.caption()`. These cascade through `Theme`
  and respect accessibility text-size scaling.
- Use **font slots** when you need to pass the choice around as a value:
  `.font(Title)`, `.font(font::Caption)`.
- Use **direct overrides** (`.size(f32)`, `.bold()`, `.italic()`,
  explicit `Font` construction) for fixed-layout cases that should NOT
  follow Theme scaling: posters, splash screens, examples that demonstrate
  a specific visual.

These are different abstraction layers; do not collapse them.

---

## 4. State injection vs `move ||`

Action handlers come in two shapes; pick the one that matches the constraint
on captured state.

### `move ||` — single binding consumed by value

```rust
button("Reset")
    .action(move || some_binding.set(Default::default()))
```

Use this when:
- The closure captures one (or two) bindings,
- AND each captured binding is genuinely consumed (no need to keep the
  outer name alive).

### `State<T>` injection — bindings shared across handlers and views

```rust
button("+1")
    .action(|State(c): State<Binding<i32>>| *c.get_mut() += 1)
    .state(&counter)
```

Use this when the binding is also referenced elsewhere in the surrounding
view — `State<T>` injection avoids littering the function with
`let foo = foo.clone();` lines per handler.

### Wrap-in-struct for many bindings

`button(...).action(|State(a): ..., State(b): ..., State(c): ..., State(d): ...|).state(&a).state(&b).state(&c).state(&d)`
is **the wrong remedy** for a multi-binding action. Wrap related bindings
in a single `Clone` struct and inject one `State<MyStruct>`:

```rust
#[derive(Clone)]
struct StreamControl {
    document_index:  Binding<i32>,
    markdown:        Binding<String>,
    char_progress:   Binding<f64>,
    stream_revision: Binding<u64>,
    streaming:       Binding<bool>,
}

let control = StreamControl { … };

button("Prev doc")
    .action(|State(c): State<StreamControl>| {
        cancel_stream(&c.streaming, &c.stream_revision);
        *c.document_index.get_mut() -= 1;
        reset_stream(&c.markdown, &c.char_progress);
    })
    .state(&control)
```

`Binding<T>: Clone` is cheap (Arc bump), so deriving `Clone` on the wrapper
is virtually free. The canonical example is `flow-markdown`'s
`StreamControl`.

Hard rule: do not stack 4+ `State<Binding<T>>` parameters in one action.
At that point, define a struct.

---

## 5. `.action`, `.action_async`, and `.on_change`

Three different trigger semantics. Pick the one whose verb fits.

| Method | When the handler runs | Use for |
|---|---|---|
| `.action(...)` | A user gesture lands on the view (tap, click). Synchronous handler. | Button presses; one-shot UI commands. |
| `.action_async(...)` | A user gesture lands; handler is awaited. Spawned as a task. | Async work triggered by tap (file picker, network call). |
| `.on_change(&binding, ...)` | A reactive value the handler observes changes. | Side effects driven by state, not by gesture (writing locale tag when a picker selection changes; persisting settings; firing analytics). |

`.on_change` is **not** a button-handler alternative — it fires on data flow,
not on user input. If two of these look interchangeable, you probably want
`.action`.

---

## 6. `text!` placeholders are i18n slot keys

Placeholder names in the `text!` format string are translation slot keys.
The compile-time translation extractor and the runtime locale resolver
both rely on a 1:1 mapping between placeholder name and identifier in
the surrounding scope.

This is why `text!` does NOT accept arbitrary expressions in placeholder
positions, even though it would be more convenient if it did.

If a binding's local identifier does not match the desired slot key, alias
explicitly:

```rust
text!("{slot}", slot = local_var)
```

Do **not** "fix" `text!` to take arbitrary expressions. Doing so silently
breaks translation extraction (which can no longer find the slot keys) and
locale reactivity. The `_text` clone aliases at the top of section
functions in `examples/flow_markdown/src/lib.rs` are the canonical
workaround for this constraint.

---

## 7. Modifier signatures: `IntoSignalF32` is special

For all reactive modifier inputs, prefer the generic `impl IntoSignal<T>`
bound. The one exception is **`f32`-typed inputs**, which use the
monomorphic `IntoSignalF32` trait (defined in
[`waterui-core::computed_f32`](../core/src/computed_f32.rs)).

Why: numeric literals default to `f64` in Rust, so a generic
`impl IntoSignal<f32>` would force callers to write `.opacity(0.5_f32)`
everywhere. `IntoSignalF32` does the cross-numeric coercion at the boundary
so `.opacity(0.5)` Just Works.

Do **not** invent sister monomorphic traits (`IntoSignalF64`,
`IntoSignalColor`, `IntoSignalCursorStyle`, etc.). For non-`f32` types, the
generic `IntoSignal<T>` bound combined with `impl_constant!`-registered
value types is sufficient. See `Phase 2` of the API ergonomics cleanup PR
for the list of currently-registered value types.

---

## 8. `impl_constant!` over manual wrappers

If you find yourself writing `Computed::constant(value)` or
`Binding::container(static_value)` to satisfy an `impl IntoSignal<T>` /
`impl IntoComputed<T>` parameter, the type probably wants
`impl_constant!`. Add it to the type's defining module instead of forcing
every call site to wrap.

Conversely, do not delete `Computed::constant` / `Binding::container` —
they are the load-bearing primitive that `impl_constant!` builds on.

---

## 9. Component constructor shape

Builder methods come in three flavors; use the one whose semantics fit:

- **No-arg flag toggles** for boolean opt-ins:
  `ColorPicker::new("Accent", &c).with_alpha().with_hdr()` — never
  `.support_alpha(true).support_hdr(true)`. The `bool` parameter is
  meaningless ceremony.

- **Single-call combined operations** when two pieces of data are always
  set together: `Snackbar::new("…").action("Undo", || …)` — never
  `.action("Undo").handler(|| …)`. Two-step builders are warranted only when
  the intermediate object is independently useful.

- **Builder-method range/configuration** for optional knobs that have a
  reasonable default: `Slider::new("Volume", &value).range(0.0..=10.0)` and
  `Stepper::new("Count", &value).range(0..=100).step(5)`. The label and
  binding are the primary arguments; the configuration follows. Slider's
  default range is `0.0..=1.0`.

- **Trait-based dispatch** when the same conceptual constructor handles
  several value types: `DatePicker::new("Due", &binding)` dispatches via
  `DatePickable` to `Date`/`Time`/`DateTime`. Don't expose
  `Type::date / Type::time / Type::datetime` as separate constructor
  names.

---

## 10. Window state is not optional

`Window::new(title, state, body)` requires the state binding positionally.
Every example calls it; making it optional was a misleading affordance.

`WindowState::default()` returns `Closed` so `binding::<WindowState>(default())`
is the idiomatic "not-yet-shown" initialization. Flip to
`WindowState::Normal` to open.

For app-internal one-shot windows (the implicit `App::new` window, popups in
hydrolysis), pass `binding(WindowState::Normal)` explicitly.

`conditional_window(&state, creator)` takes a reference to the state binding;
the closure receives an owned clone for use inside the rendered Window.

---

## 11. Color space → `.color_space(ColorSpace)`

For HDR/SDR control on a subtree, prefer the new
`ViewExt::color_space(ColorSpace)` modifier over the underlying
`metadata(HighDynamicRange::new())` / `metadata(StandardDynamicRange::new())`
calls. The `ColorSpace { Sdr, Hdr }` enum is `impl_constant!`-registered
and is a more direct surface than the metadata-key types.

The metadata types remain public for code that needs the lower-level
mechanism, but new code should use `.color_space(...)`.

---

## 12. Icon packs: function form everywhere

Every icon pack — `lucide`, `material-icon`, `sf-symbol`, `native`, plus
`waterui_icon::system_icon` — exposes its icons as **functions**, not
constants. `lucide::house()`, `mdi::home()`, `sf::house_fill()`,
`system_icon::checkmark()`. Don't add `pub const FOO: SystemIcon = …`
constants alongside; new code converges on functions.

The `SystemIcon::FOO` constants on the `SystemIcon` type itself were
removed. Use `waterui_icon::system_icon::*` functions.
