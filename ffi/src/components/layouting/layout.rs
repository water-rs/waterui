use crate::components::text::WuiHorizontalAlignment;
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::ffi::c_void;
use core::fmt;
use nami::{Signal, SignalExt};
use waterui_layout::{
    HorizontalAlignment, Layout, Point, ProposalSize, Rect, ScrollView, Size, StretchAxis, SubView,
    VerticalAlignment, ViewDimensions,
    container::{FixedContainer, LazyContainer},
    measure_layout,
    scroll::Axis,
    stack::{HStackLayout, VStackLayout},
};

use crate::{IntoFFI, IntoRust, WuiAnyView, array::WuiArray};
use crate::{WuiTypeId, views::WuiAnyViews};

opaque!(WuiLayout, Box<dyn Layout>, layout);

/// C ABI mirror of [`FixedContainer`], a view that executes an arbitrary
/// [`Layout`] over an eagerly-collected, fixed set of child views.
#[repr(C)]
pub struct WuiFixedContainer {
    /// Owning handle to the boxed [`Layout`] implementation driving this container.
    pub layout: *mut WuiLayout,
    /// The container's child views, collected eagerly at construction time.
    pub contents: WuiArray<*mut WuiAnyView>,
}

impl fmt::Debug for WuiFixedContainer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WuiFixedContainer").finish_non_exhaustive()
    }
}

/// Returns the type ID for Spacer views as a 128-bit value.
/// `Spacer` is a raw view that stretches to fill available space.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_spacer_id() -> WuiTypeId {
    WuiTypeId::of::<waterui::component::spacer::Spacer>()
}

ffi_view!(FixedContainer, WuiFixedContainer, fixed_container);

impl IntoFFI for FixedContainer {
    type FFI = WuiFixedContainer;
    fn into_ffi(self) -> Self::FFI {
        let (layout, contents) = self.into_inner();
        WuiFixedContainer {
            layout: layout.into_ffi(),
            contents: contents.into_ffi(),
        }
    }
}

/// C ABI mirror of [`LazyContainer`], a view that executes an arbitrary
/// [`Layout`] over a lazily reconstructed collection of child views.
#[repr(C)]
#[derive(Debug)]
pub struct WuiContainer {
    /// Owning handle to the boxed [`Layout`] implementation driving this container.
    pub layout: *mut WuiLayout,
    /// Handle to the lazily reconstructed child view collection.
    pub contents: *mut WuiAnyViews,
}

ffi_view!(LazyContainer, WuiContainer, layout_container);

impl IntoFFI for LazyContainer {
    type FFI = WuiContainer;
    fn into_ffi(self) -> Self::FFI {
        let (layout, contents) = self.into_inner();
        WuiContainer {
            layout: layout.into_ffi(),
            contents: contents.into_ffi(),
        }
    }
}

/// The stacking axis a layout advertises for lazy (virtualized) rendering,
/// as reported by [`waterui_layout_lazy_stack_axis`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WuiLazyStackAxis {
    /// The layout does not support lazy stacking (not a `VStackLayout`/`HStackLayout`).
    Unsupported = 0,
    /// The layout stacks children vertically (`VStackLayout`).
    Vertical = 1,
    /// The layout stacks children horizontally (`HStackLayout`).
    Horizontal = 2,
}

#[derive(Clone, Copy)]
struct LazyStackDescriptor {
    axis: WuiLazyStackAxis,
    spacing: f32,
    horizontal_alignment: WuiHorizontalAlignment,
    vertical_alignment: WuiVerticalAlignment,
}

fn lazy_stack_descriptor(layout: &dyn Layout) -> Option<LazyStackDescriptor> {
    let layout_any = layout as &dyn core::any::Any;
    if let Some(vstack) = layout_any.downcast_ref::<VStackLayout>() {
        return Some(LazyStackDescriptor {
            axis: WuiLazyStackAxis::Vertical,
            spacing: vstack.spacing.get(),
            horizontal_alignment: vstack.alignment.into_ffi(),
            vertical_alignment: VerticalAlignment::Center.into_ffi(),
        });
    }
    if let Some(hstack) = layout_any.downcast_ref::<HStackLayout>() {
        return Some(LazyStackDescriptor {
            axis: WuiLazyStackAxis::Horizontal,
            spacing: hstack.spacing.get(),
            horizontal_alignment: HorizontalAlignment::Center.into_ffi(),
            vertical_alignment: hstack.alignment.into_ffi(),
        });
    }
    None
}

