use waterui::component::shape::{Circle, Rectangle, RoundedRectangle};

use crate::{computed::waterui_computed_double, IntoFFI};

use super::waterui_nothing;

#[repr(C)]
#[derive(Debug, Default)]
pub struct waterui_rectangle(waterui_nothing);

impl IntoFFI for Rectangle {
    type FFI = waterui_rectangle;
    fn into_ffi(self) -> Self::FFI {
        waterui_rectangle::default()
    }
}
ffi_view!(
    Rectangle,
    waterui_rectangle,
    waterui_view_force_as_rectangle,
    waterui_view_rectangle_id
);

#[repr(C)]
pub struct waterui_rounded_rectangle {
    pub radius: *mut waterui_computed_double,
}

into_ffi!(RoundedRectangle, waterui_rounded_rectangle, radius);

ffi_view!(
    RoundedRectangle,
    waterui_rounded_rectangle,
    waterui_view_force_as_rounded_rectangle,
    waterui_view_rounded_rectangle_id
);

#[repr(C)]
#[derive(Debug, Default)]

pub struct waterui_circle(waterui_nothing);

ffi_view!(
    Circle,
    waterui_circle,
    waterui_view_force_as_circle,
    waterui_view_circle_id
);

impl IntoFFI for Circle {
    type FFI = waterui_circle;
    fn into_ffi(self) -> Self::FFI {
        waterui_circle::default()
    }
}
