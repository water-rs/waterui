//! The `with_env` metadata: `Metadata<Environment>` wrapped around a
//! child, overlaying the environment the subtree resolves against.
//!
//! Mirrors `WuiWithEnv`: a transparent `HostView` container — measure,
//! stretch, priority and the placement proposal all answer for the mounted
//! child — whose only effect is that the content renders under the
//! metadata's environment, not the parent's. On `UIKit` the wrapper's
//! `tintColor` follows the accent slot of the *new* environment, exactly
//! the `WuiComputedObservation(themeColor: Accent)` the Swift leaf kept.
//! The wrapper forwards its primary content, as `WuiWithEnv` declared
//! through `WuiPrimaryContentProviding`.

use alloc::rc::Rc;

use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect};
use waterui_backend_core::Environment;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child the layout face forwards to.
struct WithEnvState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for WithEnvState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WithEnvState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct WithEnvSubView {
    /// The leaf's state.
    state: Rc<WithEnvState>,
}

impl core::fmt::Debug for WithEnvSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WithEnvSubView").finish_non_exhaustive()
    }
}

impl SubView for WithEnvSubView {
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

/// `wuiContentFrame(of: contentView, in: self)`: the whole bounds when the
/// content manages its own safe area, the host's safe-area rect otherwise.
fn content_frame(host: &HostView, child: &PlatformView) -> Rect {
    crate::native_layout::content_frame(child, host)
}

/// Installs the `with_env` handler on the dispatcher: `Metadata<Environment>`
/// maps to a transparent container that renders its content under the
/// metadata's environment.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Environment>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        // The metadata's environment replaces the subtree's — `WuiWithEnv`
        // resolved `metadata.content` against `metadata.value` alone.
        let env = metadata.value;
        let mounted = ctx.with_env(&env).render(metadata.content).mount(host_view);
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(WithEnvState { child: mounted });

        // `contentView.frame = wuiContentFrame(of:in:)`.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                let frame = content_frame(host, state.child.view());
                view::set_frame(state.child.view(), frame);
            }
        });

        // `WuiPrimaryContentProviding`: the primary-content chain descends
        // into the content.
        crate::primary_content::forward(&host, state.child.view());

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
            WithEnvSubView {
                state: Rc::clone(&state),
            },
        );

        #[cfg(target_os = "ios")]
        {
            use waterui::reactive::SignalExt;
            use waterui::resolve::Resolvable;
            use waterui::theme::color::Accent;

            // `WuiComputedObservation(themeColor: Accent, env: newEnv)`: the
            // wrapper's tint follows the overlaid environment's accent.
            let tint_view = view::retain_base(host_view);
            leaf.bind(&Accent.resolve(&env).computed(), move |color| {
                let accent = {
                    let [red, green, blue, alpha] = color.components;
                    cocoa_ui::uikit::colors::extended_linear_display_p3(
                        f64::from(red),
                        f64::from(green),
                        f64::from(blue),
                        f64::from(alpha),
                    )
                };
                view::set_tint_color(&tint_view, Some(&accent));
            });
        }

        leaf.keep(sink_guard);
        leaf.keep(state);
        // The overlaid environment outlives the handler: the child's signals
        // may resolve through it after `install` returns.
        leaf.keep(env);
        leaf
    });
}
