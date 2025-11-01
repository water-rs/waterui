use alloc::boxed::Box;
use waterui_layout::{
    ChildMetadata, Layout, Point, Rect, ScrollView, Size,
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

#[unsafe(no_mangle)]
pub extern "C" fn waterui_spacer_id() -> WuiStr {
    core::any::type_name::<waterui::component::spacer::Spacer>().into_ffi()
}

ffi_view!(FixedContainer, WuiFixedContainer);

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

ffi_view!(LayoutContainer, WuiContainer);

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

#[derive(Clone, Default)]
#[repr(C)]
pub struct WuiProposalSize {
    width: f32, // May be f32::NAN
    height: f32,
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
            width: self.width.unwrap_or(f32::NAN),
            height: self.height.unwrap_or(f32::NAN),
        }
    }
}

#[derive(Default, Clone)]
#[repr(C)]
pub struct WuiChildMetadata {
    proposal: WuiProposalSize,
    priority: u8,
    stretch: bool,
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

into_ffi! {Point,
    pub struct WuiPoint {
        x: f32,
        y: f32,
    }
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

impl IntoFFI for Rect {
    type FFI = WuiRect;
    fn into_ffi(self) -> Self::FFI {
        WuiRect {
            origin: self.origin().into_ffi(),
            size: self.size().clone().into_ffi(),
        }
    }
}

into_ffi! {Size,
    pub struct WuiSize {
        width: f32,
        height: f32,
    }
}

#[repr(C)]
pub struct WuiRect {
    origin: WuiPoint,
    size: WuiSize,
}

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

ffi_view!(ScrollView, WuiScrollView);

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
