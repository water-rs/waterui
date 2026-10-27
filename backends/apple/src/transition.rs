//! Structural view transitions in the Apple backend (docs/layout-spec.md §4.5).
//!
//! A `.transition(...)` declaration is stored on the leaf its handler
//! renders as a [`TransitionClaim`]. The structural site that swaps the
//! mount — a `Dynamic` host or a container's id-keyed reconcile — claims
//! the declaration at the root of the mounted subtree: a declaration
//! beneath another modifier is never claimed, matching the rule every
//! backend shares.
//!
//! Removal splits semantic lifetime from visual resources: the site
//! retains the platform view, drops the leaf — ending every watcher and
//! subscription — then strips the ghost of focus, interaction and
//! accessibility membership before driving its exit animation. Property
//! transitions project onto Core Animation (`AppKit`) / `UIView`
//! animations (`UIKit`); the ghost's `TransitionState` clock — a
//! `FrameClock` on its window — decides when the slot and view release.
//!
//! Pixel (`captures_content`) transitions run the removal through the
//! filtered pipeline: the outgoing view is frozen once into an
//! `MTLTexture` by `cocoa_ui::capture::ViewCapture` — no CPU readback —
//! and the effect body `TransitionState::body` builds mounts inside a
//! non-interactive `HostView` overlay at window level, where
//! [`filtered::install_frozen_input`] makes the frozen texture the
//! effect's input. The ghost's `FrameClock` advances `progress`, pushes
//! a redraw into the filtered leaf and republishes `TransitionFrame`
//! every tick, so `source` follows the slot through scroll and resize.
//! A capture that fails is an error — never a property fallback.

/// The panic policy for transition contract violations — states user
/// code can trigger, like an unowned `.transition(...)` declaration or a
/// failed pixel capture. A debug build panics with the message; a
/// release build logs it and the caller continues to the violation's
/// defined end state (a dropped claim, a completed instance), since a
/// GUI framework must not crash a shipped app. Internal invariants —
/// states that cannot occur — keep `unreachable!`/`expect`.
macro_rules! transition_violation {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            panic!($($arg)*);
        } else {
            tracing::error!($($arg)*);
        }
    };
}

pub(crate) use transition_violation;

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use std::time::Instant;

use cocoa_ui::display_link::FrameClock;
use cocoa_ui::{PlatformView, Retained, focus, view};
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
use waterui::reactive::{Binding, SignalExt};
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
use waterui_backend_core::AnyView;
use waterui_core::layout::{LayoutDirection, Size};
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
use waterui_core::layout::{Point, Rect as LayoutRect};
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
use waterui_core::transition::TransitionFrame;
use waterui_core::transition::{
    GhostSlot, TransitionGeometry, TransitionPhase, TransitionProperties, TransitionSpec,
    TransitionState,
};

/// The `.transition(...)` declaration a leaf carries until a structural
/// site claims it: the spec plus the Reduce Motion resolution made where
/// the leaf rendered — each clock resolves it at `start`, as the contract
/// requires.
#[derive(Clone)]
pub struct TransitionClaim {
    /// The declared effect and timing.
    pub spec: TransitionSpec,
    /// The environment's Reduce Motion answer at render.
    pub reduce_motion: bool,
}

impl core::fmt::Debug for TransitionClaim {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransitionClaim").finish_non_exhaustive()
    }
}

/// Marks a value as crossing the capture-completion boundary; the
/// wrapped object is only dereferenced once the completion lands on the
/// main thread, where every receiver was created — the same contract
/// `filtered.rs` spells for its GPU-completion hops.
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
struct Sendable<T>(T);

#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
impl<T> Sendable<T> {
    /// Reads the wrapped value on the main thread.
    const fn get(&self) -> &T {
        &self.0
    }
}

// SAFETY: the payload is only touched on the main thread — capture
// completions run on the main queue by `ViewCapture`'s contract.
#[allow(clippy::non_send_fields_in_send_ty)]
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
unsafe impl<T> Send for Sendable<T> {}

