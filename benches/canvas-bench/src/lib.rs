//! canvas-bench — the on-device measurement app for
//! water-rs/waterui#1564 ("Measure Canvas realizations and GpuContent
//! density on Apple").
//!
//! One launch is one run: `WATERUI_BENCH_*` environment selects the
//! scenario, variant, size and `run_id`; the harness records into
//! `Documents/bench-<run_id>.json`, pulled afterwards with `devicectl`.

#![cfg(target_os = "ios")]

mod harness;
mod scenes;

use waterui::app::App;
use waterui::prelude::*;

pub fn app(env: Environment) -> App {
    let mut config = harness::BenchConfig::from_env().unwrap_or(harness::BenchConfig {
        scenario: 1,
        variant: "a".into(),
        n: 50,
        run_id: "manual".into(),
        commit: "unknown".into(),
        warmup_s: 5.0,
        steady_s: 30.0,
        thermal_wait_s: 300.0,
        animate_s: 20.0,
        scroll: true,
        scroll_speed: 240.0,
        scroll_extent: 0.0,
    });
    // Scenario 1 rows are 60pt tall (52pt shape + 8pt row padding); the
    // scroll extent is the content minus roughly one ~800pt viewport.
    if config.scroll_extent <= 0.0 && config.scenario == 1 {
        config.scroll_extent = (f64::from(config.n) * 60.0 - 800.0).max(0.0);
    }
    let handles = scenes::handles(&config);
    let mtm = cocoa_ui::MainThreadMarker::new().expect("app() runs on the main thread");
    let _harness = Box::leak(Box::new(harness::install(&config, mtm, {
        let animate = handles.animate.clone();
        let scroll = handles.scroll.clone();
        harness::SceneHandles { scroll, animate }
    })));
    App::new(move || scenes::view(&config, &handles), env)
}