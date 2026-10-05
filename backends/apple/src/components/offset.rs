//! The `offset` metadata: `Metadata<Offset>` wrapped around a child.
//!
//! Mirrors `WuiOffset`: a transparent `HostView` container that moves its
//! content without touching layout. `UIKit` writes a translation transform
//! on the content; `AppKit` moves the content's frame inside this view —
//! `AppKit` owns the geometry of a layer-backed view's layer and rewrites it
//! on every layout pass, so only a frame move survives. Every change calls
//! `invalidateCapturedRendering` so a cached capture re-renders.

use alloc::rc::Rc;
use core::cell::RefCell;

#[cfg(target_os = "ios")]
use cocoa_ui::Point;
use cocoa_ui::{PlatformView, Rect, view};
use waterui::animation::Animation;
use waterui::reactive::Signal;
use waterui::reactive::watcher::Metadata as WatchMetadata;
use waterui::style::Offset;
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

/// The leaf's live state: the mounted child and the current translation.
struct OffsetState {
    /// The mounted content.
    child: Mounted,
    /// The current x translation.
    x: f32,
    /// The current y translation.
    y: f32,
}

impl core::fmt::Debug for OffsetState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OffsetState").finish_non_exhaustive()
    }
}

/// `applyTransform` — the platform-specific way the translation lands.
fn apply_transform(state: &OffsetState, host: &PlatformView) {
    let x = f64::from(state.x);
    let y = f64::from(state.y);
    let child = state.child.view();

    #[cfg(target_os = "ios")]
    {
        // A translation transform: the content's own bounds and center keep
        // it laid out while the transform moves it visually.
        view::set_transform(
            child,
            cocoa_ui::objc2_core_graphics::CGAffineTransformMakeTranslation(x, y),
        );
    }
    #[cfg(target_os = "macos")]
    {
        // `AppKit` rewrites a layer-backed view's layer on layout, so the
        // translation is the content's frame inside this view; an
        // `NSAnimationContext` animates the frame change implicitly.
        let bounds = view::bounds(host);
        view::set_frame(
            child,
            Rect::new(
                bounds.origin.x + x,
                bounds.origin.y + y,
                bounds.size.width,
                bounds.size.height,
            ),
        );
    }
    view::invalidate_captured_rendering(host);
}

/// The wrapper's layout face: the content's answers everywhere.
struct OffsetSubView {
    /// The leaf's state.
    state: Rc<RefCell<OffsetState>>,
}

impl core::fmt::Debug for OffsetSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OffsetSubView").finish_non_exhaustive()
    }
}

impl SubView for OffsetSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.borrow().child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.borrow().child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.borrow().child.layout().is_empty()
    }
}

/// Installs the `offset` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Offset>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        // A layer keeps the offset content composited rather than redrawn.
        cocoa_ui::layer::ensure_layer(&host);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(RefCell::new(OffsetState {
            child: mounted,
            x: 0.0,
            y: 0.0,
        }));

        // The content always fills the wrapper before the transform moves
        // it visually.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host_view| {
                let state = state.borrow();
                #[cfg(target_os = "ios")]
                let bounds = view::bounds(host_view);
                #[cfg(target_os = "ios")]
                {
                    view::set_bounds(
                        state.child.view(),
                        Rect::new(0.0, 0.0, bounds.size.width, bounds.size.height),
                    );
                    view::set_center(
                        state.child.view(),
                        Point::new(bounds.size.width / 2.0, bounds.size.height / 2.0),
                    );
                }
                #[cfg(target_os = "macos")]
                {
                    apply_transform(&state, host_view);
                }
            }
        });

        // `setPlacementProposal`: the proposal selected for this wrapper is
        // the proposal its content was negotiated with.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            move |selected| {
                let state = state.borrow();
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            &*host,
            OffsetSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);

        let offset = metadata.value;
        let x = offset.x;
        let y = offset.y;

        {
            let mut s = state.borrow_mut();
            s.x = x.snapshot();
            s.y = y.snapshot();
        }
        {
            let state = state.borrow();
            apply_transform(&state, &host);
        }

        leaf.watch(&x, {
            let state = Rc::clone(&state);
            let host = host.clone();
            move |wctx| {
                state.borrow_mut().x = *wctx.value();
                with_platform_animation(wctx.metadata(), {
                    let state = Rc::clone(&state);
                    let host = host.clone();
                    move || {
                        let state = state.borrow();
                        apply_transform(&state, &host);
                    }
                });
            }
        });
        leaf.watch(&y, {
            let state = Rc::clone(&state);
            move |wctx| {
                state.borrow_mut().y = *wctx.value();
                with_platform_animation(wctx.metadata(), {
                    let state = Rc::clone(&state);
                    let host = host.clone();
                    move || {
                        let state = state.borrow();
                        apply_transform(&state, &host);
                    }
                });
            }
        });
        leaf.keep(state);
        leaf
    });
}
