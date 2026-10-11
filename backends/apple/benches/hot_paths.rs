//! Hot-path benchmarks for the Apple backend's port surface: leaf
//! mount/unmount, the binding → watcher → applied-property update path and
//! its fan-out over N mounted leaves, `set_layout_handler` running
//! `waterui-layout` frames over a container with N children,
//! `reconcile_subviews` — the id-keyed membership resync a container or
//! list pays when rows churn — and real `NSTableView` cell reuse under
//! `reloadData` + `scrollRowToVisible`.
//!
//! `#[cfg(target_os = "macos")]` only — the leaves are `AppKit` objects.
//! Criterion drives benches from the real main thread, so
//! `MainThreadMarker::new()` yields a genuine marker and every view here is
//! main-thread sound. Run with `cargo bench`; nothing gates CI on these.

#[cfg(target_os = "macos")]
mod hot_paths {
    use std::hint::black_box;

    use cocoa_ui::appkit::{HostView, Label};
    use cocoa_ui::geometry::Rect;
    use cocoa_ui::{MainThreadMarker, PlatformView};
    use criterion::{Criterion, criterion_group};
    use waterui::layout::stack::VStackLayout;
    use waterui::reactive::{Binding, binding};
    use waterui_apple::contract::{Mounted, NativeLeaf};
    use waterui_core::layout::{
        Layout, Point, ProposalSize, Size, StretchAxis, SubView, ViewDimensions,
    };

    /// A fixed-size leaf: the smallest `SubView` the negotiation path needs.
    struct BenchLeaf {
        size: Size,
    }

