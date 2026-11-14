# Previews, tests, and benchmarks

## Contents

- Which tool for which question
- `#[preview]`
- `#[waterui::test]`
- Querying the accessibility tree
- Interacting
- Waiting
- Visual tests and snapshots
- `#[waterui::bench]` and `water bench`
- Running tests

## Which tool for which question

| Question | Tool |
|---|---|
| Does this view look right? | `#[preview]` + `water preview` → PNG |
| Does this view *behave* right? | `#[waterui::test]` |
| Is it accessible? | `#[waterui::test]` — same thing |
| Is it fast enough? | `#[waterui::bench]` + `water bench` |

All three run headless. None needs a simulator or device, so there is no reason to reason
about whether a view works instead of checking.

## `#[preview]`

Any free function returning `impl View` can be rendered to a PNG. A parameter needs a
default in the attribute, `#[preview(count = 5)]`. A preview name is unique within its
crate, because the symbol is `waterui_preview_<crate>_<fn>` and a macro cannot see the
module path.

```rust
use waterui::preview;

#[preview]
pub fn demo() -> impl View {
    content(DemoState::new())
}
```

```bash
water preview demo --output preview.png
water preview demo --backend hydrolysis --theme material3 --frame 390x844
water preview demo --platform ios
water preview demo --platform android --device <serial-or-avd>
water preview --expr 'vstack((text("Hi").title(), button("Go").action(|| {})))'
water preview demo --scenario steps.toml --output-dir frames/   # Hydrolysis interaction/timeline capture
water preview test demo --code 'app.query().label("Go").tap();'  # semantic assertions on a preview
water preview test --all                                         # every #[preview] in the crate
```

`--content-timeout <seconds>` is how long the capture waits for content its views still
owe (a decoding image, a `Suspense` future) before failing with their names. `--offline`
fails on a declared font missing from the cache instead of fetching it. `preview test`
takes `--code` / `--code-file` (a Rust body receiving `app: &mut SemanticApp`),
`--platform`, `--theme`, `--frame` and `--path`; its only theme is `material3`.
`--platform ios` renders the UIKit realization on the Mac itself through Mac Catalyst,
under the iPad idiom, so it shows the iOS look without booting a simulator. It builds for
the `aarch64-apple-ios-macabi` Rust target, which `water doctor --fix` installs.
The capture draws without a window, so it does not evaluate backdrop filters: a material
background shows its tint but not the blur of what lies behind it. Check materials on a
device or simulator with `water run --platform ios`.

