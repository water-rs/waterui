//! The `rotation` metadata: `Metadata<Rotation>` wrapped around a child.
//!
//! Mirrors `WuiRotation`: a transparent `HostView` container that rotates
//! its content around `anchor` without touching layout. `UIKit` sets the
//! layer's anchor point and writes a `CGAffineTransform` rotation on the
//! content; `AppKit` lays the content out for the anchor and writes a
//! `CATransform3D` rotation on its layer — explicitly animated when the
//! watcher metadata carries an `Animation`. Every change calls
//! `invalidateCapturedRendering` so a cached capture re-renders.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::{Point, Rect, view};
use waterui::reactive::Signal;
use waterui::style::Rotation;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::components::layer_transform;
use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child and the current angle.
struct RotationState {
    /// The mounted content.
    child: Mounted,
    /// The current angle in degrees.
    angle: f32,
    /// The content size the anchor layout was last applied at — `AppKit`
    /// only, [`layer_transform::transformed_content_layer`]'s staleness key.
    #[cfg(target_os = "macos")]
    last_bounds_size: cocoa_ui::Size,
}

impl core::fmt::Debug for RotationState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RotationState").finish_non_exhaustive()
    }
}

/// `applyTransform` — the rotation lands on the content, then any captured
/// rendering is invalidated.
#[cfg_attr(target_os = "ios", allow(clippy::needless_pass_by_ref_mut))]
fn apply_transform(state: &mut RotationState, host: &HostView, anchor: Point) {
    #[cfg(target_os = "ios")]
    {
        let _ = (host, anchor);
        let radians = f64::from(state.angle).to_radians();
        view::set_transform(
            state.child.view(),
            cocoa_ui::objc2_core_graphics::CGAffineTransformMakeRotation(radians),
        );
    }
    #[cfg(target_os = "macos")]
    {
        let radians = f64::from(state.angle).to_radians();
        let child = view::retain_base(state.child.view());
        let layer = layer_transform::transformed_content_layer(
            &child,
            view::bounds(host),
            anchor,
            &mut state.last_bounds_size,
        );
        let transform =
            cocoa_ui::objc2_quartz_core::CATransform3D::new_rotation(radians, 0.0, 0.0, 1.0);
        cocoa_ui::core_animation::set_layer_transform(&layer, transform);
    }
    view::invalidate_captured_rendering(host);
}

/// The wrapper's layout face: the content's answers everywhere.
struct RotationSubView {
    /// The leaf's state.
    state: Rc<RefCell<RotationState>>,
}

impl core::fmt::Debug for RotationSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RotationSubView").finish_non_exhaustive()
    }
}

impl SubView for RotationSubView {
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

/// Installs the `rotation` handler on the dispatcher.
#[allow(clippy::too_many_lines)]
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Rotation>>(|metadata, ctx| {
        let rotation = metadata.value;
        let anchor = Point::new(f64::from(rotation.anchor.x), f64::from(rotation.anchor.y));

        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);
        // The transform needs a layer-backed content on `AppKit`; `UIKit`
        // views always have one.
        cocoa_ui::layer::ensure_layer(&host);
        cocoa_ui::layer::ensure_layer(mounted.view());

        let state = Rc::new(RefCell::new(RotationState {
            child: mounted,
            angle: 0.0,
            #[cfg(target_os = "macos")]
            last_bounds_size: cocoa_ui::Size::new(0.0, 0.0),
        }));

        // The content always fills the wrapper before the transform moves
        // it visually.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host_view| {
                #[cfg(target_os = "ios")]
                let state = state.borrow();
                #[cfg(target_os = "macos")]
                let mut state = state.borrow_mut();
                let bounds = view::bounds(host_view);
                #[cfg(target_os = "ios")]
                {
                    if let Some(layer) = cocoa_ui::layer::layer_of(state.child.view()) {
                        cocoa_ui::layer::set_anchor_point(&layer, anchor);
                    }
                    view::set_bounds(
                        state.child.view(),
                        Rect::new(0.0, 0.0, bounds.size.width, bounds.size.height),
                    );
                    view::set_center(
                        state.child.view(),
                        Point::new(bounds.size.width * anchor.x, bounds.size.height * anchor.y),
                    );
                }
                #[cfg(target_os = "macos")]
                {
                    let child = view::retain_base(state.child.view());
                    if cocoa_ui::layer::layout_transformed_content(
                        &child,
                        bounds,
                        anchor,
                        &mut state.last_bounds_size,
                    ) {
                        apply_transform(&mut state, host_view, anchor);
                    }
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
            RotationSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);

        let angle = rotation.angle;
        {
            let mut borrowed = state.borrow_mut();
            borrowed.angle = angle.snapshot();
            apply_transform(&mut borrowed, &host, anchor);
        }

        leaf.watch(&angle, {
            let state = Rc::clone(&state);
            move |wctx| {
                state.borrow_mut().angle = *wctx.value();
                #[cfg(target_os = "ios")]
                layer_transform::apply_with_animation(wctx.metadata(), {
                    let state = Rc::clone(&state);
                    let host = host.clone();
                    move || {
                        let mut borrowed = state.borrow_mut();
                        apply_transform(&mut borrowed, &host, anchor);
                    }
                });
                #[cfg(target_os = "macos")]
                {
                    let mut borrowed = state.borrow_mut();
                    let child = view::retain_base(borrowed.child.view());
                    let layer = layer_transform::transformed_content_layer(
                        &child,
                        view::bounds(&host),
                        anchor,
                        &mut borrowed.last_bounds_size,
                    );
                    let radians = f64::from(borrowed.angle).to_radians();
                    let transform = cocoa_ui::objc2_quartz_core::CATransform3D::new_rotation(
                        radians, 0.0, 0.0, 1.0,
                    );
                    layer_transform::animate_layer_transform(
                        &layer,
                        transform,
                        "wuiRotation",
                        wctx.metadata(),
                    );
                    view::invalidate_captured_rendering(&host);
                }
            }
        });
        leaf.keep(state);
        leaf
    });
}
