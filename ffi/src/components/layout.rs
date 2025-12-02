use alloc::{boxed::Box, vec::Vec};
use waterui_layout::{
    Layout, Point, ProposalSize, Rect, ScrollView, Size, StretchAxis, SubView,
    container::{Container as LayoutContainer, FixedContainer},
    scroll::Axis,
};

use crate::{IntoFFI, IntoRust, WuiAnyView, array::WuiArray};
use crate::{WuiStr, views::WuiAnyViews};

opaque!(WuiLayout, Box<dyn Layout>, layout);

#[repr(C)]
pub struct WuiFixedContainer {
    layout: *mut WuiLayout,
    contents: WuiArray<*mut WuiAnyView>,
}

// `Spacer` is a raw view, it stretches to fill available space.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_spacer_id() -> WuiStr {
    core::any::type_name::<waterui::component::spacer::Spacer>().into_ffi()
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
    layout: *mut WuiLayout,
    contents: *mut WuiAnyViews,
}

ffi_view!(LayoutContainer, WuiContainer, layout_container);

impl IntoFFI for LayoutContainer {
    type FFI = WuiContainer;
    fn into_ffi(self) -> Self::FFI {
        let (layout, contents) = self.into_inner();
        WuiContainer {
            layout: layout.into_ffi(),
            contents: contents.into_ffi(),
        }
    }
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
            width: if self.width.is_nan() {
                None
            } else {
                Some(self.width)
            },
            height: if self.height.is_nan() {
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
    pub measure:
        unsafe extern "C" fn(context: *mut core::ffi::c_void, proposal: WuiProposalSize) -> WuiSize,
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

impl SubView for WuiSubView {
    fn size_that_fits(&self, proposal: ProposalSize) -> Size {
        let result = unsafe { (self.vtable.measure)(self.context, proposal.into_ffi()) };
        unsafe { result.into_rust() }
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis.into()
    }

    fn priority(&self) -> i32 {
        self.priority
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
            size: self.size().clone().into_ffi(),
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
    size.into_ffi()
    // children array is dropped here, calling drop on each WuiSubView
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
    rects.into_ffi()
    // children array is dropped here, calling drop on each WuiSubView
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
    axis: WuiAxis,
    content: *mut WuiAnyView, // Pointer to the content view
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
