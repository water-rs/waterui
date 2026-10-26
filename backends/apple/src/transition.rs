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
//! Pixel (`captures_content`) transitions panic for now: the window
//! overlay machinery exists (`cocoa_ui::capture::ViewCapture` for the
//! frozen `MTLTexture`, `TransitionContext::new`/`TransitionFrame::new`
//! for the effect's inputs), but the filtered pipeline has no public
//! surface that accepts a frozen texture as effect input —
//! `imported_input`/`imported_texture` only ever receive the live
//! frame's own capture lease. The panic names that gap instead of
//! silently degrading to a property animation, per the contract's
//! "a failed capture is an error, never a property fallback".

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use std::time::Instant;

use cocoa_ui::display_link::FrameClock;
use cocoa_ui::{PlatformView, Retained, focus, view};
use waterui_core::layout::{LayoutDirection, Size};
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

/// Rejects a transition that still needs pixel capture: the filtered
/// pipeline has no public frozen-texture input surface yet (see the
/// module docs). Reduce Motion already resolved in `spec.start`, so a
/// capture transition whose declared `reduced()` is a property effect
/// never reaches here — the reduced path still works.
fn assert_property_transition(state: &TransitionState) {
    assert!(
        !state.captures_content(),
        "waterui-apple: pixel-capture transitions (a `Transition` whose `captures_content` is \
         true, such as disintegrate) are not realized yet — the window overlay they draw through \
         needs a `Computed<TransitionFrame>` and waterui-core does not expose a way to construct \
         one. Use a property transition (opacity, scale, move, rotation) or declare a property \
         `reduced()` form and enable Reduce Motion"
    );
}

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

/// Splits a removed mount into its live platform view and its transition
/// claim. The `Mounted`'s `unmount` detaches the view and hands the leaf
/// back; dropping the leaf here is the semantic boundary — every watcher,
/// subscription and kept object ends now — while the retained view keeps
/// the visual resources a ghost needs. The view is deactivated before the
/// caller reparents it anywhere.
pub fn retire(
    mounted: crate::contract::Mounted,
) -> (Retained<PlatformView>, Option<TransitionClaim>) {
    let view = view::retain_base(mounted.view());
    let claim = mounted.transition_claim();
    let leaf = mounted.unmount();
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
    assert_property_transition(&state);
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
    /// The retained outgoing view — visual resources only.
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
}

impl core::fmt::Debug for RemovalGhost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RemovalGhost").finish_non_exhaustive()
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
    completion: impl FnOnce() + 'static,
) -> RemovalGhost {
    let state = claim
        .spec
        .start(TransitionPhase::Removal, claim.reduce_motion);
    assert_property_transition(&state);
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
    }
}
