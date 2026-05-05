//! Vignette filter implementation.

use crate::Filter;
use nami::Signal;

/// Adds a vignette effect (darkened corners) to an image.
///
/// # Parameters
///
/// - `radius`: Inner radius where vignette starts (0.0-1.0)
/// - `softness`: How soft the vignette edge is (0.0-1.0)
///
/// # Example
///
/// ```ignore
/// use filtrate::filters::Vignette;
///
/// let subtle = Vignette(0.8, 0.3);
/// let dramatic = Vignette(0.3, 0.1);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Vignette<R, S>(pub R, pub S);

impl<R: Signal<Output = f32> + 'static, S: Signal<Output = f32> + 'static> Filter
    for Vignette<R, S>
{
    const COLOR_ONLY: bool = true;

    type Params = [f32; 2];
    type Fragments = &'static str;

    #[inline]
    fn params(&self) -> [f32; 2] {
        [self.0.get(), self.1.get()]
    }

    #[inline]
    fn fragments(&self) -> &'static str {
        include_str!("../shaders/fragments/vignette.wgsl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vignette_params() {
        let filter = Vignette(0.5f32, 0.2f32);
        assert_eq!(filter.params(), [0.5, 0.2]);
    }
}
