//! The `material_background` metadata: `IgnorableMetadata<MaterialBackground>`
//! wrapped around content a blur effect sits behind.
//!
//! Mirrors `WuiMaterialBackground`: a transparent `HostView` container —
//! measure, stretch, priority and the placement proposal all answer for the
//! mounted child — holding the platform's effect view (`UIVisualEffectView`
//! with a `UIBlurEffect` on `UIKit`, `NSVisualEffectView` on `AppKit`) plus
//! the content as direct subviews. Each layout pass centers the effect and
//! the content over the content's negotiated size: a width-constrained
//! content keeps its narrower material column while the wrapper may be
//! stamped wider.

use alloc::rc::Rc;
use core::cell::Cell;

use cocoa_ui::material::{self, MaterialLevel};
use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::background::{Material, MaterialBackground};
use waterui_core::IgnorableMetadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The `Material` thickness as the kit's neutral level.
const fn level(material: Material) -> MaterialLevel {
    match material {
        Material::UltraThin => MaterialLevel::UltraThin,
        Material::Thin => MaterialLevel::Thin,
        Material::Regular => MaterialLevel::Regular,
        Material::Thick => MaterialLevel::Thick,
        Material::UltraThick => MaterialLevel::UltraThick,
    }
}

/// The leaf's live state: the mounted child and the effect view, plus the
/// last placement proposal the wrapper was selected with — the offer the
/// layout pass echoes back when the host stamps the frame without
/// delivering a proposal.
struct MaterialBackgroundState {
    /// The mounted content.
    child: Mounted,
    /// The effect view, stamped over the content's frame each layout.
    blur: Retained<PlatformView>,
    /// The proposal selected for this wrapper, forwarded to the child.
    last_proposal: Cell<ProposalSize>,
}

impl core::fmt::Debug for MaterialBackgroundState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MaterialBackgroundState")
            .finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct MaterialBackgroundSubView {
    /// The leaf's state.
    state: Rc<MaterialBackgroundState>,
}

impl core::fmt::Debug for MaterialBackgroundSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MaterialBackgroundSubView")
            .finish_non_exhaustive()
    }
}

impl SubView for MaterialBackgroundSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.child.layout().is_empty()
    }
}

/// Installs the `material_background` handler on the dispatcher:
/// `IgnorableMetadata<MaterialBackground>` maps to a transparent container
/// with the platform's blur effect behind the content.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<IgnorableMetadata<MaterialBackground>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        let blur = material::material_view(mtm, level(metadata.value.0));

        // `wantsLayer = true`: `NSVisualEffectView` expects a layer-backed
        // host on `AppKit`.
        #[cfg(target_os = "macos")]
        let _backing_layer = cocoa_ui::shape::layer(host_view);

        // The blur behind the content; both are direct subviews laid out by
        // the layout handler, as `translatesAutoresizingMaskIntoConstraints`
        // on `WuiMaterialBackground`.
        view::add_subview(host_view, &blur);
        let mounted = ctx.render(metadata.content).mount(host_view);
        view::set_translates_autoresizing(&blur, true);
        view::set_translates_autoresizing(mounted.view(), true);

        // `WuiSafeAreaManaging`: a material is chrome, not content — the
        // blur runs behind the status bar and the home indicator while the
        // content keeps its own safe-area insets.
        host.set_manages_safe_area(true);

        let state = Rc::new(MaterialBackgroundState {
            child: mounted,
            blur,
            last_proposal: Cell::new(ProposalSize::UNSPECIFIED),
        });

        // `.background(material)` covers exactly the view it backs: the
        // content's own measurement under the delivered proposal is the
        // frame both the effect and the content get, centered in bounds.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "AppKit measures in `CGFloat`; the leaf speaks `f32`"
            )]
            move |host| {
                let bounds = view::bounds(host);
                let last = state.last_proposal.get();
                let size = state
                    .child
                    .layout()
                    .measure(ProposalSize::new(
                        last.width.or(Some(bounds.size.width as f32)),
                        last.height.or(Some(bounds.size.height as f32)),
                    ))
                    .size;
                let rect = Rect::new(
                    (bounds.size.width - f64::from(size.width)) / 2.0,
                    (bounds.size.height - f64::from(size.height)) / 2.0,
                    f64::from(size.width),
                    f64::from(size.height),
                );
                view::set_frame(state.child.view(), rect);
                view::set_frame(&state.blur, rect);
            }
        });

        // `setPlacementProposal`: remember the selected offer and forward
        // it to the content, as `WuiMaterialBackground.lastProposal`.
        let sink_guard = proposal::register_sink(host_view, {
            let state = Rc::clone(&state);
            move |selected| {
                state.last_proposal.set(selected);
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            MaterialBackgroundSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf
    });
}
