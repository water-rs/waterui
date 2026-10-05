//! Wayland smoke run: opens a window under a compositor and stays alive long
//! enough to eyeball that the first frame presents.
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
    panic!("the `wayland_smoke` example needs `hydrolysis::run`, which this target does not have");
}