fn required_lazy_stack_descriptor(layout: &dyn Layout) -> LazyStackDescriptor {
    lazy_stack_descriptor(layout)
        .unwrap_or_else(|| panic!("waterui_layout_lazy_stack_* called for unsupported layout"))
}

/// Native callback invoked when a reactive layout input changes.
pub type WuiLayoutInvalidationCallback = unsafe extern "C" fn(context: *mut c_void);

struct ForeignLayoutInvalidation {
    context: *mut c_void,
    invalidate: WuiLayoutInvalidationCallback,
    drop: WuiLayoutInvalidationCallback,
}

impl ForeignLayoutInvalidation {
    fn invalidate(&self) {
        unsafe { (self.invalidate)(self.context) };
    }
}

impl Drop for ForeignLayoutInvalidation {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.context) };
    }
}

/// Retained native invalidation target and its precise signal subscriptions.
pub struct WuiLayoutWatcher {
    _guards: Vec<nami::watcher::BoxWatcherGuard>,
    _target: Rc<ForeignLayoutInvalidation>,
}

impl fmt::Debug for WuiLayoutWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WuiLayoutWatcher").finish_non_exhaustive()
    }
}

/// Watches the reactive fields used by a layout.
///
/// # Safety
///
/// `layout` and `context` must remain valid until the returned watcher is
/// dropped. Both callbacks must run on the layout's owning UI thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_watch_invalidation(
    layout: *const WuiLayout,
    context: *mut c_void,
    invalidate: WuiLayoutInvalidationCallback,
    drop_callback: WuiLayoutInvalidationCallback,
) -> *mut WuiLayoutWatcher {
    let layout = unsafe { crate::borrow_ffi(layout) };

    let target = Rc::new(ForeignLayoutInvalidation {
        context,
        invalidate,
        drop: drop_callback,
    });
    let callback_target = Rc::clone(&target);
    let guards = layout.0.watch_invalidation(Rc::new(move || {
        let target = Rc::clone(&callback_target);
        target.invalidate();
    }));
    Box::into_raw(Box::new(WuiLayoutWatcher {
        _guards: guards,
        _target: target,
    }))
}

/// Drops a layout invalidation watcher.
///
/// # Safety
///
/// `watcher` must be returned by [`waterui_layout_watch_invalidation`] and not
/// previously dropped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_watcher_drop(watcher: *mut WuiLayoutWatcher) {
    unsafe { drop(Box::from_raw(watcher)) };
}

// ============================================================================
// ProposalSize FFI
// ============================================================================

/// C ABI mirror of [`ProposalSize`]: the size a parent layout suggests to a
/// child. Either axis may be unspecified, encoded here as `f32::NAN` to mean
/// "the child decides its own size along this axis".
#[derive(Clone, Default, Debug)]
#[repr(C)]
pub struct WuiProposalSize {
    width: f32, // May be f32::NAN for unspecified
    height: f32,
}

impl IntoRust for WuiProposalSize {
    type Rust = ProposalSize;
    unsafe fn into_rust(self) -> Self::Rust {
        ProposalSize {
            width: if self.width.is_finite() {
                Some(self.width)
            } else {
                None
            },
            height: if self.height.is_finite() {
                Some(self.height)
            } else {
                None
            },
        }
    }
}

impl IntoFFI for ProposalSize {
    type FFI = WuiProposalSize;
    fn into_ffi(self) -> Self::FFI {
        WuiProposalSize {
            width: self.width.unwrap_or(f32::NAN),
            height: self.height.unwrap_or(f32::NAN),
        }
    }
}

// ============================================================================
// StretchAxis FFI
// ============================================================================

/// FFI representation of `StretchAxis` enum.
///
/// Specifies which axis (or axes) a view stretches to fill available space.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WuiStretchAxis {
    /// No stretching - view uses its intrinsic size
    None = 0,
    /// Stretch horizontally only (expand width, use intrinsic height)
    Horizontal = 1,
    /// Stretch vertically only (expand height, use intrinsic width)
    Vertical = 2,
    /// Stretch in both directions (expand width and height)
    Both = 3,
    /// Stretch along the parent container's main axis (e.g., Spacer)
    MainAxis = 4,
    /// Stretch along the parent container's cross axis (e.g., Divider)
    CrossAxis = 5,
}