/// Rejects a sampled endpoint with a nonzero blur radius: no Apple layer
/// can take an arbitrary-radius blur without the GPU capture path (`UIKit`
/// supports no `CALayer.filters`; `AppKit`'s `CIFilter` route is
/// feature-gated behind `applied_filter`). An unsupported effect panics —
/// it is never skipped silently.
fn assert_no_blur(properties: &TransitionProperties) {
    assert!(
        properties.blur == 0.0,
        "waterui-apple: a transition sampled blur radius {}, and blur transitions are not \
         realized on this backend yet — they need the capture path the pixel overlay provides. \
         Use opacity, scale, move or rotation",
        properties.blur
    );
}

/// Every view in `root`'s subtree, pre-order — the ghost deactivation walk.
fn for_each_descendant(root: &PlatformView, f: &mut impl FnMut(&PlatformView)) {
    f(root);
    for subview in view::subviews(root) {
        for_each_descendant(&subview, f);
    }
}

/// Strips the outgoing subtree of everything semantic membership gave it:
/// keyboard focus, hit testing and accessibility identity. Runs after the
/// leaf has dropped — watchers and subscriptions are already dead; this
/// leaves only visual resources, per the ghost contract.
///
/// `UIKit` turns interaction off per view; `AppKit` has no subtree-wide
/// interaction flag, so a `HostView` answers `HitTest::Pass` — the event
/// falls to whatever lies beneath — and any `NSControl` is disabled.
fn deactivate(view: &PlatformView) {
    for_each_descendant(view, &mut |view| {
        let _ = focus::resign_first_responder(view);
        #[cfg(target_os = "ios")]
        view::set_user_interaction_enabled(view, false);
        #[cfg(target_os = "macos")]
        {
            if let Some(host) = view.downcast_ref::<cocoa_ui::appkit::HostView>() {
                host.set_hit_test_handler(|_host, _point| cocoa_ui::appkit::HitTest::Pass);
            }
            if let Some(control) = view.downcast_ref::<cocoa_ui::objc2_app_kit::NSControl>() {
                control.setEnabled(false);
            }
        }
        view::hide_from_accessibility(view);
    });
}

/// Splits a removed leaf into its live platform view and its transition
/// claim. Dropping the leaf here is the semantic boundary — every watcher,
/// subscription and kept object ends now — while the retained view keeps
/// the visual resources a ghost needs. The view is deactivated before the
/// caller reparents it anywhere.
pub fn retire(
    leaf: crate::contract::NativeLeaf,
) -> (Retained<PlatformView>, Option<TransitionClaim>) {
    let view = view::retain_base(leaf.view());
    let mut leaf = leaf;
    let claim = leaf.take_transition();
    drop(leaf);
    deactivate(&view);
    (view, claim)
}

/// The transform `properties` applies around the centre of a `size` slot:
/// scale and rotation about the centre, then translation — the documented
/// `TransitionProperties` compose order, in top-left coordinates.
#[cfg(target_os = "macos")]
fn layer_transform(
    properties: &TransitionProperties,
    size: cocoa_ui::Size,
) -> objc2_quartz_core::CATransform3D {
    use objc2_quartz_core::CATransform3D;
    let cx = size.width * 0.5;
    let cy = size.height * 0.5;
    let translation = CATransform3D::new_translation(
        f64::from(properties.translation[0]),
        f64::from(properties.translation[1]),
        0.0,
    );
    let to_centre = CATransform3D::new_translation(cx, cy, 0.0);
    let rotate = CATransform3D::new_rotation(f64::from(properties.rotation), 0.0, 0.0, 1.0);
    let scale = CATransform3D::new_scale(
        f64::from(properties.scale[0]),
        f64::from(properties.scale[1]),
        1.0,
    );
    let back = CATransform3D::new_translation(-cx, -cy, 0.0);
    translation
        .concat(to_centre)
        .concat(rotate.concat(scale).concat(back))
}

