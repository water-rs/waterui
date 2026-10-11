//! The snippet ABI: kinds, parameter types and values, precision, and the
//! canonical working-space block.

use naga::{Scalar, Span, StructMember, Type, TypeInner, VectorSize};

/// The declaration of the working-space constants, as a snippet must write it.
///
/// `luma` holds the luma coefficients of the linear working space (linear
/// Display P3), so filters never hard-code Rec. 709 values.
pub const WORKING_SPACE_WGSL: &str = "struct WorkingSpace { luma: vec3<f32> }";

/// What a snippet does to pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnippetKind {
    /// Maps one colour to one colour.
    Color,
    /// Samples its input texture around a coordinate.
    Spatial,
}

/// The filter mode a spatial snippet declares for its input sampler.
///
/// The mode is part of the snippet ABI: the executor must bind a sampler
/// honoring the declaration. Only a [`SamplerFilter::Point`] declaration makes
/// a sampled stage foldable, because folding a colour prefix into a filtered
/// sample computes `prefix(lerp(a, b))` where materializing first computes
/// `lerp(prefix(a), prefix(b))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SamplerFilter {
    /// The executor must bind a nearest (point) sampler: every sample returns
    /// an exact texel, so a colour prefix commutes with the access.
    Point,
    /// The executor may bind a filtering sampler. The stage is not foldable
    /// through this sampler.
    Filtered,
}

/// Why a spatial stage cannot fold a colour prefix into its accesses of
/// `input`. Folding is only equivalent when every access is an exact texel
/// read; see [`Snippet::parse`](crate::Snippet::parse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoldBlocker {
    /// A sample of `input` goes through the declared filtering sampler: the
    /// fold computes `prefix(lerp(a, b))` where materializing first computes
    /// `lerp(prefix(a), prefix(b))`.
    FilteringSampler,
    /// A sample of `input` gathers (`textureGather`): the fold would apply the
    /// prefix to a gathered component vector.
    Gather,
    /// A sample of `input` compares a depth reference.
    DepthComparison,
    /// `input` is used in a way the folder cannot classify.
    UnknownUse,
}

/// The precision of colour values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Precision {
    /// 32-bit floats.
    F32,
    /// 16-bit floats.
    F16,
}

impl Precision {
    pub(crate) const fn scalar(self) -> Scalar {
        match self {
            Self::F32 => Scalar::F32,
            Self::F16 => Scalar::F16,
        }
    }

    pub(crate) const fn width(self) -> u8 {
        match self {
            Self::F32 => 4,
            Self::F16 => 2,
        }
    }
}

/// The type of one snippet parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamType {
    /// `f32`.
    F32,
    /// `vec2<f32>`.
    Vec2,
    /// `vec3<f32>`.
    Vec3,
    /// `vec4<f32>`.
    Vec4,
}

impl ParamType {
    pub(crate) const fn from_inner(inner: &TypeInner) -> Option<Self> {
        match *inner {
            TypeInner::Scalar(Scalar::F32) => Some(Self::F32),
            TypeInner::Vector {
                size,
                scalar: Scalar::F32,
            } => Some(match size {
                VectorSize::Bi => Self::Vec2,
                VectorSize::Tri => Self::Vec3,
                VectorSize::Quad => Self::Vec4,
            }),
            _ => None,
        }
    }

    pub(crate) const fn inner(self) -> TypeInner {
        match self {
            Self::F32 => TypeInner::Scalar(Scalar::F32),
            Self::Vec2 => TypeInner::Vector {
                size: VectorSize::Bi,
                scalar: Scalar::F32,
            },
            Self::Vec3 => TypeInner::Vector {
                size: VectorSize::Tri,
                scalar: Scalar::F32,
            },
            Self::Vec4 => TypeInner::Vector {
                size: VectorSize::Quad,
                scalar: Scalar::F32,
            },
        }
    }

    /// Size in bytes in a uniform block.
    #[must_use]
    pub const fn size(self) -> u32 {
        match self {
            Self::F32 => 4,
            Self::Vec2 => 8,
            Self::Vec3 => 12,
            Self::Vec4 => 16,
        }
    }

    /// Alignment in bytes in a uniform block.
    #[must_use]
    pub const fn align(self) -> u32 {
        match self {
            Self::F32 => 4,
            Self::Vec2 => 8,
            Self::Vec3 | Self::Vec4 => 16,
        }
    }
}

/// A constant value for a specialized parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    /// An `f32`.
    F32(f32),
    /// A `vec2<f32>`.
    Vec2([f32; 2]),
    /// A `vec3<f32>`.
    Vec3([f32; 3]),
    /// A `vec4<f32>`.
    Vec4([f32; 4]),
}

impl ParamValue {
    /// The type of this value.
    #[must_use]
    pub const fn ty(&self) -> ParamType {
        match self {
            Self::F32(_) => ParamType::F32,
            Self::Vec2(_) => ParamType::Vec2,
            Self::Vec3(_) => ParamType::Vec3,
            Self::Vec4(_) => ParamType::Vec4,
        }
    }

    pub(crate) const fn components(&self) -> &[f32] {
        match self {
            Self::F32(v) => core::slice::from_ref(v),
            Self::Vec2(v) => v,
            Self::Vec3(v) => v,
            Self::Vec4(v) => v,
        }
    }
}

impl From<f32> for ParamValue {
    fn from(value: f32) -> Self {
        Self::F32(value)
    }
}

impl From<[f32; 2]> for ParamValue {
    fn from(value: [f32; 2]) -> Self {
        Self::Vec2(value)
    }
}

impl From<[f32; 3]> for ParamValue {
    fn from(value: [f32; 3]) -> Self {
        Self::Vec3(value)
    }
}

impl From<[f32; 4]> for ParamValue {
    fn from(value: [f32; 4]) -> Self {
        Self::Vec4(value)
    }
}

/// One declared snippet parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Param {
    /// The member name in the snippet's `Params` struct.
    pub name: String,
    /// The member type.
    pub ty: ParamType,
}

/// The inner type of the canonical working-space block.
fn working_space_type(vec3: naga::Handle<Type>) -> Type {
    Type {
        name: Some("WorkingSpace".to_owned()),
        inner: TypeInner::Struct {
            members: vec![StructMember {
                name: Some("luma".to_owned()),
                ty: vec3,
                binding: None,
                offset: 0,
            }],
            span: 16,
        },
    }
}

/// Inserts (or finds) the canonical working-space block type.
pub fn insert_working_space(module: &mut naga::Module) -> naga::Handle<Type> {
    let vec3 = module.types.insert(
        Type {
            name: None,
            inner: ParamType::Vec3.inner(),
        },
        Span::UNDEFINED,
    );
    module
        .types
        .insert(working_space_type(vec3), Span::UNDEFINED)
}

/// Whether `ty` in `module` is exactly the canonical working-space block:
/// same members, offsets and span, under any struct name.
pub fn is_working_space(module: &naga::Module, ty: naga::Handle<Type>) -> bool {
    let TypeInner::Struct { ref members, span } = module.types[ty].inner else {
        return false;
    };
    span == 16
        && matches!(
            members.as_slice(),
            [member] if member.name.as_deref() == Some("luma")
                && member.binding.is_none()
                && member.offset == 0
                && module.types[member.ty].inner == ParamType::Vec3.inner()
        )
}
