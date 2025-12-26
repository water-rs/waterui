//! Parameter array trait for zero-allocation filter params.
//!
//! Filters store their parameters as associated types implementing `ParamArray`.
//! This enables compile-time fusion of consecutive filter params without heap allocation.

/// Trait for parameter arrays that can write their values to a buffer.
///
/// Implemented for fixed-size arrays `[f32; N]` and nested tuples `(A, B)`.
/// This design enables zero-allocation parameter storage at compile time.
///
/// # Example
///
/// ```ignore
/// // Single filter with one param
/// let brightness_params: [f32; 1] = [0.5];
/// brightness_params.write_to(&mut buffer);
///
/// // Chained filters with nested tuple params
/// let chain_params: ([f32; 1], [f32; 2]) = ([0.5], [0.3, 0.1]);
/// chain_params.write_to(&mut buffer);
/// ```
pub trait ParamArray {
    /// The number of f32 values in this param array.
    const LEN: usize;

    /// Write the parameter values to the provided buffer.
    ///
    /// The buffer must have at least `Self::LEN` elements.
    fn write_to(&self, buf: &mut [f32]);
}

// Implementations for fixed-size arrays

impl ParamArray for [f32; 0] {
    const LEN: usize = 0;

    #[inline]
    fn write_to(&self, _buf: &mut [f32]) {}
}

impl ParamArray for [f32; 1] {
    const LEN: usize = 1;

    #[inline]
    fn write_to(&self, buf: &mut [f32]) {
        buf[..1].copy_from_slice(self);
    }
}

impl ParamArray for [f32; 2] {
    const LEN: usize = 2;

    #[inline]
    fn write_to(&self, buf: &mut [f32]) {
        buf[..2].copy_from_slice(self);
    }
}

impl ParamArray for [f32; 3] {
    const LEN: usize = 3;

    #[inline]
    fn write_to(&self, buf: &mut [f32]) {
        buf[..3].copy_from_slice(self);
    }
}

impl ParamArray for [f32; 4] {
    const LEN: usize = 4;

    #[inline]
    fn write_to(&self, buf: &mut [f32]) {
        buf[..4].copy_from_slice(self);
    }
}

impl ParamArray for [f32; 5] {
    const LEN: usize = 5;

    #[inline]
    fn write_to(&self, buf: &mut [f32]) {
        buf[..5].copy_from_slice(self);
    }
}

impl ParamArray for [f32; 6] {
    const LEN: usize = 6;

    #[inline]
    fn write_to(&self, buf: &mut [f32]) {
        buf[..6].copy_from_slice(self);
    }
}

impl ParamArray for [f32; 7] {
    const LEN: usize = 7;

    #[inline]
    fn write_to(&self, buf: &mut [f32]) {
        buf[..7].copy_from_slice(self);
    }
}

impl ParamArray for [f32; 8] {
    const LEN: usize = 8;

    #[inline]
    fn write_to(&self, buf: &mut [f32]) {
        buf[..8].copy_from_slice(self);
    }
}

// Implementation for nested tuples (enables chaining without allocation)

impl<A: ParamArray, B: ParamArray> ParamArray for (A, B) {
    const LEN: usize = A::LEN + B::LEN;

    #[inline]
    fn write_to(&self, buf: &mut [f32]) {
        self.0.write_to(&mut buf[..A::LEN]);
        self.1.write_to(&mut buf[A::LEN..]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_array() {
        let params: [f32; 2] = [1.0, 2.0];
        let mut buf = [0.0f32; 4];
        params.write_to(&mut buf);
        assert_eq!(buf[0], 1.0);
        assert_eq!(buf[1], 2.0);
    }

    #[test]
    fn test_nested_tuple() {
        let params: ([f32; 1], [f32; 2]) = ([1.0], [2.0, 3.0]);
        let mut buf = [0.0f32; 4];
        params.write_to(&mut buf);
        assert_eq!(buf[0], 1.0);
        assert_eq!(buf[1], 2.0);
        assert_eq!(buf[2], 3.0);
    }

    #[test]
    fn test_deeply_nested() {
        // Simulates: A.then(B).then(C)
        let params: (([f32; 1], [f32; 0]), [f32; 2]) = (([1.0], []), [2.0, 3.0]);
        let mut buf = [0.0f32; 4];
        params.write_to(&mut buf);
        assert_eq!(buf[0], 1.0);
        assert_eq!(buf[1], 2.0);
        assert_eq!(buf[2], 3.0);
    }

    #[test]
    fn test_const_len() {
        assert_eq!(<[f32; 0] as ParamArray>::LEN, 0);
        assert_eq!(<[f32; 3] as ParamArray>::LEN, 3);
        assert_eq!(<([f32; 2], [f32; 1]) as ParamArray>::LEN, 3);
    }
}
