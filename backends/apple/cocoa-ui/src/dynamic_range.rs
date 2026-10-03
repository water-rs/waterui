//! Dynamic-range propagation for layer-backed content.
//!
//! A dynamic-range choice can be attached to a view or layer, applied to a
//! layer subtree, or resolved from the nearest tagged ancestor and the
//! display's EDR capability. The association key intentionally uses the
//! existing `dev.cocoaui.dynamicRangeMode` selector string so tags written
//! here and tags written by the platform-side compatibility layer share one
//! storage location.
//!
//! # Safety
//!
//! Objective-C associated objects and view/layer hierarchy reads are used
//! here. All inputs are live, main-thread objects; associated values are
//! `NSNumber` objects retained by the Objective-C runtime for the lifetime
//! of the association.

use core::ffi::c_void;

use objc2::ffi::{
    OBJC_ASSOCIATION_RETAIN_NONATOMIC, objc_getAssociatedObject, objc_setAssociatedObject,
};
use objc2::runtime::{AnyObject, Sel};
use objc2_foundation::NSNumber;
use objc2_quartz_core::{CADynamicRangeHigh, CADynamicRangeStandard, CALayer};

use crate::{PlatformView, Retained};

/// The dynamic range requested for a layer subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicRange {
    /// Standard dynamic range.
    Standard,
    /// High dynamic range.
    High,
}

impl DynamicRange {
    const fn is_high(self) -> bool {
        matches!(self, Self::High)
    }
}

fn association_key() -> *const c_void {
    // The selector string is the storage key used by the platform-side
    // compatibility layer; registering the same string yields the same key.
    let selector = Sel::register(c"dev.cocoaui.dynamicRangeMode");
    // SAFETY: `Sel` is `repr(transparent)` over the selector pointer the
    // associated-object API expects as the key.
    unsafe { core::mem::transmute::<Sel, *const c_void>(selector) }
}

fn tag_object(object: *mut AnyObject, mode: DynamicRange) {
    let value = NSNumber::numberWithBool(mode.is_high());
    // SAFETY: `object` is a live Objective-C object supplied by a caller on
    // the main thread; `value` is a live object retained by the association.
    unsafe {
        objc_setAssociatedObject(
            object,
            association_key(),
            (&raw const *value).cast_mut().cast::<AnyObject>(),
            OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );
    }
}

fn object_tag(object: *const AnyObject) -> Option<DynamicRange> {
    // SAFETY: `object` is a live Objective-C object supplied by a caller on
    // the main thread; the returned pointer is only borrowed for this call.
    let value = unsafe { objc_getAssociatedObject(object, association_key()) };
    if value.is_null() {
        return None;
    }
    // SAFETY: this association is only written by `tag_object` and the
    // platform-side compatibility layer, both of which store an `NSNumber`.
    let number = unsafe { &*value.cast::<NSNumber>() };
    Some(if number.boolValue() {
        DynamicRange::High
    } else {
        DynamicRange::Standard
    })
}

fn view_tag(view: &PlatformView) -> Option<DynamicRange> {
    object_tag(core::ptr::from_ref::<PlatformView>(view).cast::<AnyObject>())
}

fn set_view_tag(view: &PlatformView, mode: DynamicRange) {
    tag_object(
        core::ptr::from_ref::<PlatformView>(view)
            .cast_mut()
            .cast::<AnyObject>(),
        mode,
    );
}

fn layer_tag(layer: &CALayer) -> Option<DynamicRange> {
    object_tag(core::ptr::from_ref::<CALayer>(layer).cast::<AnyObject>())
}

fn set_layer_tag(layer: &CALayer, mode: DynamicRange) {
    tag_object(
        core::ptr::from_ref::<CALayer>(layer)
            .cast_mut()
            .cast::<AnyObject>(),
        mode,
    );
}

