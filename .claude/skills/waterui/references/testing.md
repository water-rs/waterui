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

Any no-argument function returning `impl View` can be rendered to a PNG.

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
water preview --expr 'vstack((text("Hi").title(), button("Go").action(|| {})))'
```

`--expr` compiles the expression into generated preview code with `waterui::prelude::*`
in scope, so it is a real compile, not a string interpreter. The preview pipeline links
the app as a dylib, which is why generated projects carry the
`dev = ["waterui/dynamic_linking"]` feature (see `references/project.md`).

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
hydrolysis-m3 = "…"
```

**Mounting form** — the macro mounts a no-argument view function and hands you the session:

```rust
use waterui_testing::{Role, SemanticApp};

#[waterui::test(login_view, theme = hydrolysis_m3::install, viewport = (360, 320))]
fn login_flow(app: &mut SemanticApp) {
    app.query().role(Role::BUTTON).label("Login").tap();
    app.query().label("Welcome").assert_exists();
}
```

**Manual-mount form** — no view path, so the test owns the bindings the view closes over:

```rust
use waterui_testing::UiBuilder;

#[waterui::test(theme = hydrolysis_m3::install)]
fn stepper_updates(ui: UiBuilder) {
    let value = Binding::i32(2);
    let for_view = value.clone();
    let mut app = ui.mount(move || stepper("Limited", &for_view));

    app.query().label("Limited").increment();
    assert_eq!(value.get(), 3);
}
```

Attribute arguments: `theme = <installer>`, `viewport = (w, h)`, and the bare `offscreen`
flag (which switches the parameter to `&mut OffscreenApp`). The macro expands to a plain
`#[test]`, so do not also write `#[test]`, and the function must take exactly one
parameter and return `()`.

## Querying the accessibility tree

```rust
app.query()
    .role(Role::SWITCH)
    .label("Wi-Fi")            // or .label_contains("Wi-")
    .identifier("settings.wifi")
    .within(&handle)           // or .children_of(&handle)
    .enabled(true) .selected(true) .checked(true) .expanded(true) .busy(false) .hidden(false)
    .value("42")               // or .value_contains("4")
```

Terminate the query:

```rust
.assert_exists()  .assert_not_exists()  .assert_ui_focus()
.exists() -> bool     .single() -> ElementRef     .all() -> ElementSet
.optional() -> Option<ElementRef>
.wait_for_existence(timeout) -> bool      // returns, does NOT assert — wrap in assert!
.wait_for_nonexistence(timeout) -> bool
```

The two `wait_for_*` terminators return a `bool` rather than panicking; called bare they
are a wait that cannot fail. Always `assert!(query.wait_for_existence(..), "…")`.

Tag views that resist a natural label with `.a11y_id("settings.wifi")` and query
`.identifier(..)`. The same identifier reaches XCUITest (`accessibilityIdentifier`) and
Android automation, so it is not test-only scaffolding. A control whose label is hidden
visually (`.hide_label()`) still queries by its label text.

## Interacting

```rust
.tap()   .tap_at(nx, ny)   .focus()   .hover()   .hover_at(nx, ny)
.set_text("hello")   .increment()   .decrement()   .scroll_down()
.drag_by(dx, dy)   .drag_by_with(dx, dy, DragOptions { steps: 12, frame_per_step: true })
.drag_between(fx, fy, tx, ty)   .magnify(1.5)
```

**Interactions return `()` and panic when the runtime rejects the action — the call itself
is the assertion.** A `tap()` on a disabled or missing element fails the test on the spot,
so there is nothing to check afterwards. After each interaction the session settles to
quiescence automatically.

Session-level input, for cases with no element to address:

```rust
app.tap_at(x, y);   app.scroll_at(x, y, dx, dy, is_line_delta);
app.text_input("hello");
app.press_named_key("Tab");   app.press_named_key_with("Tab", modifiers);
app.press_character_key_with("a", modifiers);
```

## Waiting

Wait on the condition, never on the clock. A `Selector` is built the same way a query is:

```rust
let selector = Selector::default().label("Done");

app.wait_for_existence(&selector, Duration::from_secs(2));
app.wait_for_nonexistence(&selector, timeout);
app.wait_for_value_eq(&selector, "Done", timeout);
app.wait_for(&[app.expect_value_eq(selector, "Done")], WaitOptions::new(timeout));
```

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

Add the `offscreen` flag (the parameter becomes `&mut OffscreenApp`) or call
`ui.mount_offscreen(..)`:

```rust
#[waterui::test(demo, theme = hydrolysis_m3::install, offscreen, viewport = (390, 844))]
fn renders(app: &mut OffscreenApp) {
    app.pump_for(Duration::from_millis(120));       // advance the virtual clock exactly
    app.capture_snapshot("gallery", "cards", "settled");
}
```

`capture_snapshot(suite, case, stage)` files the image under
`WATERUI_TEST_ARTIFACTS_DIR`. When the test needs the pixels in hand or a specific output
path — an image a human or another tool will look at — take the raw snapshot instead:

```rust
let shot = app.snapshot();                       // pumps a frame; Snapshot { rgba8, width, height }
shot.save_png("/tmp/my_view.png").expect("snapshot must be writable");
```

"Visual test" means *looking at the image*. Pixel-count heuristics, opaque-pixel
thresholds, bbox approximations, dominant-color checks, and similar proxies do not verify
appearance and should not be written.

## `#[waterui::bench]` and `water bench`

Frame benchmarks live next to the tests and use the same dev-dependencies. Each mounts
the view in the offscreen GPU runtime and records whole-frame timings plus renderer
counters.

```rust
use waterui_testing::PerfApp;

#[waterui::bench(dashboard, theme = hydrolysis_m3::install, viewport = (390, 844), max_p95_us = 8_000)]
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
water bench --samples 240 --warmups 20
water bench --format html -o report.html
water bench --format json               # machine-readable
water bench --gha bench.json            # github-action-benchmark JSON
water bench --max-p95-us 8000           # tighten from the CLI (tighter of the two wins)
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