CLI and MCP previews honor `[platforms.<platform>] backend = "..."` in `Water.toml`.
An explicit backend that contradicts the declaration fails before building.
Linux previews need a `hydrolysis` declaration or, when undeclared, `--backend hydrolysis`;
the platform default `gtk4` has no preview renderer. See
[platforms and backends](project.md#platforms-and-backends).

`--expr` compiles the expression into generated preview code with `waterui::prelude::*`
in scope, so it is a real compile, not a string interpreter. The preview pipeline links
the generated preview binary against the shared WaterUI runtime, which is
why generated projects carry the `dev = ["waterui/dynamic_linking"]` feature
(see `references/project.md`).

Keeping the preview function self-contained — owning its own bindings — is what lets the
same function be embedded in a gallery, previewed, and used by tests. A screen with a
hardware-backed leaf (camera, GPU capture) stays previewable by parameterizing that leaf:
the shared body takes `preview: impl View` plus the state, the live constructor passes
the real surface, and the `#[preview]` function passes a synthetic stand-in.

## `#[waterui::test]`

Tests drive the real accessibility tree. That makes each test simultaneously an
interaction test and an accessibility assertion, which is why a component that cannot be
tested this way is a defect rather than a coverage gap.

Add the dev-dependencies:

```toml
[dev-dependencies]
waterui-testing = "…"
hydrolysis-m3 = "…"     # styled mounts only — a style-free semantic test needs none
```

The default mount is the *semantic* runtime: a GPU-free pipeline whose accessibility
tree is a product of the view tree and the widgets' semantics. It carries no style
package and answers no geometry — `bounds()`, pointer gestures and snapshots exist
only on the rendered form below.

**Mounting form** — the macro mounts a no-argument view function and hands you the session:

```rust
use waterui_testing::{Role, SemanticApp};

#[waterui::test(login_view, viewport = (360, 320))]
fn login_flow(app: &mut SemanticApp) {
    app.query().role(Role::BUTTON).label("Login").tap();
    app.query().label("Welcome").assert_exists();
}
```

**Manual-mount form** — no view path, so the test owns the bindings the view closes over:

```rust
use waterui_testing::UiBuilder;

#[waterui::test]
fn stepper_updates(ui: UiBuilder) {
    let value = Binding::i32(2);
    let for_view = value.clone();
    let mut app = ui.mount(move || stepper("Limited", &for_view));

    app.query().label("Limited").increment();
    assert_eq!(value.snapshot(), 3);
}
```

Attribute arguments: `theme = <style>` carries a `hydrolysis::Style` value (for
example `hydrolysis_m3::Material3::defaults()`), `viewport = (w, h)` sizes the window,
and the bare `offscreen` flag mounts the rendered runtime and switches the parameter
to `&mut OffscreenApp`. `offscreen` and `#[waterui::bench]` require `theme =` — the
rendered runtime is styled by construction; without `theme =` the test is semantic.
`offscreen` also needs the view path: the manual-mount form calls `mount_offscreen`
itself.
`ui().theme(style).mount(..)` is still semantic; it adds the style's tokens for
components whose bodies read them.
The macro expands to a plain `#[test]`, so do not also write `#[test]`, and the
function must take exactly one parameter and return `()`. `waterui::test` is an alias of
the macro `ui_test`, which is the name `waterui::prelude` exports, so a glob import of the
prelude leaves the built-in `#[test]` unshadowed; write `#[waterui::test]` qualified.
A styled manual mount names the style in its parameter type:
`ui: UiBuilder<Styled<hydrolysis_m3::Material3>>`.

Use the mounting form whenever the test needs only a view function. Take the
manual-mount form when the test must own the bindings the view closes over. Check
layout on a rendered session, through the elements' `bounds()` and how they relate
to each other. Never attach accessibility metadata to a decorative fill only so that a
test can find it.

## Querying the accessibility tree

```rust
app.query()
    .role(Role::SWITCH)
    .label("Wi-Fi")            // or .label_contains("Wi-")
    .identifier("settings.wifi")
    .within(&handle)           // or .children_of(&handle)
    .enabled(true) .selected(true) .checked(true) .mixed() .expanded(true) .busy(false) .hidden(false)
    .value("42")               // or .value_contains("4")
    .placeholder("Search")     // or .placeholder_contains("Sea")
    .content_type(ContentType::EmailAddress)
```

`ContentType` is re-exported by `waterui_testing`. `Selector` carries the same filters,
for the session-level methods below.

Terminate the query:

```rust
.assert_exists()  .assert_not_exists()  .assert_ui_focus()
.exists() -> bool     .single() -> ElementRef     .all() -> ElementSet
.optional() -> Option<ElementRef>
.wait_for_existence(timeout) -> bool      // returns, does NOT assert — wrap in assert!
.wait_for_nonexistence(timeout) -> bool
.wait_for_value_eq(value, timeout) -> bool
```

The `wait_for_*` terminators return a `bool` rather than panicking; called bare they
are a wait that cannot fail. Always `assert!(query.wait_for_existence(..), "…")`.

Tag views that resist a natural label with `.a11y_id("settings.wifi")` and query
`.identifier(..)`. The same identifier reaches XCUITest (`accessibilityIdentifier`) and
Android automation, so it is not test-only scaffolding. A control whose label is hidden
visually (`.hide_label()`) still queries by its label text.

## Interacting

Semantic actions exist on every session — they dispatch accessibility actions, not
pointer events:

```rust
.tap()   .focus()   .set_text("hello")   .increment()   .decrement()
.scroll_down()   .expand()   .collapse()
```

Pointer gestures resolve coordinates against the element's rendered bounds, so they
exist only on a rendered (`offscreen`) session's elements:

```rust
.tap_at(nx, ny)   .hover()   .hover_at(nx, ny)
.drag_by(dx, dy)   .drag_by_with(dx, dy, DragOptions::default().with_steps(12).with_frame_per_step(true))
.drag_between(fx, fy, tx, ty)   .magnify(1.5)
```

**Interactions return `()` and panic when the runtime rejects the action — the call itself
is the assertion.** A `tap()` on a disabled or missing element fails the test on the spot,
so there is nothing to check afterwards. After each interaction the session settles to
quiescence automatically.

Session-level input, for cases with no element to address — keyboard and text input are
semantic:

```rust
app.text_input("hello");
app.press_named_key("Tab");   app.press_named_key_with("Tab", modifiers);
app.press_character_key_with("a", modifiers);
```

A `press_*` call is a full key stroke — press, then release — so one call means the same
thing on every runtime: the semantic pipeline activates a focused control on the press and
the rendered pipeline on the release. To hold a key across calls, use `key_down` / `key_up`,
which take a `KeyCode` so named and character keys are both expressible:

```rust
let key = KeyCode::Named("Shift".to_string());
app.key_down(key.clone(), modifiers);   app.key_up(key, modifiers);
```

Pointer input exists only on a rendered session:

```rust
app.tap_at(x, y);   app.scroll_at(x, y, dx, dy, is_line_delta);   app.hover_at(x, y);
app.secondary_click_at(x, y);   app.pointer_down_at(x, y);   app.pointer_up_at(x, y);
app.magnify_at(x, y, 1.5);
app.hover_files_at(x, y, [path]);   app.drop_files_at(x, y, [path]);   app.cancel_files_hover();
```

`secondary_click_at` opens context menus; the file calls drive drop targets. Every call
above except `tap_at` has a `queue_*` twin that queues the event without settling, for a
test that pumps and captures the frames between events: `queue_scroll_at`,
`queue_hover_at`, `queue_pointer_down_at`, `queue_pointer_up_at`, `queue_magnify_at`,
`queue_hover_files_at`, `queue_drop_files_at`, `queue_cancel_files_hover`, and
`queue_secondary_click` for `secondary_click_at`.

## Waiting

Wait on the condition, never on the clock. A `Selector` is built the same way a query is:

```rust
let selector = Selector::default().label("Done");

app.wait_for_existence(&selector, Duration::from_secs(2));
app.wait_for_nonexistence(&selector, timeout);
app.wait_for_value_eq(&selector, "Done", timeout);
app.wait_for(&[app.expect_value_eq(selector, "Done")], WaitOptions::new(timeout));
```

The first three return `bool`; `wait_for` returns a `WaitResult`. `wait_for_ui_focus`
waits for keyboard focus, and `app.assert_value_eq(&selector, "Done")` asserts at once.

The frame pumps — `app.pump_for(duration)` and `app.pump_until(timeout, || cond())` —
exist on **`OffscreenApp` only**, not on `SemanticApp`; a `SemanticApp` test waits on
conditions with the methods above.

**`std::thread::sleep` does not work here and must never appear in a test.** The animation
clock advances when frames are pumped, not when wall-clock time passes, so a bare sleep
freezes it: every deferred step then lands at once on the next snapshot, and a capture
meant to show a transition mid-flight silently shows its end state. The test still passes
and the image still looks plausible, which is what makes it dangerous. To sample a phase,
pump (offscreen tests): `app.pump_for(Duration::from_millis(120))`.

## Visual tests and snapshots

Name a style with `theme =` and add the `offscreen` flag (the parameter becomes `&mut
OffscreenApp`) — or call `ui.theme(..).mount_offscreen(..)` in the manual-mount form:

```rust
#[waterui::test(demo, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (390, 844))]
fn renders(app: &mut OffscreenApp) {
    app.pump_for(Duration::from_millis(120));       // advance the virtual clock exactly
    app.capture_snapshot("gallery", "cards", "settled");
}
```

`capture_snapshot(suite, case, stage)` files the image under
`WATERUI_TEST_ARTIFACTS_DIR`. When the test needs the pixels in hand or a specific output
path — an image a human or another tool will look at — take the raw snapshot instead:

```rust
let shot = app.snapshot();                       // pumps a frame; shot.width(), shot.height(), shot.rgba8()
shot.save_png("/tmp/my_view.png").expect("snapshot must be writable");
```

`WATERUI_TEST_ARTIFACTS_DIR` defaults to a `waterui-testing-artifacts` directory under
the system temp dir. `ui().theme(style).scale_factor(2.0)` captures at 2 physical pixels
per logical pixel: a `200x100` viewport then yields a `400x200` snapshot.

"Visual test" means *looking at the image*. Pixel-count heuristics, opaque-pixel
thresholds, bbox approximations, dominant-color checks, and similar proxies do not verify
appearance and should not be written.

A whole `App` mounts through `waterui_testing::mount_app(app, style)`, which sizes the
session from the window's declared frame; a host that owns the viewport and DPI — `water
mcp` is one — uses the builder form `ui().theme(style).viewport(w,
h).scale_factor(dpi).mount_app(app)` instead. Either mounts only the app's first window
and panics when the app declares none: the app's menu bar and further windows are not
mounted, though popups the app opens at runtime (context menus, pickers) join the tree.
`ui().environment(env)` supplies the environment a manual mount's view resolves.

## `#[waterui::bench]` and `water bench`

Frame benchmarks live next to the tests and use the same dev-dependencies. Each mounts
the view in the offscreen GPU runtime — `theme = <style>` is required — and records
whole-frame timings plus renderer counters.

```rust
use waterui_testing::PerfApp;

#[waterui::bench(dashboard, theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844), max_p95_us = 8_000)]
fn dashboard_redraw(perf: &mut PerfApp) {
    perf.measure("steady-redraw", |run| run.redraw());
    perf.measure("wheel-scroll", |run| {
        run.scroll_at(195.0, 600.0, 0.0, -24.0, false);
    });
}
```

The closure runs **before every frame**: queue input (`run.pointer_move/down/up`,
`run.scroll_at`) or call `run.redraw()`, and the frame that follows is what gets timed.
`run.app()` exposes the full query and interaction API mid-run.

A bench that owns the bindings its view closes over omits the view path, takes the
builder by value, and returns the report:

```rust
use waterui_testing::{PerfReport, Styled, UiBuilder};

#[waterui::bench(theme = hydrolysis_m3::Material3::defaults(), max_rebuild_ratio = 0.2)]
fn counter_updates(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) -> PerfReport {
    let value = Binding::i32(0);
    let for_view = value.clone();
    ui.perf_with(move || stepper("Count", &for_view), |perf| {
        perf.measure("increment", |run| {
            run.app().query().label("Count").increment();
        });
    })
}
```

Budgets are attribute arguments and apply to every measurement in the bench:
`max_p95_us`, `max_mean_us`, `max_rebuild_ratio` (0.0–1.0), `max_scene_layers`,
`max_gpu_surface_layers`, `max_clip_layers`.

`max_rebuild_ratio` is the most useful one for catching regressions of the kind this
framework cares about — it fails the bench when interaction starts causing structural
rebuilds instead of precise updates.

Under plain `cargo nextest run` a bench runs in smoke mode (2 frames, no budgets) purely
as a correctness test. `water bench` runs the real measurement and enforces budgets.

```bash
water bench                             # every bench in the current crate
water bench scroll                      # only benches whose name contains "scroll"
water bench --path examples/stress
water bench --samples 240 --warmups 20 --repetitions 7   # defaults: 120 / 10 / 7
water bench --format html -o report.html
water bench --format json               # machine-readable
water bench --gha bench.json            # github-action-benchmark JSON
water bench --max-p95-us 8000           # tighten from the CLI (tighter of the two wins);
                                        # also --max-mean-us, --max-rebuild-ratio, --max-scene-layers,
                                        # --max-gpu-surface-layers, --max-clip-layers
```

## Running tests

```bash
cargo nextest run -p my-app
cargo nextest run -p my-app -E 'test(login_flow)'
cargo nextest run -p my-app -E 'test(login_flow)' --no-capture   # show output; serial
cargo test --doc                                                 # doctests only
```

`cargo nextest run` is the runner to use — it runs each test in its own process, which is
usually what you want but does mean tests cannot share process-global state. A test that
depends on a sibling's initialization is relying on an accident; fix the shared-state
assumption rather than falling back to `cargo test`. Doctests are the one thing nextest
cannot run at all.
