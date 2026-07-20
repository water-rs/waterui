//! WGSL vector bridge types for `encase`.

macro_rules! shader_vector {
    ($name:ident, $length:literal, $zero:expr, $doc:literal) => {
        #[doc = $doc]
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
        pub struct $name([f32; $length]);

        impl $name {
            /// Vector with every component set to zero.
            pub const ZERO: Self = Self($zero);

            /// Creates a shader vector from its component array.
            #[must_use]
            pub const fn from_array(value: [f32; $length]) -> Self {
                Self(value)
            }

            /// Returns the vector components as an array.
            #[must_use]
            pub const fn as_array(self) -> [f32; $length] {
                self.0
            }
        }

        impl AsRef<[f32; $length]> for $name {
            fn as_ref(&self) -> &[f32; $length] {
                &self.0
            }
        }

        impl AsMut<[f32; $length]> for $name {
            fn as_mut(&mut self) -> &mut [f32; $length] {
                &mut self.0
            }
        }

        impl From<[f32; $length]> for $name {
            fn from(value: [f32; $length]) -> Self {
                Self::from_array(value)
            }
        }

        encase::impl_vector!($length, $name, f32; using AsRef AsMut From);
    };
}

shader_vector!(
    ShaderVec2,
    2,
    [0.0; 2],
    "Two-component floating-point vector with WGSL-compatible layout."
);
shader_vector!(
    ShaderVec3,
    3,
    [0.0; 3],
    "Three-component floating-point vector with WGSL-compatible layout."
);
shader_vector!(
    ShaderVec4,
    4,
    [0.0; 4],
    "Four-component floating-point vector with WGSL-compatible layout."
);

impl ShaderVec2 {
    /// Creates a vector from its `x` and `y` components.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self([x, y])
    }

    /// Returns the `x` component.
    #[must_use]
    pub const fn x(self) -> f32 {
        self.0[0]
    }

    /// Returns the `y` component.
    #[must_use]
    pub const fn y(self) -> f32 {
        self.0[1]
    }
}

impl ShaderVec3 {
    /// Creates a vector from its `x`, `y`, and `z` components.
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self([x, y, z])
    }
}

impl ShaderVec4 {
    /// Vector with every component set to one.
    pub const ONE: Self = Self([1.0; 4]);

    /// Creates a vector from its `x`, `y`, `z`, and `w` components.
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self([x, y, z, w])
    }
}
