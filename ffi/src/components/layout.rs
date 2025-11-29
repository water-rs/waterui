use alloc::boxed::Box;
use waterui_layout::{
    ChildMetadata, ChildPlacement, Layout, LayoutContext, Point, Rect, SafeAreaEdges,
    SafeAreaInsets, ScrollView, Size,
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

// ============================================================================
// Safe Area FFI Types
// ============================================================================

/// FFI representation of safe area insets
#[derive(Clone, Default)]
#[repr(C)]
pub struct WuiSafeAreaInsets {
    pub top: f32,
    pub bottom: f32,
    pub leading: f32,
    pub trailing: f32,
}

impl IntoFFI for SafeAreaInsets {
    type FFI = WuiSafeAreaInsets;
    fn into_ffi(self) -> Self::FFI {
        WuiSafeAreaInsets {
            top: self.top,
            bottom: self.bottom,
            leading: self.leading,
            trailing: self.trailing,
        }
    }
}

impl IntoRust for WuiSafeAreaInsets {
    type Rust = SafeAreaInsets;
    unsafe fn into_rust(self) -> Self::Rust {
        SafeAreaInsets {
            top: self.top,
            bottom: self.bottom,
            leading: self.leading,
            trailing: self.trailing,
        }
    }
}

/// FFI representation of safe area edges (bitflags)
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct WuiSafeAreaEdges {
    pub bits: u8,
}

impl IntoFFI for SafeAreaEdges {
    type FFI = WuiSafeAreaEdges;
    fn into_ffi(self) -> Self::FFI {
        WuiSafeAreaEdges { bits: self.bits() }
    }
}

impl IntoRust for WuiSafeAreaEdges {
    type Rust = SafeAreaEdges;
    unsafe fn into_rust(self) -> Self::Rust {
        SafeAreaEdges::from_bits_truncate(self.bits)
    }
}

/// FFI representation of layout context
#[derive(Clone, Default)]
#[repr(C)]
pub struct WuiLayoutContext {
    pub safe_area: WuiSafeAreaInsets,
    pub ignores_safe_area: WuiSafeAreaEdges,
}

impl IntoFFI for LayoutContext {
    type FFI = WuiLayoutContext;
    fn into_ffi(self) -> Self::FFI {
        WuiLayoutContext {
            safe_area: self.safe_area.into_ffi(),
            ignores_safe_area: self.ignores_safe_area.into_ffi(),
        }
    }
}

impl IntoRust for WuiLayoutContext {
    type Rust = LayoutContext;
    unsafe fn into_rust(self) -> Self::Rust {
        LayoutContext {
            safe_area: unsafe { self.safe_area.into_rust() },
            ignores_safe_area: unsafe { self.ignores_safe_area.into_rust() },
        }
    }
}

/// FFI representation of child placement (rect + context)
#[repr(C)]
pub struct WuiChildPlacement {
    pub rect: WuiRect,
    pub context: WuiLayoutContext,
}

impl IntoFFI for ChildPlacement {
    type FFI = WuiChildPlacement;
    fn into_ffi(self) -> Self::FFI {
        WuiChildPlacement {
            rect: self.rect.into_ffi(),
            context: self.context.into_ffi(),
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
    context: WuiLayoutContext,
) -> WuiArray<WuiProposalSize> {
    // But the returned array is allocated by Rust, so caller needs to free it
    // Convert FFI types to Rust types
    let layout: &mut dyn Layout = unsafe { &mut *(*layout).0 };
    let parent = unsafe { parent.into_rust() };
    let context = unsafe { context.into_rust() };

    let children = unsafe { children.into_rust() };

    let proposals = layout.propose(parent, &children, &context);

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
    context: WuiLayoutContext,
) -> WuiSize {
    // Convert FFI types to Rust types
    let layout: &mut dyn Layout = unsafe { &mut *(*layout).0 };
    let parent = unsafe { parent.into_rust() };
    let context = unsafe { context.into_rust() };

    let children = unsafe { children.into_rust() };

    let size = layout.size(parent, &children, &context);

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
    context: WuiLayoutContext,
) -> WuiArray<WuiChildPlacement> {
    // But the returned array is allocated by Rust, so caller needs to free it
    // Convert FFI types to Rust types
    let layout: &mut dyn Layout = unsafe { &mut *(*layout).0 };
    let bound = unsafe { bound.into_rust() };
    let proposal = unsafe { proposal.into_rust() };
    let context = unsafe { context.into_rust() };

    let children = unsafe { children.into_rust() };

    let placements = layout.place(bound, proposal, &children, &context);

    placements.into_ffi()
}
