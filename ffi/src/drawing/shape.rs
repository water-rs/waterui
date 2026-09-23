use waterui::shape::{ResolvedShape, ShapeKind};

use crate::{IntoFFI, WuiArray, WuiPathCommand, reactive::WuiComputed};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WuiShapeKind {
    pub tag: i32,
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl IntoFFI for ShapeKind {
    type FFI = WuiShapeKind;

    fn into_ffi(self) -> Self::FFI {
        match self {
            ShapeKind::Rect => WuiShapeKind {
                tag: 0,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            ShapeKind::Circle => WuiShapeKind {
                tag: 1,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            ShapeKind::Ellipse => WuiShapeKind {
                tag: 2,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            ShapeKind::RoundedRect { corner_radius } => WuiShapeKind {
                tag: 3,
                top_left: corner_radius,
                top_right: corner_radius,
                bottom_right: corner_radius,
                bottom_left: corner_radius,
            },
            ShapeKind::UnevenRoundedRect {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => WuiShapeKind {
                tag: 4,
                top_left,
                top_right,
                bottom_right,
                bottom_left,
            },
            ShapeKind::Capsule => WuiShapeKind {
                tag: 5,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            ShapeKind::CustomPath => WuiShapeKind {
                tag: 6,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
        }
    }
}

#[repr(C)]
pub struct WuiResolvedShape {
    pub kind: WuiShapeKind,
    pub commands: WuiArray<WuiPathCommand>,
    pub fill: *mut WuiComputed<waterui_graphics::ResolvedColor>,
}

impl IntoFFI for ResolvedShape {
    type FFI = WuiResolvedShape;

    fn into_ffi(self) -> Self::FFI {
        let commands = self
            .commands
            .into_iter()
            .map(IntoFFI::into_ffi)
            .collect::<Vec<WuiPathCommand>>();

        WuiResolvedShape {
            kind: self.kind.into_ffi(),
            commands: WuiArray::new(commands),
            fill: self.fill.into_ffi(),
        }
    }
}

// `ResolvedShape` is a raw view rendered natively by platform backends.
ffi_view!(ResolvedShape, WuiResolvedShape, resolved_shape);
