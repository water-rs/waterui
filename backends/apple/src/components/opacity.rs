//! The `opacity` metadata: `Metadata<Opacity>` wrapped around a child.
//!
//! Mirrors `WuiOpacity`: a transparent `HostView` container that sets the
//! wrapper's own alpha — `UIView.alpha` / `NSView.alphaValue` — so the whole
//! subtree composites at the metadata's opacity. Every change animates
//! through `withPlatformAnimation` and calls
//! `invalidateCapturedRendering` so a cached capture re-renders.

use alloc::rc::Rc;

use cocoa_ui::{PlatformView, Rect, view};
use waterui::animation::Animation;
use waterui::filter::Opacity;
use waterui::reactive::Signal;
use waterui::reactive::watcher::Metadata as WatchMetadata;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// `withPlatformAnimation`: the watcher metadata's `Animation` mapped to a
/// kit timing — `Default` parses to the 0.25s bezier the FFI spells it as.
fn with_platform_animation(metadata: &WatchMetadata, body: impl FnOnce() + 'static) {
    let timing = match metadata.try_get::<Animation>() {
        None => return body(),
        Some(Animation::Default) => cocoa_ui::core_animation::Timing::Bezier {
            duration: 0.25,
            control_points: [0.42, 0.0, 0.58, 1.0],
        },
        Some(Animation::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        }) => cocoa_ui::core_animation::Timing::Bezier {
            duration: duration.as_secs_f64(),
            control_points: [x1, y1, x2, y2],
        },
        Some(Animation::Spring { stiffness, damping }) => {
            cocoa_ui::core_animation::Timing::Spring {
                stiffness: f64::from(stiffness),
                damping: f64::from(damping),
            }
        }
    };
    cocoa_ui::core_animation::animate_with(timing, body);
}

/// `applyOpacity`: the alpha lands on the wrapper, then any captured
/// rendering is invalidated.
///
/// # Panics
///
/// Panics when `alpha` is outside `0.0…=1.0` — the same `precondition`
/// `WuiOpacity` declared.
fn apply_opacity(alpha: f32, host: &PlatformView) {
    assert!(
        (0.0..=1.0).contains(&alpha),
        "Metadata<Opacity> value out of range: {alpha}"
    );
    view::set_alpha(host, f64::from(alpha));
    view::invalidate_captured_rendering(host);
}

/// The leaf's live state: the mounted child.
struct OpacityState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for OpacityState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpacityState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: the content's answers everywhere.
struct OpacitySubView {
    /// The leaf's state.
    state: Rc<OpacityState>,
}

impl core::fmt::Debug for OpacitySubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpacitySubView").finish_non_exhaustive()
    }
}

impl SubView for OpacitySubView {
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

/// Installs the `opacity` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Opacity>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(OpacityState { child: mounted });

        // The content always fills the wrapper.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host_view| {
                view::set_frame(state.child.view(), view::bounds(host_view));
            }
        });

        // `setPlacementProposal`: the proposal selected for this wrapper is
        // the proposal its content was negotiated with.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            move |selected| {
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            &*host,
            OpacitySubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);

        let value = metadata.value.value;
        apply_opacity(value.snapshot(), &host);
        leaf.watch(&value, {
            move |wctx| {
                let next = *wctx.value();
                with_platform_animation(wctx.metadata(), {
                    let host = host.clone();
                    move || apply_opacity(next, &host)
                });
            }
        });
        leaf.keep(state);
        leaf
    });
}
