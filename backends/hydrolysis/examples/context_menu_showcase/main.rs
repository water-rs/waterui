//! Live capture harness for water-rs/hydrolysis#200: a context menu with a
//! five-button accessory strip above the lifted source view, a subtitled
//! command and a destructive last item. Run under Xvfb and press the
//! secondary button on the card to open the drawn presentation.
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
        "the `context_menu_showcase` example needs `hydrolysis::run`, which this target does not have"
    );
}
