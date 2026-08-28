use waterui::shape::{ResolvedShape, ShapeKind};

use crate::{IntoFFI, WuiArray, WuiPathCommand, reactive::WuiComputed};

/// C ABI mirror of [`ShapeKind`], flattened into a discriminant tag plus the
/// per-corner radii used only by the rounded-rect variants.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiShapeKind {
    /// Discriminant: 0 = rect, 1 = circle, 2 = ellipse, 3 = rounded rect
    /// (uniform radius), 4 = uneven rounded rect (per-corner radii),
    /// 5 = capsule, 6 = custom path.
    pub tag: i32,
    /// Top-left corner radius, used by tags 3 and 4.
    pub top_left: f32,
    /// Top-right corner radius, used by tags 3 and 4.
    pub top_right: f32,
    /// Bottom-right corner radius, used by tags 3 and 4.
    pub bottom_right: f32,
    /// Bottom-left corner radius, used by tags 3 and 4.
    pub bottom_left: f32,
}

impl IntoFFI for ShapeKind {
    type FFI = WuiShapeKind;

    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Rect => WuiShapeKind {
                tag: 0,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            Self::Circle => WuiShapeKind {
                tag: 1,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            Self::Ellipse => WuiShapeKind {
                tag: 2,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            Self::RoundedRect { corner_radius } => WuiShapeKind {
                tag: 3,
                top_left: corner_radius,
                top_right: corner_radius,
                bottom_right: corner_radius,
                bottom_left: corner_radius,
            },
            Self::UnevenRoundedRect {
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
            Self::Capsule => WuiShapeKind {
                tag: 5,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            Self::CustomPath => WuiShapeKind {
                tag: 6,
                top_left: 0.0,
                top_right: 0.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
        }
    }
}

/// C ABI mirror of [`ResolvedShape`], the backend-native shape payload
/// rendered directly by native backends.
#[repr(C)]
#[derive(Debug)]
pub struct WuiResolvedShape {
    /// Shape kind for backend-side rendering optimization.
    pub kind: WuiShapeKind,
    /// Path commands in unit coordinate space.
    pub commands: WuiArray<WuiPathCommand>,
    /// Read-only signal for the environment-resolved fill color.
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
