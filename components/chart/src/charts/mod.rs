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

pub mod area;
pub mod bar;
pub mod bubble;
pub mod candlestick;
/// Shared Canvas drawing and hit-testing internals for chart views.
pub mod canvas;
pub mod choropleth;
pub mod contour;
pub mod depth;
pub mod gauge;
pub mod heatmap;
pub mod line;
pub mod pie;
pub mod radar;
pub mod scatter;
