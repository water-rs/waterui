//! Regression test for allocation-free steady-state recording and rendering.

#![cfg(all(feature = "testing", not(target_arch = "wasm32")))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashSet;
use std::mem::{align_of, size_of};
use std::sync::Arc;
use std::sync::mpsc;

use cherenkov::kurbo::{PathEl, Point, Rect};
use cherenkov::testing::{Null, NullConfig};
use cherenkov::{
    Command, Draw, Engine, FillRule, Fixed, FrameTime, Glyph, GlyphRun, GlyphStyle, Offscreen,
    OffscreenFormat, ShapeData, WorkingColor,
};

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
        pub(crate) static TRACKING: Cell<bool> = const { Cell::new(false) };
        pub(crate) static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
        pub(crate) static REALLOCATIONS: Cell<usize> = const { Cell::new(0) };
        pub(crate) static FREES: Cell<usize> = const { Cell::new(0) };
        pub(crate) static COMMAND_BUFFERS: Cell<([usize; 32], usize)> =
            const { Cell::new(([0; 32], 0)) };
    }
}

use counters::{ALLOCATIONS, COMMAND_BUFFERS, FREES, REALLOCATIONS, TRACKING};

struct ThreadAllocator;

#[global_allocator]
static ALLOCATOR: ThreadAllocator = ThreadAllocator;

// SAFETY: every method delegates to `System` under identical contracts —
// `layout`/`pointer` are passed through unchanged — and the accounting
// only touches thread-local counters.
unsafe impl GlobalAlloc for ThreadAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `layout` is the caller's allocation request, forwarded.
        let pointer = unsafe { System.alloc(layout) };
        allocation();
        observe_command_buffer(pointer, layout.size(), layout.align());
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `layout` is the caller's allocation request, forwarded.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        allocation();
        observe_command_buffer(pointer, layout.size(), layout.align());
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        release();
        // SAFETY: `pointer`/`layout` satisfy `System.dealloc` because they
        // came from `System.alloc` — the allocator contract.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: `pointer`/`layout`/`size` are the caller's request,
        // forwarded — the allocator contract.
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        reallocation();
        observe_command_buffer(pointer, size, layout.align());
        pointer
    }
}

fn allocation() {
    if TRACKING.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}

fn release() {
    if TRACKING.try_with(Cell::get).unwrap_or(false) {
        let _ = FREES.try_with(|count| count.set(count.get() + 1));
    }
}

fn reallocation() {
    if TRACKING.try_with(Cell::get).unwrap_or(false) {
        let _ = REALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}

fn observe_command_buffer(pointer: *mut u8, size: usize, align: usize) {
    if pointer.is_null()
        || align != align_of::<Command>()
        || size < size_of::<Command>()
        || !size.is_multiple_of(size_of::<Command>())
        || !TRACKING.try_with(Cell::get).unwrap_or(false)
    {
        return;
    }
    let _ = COMMAND_BUFFERS.try_with(|buffers| {
        let (mut pointers, mut len) = buffers.get();
        let pointer = pointer as usize;
        if !pointers[..len].contains(&pointer) && len < pointers.len() {
            pointers[len] = pointer;
            len += 1;
            buffers.set((pointers, len));
        }
    });
}

fn start_tracking() {
    ALLOCATIONS.with(|count| count.set(0));
    REALLOCATIONS.with(|count| count.set(0));
    FREES.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
}

fn stop_tracking() -> (usize, usize, usize) {
    TRACKING.with(|tracking| tracking.set(false));
    (
        ALLOCATIONS.with(Cell::get),
        REALLOCATIONS.with(Cell::get),
        FREES.with(Cell::get),
    )
}

fn command_buffer_count() -> usize {
    COMMAND_BUFFERS.with(|buffers| buffers.get().1)
}

#[test]
fn recording_and_rendering_reuse_ui_thread_allocations() {
    let (events, _receiver) = mpsc::channel();
    let engine = Engine::<Null>::new(NullConfig {
        events,
        reject: HashSet::new(),
        image_limits: cherenkov::ImageLimits::UNLIMITED,
    })
    .expect("init");
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {})
        .expect("surface");
    let layer = surface.layer();
    let path = ShapeData::Path {
        elements: Arc::from([
            PathEl::MoveTo(Point::new(1.0, 1.0)),
            PathEl::LineTo(Point::new(20.0, 1.0)),
            PathEl::LineTo(Point::new(10.0, 20.0)),
            PathEl::ClosePath,
        ]),
        rule: FillRule::NonZero,
    };
    let run = GlyphRun {
        font: cherenkov::FontId::new(1),
        size: 16.0,
        coords: Arc::from([]),
        glyphs: Arc::from([Glyph {
            id: 1,
            x: 1.0,
            y: 16.0,
            transform: None,
        }]),
        style: GlyphStyle::Fill,
    };

    for frame in 0..12 {
        let time = FrameTime::now();
        start_tracking();
        surface.update(|tx| {
            tx[&layer].record(|recorder| {
                recorder.fill(Fixed(path.clone()), Fixed(WorkingColor::WHITE));
                recorder.fill(
                    Fixed(Rect::new(2.0, 2.0, 24.0, 24.0)),
                    Fixed(WorkingColor::BLACK),
                );
                recorder.glyphs(Fixed(run.clone()), Fixed(WorkingColor::WHITE));
            });
        });
        let counts = stop_tracking();
        if frame >= 3 {
            assert_eq!(counts, (0, 0, 0), "frame {}", frame + 1);
        }
        engine.render(time).expect("render");
    }
    let buffers = command_buffer_count();
    assert!(
        (1..=3).contains(&buffers),
        "observed {buffers} command buffers"
    );
}

/// A steady-state edit transaction — `run_transaction` with one opacity
/// edit — holds a fixed allocation budget: after warmup every transaction
/// allocates exactly the baseline, never more. The baseline is pinned at
/// what dev measures for the same loop (2 allocations, 2 frees: the
/// transaction's `Ops` batch and its edit record); an increase is a
/// regression in the edit path, not noise.
#[test]
fn edit_transactions_reuse_their_allocations() {
    let (events, _receiver) = mpsc::channel();
    let engine = Engine::<Null>::new(NullConfig {
        events,
        reject: HashSet::new(),
        image_limits: cherenkov::ImageLimits::UNLIMITED,
    })
    .expect("init");
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {})
        .expect("surface");
    let layer = surface.layer();
    surface.update(|tx| {
        tx[&layer].record(|recorder| {
            recorder.fill(
                Fixed(Rect::new(0.0, 0.0, 4.0, 4.0)),
                Fixed(WorkingColor::WHITE),
            );
        });
    });

    for transaction in 0..12 {
        start_tracking();
        surface.update(|tx| {
            tx[&layer].opacity(0.5_f32);
        });
        let counts = stop_tracking();
        if transaction >= 3 {
            assert_eq!(counts, (2, 0, 2), "transaction {}", transaction + 1);
        }
        // Rendering drains the transaction's staged ops so `pending` does
        // not carry capacity growth into the next iteration's count.
        engine.render(FrameTime::now()).expect("render");
    }
}