impl From<WuiStretchAxis> for StretchAxis {
    fn from(axis: WuiStretchAxis) -> Self {
        match axis {
            WuiStretchAxis::None => Self::None,
            WuiStretchAxis::Horizontal => Self::Horizontal,
            WuiStretchAxis::Vertical => Self::Vertical,
            WuiStretchAxis::Both => Self::Both,
            WuiStretchAxis::MainAxis => Self::MainAxis,
            WuiStretchAxis::CrossAxis => Self::CrossAxis,
        }
    }
}

impl From<StretchAxis> for WuiStretchAxis {
    fn from(axis: StretchAxis) -> Self {
        match axis {
            StretchAxis::None => Self::None,
            StretchAxis::Horizontal => Self::Horizontal,
            StretchAxis::Vertical => Self::Vertical,
            StretchAxis::Both => Self::Both,
            StretchAxis::MainAxis => Self::MainAxis,
            StretchAxis::CrossAxis => Self::CrossAxis,
        }
    }
}

// ============================================================================
// SubView FFI Proxy
// ============================================================================

/// `VTable` for `SubView` operations.
///
/// This structure contains function pointers that allow native code to implement
/// the `SubView` protocol. The native backend provides these callbacks to participate
/// in layout negotiation.
#[repr(C)]
#[derive(Debug)]
pub struct WuiSubViewVTable {
    /// Measures the child view given a size proposal.
    /// Called potentially multiple times with different proposals during layout.
    pub measure: unsafe extern "C" fn(
        context: *mut core::ffi::c_void,
        proposal: WuiProposalSize,
    ) -> WuiViewDimensions,
    /// Cleans up the context when the subview is no longer needed.
    /// Called when the `WuiSubView` is dropped.
    pub drop: unsafe extern "C" fn(context: *mut core::ffi::c_void),
}

/// FFI representation of a `SubView` proxy.
///
/// This allows native code to participate in the layout negotiation protocol
/// by providing callbacks that can be called multiple times with different proposals.
///
/// # Memory Management
///
/// The `context` pointer is owned by this struct. When the `WuiSubView` is dropped,
/// the `vtable.drop` function will be called to clean up the context.
#[repr(C)]
#[derive(Debug)]
pub struct WuiSubView {
    /// Opaque context pointer (e.g., child view reference, cached data)
    pub context: *mut core::ffi::c_void,
    /// `VTable` containing measure and drop functions
    pub vtable: WuiSubViewVTable,
    /// Which axis this view stretches to fill available space
    pub stretch_axis: WuiStretchAxis,
    /// Layout priority (higher = measured first, gets space preference)
    pub priority: i32,
}

impl Drop for WuiSubView {
    fn drop(&mut self) {
        unsafe { (self.vtable.drop)(self.context) }
    }
}

// SAFETY: `WuiSubView` is a C-ABI handle whose `context` pointer and vtable function
// pointers are only ever invoked by the platform backend on its main/UI thread. The
// `SubView` impl reports `require_main_thread() == true`, so the layout executor never
// measures it on a worker. It must remain `#[repr(C)]`, so it cannot embed a
// `MainThreadBound` runtime guard; the native main-thread contract is the invariant.
unsafe impl Send for WuiSubView {}
unsafe impl Sync for WuiSubView {}

impl SubView for WuiSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let result = unsafe { (self.vtable.measure)(self.context, proposal.into_ffi()) };
        unsafe { result.into_rust() }
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis.into()
    }

    fn priority(&self) -> i32 {
        self.priority
    }

    fn require_main_thread(&self) -> bool {
        true
    }
}

// ============================================================================
// Geometry Types
// ============================================================================

into_ffi! {Point,
    pub struct WuiPoint {
        x: f32,
        y: f32,
    }
}

impl IntoRust for WuiPoint {
    type Rust = Point;
    unsafe fn into_rust(self) -> Self::Rust {
        Point {
            x: self.x,
            y: self.y,
        }
    }
}

into_ffi! {Size,
    pub struct WuiSize {
        width: f32,
        height: f32,
    }
}

impl IntoRust for WuiSize {
    type Rust = Size;
    unsafe fn into_rust(self) -> Self::Rust {
        Size {
            width: self.width,
            height: self.height,
        }
    }
}

#[cfg(feature = "c-api")]
crate::ffi_computed!(Size, WuiSize, size);

/// C ABI mirror of [`VerticalAlignment`], a named vertical alignment guide
/// used for stack cross-axis alignment and explicit view dimension guides.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WuiVerticalAlignment {
    /// Aligns to the top edge.
    Top = 0,
    /// Aligns to the vertical center.
    #[default]
    Center = 1,
    /// Aligns to the bottom edge.
    Bottom = 2,
    /// Aligns to the first line's text baseline.
    FirstBaseline = 3,
    /// Aligns to the last line's text baseline.
    LastBaseline = 4,
}