/// The `CGAffineTransform` spelling of [`layer_transform`] for `UIKit`:
/// `UIView.transform` already applies about the view's centre.
#[cfg(target_os = "ios")]
fn affine_transform(properties: &TransitionProperties) -> objc2_core_foundation::CGAffineTransform {
    // `UIView.transform` applies about the view's centre already, so the
    // translation carries the move alone — scale, then rotation, then
    // translation, in `CGAffineTransform`'s row-vector spelling.
    let (sin, cos) = f64::from(properties.rotation).sin_cos();
    let sx = f64::from(properties.scale[0]);
    let sy = f64::from(properties.scale[1]);
    objc2_core_foundation::CGAffineTransform {
        a: sx * cos,
        b: sx * sin,
        c: -sy * sin,
        d: sy * cos,
        tx: f64::from(properties.translation[0]),
        ty: f64::from(properties.translation[1]),
    }
}

/// Writes `properties` onto `view` immediately — the absent-state
/// snapshot an insertion starts from.
fn apply_properties(view: &PlatformView, properties: &TransitionProperties) {
    #[cfg(target_os = "macos")]
    {
        view::ensure_layer_backed(view);
        let layer = view::layer(view).expect("a layer-backed view");
        let size =
            cocoa_ui::geometry::Size::new(layer.bounds().size.width, layer.bounds().size.height);
        cocoa_ui::core_animation::set_layer_transform(&layer, layer_transform(properties, size));
        layer.setOpacity(properties.opacity);
    }
    #[cfg(target_os = "ios")]
    {
        view::set_transform(view, affine_transform(properties));
        view::set_alpha(view, f64::from(properties.opacity));
    }
}

/// Animates `view`'s alpha and transform to `properties` under `timing` —
/// the Core Animation / `UIView` projection of a sampled endpoint.
fn animate_to(
    view: &PlatformView,
    properties: &TransitionProperties,
    timing: cocoa_ui::core_animation::Timing,
) {
    #[cfg(target_os = "macos")]
    {
        view::ensure_layer_backed(view);
        let layer = view::layer(view).expect("a layer-backed view");
        let size =
            cocoa_ui::geometry::Size::new(layer.bounds().size.width, layer.bounds().size.height);
        let transform = layer_transform(properties, size);
        let opacity = properties.opacity;
        cocoa_ui::core_animation::animate_with(timing, move || {
            layer.setOpacity(opacity);
        });
        if let Some(layer) = view::layer(view) {
            // The explicit `CAAnimation` survives the layout passes that
            // rewrite the layer's geometry.
            cocoa_ui::core_animation::animate_layer_transform(
                &layer,
                transform,
                "waterui.transition.transform",
                Some(timing),
            );
        }
    }
    #[cfg(target_os = "ios")]
    {
        let transform = affine_transform(properties);
        let alpha = f64::from(properties.opacity);
        let view = view::retain_base(view);
        cocoa_ui::core_animation::animate_with(timing, move || {
            view::set_transform(&view, transform);
            view::set_alpha(&view, alpha);
        });
    }
}

/// Plays a property transition on `view` under `state`'s timing: removal
/// animates to the absent endpoint, insertion snaps there and animates
/// back to identity — `TransitionPhase`'s played-backwards rule. `size`
/// and `direction` describe the slot the transition resolves in.
fn play_property_transition(
    view: &PlatformView,
    state: &TransitionState,
    phase: TransitionPhase,
    size: cocoa_ui::Size,
    direction: LayoutDirection,
) {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; kit geometry is f64"
    )]
    let geometry =
        TransitionGeometry::new(Size::new(size.width as f32, size.height as f32), direction);
    let absent = state.absent_properties(geometry);
    assert_no_blur(&absent);
    let timing = crate::animation::timing(state.animation());
    match phase {
        TransitionPhase::Removal => animate_to(view, &absent, timing),
        TransitionPhase::Insertion => {
            apply_properties(view, &absent);
            animate_to(view, &TransitionProperties::IDENTITY, timing);
        }
    }
}

/// The view's own bounds size and layout direction — resolved at play
/// time, once a real layout frame exists.
fn geometry_of(view: &PlatformView) -> (cocoa_ui::Size, LayoutDirection) {
    let direction = if view::is_right_to_left(view) {
        LayoutDirection::RightToLeft
    } else {
        LayoutDirection::LeftToRight
    };
    (view::bounds(view).size, direction)
}

