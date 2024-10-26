use waterui_core::raw_view;
use waterui_reactive::Computed;

#[derive(Debug)]
#[must_use]
pub struct Rectangle;

#[derive(Debug)]
#[must_use]
pub struct RoundedRectangle {
    pub radius: Computed<f64>,
}

#[derive(Debug)]
#[must_use]
pub struct Circle;

pub trait Shape {}

macro_rules! impl_shape {
    ($($ty:ty),*) => {
        $(
            raw_view!($ty);
            impl Shape for $ty {}
        )*
    };
}

impl_shape!(Rectangle, RoundedRectangle, Circle);
