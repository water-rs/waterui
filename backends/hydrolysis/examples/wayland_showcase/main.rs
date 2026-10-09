//! Wayland showcase: a window exercising the runner's widgets and input paths
//! under a real compositor session.
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
        "the `wayland_showcase` example needs `hydrolysis::run`, which this target does not have"
    );
}