/// Starts the insertion of a freshly mounted child: the property
/// projection is installed once the view's first layout frame is on
/// screen — `on_commit` fires when the current transaction commits, the
/// point the added layer exists — so the absent snapshot and the return
/// animation read real geometry.
pub fn play_insertion(view: &PlatformView, claim: &TransitionClaim) {
    let state = claim
        .spec
        .start(TransitionPhase::Insertion, claim.reduce_motion);
    // Insertion `captures_content` keeps its declared contract — live
    // capture through the ordinary view-effect path the body's own
    // `FilteredView` realizes — while the property endpoints the ghost
    // mirrors stay guarded below.
    assert!(
        !state.captures_content(),
        "waterui-apple: pixel capture on insertion is not realized yet — the live-capture \
         effect body needs the mounted content's own window overlay; use a property insertion \
         (opacity, scale, move, rotation) or an asymmetric transition"
    );
    let view = view::retain_base(view);
    cocoa_ui::core_animation::on_commit(move || {
        let (size, direction) = geometry_of(&view);
        play_property_transition(&view, &state, TransitionPhase::Insertion, size, direction);
    });
}

/// A removal transition in flight: the ghost view, the layout slot it
/// retains, and the clock ending both. The site owns one per ghosted
/// member; dropping it releases the platform objects.
pub struct RemovalGhost {
    /// The retained outgoing view — visual resources only. For a pixel
    /// transition it stays attached and hidden as the slot's placement
    /// anchor: containers keep framing it, and the overlay reads its
    /// window-space bounds for `TransitionFrame.source` every tick.
    pub view: Retained<PlatformView>,
    /// The frozen layout membership, for sites that hold a slot;
    /// conditional replacement holds none.
    pub slot: Option<GhostSlot>,
    /// The removal clock, shared with the display driver.
    ///
    /// Held, not read: the site's only interaction with a ghost is holding
    /// the record until `start_removal`'s completion fires.
    #[allow(dead_code)]
    pub state: Rc<RefCell<TransitionState>>,
    /// The display clock advancing `state`; holding it keeps the ghost
    /// ticking.
    #[allow(dead_code)]
    clock: FrameClock,
    /// The pixel path's retained objects — window overlay, rendered
    /// effect leaf and the one-shot capture. `None` for a property
    /// transition, and absent entirely in a build without the filtered
    /// pipeline.
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    pixel: Option<Rc<PixelGhost>>,
}

impl core::fmt::Debug for RemovalGhost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemovalGhost").finish_non_exhaustive()
    }
}

impl Drop for RemovalGhost {
    /// Releasing the ghost ends the overlay's screen membership — the
    /// `Mounted` inside detaches the filtered view, and the overlay
    /// itself leaves its window here. The outgoing view is the site's
    /// to detach; its completion does that before this record drops.
    fn drop(&mut self) {
        #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
        if let Some(pixel) = self.pixel.take() {
            pixel.capture.shutdown();
            view::remove_from_superview(&pixel.overlay);
        }
    }
}

/// One pixel removal's retained machinery — everything the window
/// overlay needs until the transition clock ends. Owned by
/// [`RemovalGhost`]; the capture completion and the frame clock reach it
/// through a `Weak`.
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
struct PixelGhost {
    /// The non-interactive window-level container the effect leaf mounts
    /// into — a `HostView` so its hit test falls through on `AppKit` and
    /// one flag covers the subtree on `UIKit`.
    overlay: Retained<PlatformView>,
    /// The rendered transition body — `FilteredView` inside — parked
    /// until the frozen texture lands and `filtered::install_frozen_input`
    /// accepts it.
    leaf: RefCell<Option<crate::contract::NativeLeaf>>,
    /// The mounted effect leaf, once its input is installed.
    mounted: RefCell<Option<crate::contract::Mounted>>,
    /// The one-shot capture over the outgoing view.
    capture: Rc<cocoa_ui::capture::ViewCapture>,
    /// The current capture attempt's target — becomes the installed
    /// input when it lands.
    target: RefCell<Option<Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>>>>,
    /// The filtered state the frozen input was installed into — the
    /// tick's redraw target.
    filtered: RefCell<Option<Rc<crate::components::filtered::FilteredState>>>,
    /// The frame signal feeding the effect body — republished every tick.
    frame: Binding<TransitionFrame>,
    /// A capture attempt is outstanding — the next tick does not re-issue.
    in_flight: Cell<bool>,
    /// The texture landed and the leaf mounted.
    landed: Cell<bool>,
}

