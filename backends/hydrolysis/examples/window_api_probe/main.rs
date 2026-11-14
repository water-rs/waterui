//! Probe for the runtime window API: asserts the window handle, scale factor,
//! and title plumbing through the platform runner.
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
        "the `window_api_probe` example needs `hydrolysis::run`, which this target does not have"
    );
}
