//! Stable render identity (water-rs/hydrolysis#205, P5).
//!
//! `RenderId` lives exactly as long as the visual node: a signal-driven update
//! of the same node keeps it, while a structural replacement allocates a fresh
//! id only on the replaced mounts. `PresentationId` tells placements of the
//! same node apart, so an ordinary placement and a hosted preview/accessory
//! can never collide on an engine mount key.

use core::time::Duration;
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
use crate::renderer::{PresentationId, RenderId, RenderKey, RetainedSubview};

fn tree_ids(runtime: &HeadlessRuntime) -> Vec<RenderId> {
    let mut ids = Vec::new();
    runtime
        .renderer()
        .render_tree_root()
        .expect("a pumped window must hold a retained tree")
        .collect_render_ids(&mut ids);
    ids
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
/// `RenderId` across the animated frames.
#[test]
fn scalar_update_preserves_render_ids() {
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
    let before = tree_ids(&runtime);

    value.set(0.25);
    for frame in 1..=3 {
        let at = start + Duration::from_millis(frame * 16);
        let _ = runtime.pump_at(false, at);
        assert_eq!(
            tree_ids(&runtime),
            before,
            "a signal update must preserve every visual node's RenderId (frame {frame})"
        );
    }
}

/// A `Dynamic` swap is a structural replacement: the host and the untouched
/// sibling keep their ids, and only the swapped child's subtree mounts get
/// fresh ones — never the ids the removed subtree held.
#[test]
fn dynamic_swap_replaces_only_the_swapped_mounts() {
    let (handler, dynamic) = Dynamic::new();
    handler.set(text("first"));
    let mut runtime =
        runtime(move || AnyView::new(vstack((text("kept sibling"), dynamic.clone()))));
    let start = Instant::now();
    let _ = runtime.pump_at(true, start);
    let before = tree_ids(&runtime);

    handler.set(vstack((text("second"), ().size(40.0, 20.0))));
    let _ = runtime.pump_at(true, start + Duration::from_millis(16));
    let after = tree_ids(&runtime);

    // The mounts that retired must be exactly the swapped subtree: one
    // contiguous run in pre-order. Every mount outside it — ancestors, the
    // kept sibling, the `Dynamic` host itself — kept its id, and the new
    // subtree's ids are all fresh (never recycled from the removed set).
    let removed_positions: Vec<usize> = before
        .iter()
        .enumerate()
        .filter_map(|(i, id)| (!after.contains(id)).then_some(i))
        .collect();
    assert!(
        !removed_positions.is_empty(),
        "the swap must retire the replaced subtree's ids: {before:?} -> {after:?}"
    );
    assert_eq!(
        removed_positions[removed_positions.len() - 1] - removed_positions[0] + 1,
        removed_positions.len(),
        "retired mounts must be a single contiguous subtree, not scattered \
         siblings: {before:?} -> {after:?}"
    );
    assert!(
        after.iter().any(|id| !before.contains(id)),
        "the rebuilt subtree must carry fresh ids, not reused ones: {before:?} -> {after:?}"
    );
    assert!(
        before.len() - removed_positions.len() <= after.len(),
        "every mount outside the replaced subtree must survive: {before:?} -> {after:?}"
    );
}

/// A collection reconcile keeps surviving items' nodes: a removal drops only
/// the removed item's ids, and a same-id rebuild retires only that item's ids.
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

    // Removal: only the dropped item's ids leave; every survivor keeps its node.
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
    let before = tree_ids(&runtime);

    let _removed = list.remove(1);
    let _ = runtime.pump_at(true, start + Duration::from_millis(16));
    let after = tree_ids(&runtime);
    assert!(
        before.len() > after.len(),
        "removing an item must retire its mounts: {before:?} -> {after:?}"
    );
    assert!(
        after.iter().all(|id| before.contains(id)),
        "surviving mounts must keep their ids: {before:?} -> {after:?}"
    );

    // Same-id rebuild: only the rebuilt item's subtree gets fresh ids.
    let _ = list.replace(vec![Shade { id: 0, level: 0 }, Shade { id: 2, level: 3 }]);
    let _ = runtime.pump_at(true, start + Duration::from_millis(32));
    let rebuilt = tree_ids(&runtime);
    let retired: Vec<_> = after.iter().filter(|id| !rebuilt.contains(id)).collect();
    let added: Vec<_> = rebuilt.iter().filter(|id| !after.contains(id)).collect();
    assert_eq!(
        retired.len(),
        added.len(),
        "a same-id rebuild must swap the touched subtree's ids one-for-one: {after:?} -> {rebuilt:?}"
    );
    assert!(
        !retired.is_empty(),
        "the rebuilt item must not keep the stale subtree's ids"
    );
    assert!(
        rebuilt.len() == after.len(),
        "a field change must not add or drop mounts: {after:?} -> {rebuilt:?}"
    );
}

/// The mount key pairs a visual node with a placement: the same node's
/// ordinary placement and its hosted preview/accessory placement can never
/// collide, and two hosted instances never share a `PresentationId`.
#[test]
fn ordinary_and_hosted_presentations_cannot_collide() {
    let node_id = RenderId::next();
    let ordinary = RenderKey {
        render: node_id,
        presentation: PresentationId::ORDINARY,
    };
    let hosted = RenderKey {
        render: node_id,
        presentation: PresentationId::next(),
    };
    assert_ne!(
        ordinary, hosted,
        "a hosted placement must key differently from the ordinary placement"
    );

    let preview = RetainedSubview::new(AnyView::new(text("preview")));
    let accessory = RetainedSubview::new(AnyView::new(text("accessory")));
    assert_ne!(preview.presentation, PresentationId::ORDINARY);
    assert_ne!(accessory.presentation, PresentationId::ORDINARY);
    assert_ne!(
        preview.presentation, accessory.presentation,
        "distinct hosted instances must never share a PresentationId"
    );
    assert_ne!(
        RenderKey {
            render: node_id,
            presentation: preview.presentation,
        },
        ordinary,
        "a preview mount of the same node must not steal the ordinary mount"
    );
}
