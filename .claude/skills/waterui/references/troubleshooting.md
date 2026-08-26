# Troubleshooting

## Contents

- Silent bugs (compile fine, behave wrong)
- Compile errors
- Runtime panics
- Layout problems
- Build and run problems
- Diagnosing without a device

## Silent bugs (compile fine, behave wrong)

These are the expensive ones — the type system cannot catch them, so recognize them by
symptom.

**The UI never updates.** A `.get()` reached a view-building expression, turning a signal
into a one-time snapshot. Search the view function for `.get()`; every occurrence outside a
handler or a `.map()` closure is suspect.

```rust
view.opacity(fade.get())      // frozen
view.opacity(fade.clone())    // reactive
```

**Text is stale.** Same cause, wearing a different hat: `text(format!("Count: {}", n.get()))`
formats once. Use `text!("Count: {n}")`.

**Typed input resets, or a control loses focus on every keystroke.** Something above it is a
`watch` that rebuilds the subtree. Replace it — reactive text with `text!`, a reactive
property by passing the signal, a dynamic set of views with `Lazy::for_each` / `List::for_each`.

**A list re-renders entirely when one row changes.** The collection is behind a `watch` over
a `Vec` instead of a `Lazy::for_each` / `List::for_each` over an `Identifiable` collection.

**Rows shuffle or lose state after an insert.** The `#[id]` field is not stable — it is an
array index, or regenerated per render. The id is the diffing key and must identify the same
logical row across updates.

**A handler receives the wrong binding.** Multiple `State<T>` of the same type bind
positionally: the first `.state()` call feeds the first `State<T>` parameter. Reorder, or
switch to the app-state-struct pattern where one injected struct replaces all of them.

**An animation snapshot shows the end state instead of the transition.** A test used
`std::thread::sleep`. The animation clock advances with pumped frames, not wall-clock time.
Use `app.pump_for(duration)`.

## Compile errors

| Error | Cause | Fix |
|---|---|---|
| `no function or associated item named 'new' found for struct 'Binding'` | `Binding::new` does not exist | `Binding::i32(v)` / `Binding::bool(v)` / `Binding::container(v)` |
| "type annotations needed for `Binding<_>`" after `binding(v)` | `binding` takes `impl Into<T>`, so `T` is unconstrained | use a typed constructor, annotate the `let`, or let a control such as `toggle(.., &b)` pin it |
| `cannot find function 'when' in this scope` | not in the prelude | `use waterui::widget::condition::when;` |
| `cannot find function 'binding' in this scope` | not in the prelude | `use waterui::reactive::binding;` |
| `no method named 'is_empty'` on a signal | signal string methods are prefixed | `.str_is_empty()`, `.str_len()`, `.str_contains(..)` |
| type annotations needed on `.select(1.0_f32, 0.3)` | suffixed literal fights inference | `.select(1.0 as f32, 0.3)` |
| `this function takes 2 arguments but 1 was supplied` on `.scale` | transforms are per-axis | `.scale(x, y)`, `.offset(x, y)`, `.size(w, h)` |
| `the trait bound ...: Identifiable is not satisfied` | collection item lacks an id | `#[derive(Clone, Identifiable)]` with an `#[id]` field |
| `expected 'Binding<T>', found '&Binding<T>'` (or the reverse) | two-way controls borrow, modifiers own | controls take `&binding`; modifiers take `binding.clone()` |
| `cannot find macro 'text'` | macro not imported | `use waterui::prelude::*;` then bare `text!` — never `waterui::text!` |
| `ForEach<..>: View is not satisfied` (or "expected an `FnOnce()` closure") | `ForEach` is a `Views` collection, not a view | `Lazy::for_each(data, f)`, `Lazy::vstack(ForEach::new(..))`, or `List::for_each` |
| `.title("Inbox")` — "this method takes 0 arguments" | `Text::title()` (font size) shadows the navigation `.title(impl IntoText)` | apply the navigation title to the container, not to a bare `text(..)` |
| `borrowed data escapes outside of function` on a view helper | views are `'static` | take `&'static str`, `Str`, or `impl IntoText` |
| `.shadow()` — "takes 1 argument but 0 were supplied" | it takes a shadow | `.shadow(shadow)` |
| `could not find 'chart' / 'map' / 'barcode' / 'webview' in 'waterui'` | the component is behind a cargo feature | enable it: `waterui = { …, features = ["map"] }` (defaults are `gpu`, `assets`, `media`, `inspector`, `snackbar`) |
| `the trait bound 'Url: From<Str>' is not satisfied` | `Url` converts from `&'static str` only | parse it: `s.as_str().parse::<Url>()` |
| `Computed<Option<View>>` rejected where a view is expected | a *signal* of `Option<View>` is not a view | `when(..)`, or keep the view and use `.visible(signal)` |
| `text!` rejects an expression in a placeholder | placeholders are i18n slot keys | alias it: `text!("{slot}", slot = expr)` |
| mismatched types between `if` branches returning views | different concrete view types | `when(..).otherwise(..)`, or `.anyview()` on each arm |

