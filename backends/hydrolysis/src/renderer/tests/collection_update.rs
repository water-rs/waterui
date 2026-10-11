//! <https://github.com/water-rs/hydrolysis/issues/227>: retained `for_each`
//! rows must re-materialize when an item with the same id changes its fields —
//! and *only* those rows.
//!
//! A collection notification carries a typed `CollectionChange` naming the
//! index ranges it touched; the retained paths rebuild only the ids occupying
//! `replaced` positions while every other surviving id keeps its node.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use nami::collection::List;
use waterui::ViewExt;
use waterui::graphics::Color;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Identifiable;
use waterui_core::views::ForEach;
use waterui_layout::AbsoluteLayout;
use waterui_layout::container::LazyContainer;
use waterui_layout::stack::{VStackLayout, zstack};

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

/// An item whose id stays stable while a baked field changes: `get_id` resolves
/// the same identity, so id reconciliation alone cannot see the content change.
#[derive(Clone)]
struct Shade {
    id: u64,
    level: u8,
}

impl Identifiable for Shade {
    type Id = u64;

    fn id(&self) -> Self::Id {
        self.id
    }
}

fn runtime(build: impl Fn() -> AnyView + 'static) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(build),
        400,
        640,
        MinimalTestTheme::default(),
    )
}

/// `AbsoluteLayout` in a `LazyContainer` routes the `ForEach` to the
/// non-virtualized `CollectionNode` path. Items stack at the same origin, so
/// the last (topmost) row decides the composited pixels.
fn collection_overlay(list: &List<Shade>) -> AnyView {
    let list = list.clone();
    AnyView::new(zstack((
        ().size(360.0, 600.0),
        LazyContainer::new(
            AbsoluteLayout,
            ForEach::new(list, |item: Shade| {
                Color::srgb(40 + item.level * 50, 90, 160).size(80.0, 40.0)
            }),
        ),
    )))
}

/// Replacing the collection with same-id items whose fields changed must
/// re-materialize the retained row's node instead of reusing the stale subtree.
#[test]
fn collection_same_id_item_update_rematerializes_row() {
    let list: List<Shade> = List::from(vec![Shade { id: 0, level: 0 }, Shade { id: 1, level: 1 }]);
    let mut runtime = {
        let list = list.clone();
        runtime(move || collection_overlay(&list))
    };
    let start = Instant::now();
    let before = runtime
        .pump_at(true, start)
        .snapshot
        .expect("initial overlay frame must produce a snapshot");

    // Same ids, changed field on the topmost (last) row.
    let _ = list.replace(vec![Shade { id: 0, level: 0 }, Shade { id: 1, level: 3 }]);
    let after = runtime
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("update frame must produce a snapshot");
    assert!(
        after.rgba8 != before.rgba8,
        "the changed row must repaint: a same-id content update must re-materialize \
         the retained row, not replay the stale subtree"
    );
}

/// The lazy stack (`VStackLayout` in a `LazyContainer`) routes to the
/// virtualized `LazyStackNode` path backed by `VisibleSubviewCache`.
#[test]
fn lazy_stack_same_id_item_update_rematerializes_row() {
    let list: List<Shade> = List::from(vec![Shade { id: 0, level: 0 }, Shade { id: 1, level: 1 }]);
    let mut runtime = {
        let list = list.clone();
        runtime(move || {
            AnyView::new(zstack((
                ().size(360.0, 600.0),
                LazyContainer::new(
                    VStackLayout::default(),
                    ForEach::new(list.clone(), |item: Shade| {
                        Color::srgb(40 + item.level * 50, 90, 160).size(80.0, 40.0)
                    }),
                ),
            )))
        })
    };
    let start = Instant::now();
    let before = runtime
        .pump_at(true, start)
        .snapshot
        .expect("initial lazy-stack frame must produce a snapshot");

    let _ = list.replace(vec![Shade { id: 0, level: 0 }, Shade { id: 1, level: 3 }]);
    let after = runtime
        .pump_at(true, start + Duration::from_millis(16))
        .snapshot
        .expect("update frame must produce a snapshot");
    assert!(
        after.rgba8 != before.rgba8,
        "the changed row must repaint: a same-id content update must re-materialize \
         the retained row, not replay the stale subtree"
    );
}

/// #227 review: an in-place `set` on one item of a 1,000-item collection must
/// rebuild exactly that row — not every surviving entry. The observable is
/// the `ForEach` generator's invocation count: `get_view` runs the generator,
/// and a `CollectionNode` calls `get_view` only when it materializes a node —
/// so a delta of one means exactly one row was rebuilt.
#[test]
fn collection_set_rebuilds_only_the_touched_row() {
    let builds = Rc::new(Cell::new(0usize));
    let list: List<Shade> = List::from(
        (0..1000u64)
            .map(|id| Shade { id, level: 0 })
            .collect::<Vec<_>>(),
    );
    let mut runtime = {
        let list = list.clone();
        let builds = builds.clone();
        runtime(move || {
            let builds = builds.clone();
            AnyView::new(zstack((
                ().size(360.0, 600.0),
                LazyContainer::new(
                    AbsoluteLayout,
                    ForEach::new(list.clone(), move |item: Shade| {
                        builds.set(builds.get() + 1);
                        Color::srgb(40 + item.level * 50, 90, 160).size(80.0, 40.0)
                    }),
                ),
            )))
        })
    };
    let start = Instant::now();
    runtime.pump_at(true, start);
    let baseline = builds.get();
    assert_eq!(baseline, 1000, "all rows materialize once at first layout");

    list.set(500, Shade { id: 500, level: 3 });
    runtime.pump_at(true, start + Duration::from_millis(16));

    assert_eq!(
        builds.get() - baseline,
        1,
        "replacing item 500 must rebuild exactly that row — no other retained \
         row may be re-materialized"
    );
}

/// #227 review: appending one item rebuilds no existing row. Same observable:
/// the generator runs once for the appended item and zero times for the
/// previous membership.
#[test]
fn collection_push_rebuilds_no_existing_row() {
    let builds = Rc::new(Cell::new(0usize));
    let list: List<Shade> = List::from(
        (0..4u64)
            .map(|id| Shade { id, level: 0 })
            .collect::<Vec<_>>(),
    );
    let mut runtime = {
        let list = list.clone();
        let builds = builds.clone();
        runtime(move || {
            let builds = builds.clone();
            AnyView::new(zstack((
                ().size(360.0, 600.0),
                LazyContainer::new(
                    AbsoluteLayout,
                    ForEach::new(list.clone(), move |item: Shade| {
                        builds.set(builds.get() + 1);
                        Color::srgb(40 + item.level * 50, 90, 160).size(80.0, 40.0)
                    }),
                ),
            )))
        })
    };
    let start = Instant::now();
    runtime.pump_at(true, start);
    let baseline = builds.get();
    assert_eq!(baseline, 4);

    list.push(Shade { id: 4, level: 2 });
    runtime.pump_at(true, start + Duration::from_millis(16));

    assert_eq!(
        builds.get() - baseline,
        1,
        "appending must build only the new row — no existing row is rebuilt"
    );
}