#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
impl core::fmt::Debug for PixelGhost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PixelGhost").finish_non_exhaustive()
    }
}

/// The slot's current rect in top-left window logical coordinates — the
/// anchor view's window-space bounds. `AppKit` answers bottom-left, so
/// the origin mirrors against the window's content height.
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
#[expect(
    clippy::cast_possible_truncation,
    reason = "window-space coordinates and scale fit f32"
)]
fn source_in_window(anchor: &PlatformView) -> LayoutRect {
    let rect = view::bounds_in_window(anchor);
    #[cfg(target_os = "macos")]
    let y = {
        let height = view::window(anchor)
            .and_then(|window| window.contentView())
            .map_or(0.0, |content| view::bounds(&content).size.height);
        height - rect.origin.y - rect.size.height
    };
    #[cfg(target_os = "ios")]
    let y = rect.origin.y;
    LayoutRect::new(
        Point::new(rect.origin.x as f32, y as f32),
        Size::new(rect.size.width as f32, rect.size.height as f32),
    )
}

/// Republishes the ghost's `TransitionFrame` — the slot in window
/// coordinates, the overlay size, the backing scale and the layout
/// direction — so the effect follows scrolling and resizing between
/// frames (§4.5).
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
#[expect(
    clippy::cast_possible_truncation,
    reason = "window-space coordinates and scale fit f32"
)]
fn publish_frame(pixel: &PixelGhost, anchor: &PlatformView) {
    let window_size = view::bounds(&pixel.overlay).size;
    pixel.frame.set(TransitionFrame::new(
        source_in_window(anchor),
        Size::new(window_size.width as f32, window_size.height as f32),
        view::backing_scale_factor(anchor) as f32,
        if view::is_right_to_left(anchor) {
            LayoutDirection::RightToLeft
        } else {
            LayoutDirection::LeftToRight
        },
    ));
}

/// The view the window-level overlay mounts into — `contentView` on
/// `AppKit`, the `UIWindow` itself on `UIKit`.
#[cfg(all(
    target_os = "macos",
    any(feature = "applied_filter", feature = "view_effect")
))]
fn overlay_parent(view: &PlatformView) -> Option<Retained<PlatformView>> {
    view::window(view)
        .and_then(|window| window.contentView())
        .map(|content| view::retain_base(&content))
}

/// The view the window-level overlay mounts into — the `UIWindow`
/// itself, which is a `UIView`.
#[cfg(all(
    target_os = "ios",
    any(feature = "applied_filter", feature = "view_effect")
))]
fn overlay_parent(view: &PlatformView) -> Option<Retained<PlatformView>> {
    view::window(view).map(|window| view::retain_base(&window))
}

