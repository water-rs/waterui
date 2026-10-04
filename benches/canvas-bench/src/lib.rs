//! canvas-bench — the on-device measurement app for
//! water-rs/waterui#1564 ("Measure Canvas realizations and GpuContent
//! density on Apple").
//!
//! One launch walks the whole cell matrix itself: the harness drives a
//! `Binding<Option<CellSpec>>` through a `watch` on the root view, so
//! every cell mounts, warms up, measures and tears down inside the same
//! process. The single `Documents/bench-<run_id>.json` result is pulled
//! afterwards with `devicectl`.

#![cfg(target_os = "ios")]

mod harness;
mod scenes;

use std::cell::RefCell;
use std::rc::Rc;

use waterui::app::App;
use waterui::prelude::*;

pub fn app(env: Environment) -> App {
    let config = harness::MatrixConfig::from_env();
    let cell_binding = Binding::container(None::<harness::CellSpec>);
    let handles_slot = Rc::new(RefCell::new(None::<harness::SceneHandles>));
    let mtm = cocoa_ui::MainThreadMarker::new().expect("app() runs on the main thread");
    let _harness = Box::leak(Box::new(harness::install(
        &config,
        mtm,
        cell_binding.clone(),
        handles_slot.clone(),
    )));
    let slot = handles_slot.clone();
    App::new(
        move || {
            let slot = slot.clone();
            watch(cell_binding.clone(), move |cell: Option<harness::CellSpec>| {
                cell.map(|spec| scenes::cell_view(&spec, &slot))
            })
        },
        env,
    )
}
