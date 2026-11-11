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

**The UI never updates.** A `.snapshot()` reached a view-building expression, turning a signal
into a one-time value. Search the view function for `.snapshot()`; every occurrence outside a
handler or a `.map()` closure is suspect.

```rust
view.opacity(fade.snapshot())      // frozen
view.opacity(fade.clone())    // reactive
```

The same applies inside a `when(..)` / `.otherwise(..)` payload: a branch is built when its
condition becomes true, so a `.snapshot()` there stays frozen while the condition holds.
Pass the signal instead.

**Text is stale.** Same cause, wearing a different hat: `text(format!("Count: {}", n.snapshot()))`
formats once. Use `text!("Count: {n}")`. (Plain `text(format!(..))` over a *non-signal*
value is fine — the defect is specifically `.snapshot()` on a signal.)

**Typed input resets, or a control loses focus on every keystroke.** Something above it is a
`watch` that rebuilds the subtree. Replace it — reactive text with `text!`, a reactive
property by passing the signal, a dynamic set of views with `Lazy::for_each` / `List::for_each`.

**A list re-renders entirely when one row changes.** The collection is behind a `watch` over
a `Vec` instead of a `Lazy::for_each` / `List::for_each` over an `Identifiable` collection.
For a *derived* row set (filtering, sorting), wrap the signal in `SignalCollection` rather
than reaching for `watch`.

**Rows shuffle or lose state after an insert.** The `#[id]` field is not stable — it is an
array index, or regenerated per render. The id is the diffing key and must identify the same
logical row across updates.

**A handler receives the wrong binding.** Multiple parameters of the same state type bind
positionally: the first `.state()` call feeds the first parameter of that type — bare
`#[state]` types and `State<T>` share the sequence. Reorder, or switch to the
app-state-struct pattern where one injected struct replaces all of them.

**A row-level `.on_tap` never fires.** Only the topmost interactive view under the pointer
receives a press: a view with a handler or inherent interactivity (a button, a slider)
captures it, while layout containers and plain content let it through. A tappable child
therefore shadows its ancestor's `.on_tap`; put the handler on the view that should win, or
remove the nested one. Recognizers on the *same* view combine (see interaction.md).

**The app quits when its last window closes, or an on-demand window cannot reopen.** The
default `LastWindowPolicy::Quit` ends the app with its last window. A tray, resident or
on-demand-window app sets `App::on_last_window_closed(LastWindowPolicy::StayResident)`
(iOS and Android reject an app that declares no window) and opens windows with
`window.show(&env)` (`Window::show`): a `conditional_window` lives only as long as the
window hosting it.

**A snackbar shown from a callback does nothing.** `SnackbarManager` reaches handlers as a
parameter, but a callback that is not a WaterUI handler (a `GpuContentView::on_input`, a
closure handed to another component) has none. Read it while building the view with
`use_env(|manager: SnackbarManager| ..)` and move it into the callback.

**Rounded corners come out as a capsule.** `RoundedRectangle::new(r)` takes a fraction of
the shorter side, not points — `new(12.0)` saturates at fully-rounded. Use `new(0.1)`-scale
values, `Capsule` when fully-rounded is the intent, or `FixedRoundedRectangle::new(points)`
(and `FixedUnevenRoundedRectangle`) for an absolute radius. `UnevenRoundedRectangle`'s four
radii are in reading order (top_leading, top_trailing, bottom_leading, bottom_trailing),
not clockwise.

**Padding lands on the wrong edges.** `EdgeInsets::new(top, leading, bottom, trailing)` —
SwiftUI's order, not CSS — and `EdgeInsets::symmetric(vertical, horizontal)`.

**A background task never runs, or an animation loop dies instantly.** The
`spawn_local(..)` handle was dropped — it cancels the future on drop. Call `.detach()`.

