# waterui-testing

Accessibility-first, headless UI testing for [WaterUI](https://github.com/lexoliu/waterui).
Tests run under plain `cargo test` (use `cargo nextest run` in this workspace) against the
Hydrolysis renderer — no simulator, no device, no custom runner.

## Quick start

```toml
[dev-dependencies]
waterui-testing = "0.3"
hydrolysis-m3 = "0.2"
```

```rust
use waterui_testing::{Role, SemanticApp, UiBuilder};

// Mounting form: the macro mounts a no-arg view function.
#[waterui::test(login_view, theme = hydrolysis_m3::install, viewport = (360, 320))]
fn login_flow(app: &mut SemanticApp) {
    app.query().role(Role::BUTTON).label("Login").tap();
    app.query().label("Welcome").assert_exists();
}

// Manual-mount form: the test owns Bindings the view closes over.
#[waterui::test(theme = hydrolysis_m3::install)]
fn stepper_updates(ui: UiBuilder) {
    let value = Binding::i32(2);
    let value_for_view = value.clone();
    let mut app = ui.mount(move || stepper("Limited", &value_for_view));
    app.query().label("Limited").increment();
    assert_eq!(value.get(), 3);
}
```

## Design

- **Theme and render mode are orthogonal.** `.theme(installer)` swaps the theme package
  (plain Hydrolysis test theme by default); `mount()` is the fast semantic runtime,
  `mount_offscreen()` the GPU-backed one. Any theme works in either mode.
- **Interactions are assertions.** `tap`, `set_text`, `increment`, `focus`, drags and key
  presses return `()` and panic when the runtime reports the accessibility action
  unhandled. Tests for disabled/clamped controls assert the panic (`catch_unwind`).
- **Quiescence, not sleeps.** After every interaction the session pumps until the runtime
  is genuinely idle — no queued input, no spawned tasks, no scheduled animations or
  patches. Waits (`wait_for_existence`, `wait_for`) pump hot while work is scheduled and
  touch wall-clock time only for work outside the runtime.
- **Virtual frame clock.** Every pump advances the animation clock exactly one frame, so
  transition sampling is deterministic under any host load. `OffscreenApp::pump_for`
  lands on an exact phase; snapshots capture it via
  `capture_snapshot(suite, case, stage)` (PNGs under `WATERUI_TEST_ARTIFACTS_DIR`).
- **Semantic queries.** `app.query().role(Role::SWITCH).label("Wi-Fi")` with
  `.assert_exists()` / `.single()` / `.all()`, XCTest-style expectations
  (`Expectation`, `WaitOptions`, inverted and ordered waits), and scoping through element
  handles (`Query::within`). Views tagged `.a11y_id("settings.wifi")` resolve via
  `.identifier("settings.wifi")` — the same identifier surfaces to XCUITest and Android
  automation in the native backends.
- **Gesture and keyboard control.** `DragOptions` paces drags (`frame_per_step` gives
  recognizers a real motion timeline); `press_named_key_with` / `press_character_key_with`
  hold explicit `Modifiers`.
- **Performance harness.** `ui().perf(view)` / `perf_with` measure steady-state offscreen
  frames (`PerfConfig`, `PerfReport`) with per-phase Hydrolysis timings and process
  resource samples.

Because the tree under test is the [Hydrolysis accessibility tree](../backends/hydrolysis),
every test doubles as an accessibility-correctness test: a component that cannot be
driven through this crate is a component assistive technology cannot drive either.