/// Starts a `captures_content` removal: freeze the outgoing view once
/// into a private `MTLTexture` (no CPU readback), render the effect body
/// the `TransitionState` builds, install the frozen texture as its
/// input and mount it in a window-level overlay above content. A
/// `Deferred` capture retries on the next clock tick under the current
/// context generation; a `Failed` capture is the contract's error —
/// never a property fallback.
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
#[expect(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    reason = "the pixel ghost is one wiring pass; window-space coordinates fit f32"
)]
fn start_pixel_removal(
    mtm: cocoa_ui::MainThreadMarker,
    view: &Retained<PlatformView>,
    renderer: &crate::contract::Renderer,
    state: TransitionState,
    slot: Option<GhostSlot>,
    completion: impl FnOnce() + 'static,
) -> RemovalGhost {
    let Some(parent) = overlay_parent(view) else {
        panic!(
            "waterui-apple: a pixel removal needs its outgoing view in a window — the overlay it draws through has nowhere to mount"
        );
    };
    let overlay = {
        #[cfg(target_os = "macos")]
        let overlay = cocoa_ui::appkit::HostView::new(mtm, cocoa_ui::Rect::ZERO);
        #[cfg(target_os = "ios")]
        let overlay = cocoa_ui::uikit::HostView::new(mtm, cocoa_ui::Rect::ZERO);
        #[cfg(target_os = "macos")]
        overlay.set_hit_test_handler(|_host, _point| cocoa_ui::appkit::HitTest::Pass);
        let overlay = view::retain_base(&overlay);
        #[cfg(target_os = "ios")]
        view::set_user_interaction_enabled(&overlay, false);
        view::hide_from_accessibility(&overlay);
        view::set_frame(&overlay, view::bounds(&parent));
        view::set_autoresizing_flexible_size(&overlay);
        view::add_subview(&parent, &overlay);
        overlay
    };

    // The frame signal the effect body reads — the first tick republishes
    // it against the live anchor, so the start value only seeds the type.
    let frame = Binding::container(TransitionFrame::new(
        source_in_window(view),
        Size::new(
            view::bounds(&parent).size.width as f32,
            view::bounds(&parent).size.height as f32,
        ),
        view::backing_scale_factor(view) as f32,
        if view::is_right_to_left(view) {
            LayoutDirection::RightToLeft
        } else {
            LayoutDirection::LeftToRight
        },
    ));
    // The backend owns the captured realization, so the body takes empty
    // content and receives the frozen texture as its effect input.
    let leaf = renderer.render(state.body(AnyView::new(()), frame.computed()));

    let capture = Rc::new(cocoa_ui::capture::ViewCapture::new(mtm, view.clone()));
    // The contract only asks the redraw hook to exist; a frozen source
    // never re-captures, so surface redraws during the one-shot scope
    // change nothing it would sample twice.
    capture.set_on_redraw(|| {});
    let pixel = Rc::new(PixelGhost {
        overlay,
        leaf: RefCell::new(Some(leaf)),
        mounted: RefCell::new(None),
        capture,
        target: RefCell::new(None),
        filtered: RefCell::new(None),
        frame,
        in_flight: Cell::new(false),
        landed: Cell::new(false),
    });

    // One capture attempt: allocate the target on the context generation
    // live right now, then hand it to `ViewCapture`. The runtime is
    // resolved once here — it survives context generations, which the
    // retry re-binds by re-allocating the target.
    let runtime = crate::gpu_runtime::runtime(renderer.context().env());
    let attempt = {
        let pixel = Rc::downgrade(&pixel);
        let view = view.clone();
        move || {
            let Some(pixel) = pixel.upgrade() else {
                return;
            };
            let context = runtime.context();
            let scale = view::backing_scale_factor(&view);
            let (width, height) = crate::components::filtered::pixel_size(&view, scale);
            assert!(
                width > 0 && height > 0,
                "waterui-apple: a pixel removal captured a zero-size slot — the outgoing view \
                 must still carry its removal frame when `start_removal` runs"
            );
            let target = crate::components::filtered::new_capture_input(&context, width, height);
            let generation = context.generation();
            *pixel.target.borrow_mut() = Some(target);
            pixel.in_flight.set(true);
            let weak = Sendable(Rc::downgrade(&pixel));
            let anchor = Sendable(view.clone());
            pixel.capture.capture(
                pixel.target.borrow().as_ref().expect("target just stored"),
                generation,
                move |outcome| {
                    let Some(pixel) = weak.get().upgrade() else {
                        return;
                    };
                    pixel.in_flight.set(false);
                    match outcome {
                        Ok(()) => {
                            let anchor = anchor.get();
                            // The frozen frame exists: the original stops
                            // drawing and the filtered overlay takes over.
                            view::set_hidden(anchor, true);
                            let texture =
                                pixel.target.borrow_mut().take().expect("the landed target");
                            let leaf = pixel.leaf.borrow_mut().take().expect("one landing");
                            let Some(filtered) = crate::components::filtered::install_frozen_input(
                                leaf.view(),
                                texture,
                            ) else {
                                panic!(
                                    "waterui-apple: a `captures_content` removal's body produced \
                                     no filtered surface to take the frozen input"
                                );
                            };
                            *pixel.filtered.borrow_mut() = Some(filtered);
                            view::set_frame(leaf.view(), cocoa_ui::view::bounds(&pixel.overlay));
                            view::set_autoresizing_flexible_size(leaf.view());
                            *pixel.mounted.borrow_mut() = Some(leaf.mount(&pixel.overlay));
                            crate::components::filtered::request_render(
                                &pixel.filtered.borrow().clone().expect("installed"),
                            );
                            pixel.landed.set(true);
                            pixel.capture.shutdown();
                        }
                        Err(cocoa_ui::capture::CaptureError::Deferred) => {
                            // Retryable — the next clock tick re-issues on
                            // the then-current context generation.
                            pixel.target.borrow_mut().take();
                        }
                        Err(cocoa_ui::capture::CaptureError::Failed(error)) => {
                            panic!(
                                "waterui-apple: pixel removal capture failed: {error} — a failed \
                                 capture is an error, never a property fallback"
                            );
                        }
                    }
                },
            );
        }
    };
    attempt();
    let attempt = Sendable(attempt);

    let completion = Rc::new(RefCell::new(Some(completion)));
    let driver = Rc::new(RefCell::new(state));
    let last = Cell::new(Instant::now());
    let clock = FrameClock::new(mtm, {
        let driver = Rc::clone(&driver);
        let completion = Rc::clone(&completion);
        let pixel = Rc::downgrade(&pixel);
        let anchor = view.clone();
        move || {
            let Some(pixel) = pixel.upgrade() else {
                return;
            };
            publish_frame(&pixel, &anchor);
            if pixel.landed.get() {
                if let Some(filtered) = pixel.filtered.borrow().as_ref() {
                    crate::components::filtered::request_render(filtered);
                }
            } else if !pixel.in_flight.get() {
                (attempt.get())();
            }
            let now = Instant::now();
            let delta = now.duration_since(last.replace(now));
            let done = !driver.borrow_mut().advance(delta);
            if done && let Some(complete) = completion.borrow_mut().take() {
                complete();
            }
        }
    });
    clock.start(view);
    RemovalGhost {
        view: view.clone(),
        slot,
        state: driver,
        clock,
        #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
        pixel: Some(pixel),
    }
}