impl IntoFFI for VerticalAlignment {
    type FFI = WuiVerticalAlignment;

    fn into_ffi(self) -> Self::FFI {
        if self == Self::Top {
            WuiVerticalAlignment::Top
        } else if self == Self::Bottom {
            WuiVerticalAlignment::Bottom
        } else if self == Self::FirstBaseline {
            WuiVerticalAlignment::FirstBaseline
        } else if self == Self::LastBaseline {
            WuiVerticalAlignment::LastBaseline
        } else {
            WuiVerticalAlignment::Center
        }
    }
}

impl IntoRust for WuiVerticalAlignment {
    type Rust = VerticalAlignment;

    unsafe fn into_rust(self) -> Self::Rust {
        match self {
            Self::Top => VerticalAlignment::Top,
            Self::Center => VerticalAlignment::Center,
            Self::Bottom => VerticalAlignment::Bottom,
            Self::FirstBaseline => VerticalAlignment::FirstBaseline,
            Self::LastBaseline => VerticalAlignment::LastBaseline,
        }
    }
}

/// C ABI mirror of one explicit horizontal alignment guide entry from
/// [`ViewDimensions`], pairing a named alignment with its measured offset.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct WuiHorizontalGuide {
    alignment: WuiHorizontalAlignment,
    value: f32,
}

impl IntoRust for WuiHorizontalGuide {
    type Rust = (HorizontalAlignment, f32);

    unsafe fn into_rust(self) -> Self::Rust {
        (unsafe { self.alignment.into_rust() }, self.value)
    }
}

/// C ABI mirror of one explicit vertical alignment guide entry from
/// [`ViewDimensions`], pairing a named alignment with its measured offset.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct WuiVerticalGuide {
    alignment: WuiVerticalAlignment,
    value: f32,
}

impl IntoRust for WuiVerticalGuide {
    type Rust = (VerticalAlignment, f32);

    unsafe fn into_rust(self) -> Self::Rust {
        (unsafe { self.alignment.into_rust() }, self.value)
    }
}

/// C ABI mirror of [`ViewDimensions`]: a measured size together with any
/// explicit horizontal/vertical alignment guides the view exposes to its
/// parent layout.
#[repr(C)]
pub struct WuiViewDimensions {
    size: WuiSize,
    horizontal_guides: WuiArray<WuiHorizontalGuide>,
    vertical_guides: WuiArray<WuiVerticalGuide>,
}

impl fmt::Debug for WuiViewDimensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WuiViewDimensions")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl IntoFFI for ViewDimensions {
    type FFI = WuiViewDimensions;

    fn into_ffi(self) -> Self::FFI {
        let horizontal_guides = self
            .explicit_horizontal_guides()
            .map(|(alignment, value)| WuiHorizontalGuide {
                alignment: alignment.into_ffi(),
                value,
            })
            .collect::<Vec<_>>();
        let vertical_guides = self
            .explicit_vertical_guides()
            .map(|(alignment, value)| WuiVerticalGuide {
                alignment: alignment.into_ffi(),
                value,
            })
            .collect::<Vec<_>>();

        WuiViewDimensions {
            size: self.size.into_ffi(),
            horizontal_guides: WuiArray::new(horizontal_guides),
            vertical_guides: WuiArray::new(vertical_guides),
        }
    }
}

impl IntoRust for WuiViewDimensions {
    type Rust = ViewDimensions;

    unsafe fn into_rust(self) -> Self::Rust {
        let mut dimensions = ViewDimensions::new(unsafe { self.size.into_rust() });
        for (alignment, value) in unsafe { self.horizontal_guides.into_rust() } {
            dimensions.set_horizontal(alignment, value);
        }
        for (alignment, value) in unsafe { self.vertical_guides.into_rust() } {
            dimensions.set_vertical(alignment, value);
        }
        dimensions
    }
}

/// C ABI mirror of [`Rect`]: an axis-aligned rectangle expressed as an
/// origin point and a size, relative to its parent's coordinate space.
#[repr(C)]
#[derive(Debug)]
pub struct WuiRect {
    origin: WuiPoint,
    size: WuiSize,
}

impl IntoRust for WuiRect {
    type Rust = Rect;
    unsafe fn into_rust(self) -> Self::Rust {
        unsafe { Rect::new(self.origin.into_rust(), self.size.into_rust()) }
    }
}

