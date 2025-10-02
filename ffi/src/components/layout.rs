use alloc::boxed::Box;
use waterui_layout::{
    ChildMetadata, Container, Layout, Point, Rect, ScrollView, Size, scroll::Axis,
};

use crate::{IntoFFI, IntoRust, WuiAnyView, WuiTypeId, array::WuiArray};

ffi_type!(WuiLayout, Box<dyn Layout>, waterui_drop_layout);

#[repr(C)]
pub struct WuiContainer {
    layout: *mut WuiLayout,
    contents: WuiArray<*mut WuiAnyView>,
}

#[unsafe(no_mangle)]
pub extern "C" fn waterui_spacer_id() -> WuiTypeId {
    core::any::TypeId::of::<waterui::component::layout::spacer::Spacer>().into_ffi()
}

ffi_view!(
    Container,
    WuiContainer,
    waterui_container_id,
    waterui_force_as_container
);

impl IntoFFI for Container {
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
#[derive(Clone, Default)]
pub struct WuiProposalSize {
    width: f64, // May be f64::NAN
    height: f64,
}

impl IntoRust for WuiProposalSize {
    type Rust = waterui_layout::ProposalSize;
    unsafe fn into_rust(self) -> Self::Rust {
        waterui_layout::ProposalSize {
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

impl IntoFFI for waterui_layout::ProposalSize {
    type FFI = WuiProposalSize;
    fn into_ffi(self) -> Self::FFI {
        WuiProposalSize {
            width: self.width.unwrap_or(f64::NAN),
            height: self.height.unwrap_or(f64::NAN),
        }
    }
}

#[repr(C)]
#[derive(Clone, Default)]
pub struct WuiChildMetadata {
    proposal: WuiProposalSize,
    priority: u8,
    stretch: bool,
}

impl IntoRust for WuiChildMetadata {
    type Rust = ChildMetadata;
    unsafe fn into_rust(self) -> Self::Rust {
        ChildMetadata::new(
            unsafe { self.proposal.into_rust() },
            self.priority,
            self.stretch,
        )
    }
}

impl IntoFFI for ChildMetadata {
    type FFI = WuiChildMetadata;
    fn into_ffi(self) -> Self::FFI {
        WuiChildMetadata {
            proposal: self.proposal().clone().into_ffi(),
            priority: self.priority(),
            stretch: self.stretch(),
        }
    }
}

/// Proposes sizes for children based on parent constraints and child metadata.
///
/// # Safety
///
/// The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
/// The caller must ensure the layout object remains valid for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_propose(
    layout: *mut WuiLayout,
    parent: WuiProposalSize,
    children: WuiArray<WuiChildMetadata>,
) -> WuiArray<WuiProposalSize> {
    // But the returned array is allocated by Rust, so caller needs to free it
    // Convert FFI types to Rust types
    let layout: &mut dyn Layout = unsafe { &mut *(*layout).0 };
    let parent = unsafe { parent.into_rust() };

    let children = unsafe { children.into_rust() };

    let proposals = layout.propose(parent, &children);

    proposals.into_ffi()
}

/// Calculates the size required by the layout based on parent constraints and child metadata.
///
/// # Safety
///
/// The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
/// The caller must ensure the layout object remains valid for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_size(
    layout: *mut WuiLayout,
    parent: WuiProposalSize,
    children: WuiArray<WuiChildMetadata>,
) -> WuiSize {
    // Convert FFI types to Rust types
    let layout: &mut dyn Layout = unsafe { &mut *(*layout).0 };
    let parent = unsafe { parent.into_rust() };

    let children = unsafe { children.into_rust() };

    let size = layout.size(parent, &children);

    size.into_ffi()
}

#[repr(C)]
pub struct WuiPoint {
    x: f64,
    y: f64,
}

impl IntoRust for WuiPoint {
    type Rust = waterui_layout::Point;
    unsafe fn into_rust(self) -> Self::Rust {
        waterui_layout::Point {
            x: self.x,
            y: self.y,
        }
    }
}

impl IntoRust for WuiSize {
    type Rust = waterui_layout::Size;
    unsafe fn into_rust(self) -> Self::Rust {
        waterui_layout::Size {
            width: self.width,
            height: self.height,
        }
    }
}

impl IntoRust for WuiRect {
    type Rust = waterui_layout::Rect;
    unsafe fn into_rust(self) -> Self::Rust {
        unsafe { waterui_layout::Rect::new(self.origin.into_rust(), self.size.into_rust()) }
    }
}

ffi_struct!(Point, WuiPoint, x, y);
ffi_struct!(Size, WuiSize, width, height);

impl IntoFFI for Rect {
    type FFI = WuiRect;
    fn into_ffi(self) -> Self::FFI {
        WuiRect {
            origin: self.origin().into_ffi(),
            size: self.size().clone().into_ffi(),
        }
    }
}

#[repr(C)]
pub struct WuiSize {
    width: f64,
    height: f64,
}

#[repr(C)]
pub struct WuiRect {
    origin: WuiPoint,
    size: WuiSize,
}

#[repr(C)]
pub enum WuiAxis {
    Horizontal,
    Vertical,
    All,
}
impl crate::IntoFFI for Axis {
    type FFI = WuiAxis;
    fn into_ffi(self) -> Self::FFI {
        match self {
            <Axis>::Horizontal => WuiAxis::Horizontal,
            <Axis>::Vertical => WuiAxis::Vertical,
            _ => WuiAxis::All,
        }
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

ffi_view!(
    ScrollView,
    WuiScrollView,
    waterui_scroll_view_id,
    waterui_force_as_scroll_view
);

/// Places child views within the specified bounds based on layout constraints and child metadata.
///
/// # Safety
///
/// The `layout` pointer must be valid and point to a properly initialized `WuiLayout`.
/// The caller must ensure the layout object remains valid for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_layout_place(
    layout: *mut WuiLayout,
    bound: WuiRect,
    proposal: WuiProposalSize,
    children: WuiArray<WuiChildMetadata>,
) -> WuiArray<WuiRect> {
    // But the returned array is allocated by Rust, so caller needs to free it
    // Convert FFI types to Rust types
    let layout: &mut dyn Layout = unsafe { &mut *(*layout).0 };
    let bound = unsafe { bound.into_rust() };
    let proposal = unsafe { proposal.into_rust() };

    let children = unsafe { children.into_rust() };

    let rects = layout.place(bound, proposal, &children);

    rects.into_ffi()
}