fn set_preferred(layer: &CALayer, mode: DynamicRange) {
    // SAFETY: the extern statics are immutable framework constants.
    layer.setPreferredDynamicRange(unsafe {
        match mode {
            DynamicRange::Standard => CADynamicRangeStandard,
            DynamicRange::High => CADynamicRangeHigh,
        }
    });
}

fn sublayers(layer: &CALayer) -> Vec<Retained<CALayer>> {
    // SAFETY: `layer` is live and the hierarchy is read on the main thread.
    unsafe { layer.sublayers() }
        .map(|layers| layers.to_vec())
        .unwrap_or_default()
}

/// Applies `mode` to `layer` and recursively to its sublayers.
///
/// A sublayer carrying its own tag keeps that local mode and propagates it
/// to its children; this preserves explicit nested overrides.
pub fn apply_to_layer(mode: DynamicRange, layer: &CALayer) {
    let local_mode = layer_tag(layer).unwrap_or(mode);
    set_layer_tag(layer, local_mode);
    set_preferred(layer, local_mode);
    for sublayer in sublayers(layer) {
        apply_to_layer(local_mode, &sublayer);
    }
}

/// Applies `mode` to `view`'s backing layer and its existing sublayers, and
/// tags the view so descendants can resolve the nearest explicit override.
pub fn apply_to_view(mode: DynamicRange, view: &PlatformView) {
    set_view_tag(view, mode);
    if let Some(layer) = crate::shape::layer(view) {
        set_layer_tag(&layer, mode);
        set_preferred(&layer, mode);
        for sublayer in sublayers(&layer) {
            apply_to_layer(mode, &sublayer);
        }
    }
}

/// The view's immediate superview.
#[must_use]
pub fn superview(view: &PlatformView) -> Option<Retained<PlatformView>> {
    #[cfg(target_os = "macos")]
    // SAFETY: superview walking is a main-thread read of the view hierarchy.
    unsafe {
        view.superview()
    }
    #[cfg(target_os = "ios")]
    {
        view.superview()
    }
}

fn override_starting_at(view: Option<&PlatformView>) -> Option<DynamicRange> {
    let mut current = view.map(Retained::from);
    while let Some(node) = current {
        if let Some(mode) = view_tag(&node) {
            return Some(mode);
        }
        current = superview(&node);
    }
    None
}

#[cfg(target_os = "ios")]
fn display_mode(view: &PlatformView) -> Option<DynamicRange> {
    let screen = view.window()?.windowScene()?.screen();
    Some(if screen.potentialEDRHeadroom() > 1.0 {
        DynamicRange::High
    } else {
        DynamicRange::Standard
    })
}

#[cfg(target_os = "macos")]
fn display_mode(view: &PlatformView) -> Option<DynamicRange> {
    let screen = view.window()?.screen()?;
    Some(
        if screen.maximumPotentialExtendedDynamicRangeColorComponentValue() > 1.0 {
            DynamicRange::High
        } else {
            DynamicRange::Standard
        },
    )
}

/// Resolves the nearest tagged view starting at `view`, falling back to the
/// display's EDR capability. `None` means the view is not on a display and no
/// ancestor carries an override.
#[must_use]
pub fn resolve(view: &PlatformView) -> Option<DynamicRange> {
    override_starting_at(Some(view)).or_else(|| display_mode(view))
}

/// Resolves the inherited mode for content inside `view`: tagged ancestors
/// are considered before the display's EDR capability.
///
/// # Panics
///
/// When `view` is not attached to a display and no ancestor is tagged.
#[must_use]
pub fn require_inherited(view: &PlatformView) -> DynamicRange {
    override_starting_at(superview(view).as_deref())
        .or_else(|| display_mode(view))
        .expect("Dynamic range resolution requires a view attached to a display")
}

/// Resolves `view`'s effective dynamic range and applies it to `layer`.
pub fn apply_resolved_to_layer(layer: &CALayer, view: &PlatformView) {
    if let Some(mode) = resolve(view) {
        apply_to_layer(mode, layer);
    }
}
