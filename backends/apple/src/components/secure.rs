//! The `secure` metadata: `Metadata<Secure>` wrapped around a child that
//! must not appear in screenshots.
//!
//! Mirrors `WuiSecure`: a transparent `HostView` container — measure,
//! stretch, priority and the placement proposal all answer for the mounted
//! child — that on `UIKit` carries a nearly invisible secure text field
//! beneath the content, the secure-mode overlay the Swift leaf built, and
//! on `AppKit` relies on the window-level security the platform applies.

use alloc::rc::Rc;

use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect};
use waterui::metadata::secure::Secure;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HostView, text_field::SecureField};

/// The leaf's live state: the mounted child the layout face forwards to,
/// plus — `UIKit` only — the secure overlay that triggers screen-capture
/// redaction.
struct SecureState {
    /// The mounted content.
    child: Mounted,
    /// The masked field laid beneath the content (`UIKit` only).
    #[cfg(target_os = "ios")]
    overlay: cocoa_ui::Retained<SecureField>,
}

impl core::fmt::Debug for SecureState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SecureState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct SecureSubView {
    /// The leaf's state.
    state: Rc<SecureState>,
}

impl core::fmt::Debug for SecureSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SecureSubView").finish_non_exhaustive()
    }
}

impl SubView for SecureSubView {
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

/// Installs the `secure` handler on the dispatcher: `Metadata<Secure>` maps
/// to a transparent container whose `UIKit` secure-field overlay marks the
/// subtree as capture-protected.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Secure>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        // `UIKit`: the secure text field sits beneath the content —
        // `insertSubview(secureField, at: 0)` before `addSubview(contentView)`.
        // Nearly invisible, but enough to put the container into secure mode.
        #[cfg(target_os = "ios")]
        let overlay = {
            let field = SecureField::new(mtm);
            let field_view: &PlatformView = &field;
            view::set_user_interaction_enabled(field_view, false);
            view::set_alpha(field_view, 0.01);
            view::add_subview(host_view, field_view);
            field
        };

        let mounted = ctx.render(metadata.content).mount(host_view);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(SecureState {
            child: mounted,
            #[cfg(target_os = "ios")]
            overlay,
        });

        // Both platforms lay the secure field and the content over the full
        // bounds.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                let bounds = view::bounds(host);
                #[cfg(target_os = "ios")]
                {
                    let field_view: &PlatformView = &state.overlay;
                    view::set_frame(field_view, bounds);
                }
                view::set_frame(state.child.view(), bounds);
            }
        });

        // `setPlacementProposal`: the proposal selected for this wrapper is
        // the proposal its content was negotiated with.
        let sink_guard = proposal::register_sink(host_view, {
            let state = Rc::clone(&state);
            move |selected| {
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            SecureSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf
    });
}
