//! Wall-clock time and allocations of turning a view tree into retained
//! Hydrolysis nodes, per workload in [`workloads`]. Instruction counts of
//! the same workloads are `dispatch_ir`.
//!
//! Wall-clock numbers come from a quiet dedicated machine only; allocation
//! counts and bytes are deterministic anywhere.
//!
//! Run with `cargo bench -p hydrolysis --features testing --bench dispatch`.

mod workloads;

use divan::{AllocProfiler, Bencher};

use workloads::{LazyRows, Navigation, Remount, SignalUpdate};

#[global_allocator]
static ALLOCATOR: AllocProfiler = AllocProfiler::system();

/// Mounts a fresh `view` per iteration in place of the previous one.
fn bench_mount<V: waterui::View>(bencher: Bencher, view: fn() -> V) {
    let mut remount = Remount::warmed(view());
    bencher
        .with_inputs(view)
        .bench_local_values(|view| remount.mount(view));
}

#[divan::bench]
fn modifier_chain(bencher: Bencher) {
    bench_mount(bencher, workloads::modifier_chain);
}

#[divan::bench]
fn wide_stack(bencher: Bencher) {
    bench_mount(bencher, workloads::wide_stack);
}

#[divan::bench]
fn settings_form(bencher: Bencher) {
    bench_mount(bencher, workloads::settings_form);
}

#[divan::bench]
fn gallery_page(bencher: Bencher) {
    bench_mount(bencher, workloads::gallery_page);
}

#[divan::bench]
fn dynamic_replace(bencher: Bencher) {
    bench_mount(bencher, workloads::replacement);
}

#[divan::bench]
fn lazy_list_rows(bencher: Bencher) {
    let mut rows = LazyRows::new();
    bencher.bench_local(|| rows.advance());
}

#[divan::bench]
fn navigation_push_pop(bencher: Bencher) {
    let mut navigation = Navigation::new();
    bencher.bench_local(|| navigation.push_and_pop());
}

#[divan::bench]
fn signal_update(bencher: Bencher) {
    let mut update = SignalUpdate::new();
    bencher.bench_local(|| update.increment());
}

fn main() {
    divan::main();
}