/// Starts the removal of `view` — already [`retire`]d — under `claim`'s
/// effect. `slot` freezes the layout membership for collection sites;
/// `None` for conditional replacement, which holds no slot. `completion`
/// fires once when the transition clock ends: the site drops the record,
/// detaches the view and invalidates then.
pub fn start_removal(
    mtm: cocoa_ui::MainThreadMarker,
    view: &Retained<PlatformView>,
    claim: &TransitionClaim,
    slot: Option<GhostSlot>,
    renderer: &crate::contract::Renderer,
    completion: impl FnOnce() + 'static,
) -> RemovalGhost {
    let state = claim
        .spec
        .start(TransitionPhase::Removal, claim.reduce_motion);
    #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
    if state.captures_content() {
        return start_pixel_removal(mtm, view, renderer, state, slot, completion);
    }
    #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
    let _ = renderer;
    #[cfg(not(any(feature = "applied_filter", feature = "view_effect")))]
    assert!(
        !state.captures_content(),
        "waterui-apple: pixel-capture transitions need the filtered pipeline — build with \
         `applied_filter` or `view_effect`; a failed capture is an error, never a property fallback"
    );
    let (size, direction) = geometry_of(view);
    play_property_transition(view, &state, TransitionPhase::Removal, size, direction);

    let completion = Rc::new(RefCell::new(Some(completion)));
    let driver = Rc::new(RefCell::new(state));
    let last = Cell::new(Instant::now());
    let clock = FrameClock::new(mtm, {
        let driver = Rc::clone(&driver);
        let completion = Rc::clone(&completion);
        move || {
            let now = Instant::now();
            let delta = now.duration_since(last.replace(now));
            let done = !driver.borrow_mut().advance(delta);
            if done && let Some(complete) = completion.borrow_mut().take() {
                complete();
            }
        }
    });
    clock.start(view);
    RemovalGhost {
        view: view.clone(),
        slot,
        state: driver,
        clock,
        #[cfg(any(feature = "applied_filter", feature = "view_effect"))]
        pixel: None,
    }
}
