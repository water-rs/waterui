//! water-rs/waterui#1387 structural view transitions: a `.transition(...)`
//! collection member that leaves the semantic membership exits the semantic
//! tree immediately, while its layout slot is held at the size it last
//! measured until the removal clock completes and is then released
//! (layout-spec.md §4.5).

use super::*;
use std::time::Duration;
use waterui::PropertyTransition;
use waterui::animation::Animation;
use waterui::component::text;
use waterui::layout::stack::VStack;
use waterui::reactive::collection::List as Membership;
use waterui_core::id::SelfId;

#[test]
fn exiting_member_leaves_the_semantic_tree_immediately_but_holds_its_slot() {
    let mut rows: Membership<SelfId<u64>> = Membership::from(vec![SelfId::new(1), SelfId::new(2)]);
    let view_rows = rows.clone();
    let mut app = ui()
        .viewport(240, 160)
        .theme(TestStyle::new())
        .mount_offscreen(move || {
            let rows = view_rows.clone();
            VStack::for_each(rows, move |id: SelfId<u64>| {
                text(format!("Row {}", *id))
                    .transition(PropertyTransition::opacity())
                    .animation(Animation::linear(Duration::from_millis(1_000)))
            })
        });

    assert_eq!(app.query().role(Role::LABEL).all().len(), 2);
    let resting_y = app
        .query()
        .role(Role::LABEL)
        .label("Row 2")
        .single()
        .bounds()
        .y();

    // Remove the first row: the semantic tree drops it on the very next
    // frame, before any of its removal clock has run.
    let _ = rows.remove(0);
    app.pump_for(Duration::from_millis(16));
    assert_eq!(
        app.query().role(Role::LABEL).all().len(),
        1,
        "the exiting member must leave the semantic tree immediately: its \
         removal ghost paints only and carries no semantics (§4.5)"
    );
    assert!(
        app.query()
            .role(Role::LABEL)
            .label("Row 1")
            .all()
            .is_empty(),
        "the exiting row is not queryable while its ghost still paints"
    );

    // Mid-exit the slot is held: the surviving row keeps the y it had.
    app.pump_for(Duration::from_millis(500));
    let mid_y = app
        .query()
        .role(Role::LABEL)
        .label("Row 2")
        .single()
        .bounds()
        .y();
    assert!(
        (mid_y - resting_y).abs() <= 1.0,
        "the removed row's slot must be held until the clock completes: the \
         sibling stays at its resting y (resting {resting_y}, mid {mid_y})"
    );

    // After the clock completes the slot releases and the sibling slides up.
    app.pump_for(Duration::from_millis(600));
    let settled_y = app
        .query()
        .role(Role::LABEL)
        .label("Row 2")
        .single()
        .bounds()
        .y();
    assert!(
        settled_y < resting_y - 10.0,
        "once the removal clock completes the slot is released: the sibling \
         slides up to the freed space (resting {resting_y}, settled {settled_y})"
    );
}
