//! Parallel child measurement honoring [`SubView::require_main_thread`].
//!
//! Layout containers measure their children to negotiate sizes. With the
//! `parallel` feature enabled, children whose measurement is thread-safe
//! ([`SubView::require_main_thread`] returns `false` — text, media, shapes, fixed
//! sizes) are measured on a worker pool, while main-thread-bound children are
//! measured on the calling thread. Without the feature (e.g. `no_std`/embedded),
//! measurement is serial. Result order always matches `children`.

use alloc::vec::Vec;
use waterui_core::layout::SubView;

/// Measure every child by applying `measure` to it, parallelizing the children
/// that do not require the main thread.
///
/// `measure` runs on a worker thread for `require_main_thread() == false` children
/// (under the `parallel` feature) and on the calling thread for the rest; the
/// returned vector is in the same order as `children`.
#[must_use]
///
/// # Panics
///
/// Panics if the internal measurement bookkeeping leaves a child unmeasured.
pub fn measure_children<T, F>(children: &[&dyn SubView], measure: F) -> Vec<T>
where
    F: Fn(&dyn SubView) -> T + Send + Sync,
    T: Send,
{
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;

        let mut results: Vec<Option<T>> = (0..children.len()).map(|_| None).collect();

        // Main-thread-bound children are measured here, on the calling thread.
        for (index, child) in children.iter().enumerate() {
            if child.require_main_thread() {
                results[index] = Some(measure(*child));
            }
        }

        // The remaining children are pure measures; run them across the worker pool.
        let parallel: Vec<(usize, T)> = children
            .par_iter()
            .enumerate()
            .filter(|(_, child)| !child.require_main_thread())
            .map(|(index, child)| (index, measure(*child)))
            .collect();
        for (index, value) in parallel {
            results[index] = Some(value);
        }

        results
            .into_iter()
            .map(|value| value.expect("every child is measured exactly once"))
            .collect()
    }

    #[cfg(not(feature = "parallel"))]
    {
        children.iter().map(|child| measure(*child)).collect()
    }
}
