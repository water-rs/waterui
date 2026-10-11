//! Material 3 interaction-state showcase: selected, hovered and pressed list
//! rows, a FAB, and a keyboard-focusable button. Runs a real winit window for
//! screenshot capture on the X display. Not committed.
//!
//! Supported targets: those with `hydrolysis::run`, which excludes Android
//! (the Kotlin host owns the Activity there) and wasm32 without `web`.

#[cfg(hydrolysis_run)]
mod windowed;

#[cfg(hydrolysis_run)]
fn main() {
    windowed::main();
}

#[cfg(not(hydrolysis_run))]
fn main() {
    panic!(
        "the `m3_gallery_shots` example needs `hydrolysis::run`, which this target does not have"
    );
}
