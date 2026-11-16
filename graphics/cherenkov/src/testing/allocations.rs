//! Counting the heap allocations one thread makes, for tests that pin an
//! allocation budget on a code path.
//!
//! A test binary installs [`ThreadAllocator`] as its global allocator:
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOCATOR: cherenkov::testing::allocations::ThreadAllocator =
//!     cherenkov::testing::allocations::ThreadAllocator;
//! ```
//!
//! and measures a closure with [`count`]. Only the calling thread's
//! allocations are counted, so work another thread does at the same time —
//! a render thread, the test harness — never reaches the result.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg_attr(
    target_os = "android",
    expect(
        clippy::missing_const_for_thread_local,
        reason = "every initializer is already `const {}`; the lint fires on \
                  Android because that target's `thread_local!` expansion routes \
                  const initializers through a generated non-const `__init` fn \
                  (rust-lang/rust-clippy#13422)"
    )
)]
mod counters {
    use std::cell::Cell;

    thread_local! {
        pub(super) static TRACKING: Cell<bool> = const { Cell::new(false) };
        pub(super) static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
        pub(super) static REALLOCATIONS: Cell<usize> = const { Cell::new(0) };
        pub(super) static FREES: Cell<usize> = const { Cell::new(0) };
    }
}

use counters::{ALLOCATIONS, FREES, REALLOCATIONS, TRACKING};

/// Set by the first allocation through [`ThreadAllocator`]: a binary that
/// never installed it would count nothing, and every budget would pass.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// The heap operations one thread made while [`count`] measured it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AllocationCounts {
    /// Fresh allocations, zeroed ones included.
    pub allocations: usize,
    /// Reallocations of an existing block.
    pub reallocations: usize,
    /// Blocks released.
    pub frees: usize,
}

/// A global allocator that forwards to [`System`] and counts what the
/// measuring thread does.
#[derive(Debug)]
pub struct ThreadAllocator;

/// Adds one to `counter` while the calling thread measures. `try_with`
/// keeps an allocation made during thread-local teardown from panicking
/// inside the allocator.
fn tally(counter: &'static std::thread::LocalKey<Cell<usize>>) {
    if TRACKING.try_with(Cell::get).unwrap_or(false) {
        let _ = counter.try_with(|count| count.set(count.get() + 1));
    }
}

// SAFETY: every method delegates to `System` under identical contracts —
// `layout`, `pointer` and `size` pass through unchanged — and the accounting
// only touches an atomic flag and thread-local counters, never allocating.
unsafe impl GlobalAlloc for ThreadAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        INSTALLED.store(true, Ordering::Relaxed);
        tally(&ALLOCATIONS);
        // SAFETY: `layout` is the caller's allocation request, forwarded.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        INSTALLED.store(true, Ordering::Relaxed);
        tally(&ALLOCATIONS);
        // SAFETY: `layout` is the caller's allocation request, forwarded.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        tally(&FREES);
        // SAFETY: `pointer` and `layout` came from `System.alloc` — the
        // allocator contract.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        tally(&REALLOCATIONS);
        // SAFETY: `pointer`, `layout` and `size` are the caller's request,
        // forwarded — the allocator contract.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

/// Runs `measured` and returns what it produced with the heap operations the
/// calling thread made meanwhile.
///
/// # Panics
///
/// Panics when [`ThreadAllocator`] is not the binary's global allocator, and
/// when called inside another `count` on the same thread.
pub fn count<R>(measured: impl FnOnce() -> R) -> (R, AllocationCounts) {
    assert!(
        INSTALLED.load(Ordering::Relaxed),
        "`ThreadAllocator` is not this binary's global allocator, so nothing would be counted"
    );
    assert!(
        !TRACKING.with(Cell::get),
        "allocation counting does not nest"
    );
    ALLOCATIONS.with(|count| count.set(0));
    REALLOCATIONS.with(|count| count.set(0));
    FREES.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    let result = measured();
    TRACKING.with(|tracking| tracking.set(false));
    let counts = AllocationCounts {
        allocations: ALLOCATIONS.with(Cell::get),
        reallocations: REALLOCATIONS.with(Cell::get),
        frees: FREES.with(Cell::get),
    };
    (result, counts)
}
