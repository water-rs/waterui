//! Stable node identity across frames (water-rs/hydrolysis#205, P5;
//! water-rs/waterui#1809 mount keys).
//!
//! A `NodeCell` lives exactly as long as the visual node it anchors: a
//! signal-driven update of the same node keeps its cell, while a structural
//! replacement allocates fresh cells only on the replaced mounts. The cell's
//! address is the mount key — the mount a node's retained layer carries, and
//! the owner a watcher marks — so two placements of the same logical content
//! (an ordinary placement and a hosted preview/accessory) can never collide:
//! distinct built subtrees hold distinct cells by construction.

use core::time::Duration;
use std::rc::Rc;
use std::time::Instant;

use waterui::animation::Animation;
use waterui::component::text;
use waterui::graphics::Color;
use waterui::{Binding, SignalExt as _, ViewExt as _};
use waterui_core::AnyView;
use waterui_core::dynamic::Dynamic;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Identifiable;
use waterui_core::views::ForEach;
use waterui_layout::AbsoluteLayout;
use waterui_layout::container::LazyContainer;
use waterui_layout::stack::vstack;
use waterui_layout::stack::zstack;

use nami::collection::List;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::renderer::mount::cell::NodeCell;

/// Collect the tree's cells as live `Rc`s: holding them keeps every retired
/// address un-reused across a comparison, so two nodes compare by
/// `Rc::ptr_eq` exactly — never by an address an allocator could recycle.
fn tree_cells(runtime: &HeadlessRuntime) -> Vec<Rc<NodeCell>> {
    let mut cells = Vec::new();
    runtime
        .renderer()
        .render_tree_root()
        .expect("a pumped window must hold a retained tree")
        .collect_cells(&mut cells);
    cells
}

fn contains_cell(cells: &[Rc<NodeCell>], cell: &Rc<NodeCell>) -> bool {
    cells.iter().any(|other| Rc::ptr_eq(other, cell))
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

/// A paint/animation update walks signals, never structure: every node —
/// including the transform wrapper around the updated content — must keep its
/// `NodeCell` across the animated frames.
#[test]
fn scalar_update_preserves_node_cells() {
    let value = Binding::f32(1.0);
    let mut runtime = {
        let value = value.clone();
        runtime(move || {
            let animated = value.with(Animation::linear(Duration::from_millis(1_000)));
            AnyView::new(vstack((
                text("kept sibling"),
                ().size(80.0, 80.0).scale(animated.clone(), animated),
            )))
        })
    };
    let start = Instant::now();
    let _ = runtime.pump_at(true, start);
    let before = tree_cells(&runtime);

    value.set(0.25);
    for frame in 1..=3 {
        let at = start + Duration::from_millis(frame * 16);
        let _ = runtime.pump_at(false, at);
        let after = tree_cells(&runtime);
        assert_eq!(
            after.len(),
            before.len(),
            "a signal update must keep the node count (frame {frame})"
        );
        assert!(
            after.iter().zip(&before).all(|(a, b)| Rc::ptr_eq(a, b)),
            "a signal update must preserve every visual node's cell (frame {frame})"
        );
    }
}

/// A `Dynamic` swap is a structural replacement: the host and the untouched
/// sibling keep their cells, and only the swapped child's subtree mounts get
/// fresh ones — never the cells the removed subtree held.
#[test]
fn dynamic_swap_replaces_only_the_swapped_mounts() {
    let (handler, dynamic) = Dynamic::new();
    handler.set(text("first"));
    let mut runtime =
        runtime(move || AnyView::new(vstack((text("kept sibling"), dynamic.clone()))));
    let start = Instant::now();
    let _ = runtime.pump_at(true, start);
    let before = tree_cells(&runtime);

    handler.set(vstack((text("second"), ().size(40.0, 20.0))));
    let _ = runtime.pump_at(true, start + Duration::from_millis(16));
    let after = tree_cells(&runtime);

    // The mounts that retired must be exactly the swapped subtree: one
    // contiguous run in pre-order. Every mount outside it — ancestors, the
    // kept sibling, the `Dynamic` host itself — kept its cell, and the new
    // subtree's cells are all fresh (never recycled from the removed set:
    // `before` still holds its cells alive, so an address cannot alias).
    let removed_positions: Vec<usize> = before
        .iter()
        .enumerate()
        .filter_map(|(i, cell)| (!contains_cell(&after, cell)).then_some(i))
        .collect();
    assert!(
        !removed_positions.is_empty(),
        "the swap must retire the replaced subtree's cells"
    );
    assert_eq!(
        removed_positions[removed_positions.len() - 1] - removed_positions[0] + 1,
        removed_positions.len(),
        "retired mounts must be a single contiguous subtree, not scattered \
         siblings"
    );
    assert!(
        after.iter().any(|cell| !contains_cell(&before, cell)),
        "the rebuilt subtree must carry fresh cells, not reused ones"
    );
    assert!(
        before.len() - removed_positions.len() <= after.len(),
        "every mount outside the replaced subtree must survive"
    );
}

/// A collection reconcile keeps surviving items' nodes: a removal drops only
/// the removed item's cells, and a same-id rebuild retires only that item's
/// cells.
#[test]
fn collection_reconcile_replaces_only_the_touched_mounts() {
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

    let overlay = |list: &List<Shade>| {
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
    };

    // Removal: only the dropped item's cells leave; every survivor keeps its
    // node.
    let list: List<Shade> = List::from(vec![
        Shade { id: 0, level: 0 },
        Shade { id: 1, level: 1 },
        Shade { id: 2, level: 2 },
    ]);
    let mut runtime = {
        let list = list.clone();
        runtime(move || overlay(&list))
    };
    let start = Instant::now();
    let _ = runtime.pump_at(true, start);
    let before = tree_cells(&runtime);

    let _removed = list.remove(1);
    let _ = runtime.pump_at(true, start + Duration::from_millis(16));
    let after = tree_cells(&runtime);
    assert!(
        before.len() > after.len(),
        "removing an item must retire its mounts"
    );
    assert!(
        after.iter().all(|cell| contains_cell(&before, cell)),
        "surviving mounts must keep their cells"
    );

    // Same-id rebuild: only the rebuilt item's subtree gets fresh cells.
    let _ = list.replace(vec![Shade { id: 0, level: 0 }, Shade { id: 2, level: 3 }]);
    let _ = runtime.pump_at(true, start + Duration::from_millis(32));
    let rebuilt = tree_cells(&runtime);
    let retired = after
        .iter()
        .filter(|cell| !contains_cell(&rebuilt, cell))
        .count();
    let added = rebuilt
        .iter()
        .filter(|cell| !contains_cell(&after, cell))
        .count();
    assert_eq!(
        retired, added,
        "a same-id rebuild must swap the touched subtree's cells one-for-one"
    );
    assert!(
        retired != 0,
        "the rebuilt item must not keep the stale subtree's cells"
    );
    assert!(
        rebuilt.len() == after.len(),
        "a field change must not add or drop mounts"
    );
}
