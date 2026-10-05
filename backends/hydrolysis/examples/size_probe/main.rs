//! Interactive probe: reports the window's live size as it changes, for
//! checking the platform runner's resize plumbing by hand.
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
    panic!("the `size_probe` example needs `hydrolysis::run`, which this target does not have");
}