impl IntoFFI for Rect {
    type FFI = WuiRect;
    fn into_ffi(self) -> Self::FFI {
        WuiRect {
            origin: self.origin().into_ffi(),
            size: (*self.size()).into_ffi(),
        }
    }
}

#[cfg(feature = "c-api")]
crate::ffi_binding!(Rect, WuiRect, rect);
crate::ffi_watcher!(Rect, WuiRect, rect);

// ============================================================================
// Layout API Functions
// ============================================================================

/// Calculates the size required by the layout given a proposal and child proxies.
///
/// This function implements the new SubView-based negotiation protocol where
/// layouts can query children multiple times with different proposals.
///
/// # Safety
///
/// - The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
/// - The `children` array must contain valid `WuiSubView` entries.
/// - The measure callbacks in each child must be safe to call.
/// - The `children` array will be consumed and dropped after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_measure(
    layout: *mut WuiLayout,
    proposal: WuiProposalSize,
    mut children: WuiArray<WuiSubView>,
) -> WuiViewDimensions {
    let layout: &dyn Layout = unsafe { &*(*layout).0 };
    let proposal = unsafe { proposal.into_rust() };
    let children_slice = children.as_mut_slice();
    let subview_refs: Vec<&dyn SubView> =
        children_slice.iter().map(|s| s as &dyn SubView).collect();
    let dimensions = measure_layout(layout, proposal, &subview_refs);
    children.consume();
    dimensions.into_ffi()
}

/// Places child views within the specified bounds.
///
/// Returns an array of Rect values representing the position and size of each child.
///
/// # Safety
///
/// - The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
/// - The `children` array must contain valid `WuiSubView` entries.
/// - The measure callbacks in each child must be safe to call.
/// - The `children` array will be consumed and dropped after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_place(
    layout: *mut WuiLayout,
    bounds: WuiRect,
    mut children: WuiArray<WuiSubView>,
) -> WuiArray<WuiRect> {
    let layout: &dyn Layout = unsafe { &*(*layout).0 };
    let bounds = unsafe { bounds.into_rust() };

    // Get slice of WuiSubView and create trait object references
    let children_slice = children.as_mut_slice();
    let subview_refs: Vec<&dyn SubView> =
        children_slice.iter().map(|s| s as &dyn SubView).collect();

    let rects = layout.place(bounds, &subview_refs);

    children.consume();

    rects.into_ffi()
}

/// Returns the lazy-stack axis the layout advertises, if any.
///
/// # Safety
///
/// The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_lazy_stack_axis(
    layout: *mut WuiLayout,
) -> WuiLazyStackAxis {
    let layout: &dyn Layout = unsafe { &*(*layout).0 };
    lazy_stack_descriptor(layout)
        .map_or(WuiLazyStackAxis::Unsupported, |descriptor| descriptor.axis)
}

/// Returns the lazy-stack inter-item spacing the layout requires.
///
/// # Safety
///
/// The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_lazy_stack_spacing(layout: *mut WuiLayout) -> f32 {
    let layout: &dyn Layout = unsafe { &*(*layout).0 };
    required_lazy_stack_descriptor(layout).spacing
}

/// Returns the lazy-stack cross-axis horizontal alignment the layout requires.
///
/// # Safety
///
/// The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_lazy_stack_horizontal_alignment(
    layout: *mut WuiLayout,
) -> WuiHorizontalAlignment {
    let layout: &dyn Layout = unsafe { &*(*layout).0 };
    required_lazy_stack_descriptor(layout).horizontal_alignment
}

/// Returns the lazy-stack cross-axis vertical alignment the layout requires.
///
/// # Safety
///
/// The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_lazy_stack_vertical_alignment(
    layout: *mut WuiLayout,
) -> WuiVerticalAlignment {
    let layout: &dyn Layout = unsafe { &*(*layout).0 };
    required_lazy_stack_descriptor(layout).vertical_alignment
}

// ============================================================================
// ScrollView
// ============================================================================

into_ffi! {Axis, non_exhaustive,
    pub enum WuiAxis {
        Horizontal,
        Vertical,
        All,
    }
}