**An on-demand window never opens (or opens duplicated).** `conditional_window` returns an
*invisible view that must still be placed in the tree*, and its `WindowPresentation` must
be created once alongside the state binding — recreating it inside a rebuilt subtree resets
the open-once guard.

**An animation snapshot shows the end state instead of the transition.** A test used
`std::thread::sleep`. The animation clock advances with pumped frames, not wall-clock time.
Use `app.pump_for(duration)`.

**A test's wait "passes" but checks nothing.** The query-chained
`.wait_for_existence(timeout)` returns a `bool` and does not assert — wrap it in
`assert!(..)`.

## Compile errors

| Error | Cause | Fix |
|---|---|---|
| `no function or associated item named 'new' found for struct 'Binding'` | `Binding::new` does not exist | `Binding::i32(v)` / `Binding::bool(v)` / `Binding::container(v)`; `Binding::default()` for `Binding<Option<T>>` |
| "type annotations needed for `Binding<_>`" after `binding(v)` | `binding` takes `impl Into<T>`, so `T` is unconstrained | typed constructor, `let` annotation, turbofish `binding::<T>(v)`, or let a control / typed helper parameter pin it |
| `cannot find function 'when' in this scope` | not in the prelude | `use waterui::widget::condition::when;` |
| `cannot find macro 's'` (or a lint suggesting `s!`) | `s!` is nami's macro, not in the prelude | `use waterui::reactive::s;` |
| ``expected trait, found derive macro `Identifiable` `` on a manual `impl` | the prelude carries the derive, not the trait | `use waterui::Identifiable;` |
| `E0525` / `E0507` pointing at `when(..)`, `.or(..)` or `.otherwise(..)` | payloads are `Fn` builders that run each time their branch becomes active, so the closure may not move a captured value out | clone captured values inside the body and build the view there: `when(c, move \|\| text("Saved").opacity(fade.clone()))` |
| `cannot find type 'ListDelete' / 'ListMove'` | not in the prelude | `use waterui::component::list::{ListDelete, ListMove};` |
| `cannot find type 'TapGesture' / 'CursorStyle' / 'Transferable'` | interaction types are not in the prelude | `use waterui::gesture::…;` / `use waterui::cursor::…;` / `use waterui::drag_drop::…;` |
| `cannot find type 'PhotoEvent'` | the real name is `photo::Event` | `use waterui::media::photo::Event as PhotoEvent;` |
| mismatched types on `LongPressGesture::new(Duration::…)` | it takes a `u32` in backend time units | `LongPressGesture::new(500)` |
| `no method named 'drop_hover'` | it exists only on the value `.drop_destination(..)` returns | chain it directly after `.drop_destination` |
| `the trait bound 'MyType: Transferable' is not satisfied` on `.draggable`/`.drop_destination` | a drag value must be marked transferable | `impl Transferable for MyType {}` (plus `impl_constant!(MyType)` to pass a plain value) |
| `no method named 'is_empty'` on a signal | signal string methods are prefixed | `.str_is_empty()`, `.str_len()`, `.str_contains(..)` |
| mismatched arms in `.select(TokenA, TokenB)` | both arms must be one concrete type | convert first: `let a: Color = Accent.into();` |
| `this function takes 2 arguments but 1 was supplied` on `.scale` | transforms are per-axis | `.scale(x, y)`, `.offset(x, y)`, `.size(w, h)` |
| `.size(24)` on a `Text` then `.size(w, h)` fails | `Text::size` (font size) shadows the frame modifier | use `.width(..)`/`.height(..)` on text, or size the container |
| `this function takes 2 arguments` on `row("Title")` | `row`/`detail_row` take (label, value) | `row("Streak", "14 days")` |
| wrong `row` / `grid` resolves | list `row` vs grid `row` name collision | `use waterui::layout::grid::{grid as layout_grid, row as grid_row};` |
| `` `Row` has no stable identity `` (an unsatisfied `Identifiable` bound) | collection item lacks an id | `#[derive(Clone, Identifiable)]` + `#[id]`, or manual `impl` for enums (disjoint id ranges) |
| ``cannot borrow `rows` as mutable`` on `push`/`remove`/`replace` | list mutators take `&mut self` | `let mut rows = …`, or `mut state: AppState` in a handler; a clone shares the same list |
| ``can't compare `Row` with `Row` `` on `replace` or `SignalCollection` | the diff compares surviving rows | `#[derive(Clone, PartialEq, Identifiable)]` |
| `expected 'Binding<T>', found '&Binding<T>'` (or the reverse) | two-way controls borrow, modifiers own | controls take `&binding`; modifiers take `binding.clone()` |
| `cannot find macro 'text'` | macro not imported | `use waterui::prelude::*;` then bare `text!` — never `waterui::text!` (same rule for `include_markdown!`); `shader!` is not in the prelude: `use waterui::graphics::shader;`, then bare `shader!` |
| `` `ForEach<..>` is not a WaterUI view `` (or "expected an `FnOnce()` closure") | `ForEach` is a `Views` collection, not a view | `Lazy::for_each(data, f)`, `VStack::for_each(data, f)`, or `List::for_each` |
| `.title("Inbox")` — "this method takes 0 arguments" | `Text::title()` (font size) shadows the navigation title | put the nav title on the container, or use `NavigationView::new(title, content)` |
| `borrowed data escapes outside of function` on a view helper | views are `'static` | take `&'static str` / `Str` / `impl IntoText`; a helper that only *reads* a `&T` during construction can return `impl View + use<>` |
| `no method named 'map'` on a signal (outside the prelude) | `SignalExt` not in scope | `use waterui::reactive::SignalExt;` |
| `use of undeclared crate 'tracing'` | logging goes through the re-export | `use waterui::log::debug;` then `debug!(..)` — no extra dependency |
| clippy `future_not_send` on an async helper | UI futures legitimately hold non-`Send` state | item-level `#[expect(clippy::future_not_send, reason = "…")]` |
| `could not find 'webview' / 'chart' / 'barcode' / 'particle' / 'canvas' in 'waterui'` | each is behind a cargo feature of the same name | enable it (`features = ["chart"]`) |
| `could not find 'map' in 'waterui'` | `map` is not a facade module or feature | add `waterui-map` to `Cargo.toml` and import `waterui_map::…` |
| `the trait bound 'Url: From<Str>' is not satisfied` | `Url` converts from `&'static str` only | `Url::parse(s)` (returns `Option`) — or `Url::parse_user_input(s)` for human-typed addresses |
| `Computed<Option<View>>` rejected where a view is expected | a *signal* of `Option<View>` is not a view | `when(..)`, or keep the view and use `.visible(signal)` |
| `text!` rejects an expression in a placeholder | placeholders are i18n slot keys | alias it: `text!("{slot}", slot = expr)` |
| mismatched types between `if` branches returning views | different concrete view types | wrap the `if`/`match` in `view! { .. }`, or put `#[view_builder]` on the function so tail-position branches may return different view types under one `impl View` (both turn the branches into `Fn` closures over a `watch`, so clone captured values inside them); or `when(..).otherwise(..)`, or `.anyview()` / `AnyView::new(..)` on each arm |