    impl SubView for BenchLeaf {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.size)
        }

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    /// Mount → unmount a representative leaf (`Label`) against a host view.
    fn leaf_mount_unmount(c: &mut Criterion) {
        let mtm = MainThreadMarker::new().expect("benches run on the main thread");
        let parent = HostView::new(mtm, Rect::new(0.0, 0.0, 800.0, 600.0));
        c.bench_function("leaf_mount_unmount", |b| {
            b.iter(|| {
                let label = Label::new(mtm);
                let view: &PlatformView = &label;
                let leaf = NativeLeaf::new(
                    view,
                    BenchLeaf {
                        size: Size::new(40.0, 20.0),
                    },
                );
                let mounted = leaf.mount(&parent);
                black_box(mounted.unmount());
            });
        });
    }

    /// Binding write → watcher → applied platform property (`set_text`).
    fn binding_update_path(c: &mut Criterion) {
        let mtm = MainThreadMarker::new().expect("benches run on the main thread");
        let parent = HostView::new(mtm, Rect::new(0.0, 0.0, 800.0, 600.0));
        let label = Label::new(mtm);
        let text: Binding<String> = binding(String::new());
        let view: &PlatformView = &label;
        let mut leaf = NativeLeaf::new(
            view,
            BenchLeaf {
                size: Size::new(40.0, 20.0),
            },
        );
        leaf.bind(&text, {
            let label = label.clone();
            move |value| label.set_text(&value)
        });
        let _mounted = leaf.mount(&parent);
        c.bench_function("binding_update_to_label", |b| {
            b.iter(|| {
                text.set(String::from("bench"));
                text.set(String::from("update"));
            });
        });
    }

    /// `set_layout_handler` running a `waterui-layout` `VStack` placement over a
    /// container with N leaf children, applying each returned frame.
    fn frame_application(c: &mut Criterion) {
        let mtm = MainThreadMarker::new().expect("benches run on the main thread");
        let mut group = c.benchmark_group("frame_application");
        for count in [8_usize, 64] {
            let host = HostView::new(mtm, Rect::new(0.0, 0.0, 640.0, 480.0));
            let host_view: &PlatformView = &host;
            let mut children = Vec::with_capacity(count);
            for index in 0..count {
                let label = Label::new(mtm);
                let view: &PlatformView = &label;
                let leaf = NativeLeaf::new(
                    view,
                    BenchLeaf {
                        size: Size::new(
                            50.0,
                            10.0 + f32::from(u8::try_from(index % 4).unwrap_or_default()),
                        ),
                    },
                );
                children.push(leaf.mount(host_view));
            }
            let layout = VStackLayout::default();
            host.set_layout_handler(move |host_view| {
                let bounds = host_view.bounds();
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "the layout contract is f32; view extents always fit"
                )]
                let proposal = ProposalSize::new(
                    Some(bounds.size.width as f32),
                    Some(bounds.size.height as f32),
                );
                let child_layouts: Vec<&dyn SubView> =
                    children.iter().map(Mounted::layout).collect();
                let placements = layout.place(
                    waterui_core::layout::Rect::new(
                        Point::new(0.0, 0.0),
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "the layout contract is f32; view extents always fit"
                        )]
                        Size::new(bounds.size.width as f32, bounds.size.height as f32),
                    ),
                    proposal,
                    &child_layouts,
                );
                for (child, placement) in children.iter().zip(placements.iter()) {
                    cocoa_ui::view::set_frame(
                        child.view(),
                        Rect::new(
                            f64::from(placement.frame.x()),
                            f64::from(placement.frame.y()),
                            f64::from(placement.frame.width()),
                            f64::from(placement.frame.height()),
                        ),
                    );
                }
            });
            group.bench_function(format!("children_{count}"), |b| {
                b.iter(|| {
                    host.set_needs_layout();
                    host.layout_if_needed();
                });
            });
        }
        group.finish();
    }

    /// One signal, N mounted leaves: the watcher fan-out a tree of bound
    /// properties pays when shared state changes — the reactive update the
    /// binding → watcher → kit-call path scales on.
    fn reactive_fanout(c: &mut Criterion) {
        let mtm = MainThreadMarker::new().expect("benches run on the main thread");
        let parent = HostView::new(mtm, Rect::new(0.0, 0.0, 800.0, 600.0));
        let parent_view: &PlatformView = &parent;
        let mut group = c.benchmark_group("reactive_fanout");
        for count in [8_usize, 64] {
            let shared: Binding<bool> = binding(false);
            let mut mounted = Vec::with_capacity(count);
            for _ in 0..count {
                let label = Label::new(mtm);
                let view: &PlatformView = &label;
                let mut leaf = NativeLeaf::new(
                    view,
                    BenchLeaf {
                        size: Size::new(40.0, 20.0),
                    },
                );
                let target = cocoa_ui::view::retain_base(view);
                leaf.bind(&shared, move |on| cocoa_ui::view::set_hidden(&target, on));
                mounted.push(leaf.mount(parent_view));
            }
            let mut on = false;
            group.bench_function(format!("leaves_{count}"), |b| {
                b.iter(|| {
                    on = !on;
                    shared.set(on);
                });
            });
            // The mounts are pure keep-alive: they must outlive the
            // measured section, then unmount all at once.
            drop(mounted);
        }
        group.finish();
    }

    /// `reconcile_subviews` over a mounted row set: the membership/order
    /// resync a container or list pays when its ids churn — with each row's
    /// leaf surviving unchanged, which is the reuse the path exists for.
    fn subview_reconcile(c: &mut Criterion) {
        let mtm = MainThreadMarker::new().expect("benches run on the main thread");
        let mut group = c.benchmark_group("subview_reconcile");
        for count in [8_usize, 64] {
            let host = HostView::new(mtm, Rect::new(0.0, 0.0, 800.0, 600.0));
            let host_view: &PlatformView = &host;
            let mut children = Vec::with_capacity(count);
            for _ in 0..count {
                let label = Label::new(mtm);
                let view: &PlatformView = &label;
                let child = cocoa_ui::view::retain_base(view);
                cocoa_ui::view::add_subview(host_view, &child);
                children.push(child);
            }
            group.bench_function(format!("rows_{count}"), |b| {
                b.iter(|| {
                    children.rotate_left(1);
                    cocoa_ui::view::reconcile_subviews(host_view, &children);
                });
            });
        }
        group.finish();
    }

    /// Real `NSTableView` cell reuse: the row provider asks the table's
    /// reuse pool (`makeView(withIdentifier:)`) for a cell before
    /// building one, and stepping `scrollRowToVisible` through the row
    /// set inside a real, never-shown window drives the recycle → dequeue
    /// churn every scroll of a long list pays. Not a compile-only check —
    /// the window is required for `AppKit`'s layout pass to run.
    fn table_cell_reuse(c: &mut Criterion) {
        const ROWS: usize = 512;
        use cocoa_ui::Retained;
        use cocoa_ui::appkit::{Window, WindowStyle};
        use cocoa_ui::objc2::MainThreadOnly;
        use cocoa_ui::objc2::msg_send;
        use cocoa_ui::objc2_app_kit::{
            NSScrollView, NSTableCellView, NSTableColumn, NSUserInterfaceItemIdentification,
        };
        use cocoa_ui::objc2_foundation::NSString;

        let mtm = MainThreadMarker::new().expect("benches run on the main thread");
        let window = Window::new(
            mtm,
            Rect::new(0.0, 0.0, 480.0, 320.0),
            WindowStyle::TITLED | WindowStyle::CLOSABLE,
        );
        // SAFETY: `initWithFrame:` is `NSScrollView`'s designated
        // initializer; `mtm` proves main-thread confinement.
        let scroll: Retained<NSScrollView> = unsafe {
            msg_send![cocoa_ui::objc2_app_kit::NSScrollView::alloc(mtm), initWithFrame:
            cocoa_ui::objc2_core_foundation::CGRect::new(
                cocoa_ui::objc2_core_foundation::CGPoint::new(0.0, 0.0),
                cocoa_ui::objc2_core_foundation::CGSize::new(480.0, 320.0),
            )]
        };
        scroll.setHasVerticalScroller(true);
        let table = cocoa_ui::appkit::TableView::new(mtm);
        let column = NSTableColumn::initWithIdentifier(
            NSTableColumn::alloc(mtm),
            &NSString::from_str("content"),
        );
        table.set_columns(&[column]);
        scroll.setDocumentView(Some(&table));
        window.set_content_view(&scroll);

        let max_row = isize::try_from(ROWS).expect("row count fits NSInteger");
        table.set_row_count_handler(move || ROWS);
        table.set_row_height_handler(|_| 24.0);
        let reuse_id = NSString::from_str("bench.cell");
        // Weak: the handler lives in the table's ivars, so a strong
        // capture would cycle. `upgrade` can only fail after the table is
        // gone, which the window keeps alive for the whole bench.
        let table_for_cells = cocoa_ui::objc2::rc::Weak::from_retained(&table);
        let calls = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let created = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        let pool_hits = std::rc::Rc::new(std::cell::Cell::new(0_usize));
        table.set_cell_view_handler({
            let calls = std::rc::Rc::clone(&calls);
            let created = std::rc::Rc::clone(&created);
            let pool_hits = std::rc::Rc::clone(&pool_hits);
            move |_, _row| {
                calls.set(calls.get() + 1);
                let table = table_for_cells
                    .load()
                    .expect("the window keeps the table alive for the bench");
                // SAFETY: `owner: None` is the documented delegation
                // default for `makeView(withIdentifier:owner:)`; a pool
                // hit returns the `NSTableCellView` class registered
                // under the id, so the unchecked cast is sound.
                let pooled: Option<Retained<NSTableCellView>> = unsafe {
                    table
                        .makeViewWithIdentifier_owner(&reuse_id, None)
                        .map(|view| Retained::cast_unchecked(view))
                };
                let cell = pooled.map_or_else(
                    || {
                        created.set(created.get() + 1);
                        let cell = NSTableCellView::new(mtm);
                        cell.setIdentifier(Some(&reuse_id));
                        cell
                    },
                    |cell| {
                        pool_hits.set(pool_hits.get() + 1);
                        cell
                    },
                );
                Some(cell.into_super())
            }
        });

        // `reloadData` is what actually asks the handlers for rows — the
        // setters only store closures — then a layout pass primes the
        // pool: rows inside the clip view's bounds get fabricated.
        table.reload_data();
        scroll.layoutSubtreeIfNeeded();
        assert!(
            calls.get() > 0 && created.get() > 0,
            "table produced no rows after reloadData + layout (calls={}, created={})",
            calls.get(),
            created.get()
        );

        // Drive a real scroll through the row set so previously-created
        // cells leave the visible range and the pool dequeues them back:
        // a reuse pool that never answers `makeView(withIdentifier:)` is
        // the silent no-reuse path this asserts against.
        for row in 0..64_isize.min(max_row) {
            table.scrollRowToVisible(row);
            table.layoutSubtreeIfNeeded();
        }
        assert!(
            pool_hits.get() > 0,
            "scrolled {} rows without one pool dequeue (calls={}, created={})",
            64_isize.min(max_row),
            calls.get(),
            created.get()
        );

        let mut group = c.benchmark_group("table_cell_reuse");
        group.bench_function("scroll_step_512_rows", |b| {
            let mut row = 0_isize;
            b.iter(|| {
                row = (row + 1) % max_row;
                table.scrollRowToVisible(row);
                table.layoutSubtreeIfNeeded();
            });
        });
        group.finish();
    }

    criterion_group!(
        benches,
        leaf_mount_unmount,
        binding_update_path,
        frame_application,
        reactive_fanout,
        subview_reconcile,
        table_cell_reuse
    );
}

#[cfg(target_os = "macos")]
criterion::criterion_main!(hot_paths::benches);

#[cfg(not(target_os = "macos"))]
fn main() {}
