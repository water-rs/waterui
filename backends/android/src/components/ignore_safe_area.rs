//! `Metadata<IgnoreSafeArea>` — the `docs/layout-spec.md` §7.1 opt-out.
//!
//! The declared regions and edges land as a mask on a transparent
//! `RustViewGroup`'s `ignoredSafeAreaMask` field; every descendant
//! container accumulates it up the `ViewParent` chain and lays its
//! children out clear of only the regions still unmarked. The wrapper is
//! itself a band-reaching child — the parent group's `nativeLayout`
//! extends it through the safe rect to its band target on each edge its
//! frame touches — and inside the wrapper the child fills the already
//! adjusted bounds whole.

use alloc::boxed::Box;
use alloc::vec::Vec;

use waterui_core::Metadata;
use waterui_core::layout::{
    Layout, ProposalSize, Rect, Size, StretchAxis, SubView, SubviewPlacement,
};
use waterui_layout::IgnoreSafeArea;

use crate::components::container;
use crate::dispatch::Dispatcher;
use crate::jvm;

/// The wrapper's layout: the child fills the bounds `nativeLayout` already
/// shrank past the regions the accumulated mask leaves unmarked.
#[derive(Debug)]
struct FillLayout;

impl Layout for FillLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        children
            .first()
            .map_or_else(Size::zero, |child| child.measure(proposal).size)
    }

    fn place(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        children
            .iter()
            .map(|_| SubviewPlacement::new(bounds, proposal))
            .collect()
    }

    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        children.first().copied().unwrap_or(StretchAxis::None)
    }
}

/// Installs the `ignore_safe_area` handler: a mask-bearing `RustViewGroup`
/// whose layout fills its bounds.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<IgnoreSafeArea>>(|metadata, ctx| {
        let platform = ctx.platform().clone();
        let group = container::new_group(&platform);
        jvm::with_env(|env| {
            platform
                .bindings()
                .set_ignored_safe_area_mask(
                    env,
                    group.as_ref(),
                    crate::native_layout::declared_mask(metadata.value),
                )
                .expect("setIgnoredSafeAreaMask must not throw");
        });
        let state = container::wire_container_state(Box::new(FillLayout), None, &platform, &group);
        let child = ctx.renderer().render(metadata.content).mount(&group);
        state.borrow_mut().children = alloc::vec![child];
        container::finish_container(&state, group, &platform)
    });
}