Do not add crate-, module-, or file-level `allow` attributes to silence a lint. If a lint is
genuinely a false positive, use a narrowly scoped item-level `expect` with a reason.

## Runtime panics

**"Environment state `T` not found at position N".** A handler asked for a `State<T>` or a
`#[state]`-marked `T` that nothing injected (the message without a position is the same
failure from an extraction outside a handler, such as `use_env`). Add `.state(&value)` on the button, or on an ancestor
container if several handlers need it, or `App::state(&value)` for an application-scoped
handler such as a menu-bar command.

**"Environment value `T` not found".** Same, but for `Use<T>` — the value must be installed
in the environment (typically in `app(env)`), not passed via `.state()`.

**"Local executor not set".** `spawn_local` ran before the backend installed the
thread-local executor — before it mounted the app's view, while `main` or a
`Store::new` was still running. Move the call into `.task(..)`, `.on_appear`, or
a handler body.

**A panic under `LabelDisplayMode::IconOnly`.** Some label in that subtree has no
`.icon(..)` (a `Window` toolbar installs `IconOnly` over its whole subtree). Scope the
install more narrowly or give every label an icon.

**`Image::new` panics at construction.** The pixel buffer length must be exactly
`width * height * 4` (RGBA8) and the image must have an area. This is fast-fail working as
designed — fix the buffer.