Do not add crate-, module-, or file-level `allow` attributes to silence a lint. If a lint is
genuinely a false positive, use a narrowly scoped item-level `expect` with a reason.

## Runtime panics

**"Environment state `T` not found".** A handler asked for `State<T>` that nothing injected.
Add `.state(&value)` on the button, or on an ancestor container if several handlers need it.

**"Environment value `T` not found".** Same, but for `Use<T>` — the value must be installed
in the environment (typically in `app(env)`), not passed via `.state()`.

**A Dew backend panic naming an unsupported view.** That is fast-fail working as designed:
the constrained target genuinely cannot render that view. Choose a supported composition
rather than trying to make the panic go away.

**A panic during a test interaction.** Interactions panic when the runtime rejects the
action, and that *is* the assertion — the element was missing, hidden, or disabled. Fix the
view or the query; do not wrap the call in a guard.

## Layout problems

**A view is invisible or zero-sized.** Its parent proposed no space. Read the accessibility
bounds (`water inspector`, or `.all()` in a test) to find which container mis-sized it —
a screenshot tells you something is wrong, bounds tell you where.

Resist the reflex to fix this with an explicit `.frame()` / `.size()` on the child. Pinning
a size around a container that fails to size its children hides the defect *and* destroys
the reproduction: the next person sees a working screen over a bug nobody can find. Fix the
container, and keep the failing case as a test.

**An edge-anchored overlay drifts or vanishes when the window grows.** The overlay layer is
a content-sized `zstack`. A window overlay layer must fill the window — wrap it in
`AbsoluteLayout` with `StretchAxis::Both`.

**A rounded rectangle's corners look stretched.** Corner radius comes from the shape kind,
not from unit-space path commands, which scale with the aspect ratio. Test shape code
against a deliberately non-square rectangle.

## Build and run problems

**`water run` returned to the prompt.** The app crashed — a run that stays alive does not
exit. Re-run with `--logs debug` and read the tail.

**Android network calls fail with a DNS/resolution error.** `[permissions.internet]` is
missing from `Water.toml`; Android denies resolution outright rather than reporting a
permission failure.

**A change to the CLI has no effect.** The `water` on `PATH` is a previously installed
binary. Reinstall with `cargo install --path cli`, or invoke the freshly built one directly.

**Scrolling or interaction is janky in a dev build only.** Check that the build has a
release-ish profile; a full stack compiled at `-O0` with debug info is slow in a way that
looks like a rendering bug.

## Diagnosing without a device

Reach for these before reaching for a simulator — they are faster and they produce evidence
you can attach to a bug report.

```bash
water preview my_view --backend hydrolysis --theme material3 --output preview.png
cargo nextest run -p my-app -E 'test(the_case)' --no-capture
water inspector
water bench --format json
```

A test that asserts on the accessibility tree tells you *what* the tree contains; a PNG
tells you what it looks like; a bench with `max_rebuild_ratio` tells you whether an
interaction is updating precisely or rebuilding. Between them, almost every WaterUI bug is
reproducible headlessly.

One caution when reading test results: a passing test is only evidence about what it
actually observes. Before trusting a green run, ask what the assertion would have to see in
order to fail. An assertion that cannot fail is not coverage.
