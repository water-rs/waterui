//! White point filter implementation.

use crate::Filter;
use nami::Signal;

/// Adjusts white balance using a target white point.
///
/// The provided RGB triplet defines the source white point to normalize.
///
/// # Parameters
///
/// - `red`: Red channel white-point component
/// - `green`: Green channel white-point component
/// - `blue`: Blue channel white-point component
#[derive(Debug, Clone, Copy)]
pub struct WhitePoint<R, G, B>(pub R, pub G, pub B);

impl<R, G, B> Filter for WhitePoint<R, G, B>
where
    R: Signal<Output = f32> + 'static,
    G: Signal<Output = f32> + 'static,
    B: Signal<Output = f32> + 'static,
{
    const COLOR_ONLY: bool = true;

    type Params = [f32; 3];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 3] {
        [self.0.get(), self.1.get(), self.2.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/white_point.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_white_point_params() {
        let filter = WhitePoint(1.1f32, 1.0f32, 0.9f32);
        assert_eq!(filter.params(), [1.1, 1.0, 0.9]);
    }
}