**A `Code` view panics with "Invalid language".** `code(language, content)` and `Code::new`
panic when the language does not convert, so a token from runtime data (a fence info
string, a server-supplied tag) crashes on the first unknown value. Parse it first with
`Language::try_from(tok)` (`waterui::text::highlight::Language`), fall back to
`Language::Plaintext`, and keep the original token with `.info(tok)`.

**`Id::try_from(0)` fails.** `Id` is non-zero; offset 0-based indices by `+ 1`.

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
a content-sized `zstack`. A window overlay layer must fill the window — use `absolute(..)`
and place children with `PositionExt` / `.pin(..)`.

**A rounded rectangle's corners look stretched.** Corner radius comes from the shape kind,
not from unit-space path commands, which scale with the aspect ratio. Test shape code
against a deliberately non-square rectangle.

**A map renders nothing on a self-drawn backend.** Nothing installs a map realization for
you off Apple: `app(env)` must both call `waterui_map_gpu::install(&mut env)` and put a
`MapGpuOptions` tile source into the environment. See [media.md](media.md).

## Build and run problems

**`water run` returned to the prompt.** The app crashed — a run that stays alive does not
exit. Re-run with `--logs debug` and read the tail.

**Android network calls fail with a DNS/resolution error.** `[permissions.internet]` is
missing from `Water.toml`; Android denies resolution outright rather than reporting a
permission failure. Debug builds declare it for the inspector endpoint anyway, so the
failure appears in a release or packaged build only.

**Every `water mcp` tool call fails with "the app binary did not answer `initialize`".** The
app panicked at startup (typically no usable GPU adapter) and the panic is on `water mcp`'s stderr —
see `references/mcp.md`, "Session and startup failures".

**A framework-revision or lock error from the CLI.** The project pins its framework
through `Water.toml` `[framework]` and `Water.lock`, both written by `water channel`; never
edit them by hand. Run the command the error names: `water channel` shows the selection,
`water channel dev|nightly|stable` selects or updates it, and `water channel dev --rev <sha>`
pins a dev commit.

**A backend conflicts with `Water.toml`.** `[platforms.<platform>] backend = "..."`
is the project's declared backend; a different CLI/MCP override is rejected before
building. Use the declared backend or change the declaration. Linux previews require
`hydrolysis`, not the default `gtk4`; see [backend selection](project.md#platforms-and-backends).

**`water preview` cannot load the app, or dev rebuilds are cold.** The crate is missing the
`dev = ["waterui/dynamic_linking"]` feature stanza generated projects carry — see
`references/project.md`.

**The CLI is out of date, or a change to it has no effect.** The `water` on `PATH` is a
previously installed binary. `water update` replaces it in place when it was installed by
the release installer, names the owning package manager's command for a Homebrew or cargo
install, and refuses when it cannot tell how the binary was installed;
`water update --check` reports the newest release without installing.

**Scrolling or interaction is janky in a dev build only.** Check that the build has a
release-ish profile; a full stack compiled at `-O0` with debug info is slow in a way that
looks like a rendering bug.

**A manifest with `default-features = false` compiles in the workspace but not standalone.**
Sibling crates' feature unification was filling the gap. List every needed feature
explicitly.

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
