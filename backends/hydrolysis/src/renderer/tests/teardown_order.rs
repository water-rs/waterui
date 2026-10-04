//! water-rs/waterui#1213: the semantic emit pass — the runtime
//! `#[waterui::test]` mounts — must retain and release frame-scoped
//! subscriptions in the same order `HydrolysisRenderer::flush_window_tree`
//! does, so watcher teardown that re-enters a `nami::WatcherManager`
//! (water-rs/nami#23) behaves identically under both runtimes.
//!
//! `flush_window_semantics` used to diverge on both frame boundaries: it
//! cleared the pure-emission registries before the structural patch instead
//! of after it, and it ran `lifecycle.finish_rebuild_frame` — which releases
//! the frame's retained signal-watch guards — before
//! `navigation.finish_rebuild_frame`, where the rendered pump runs it after.
//! The probes below pin one `Retain` to each boundary: a `Retain` on a view
//! inside a `when` subtree dies when the patch drops the subtree, a `Retain`
//! installed in a navigation slot dies at the navigation teardown when the
//! slot's retained owner leaves the tree, and a `signal_watches` entry not
//! refreshed this frame dies at the lifecycle rollover's stale-entry prune.
//! The rendered order is `[patch, navigation, lifecycle]`; the old semantic
//! order was `[patch, lifecycle, navigation]`.

use std::any::TypeId;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use nami::Binding;
use waterui::ViewExt as _;
use waterui::widget::condition::when;
use waterui_core::{AnyView, Environment, Retain};
use waterui_text::text;

use crate::renderer::SemanticCore;
use crate::renderer::navigation::navigation_state::{NavigationKey, NavigationSlot};

/// Logs `tag` into `drops` when its last owner drops it.
struct DropTag {
    tag: &'static str,
    drops: Rc<RefCell<Vec<&'static str>>>,
}

impl DropTag {
    fn into_rc(tag: &'static str, drops: &Rc<RefCell<Vec<&'static str>>>) -> Self {
        Self {
            tag,
            drops: Rc::clone(drops),
        }
    }
}

impl Drop for DropTag {
    fn drop(&mut self) {
        self.drops.borrow_mut().push(self.tag);
    }
}

#[test]
fn semantic_flush_releases_frame_state_in_renderer_order() {
    let mut env = Environment::new();
    crate::testing::install_theme(&mut env);
    crate::localization::install(&mut env);

    let drops = Rc::new(RefCell::new(Vec::new()));

    // The `when` subtree's `Retain` dies where the rendered pump dies it:
    // while the patch drops the gated subtree. The tag is built inside the
    // branch so the subtree's `Retain` is its only owner — a tag shared with
    // the `when` chain would outlive the patch, retained by the `Dynamic`
    // node's own watcher.
    let show = Binding::container(true);
    let view = {
        let show = show.clone();
        let drops = Rc::clone(&drops);
        move || {
            let drops = Rc::clone(&drops);
            when(show.clone(), move || {
                text("probe").retain(DropTag {
                    tag: "patch",
                    drops: Rc::clone(&drops),
                })
            })
        }
    };
    let mut core = SemanticCore::new(Instant::now());
    core.capture_window_semantics(AnyView::new(view), &env);

    // A navigation slot whose retained owner left the tree dies at
    // `navigation.finish_rebuild_frame`'s prune.
    let owner = Rc::new(());
    let slot = NavigationSlot::new(core.signals.clone());
    slot.controller
        .retain(Retain::new(DropTag::into_rc("navigation", &drops)));
    core.navigation
        .slots
        .insert(NavigationKey::for_rc(&owner), slot);
    drop(owner);

    // A signal watch not refreshed this frame dies at
    // `lifecycle.finish_rebuild_frame`'s stale-entry prune.
    core.lifecycle.signal_watches.insert(
        usize::MAX,
        TypeId::of::<()>(),
        Box::new(()),
        Retain::new(DropTag::into_rc("watch-guard", &drops)),
    );

    show.set(false);
    assert!(
        core.flush_window_semantics(&env),
        "semantic flush after a built tree must emit"
    );

    assert_eq!(
        drops.borrow().as_slice(),
        &["patch", "navigation", "watch-guard"],
    );
}
