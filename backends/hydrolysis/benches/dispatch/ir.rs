//! Callgrind (gungraun) instruction counts of the dispatch workloads in
//! [`workloads`]; their wall-clock time and allocations are `dispatch`.
//!
//! Setup builds the runtime and the view outside the measured region, and
//! each benchmark returns its runtime so tearing it down is not counted
//! either: a count is one mount, one scroll step, one push and pop, or one
//! signal update.
//!
//! Run with `cargo bench -p hydrolysis --features testing --bench dispatch_ir`.

mod workloads;

#[cfg(not(target_arch = "wasm32"))]
mod dispatch_bench {
    use std::hint::black_box;

    use gungraun::{library_benchmark, library_benchmark_group};
    use waterui::View;

    use super::workloads::{self, LazyRows, Navigation, Remount, SignalUpdate};

    /// A warmed runtime and the mount it will perform: the boxed call is the
    /// one indirection a measurement adds, and inside it `mount` is
    /// monomorphized for the workload's own view type, never erased.
    struct Pending {
        remount: Remount,
        mount: Box<dyn FnOnce(&mut Remount)>,
    }

    fn pending<V: View + 'static>(view: fn() -> V) -> Pending {
        let next = view();
        Pending {
            remount: Remount::warmed(view()),
            mount: Box::new(move |remount: &mut Remount| remount.mount(next)),
        }
    }

    fn mount(mut pending: Pending) -> Remount {
        (pending.mount)(black_box(&mut pending.remount));
        pending.remount
    }

    fn modifier_chain_setup() -> Pending {
        pending(workloads::modifier_chain)
    }

    fn wide_stack_setup() -> Pending {
        pending(workloads::wide_stack)
    }

    fn settings_form_setup() -> Pending {
        pending(workloads::settings_form)
    }

    fn gallery_page_setup() -> Pending {
        pending(workloads::gallery_page)
    }

    fn dynamic_replace_setup() -> Pending {
        pending(workloads::replacement)
    }

    #[library_benchmark]
    #[bench::mount(setup = modifier_chain_setup)]
    fn modifier_chain(pending: Pending) -> Remount {
        mount(pending)
    }

    #[library_benchmark]
    #[bench::mount(setup = wide_stack_setup)]
    fn wide_stack(pending: Pending) -> Remount {
        mount(pending)
    }

    #[library_benchmark]
    #[bench::mount(setup = settings_form_setup)]
    fn settings_form(pending: Pending) -> Remount {
        mount(pending)
    }

    #[library_benchmark]
    #[bench::mount(setup = gallery_page_setup)]
    fn gallery_page(pending: Pending) -> Remount {
        mount(pending)
    }

    #[library_benchmark]
    #[bench::mount(setup = dynamic_replace_setup)]
    fn dynamic_replace(pending: Pending) -> Remount {
        mount(pending)
    }

    #[library_benchmark]
    #[bench::step(setup = LazyRows::new)]
    fn lazy_list_rows(mut rows: LazyRows) -> LazyRows {
        rows.advance();
        rows
    }

    #[library_benchmark]
    #[bench::push_pop(setup = Navigation::new)]
    fn navigation_push_pop(mut navigation: Navigation) -> Navigation {
        navigation.push_and_pop();
        navigation
    }

    #[library_benchmark]
    #[bench::increment(setup = SignalUpdate::new)]
    fn signal_update(mut update: SignalUpdate) -> SignalUpdate {
        update.increment();
        update
    }

    library_benchmark_group!(
        name = dispatch,
        benchmarks = [
            modifier_chain,
            wide_stack,
            settings_form,
            gallery_page,
            dynamic_replace,
            lazy_list_rows,
            navigation_push_pop,
            signal_update
        ]
    );
}

#[cfg(not(target_arch = "wasm32"))]
use dispatch_bench::dispatch;
#[cfg(not(target_arch = "wasm32"))]
use gungraun::main;

#[cfg(not(target_arch = "wasm32"))]
main!(library_benchmark_groups = dispatch);

// `harness = false` still needs a `main` to link on wasm32.
#[cfg(target_arch = "wasm32")]
fn main() {}
