//! The flattened layout of a WGSL type: every value is a run of 32-bit
//! scalar slots, one register each.

use naga::{ArraySize, Module, ScalarKind, TypeInner};

use super::LaneError;

/// How a slot's 32 bits are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `f32` (and `f16`, which the executor evaluates as `f32`).
    Float,
    /// `i32`.
    Sint,
    /// `u32`.
    Uint,
    /// `bool`, stored as `0` or `1`.
    Bool,
}

/// A type's slot layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    /// One slot.
    Scalar(Kind),
    /// `n` slots of one kind.
    Vector(u32, Kind),
    /// `columns` column vectors of `rows` floats, column-major.
    Matrix {
        /// Column count.
        columns: u32,
        /// Row count.
        rows: u32,
    },
    /// `count` elements laid out back to back.
    Array(Box<Self>, u32),
    /// Members laid out in declaration order.
    Struct(Vec<Self>),
    /// A texture or sampler: one slot holding the handle the host bound
    /// to it, the same in every lane.
    Opaque,
}

impl Ty {
    /// The layout of `inner`.
    pub fn of(module: &Module, inner: &TypeInner) -> Result<Self, LaneError> {
        Ok(match *inner {
            TypeInner::Scalar(scalar) => Self::Scalar(kind(scalar)?),
            TypeInner::Vector { size, scalar } => Self::Vector(size as u32, kind(scalar)?),
            TypeInner::Matrix {
                columns,
                rows,
                scalar,
            } => {
                if kind(scalar)? != Kind::Float {
                    return Err(LaneError::unsupported("a non-float matrix"));
                }
                Self::Matrix {
                    columns: columns as u32,
                    rows: rows as u32,
                }
            }
            TypeInner::Array { base, size, .. } => {
                let ArraySize::Constant(count) = size else {
                    return Err(LaneError::unsupported(
                        "a runtime-sized or override-sized array",
                    ));
                };
                Self::Array(
                    Box::new(Self::of(module, &module.types[base].inner)?),
                    count.get(),
                )
            }
            TypeInner::Struct { ref members, .. } => Self::Struct(
                members
                    .iter()
                    .map(|member| Self::of(module, &module.types[member.ty].inner))
                    .collect::<Result<_, _>>()?,
            ),
            TypeInner::Pointer { base, .. } => Self::of(module, &module.types[base].inner)?,
            TypeInner::ValuePointer { size, scalar, .. } => match size {
                Some(size) => Self::Vector(size as u32, kind(scalar)?),
                None => Self::Scalar(kind(scalar)?),
            },
            TypeInner::Image { .. } | TypeInner::Sampler { .. } => Self::Opaque,
            ref other => {
                return Err(LaneError::unsupported(format!("the type {other:?}")));
            }
        })
    }

    /// The number of slots.
    pub fn slots(&self) -> usize {
        match self {
            Self::Scalar(_) | Self::Opaque => 1,
            Self::Vector(n, _) => *n as usize,
            Self::Matrix { columns, rows } => (*columns * *rows) as usize,
            Self::Array(element, count) => element.slots() * *count as usize,
            Self::Struct(members) => members.iter().map(Self::slots).sum(),
        }
    }

    /// The scalar kind of a scalar or vector (or the float of a matrix).
    pub fn scalar(&self) -> Result<Kind, LaneError> {
        match self {
            Self::Scalar(kind) | Self::Vector(_, kind) => Ok(*kind),
            Self::Matrix { .. } => Ok(Kind::Float),
            other => Err(LaneError::unsupported(format!(
                "arithmetic on the composite {other:?}"
            ))),
        }
    }

    /// Element `index`'s slot offset and type.
    pub fn element(&self, index: u32) -> Result<(usize, Self), LaneError> {
        match self {
            Self::Vector(n, kind) if index < *n => Ok((index as usize, Self::Scalar(*kind))),
            Self::Matrix { columns, rows } if index < *columns => {
                Ok(((index * rows) as usize, Self::Vector(*rows, Kind::Float)))
            }
            Self::Array(element, count) if index < *count => {
                Ok((index as usize * element.slots(), (**element).clone()))
            }
            Self::Struct(members) if (index as usize) < members.len() => {
                let offset = members[..index as usize].iter().map(Self::slots).sum();
                Ok((offset, members[index as usize].clone()))
            }
            other => Err(LaneError::unsupported(format!(
                "element {index} of {other:?}"
            ))),
        }
    }

    /// A dynamically indexed element's type, stride in slots, and the
    /// element count.
    pub fn indexed(&self) -> Result<(Self, usize, u32), LaneError> {
        match self {
            Self::Vector(n, kind) => Ok((Self::Scalar(*kind), 1, *n)),
            Self::Matrix { columns, rows } => {
                Ok((Self::Vector(*rows, Kind::Float), *rows as usize, *columns))
            }
            Self::Array(element, count) => Ok(((**element).clone(), element.slots(), *count)),
            other => Err(LaneError::unsupported(format!(
                "a dynamic index into {other:?}"
            ))),
        }
    }
}

fn kind(scalar: naga::Scalar) -> Result<Kind, LaneError> {
    Ok(match (scalar.kind, scalar.width) {
        (ScalarKind::Float | ScalarKind::AbstractFloat, _) => Kind::Float,
        (ScalarKind::Sint, 4) | (ScalarKind::AbstractInt, _) => Kind::Sint,
        (ScalarKind::Uint, 4) => Kind::Uint,
        (ScalarKind::Bool, _) => Kind::Bool,
        (kind, width) => {
            return Err(LaneError::unsupported(format!(
                "the {width}-byte {kind:?} scalar"
            )));
        }
    })
}