/// C ABI mirror of [`ScrollView`], a container that scrolls content larger
/// than its own frame along one or both axes.
#[repr(C)]
#[derive(Debug)]
pub struct WuiScrollView {
    /// The axis (or axes) along which this scroll view allows scrolling.
    pub axis: WuiAxis,
    /// The scrollable content view.
    pub content: *mut WuiAnyView,
    /// Read-only signal for the horizontal scroll target requested via
    /// `ScrollController::scroll_to`, or null if no controller is attached.
    pub target_x: *mut crate::reactive::WuiComputed<f32>,
    /// Read-only signal for the vertical scroll target requested via
    /// `ScrollController::scroll_to`, or null if no controller is attached.
    pub target_y: *mut crate::reactive::WuiComputed<f32>,
    /// Monotonically increasing generation that changes each time a new
    /// scroll request is issued, letting the backend detect a repeated
    /// request to the same target. Null if no controller is attached.
    pub scroll_generation: *mut crate::reactive::WuiComputed<i32>,
}

impl IntoFFI for ScrollView {
    type FFI = WuiScrollView;
    fn into_ffi(self) -> Self::FFI {
        let (axis, content, controller) = self.into_inner();
        let (target_x, target_y, scroll_generation) = controller.map_or_else(
            || {
                (
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                )
            },
            |controller| {
                let target = controller.target();
                (
                    target.clone().map(|point| point.x).computed().into_ffi(),
                    target.map(|point| point.y).computed().into_ffi(),
                    controller.generation().into_ffi(),
                )
            },
        );
        WuiScrollView {
            axis: axis.into_ffi(),
            content: content.into_ffi(),
            target_x,
            target_y,
            scroll_generation,
        }
    }
}

ffi_view!(ScrollView, WuiScrollView, scroll_view);

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use nami::{Computed, SignalExt, binding};
    use waterui_layout::stack::{HStackLayout, VStackLayout};

    fn with_layout(layout: impl Layout + 'static, f: impl FnOnce(*mut WuiLayout)) {
        let mut layout = WuiLayout(Box::new(layout));
        f(&raw mut layout);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "spacing is copied verbatim across FFI from a Computed::constant with no intervening arithmetic, so exact equality is the correct assertion"
    )]
    fn lazy_stack_queries_report_vstack_configuration() {
        with_layout(
            VStackLayout {
                alignment: HorizontalAlignment::Trailing,
                spacing: Computed::constant(12.0),
            },
            |layout| unsafe {
                assert_eq!(
                    waterui_layout_lazy_stack_axis(layout),
                    WuiLazyStackAxis::Vertical
                );
                assert_eq!(waterui_layout_lazy_stack_spacing(layout), 12.0);
                assert_eq!(
                    waterui_layout_lazy_stack_horizontal_alignment(layout),
                    WuiHorizontalAlignment::Trailing
                );
            },
        );
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "spacing is copied verbatim across FFI from a Computed::constant with no intervening arithmetic, so exact equality is the correct assertion"
    )]
    fn lazy_stack_queries_report_hstack_configuration() {
        with_layout(
            HStackLayout {
                alignment: VerticalAlignment::Bottom,
                spacing: Computed::constant(7.0),
            },
            |layout| unsafe {
                assert_eq!(
                    waterui_layout_lazy_stack_axis(layout),
                    WuiLazyStackAxis::Horizontal
                );
                assert_eq!(waterui_layout_lazy_stack_spacing(layout), 7.0);
                assert_eq!(
                    waterui_layout_lazy_stack_vertical_alignment(layout),
                    WuiVerticalAlignment::Bottom
                );
            },
        );
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "spacing is copied verbatim across FFI from a Computed::constant with no intervening arithmetic, so exact equality is the correct assertion"
    )]
    fn layout_watcher_forwards_precise_signal_invalidation() {
        struct Target(Rc<Cell<usize>>);

        unsafe extern "C" fn invalidate(context: *mut c_void) {
            let target = unsafe { &*(context as *const Target) };
            target.0.set(target.0.get() + 1);
        }

        unsafe extern "C" fn drop_target(context: *mut c_void) {
            unsafe { drop(Box::from_raw(context.cast::<Target>())) };
        }

        let spacing = binding(4.0_f32);
        let mut layout = WuiLayout(Box::new(VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: spacing.computed(),
        }));
        let invalidations = Rc::new(Cell::new(0));
        let context = Box::into_raw(Box::new(Target(Rc::clone(&invalidations)))).cast();
        let watcher = unsafe {
            waterui_layout_watch_invalidation(&raw const layout, context, invalidate, drop_target)
        };

        spacing.set(12.0);
        assert_eq!(invalidations.get(), 1);
        assert_eq!(
            unsafe { waterui_layout_lazy_stack_spacing(&raw mut layout) },
            12.0
        );

        unsafe { waterui_layout_watcher_drop(watcher) };
    }
}
