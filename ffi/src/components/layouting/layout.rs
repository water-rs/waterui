use crate::components::text::WuiHorizontalAlignment;
use alloc::{boxed::Box, vec::Vec};
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

#[repr(C)]
pub struct WuiFixedContainer {
    pub layout: *mut WuiLayout,
    pub contents: WuiArray<*mut WuiAnyView>,
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

#[repr(C)]
pub struct WuiContainer {
    pub layout: *mut WuiLayout,
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

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WuiLazyStackAxis {
    Unsupported = 0,
    Vertical = 1,
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
            spacing: vstack.spacing,
            horizontal_alignment: vstack.alignment.into_ffi(),
            vertical_alignment: VerticalAlignment::Center.into_ffi(),
        });
    }
    if let Some(hstack) = layout_any.downcast_ref::<HStackLayout>() {
        return Some(LazyStackDescriptor {
            axis: WuiLazyStackAxis::Horizontal,
            spacing: hstack.spacing,
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

// ============================================================================
// ProposalSize FFI
// ============================================================================

#[derive(Clone, Default)]
#[repr(C)]
pub struct WuiProposalSize {
    width: f32, // May be f32::NAN for unspecified
    height: f32,
}

impl IntoRust for WuiProposalSize {
    type Rust = ProposalSize;
    unsafe fn into_rust(self) -> Self::Rust {
        ProposalSize {
            width: if !self.width.is_finite() {
                None
            } else {
                Some(self.width)
            },
            height: if !self.height.is_finite() {
                None
            } else {
                Some(self.height)
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

/// FFI representation of StretchAxis enum.
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
            WuiStretchAxis::None => StretchAxis::None,
            WuiStretchAxis::Horizontal => StretchAxis::Horizontal,
            WuiStretchAxis::Vertical => StretchAxis::Vertical,
            WuiStretchAxis::Both => StretchAxis::Both,
            WuiStretchAxis::MainAxis => StretchAxis::MainAxis,
            WuiStretchAxis::CrossAxis => StretchAxis::CrossAxis,
        }
    }
}

impl From<StretchAxis> for WuiStretchAxis {
    fn from(axis: StretchAxis) -> Self {
        match axis {
            StretchAxis::None => WuiStretchAxis::None,
            StretchAxis::Horizontal => WuiStretchAxis::Horizontal,
            StretchAxis::Vertical => WuiStretchAxis::Vertical,
            StretchAxis::Both => WuiStretchAxis::Both,
            StretchAxis::MainAxis => WuiStretchAxis::MainAxis,
            StretchAxis::CrossAxis => WuiStretchAxis::CrossAxis,
        }
    }
}

// ============================================================================
// SubView FFI Proxy
// ============================================================================

/// VTable for SubView operations.
///
/// This structure contains function pointers that allow native code to implement
/// the SubView protocol. The native backend provides these callbacks to participate
/// in layout negotiation.
#[repr(C)]
pub struct WuiSubViewVTable {
    /// Measures the child view given a size proposal.
    /// Called potentially multiple times with different proposals during layout.
    pub measure: unsafe extern "C" fn(
        context: *mut core::ffi::c_void,
        proposal: WuiProposalSize,
    ) -> WuiViewDimensions,
    /// Cleans up the context when the subview is no longer needed.
    /// Called when the WuiSubView is dropped.
    pub drop: unsafe extern "C" fn(context: *mut core::ffi::c_void),
}

/// FFI representation of a SubView proxy.
///
/// This allows native code to participate in the layout negotiation protocol
/// by providing callbacks that can be called multiple times with different proposals.
///
/// # Memory Management
///
/// The `context` pointer is owned by this struct. When the `WuiSubView` is dropped,
/// the `vtable.drop` function will be called to clean up the context.
#[repr(C)]
pub struct WuiSubView {
    /// Opaque context pointer (e.g., child view reference, cached data)
    pub context: *mut core::ffi::c_void,
    /// VTable containing measure and drop functions
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

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WuiVerticalAlignment {
    Top = 0,
    #[default]
    Center = 1,
    Bottom = 2,
    FirstBaseline = 3,
    LastBaseline = 4,
}

impl IntoFFI for VerticalAlignment {
    type FFI = WuiVerticalAlignment;

    fn into_ffi(self) -> Self::FFI {
        if self == VerticalAlignment::Top {
            WuiVerticalAlignment::Top
        } else if self == VerticalAlignment::Bottom {
            WuiVerticalAlignment::Bottom
        } else if self == VerticalAlignment::FirstBaseline {
            WuiVerticalAlignment::FirstBaseline
        } else if self == VerticalAlignment::LastBaseline {
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
            WuiVerticalAlignment::Top => VerticalAlignment::Top,
            WuiVerticalAlignment::Center => VerticalAlignment::Center,
            WuiVerticalAlignment::Bottom => VerticalAlignment::Bottom,
            WuiVerticalAlignment::FirstBaseline => VerticalAlignment::FirstBaseline,
            WuiVerticalAlignment::LastBaseline => VerticalAlignment::LastBaseline,
        }
    }
}

#[derive(Clone, Copy, Default)]
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

#[derive(Clone, Copy, Default)]
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

#[repr(C)]
pub struct WuiViewDimensions {
    size: WuiSize,
    horizontal_guides: WuiArray<WuiHorizontalGuide>,
    vertical_guides: WuiArray<WuiVerticalGuide>,
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

/// Releases the alignment-guide arrays owned by a `WuiViewDimensions`.
///
/// # Safety
///
/// `dimensions` must be a value previously produced by waterui and not already consumed;
/// it must not be used again after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_view_dimensions(dimensions: WuiViewDimensions) {
    dimensions.horizontal_guides.consume();
    dimensions.vertical_guides.consume();
}

#[repr(C)]
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
    children.consume_and_drop_elements();
    dimensions.into_ffi()
}

/// Returns the size the layout reports as fitting the given proposal.
///
/// # Safety
///
/// - The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
/// - The `children` array must contain valid `WuiSubView` entries; it is consumed and dropped after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_size_that_fits(
    layout: *mut WuiLayout,
    proposal: WuiProposalSize,
    mut children: WuiArray<WuiSubView>,
) -> WuiSize {
    let layout: &dyn Layout = unsafe { &*(*layout).0 };
    let proposal = unsafe { proposal.into_rust() };

    // Get slice of WuiSubView and create trait object references
    let children_slice = children.as_mut_slice();
    let subview_refs: Vec<&dyn SubView> =
        children_slice.iter().map(|s| s as &dyn SubView).collect();

    let size = layout.size_that_fits(proposal, &subview_refs);

    // Explicitly drop the children array and its elements (WuiSubViews)
    // This releases the strong reference to the Swift SubViewProxy held by WuiSubView
    children.consume_and_drop_elements();

    size.into_ffi()
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

    // Explicitly drop the children array and its elements (WuiSubViews)
    // This releases the strong reference to the Swift SubViewProxy held by WuiSubView
    children.consume_and_drop_elements();

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
        .map(|descriptor| descriptor.axis)
        .unwrap_or(WuiLazyStackAxis::Unsupported)
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

into_ffi! {Axis,All,
    pub enum WuiAxis {
        Horizontal,
        Vertical,
        All,
    }
}

#[repr(C)]
pub struct WuiScrollView {
    pub axis: WuiAxis,
    pub content: *mut WuiAnyView, // Pointer to the content view
}

impl IntoFFI for ScrollView {
    type FFI = WuiScrollView;
    fn into_ffi(self) -> Self::FFI {
        let (axis, content) = self.into_inner();
        WuiScrollView {
            axis: axis.into_ffi(),
            content: content.into_ffi(),
        }
    }
}

ffi_view!(ScrollView, WuiScrollView, scroll_view);

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_layout::stack::{HStackLayout, VStackLayout};

    fn with_layout(layout: impl Layout + 'static, f: impl FnOnce(*mut WuiLayout)) {
        let mut layout = WuiLayout(Box::new(layout));
        f(&mut layout as *mut WuiLayout);
    }

    #[test]
    fn lazy_stack_queries_report_vstack_configuration() {
        with_layout(
            VStackLayout {
                alignment: HorizontalAlignment::Trailing,
                spacing: 12.0,
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
    fn lazy_stack_queries_report_hstack_configuration() {
        with_layout(
            HStackLayout {
                alignment: VerticalAlignment::Bottom,
                spacing: 7.0,
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
}
