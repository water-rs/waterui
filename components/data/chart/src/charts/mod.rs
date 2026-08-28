//! High-level chart view wrappers rendered with Canvas.

macro_rules! impl_chart_debug {
    ($name:ident, $signal:ident, $output:ty) => {
        impl<$signal> core::fmt::Debug for $name<$signal>
        where
            $signal: nami::Signal<Output = $output>,
        {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}

pub(crate) use impl_chart_debug;

mod cartesian;
mod core;
mod density;
mod financial;
mod geo;
mod polar;
mod specialized;

pub use cartesian::{area, bar, bubble, line, scatter};
pub use core::canvas;
pub use density::{contour, heatmap};
pub use financial::{candlestick, depth};
pub use geo::choropleth;
pub use polar::{pie, radar};
pub use specialized::gauge;
