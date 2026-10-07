//! The committed layer ops a [`SurfaceTree`](crate::SurfaceTree) applies,
//! and the change sets a [`Shared`](crate::Shared) queue drains. Everything
//! crossing to a consumer is owned; there are no locks anywhere in the
//! queue.

use kurbo::{Affine, Vec2};

use crate::Target;
use crate::WorkingColor;
use crate::animation::Animation;
use crate::backdrop::BackdropSample;
use crate::display_list::{Picture, SlotUpdate};
use crate::projective::Projective;
use crate::shape::ShapeData;
use crate::style::{BlendMode, FilterId};

/// Identifier of a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceId(u64);

impl SurfaceId {
    /// Creates an identifier from a raw value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Identifier of a layer within a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(u64);

impl LayerId {
    /// Creates an identifier from a raw value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Identifier of a backdrop group, allocated per surface by
/// [`Shared::allocate_backdrop`](crate::Shared::allocate_backdrop).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BackdropId(u64);

impl BackdropId {
    /// Creates an identifier from a raw value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// A property target plus the animation that reaches it.
#[derive(Clone, Debug)]
pub struct Prop<T> {
    /// The value the property moves to.
    pub target: T,
    /// The animation applied, if any; `None` snaps.
    pub animation: Option<Animation>,
}

/// What a layer draws, crossing to the consumer.
#[derive(Clone, Debug)]
pub enum ContentOp {
    /// The whole display list of a live content, sent on first commit.
    Replace(Picture),
    /// New values for a live content's bound slots.
    Update(Vec<SlotUpdate>),
    /// A shared immutable picture.
    Picture(Picture),
}

/// One layer mutation in a committed change set.
#[derive(Clone, Debug)]
pub enum LayerOp {
    /// Create a detached layer node.
    Create(LayerId),
    /// Remove a layer node — only it. Its children stay in the tree,
    /// detached and undrawn, until their own `Remove` or re-attachment.
    Remove(LayerId),
    /// Set the local transform.
    Transform(LayerId, Prop<Affine>),
    /// Sets the translation in local coordinates; initially zero.
    Translation(LayerId, Prop<Vec2>),
    /// Sets the unwrapped rotation angle in radians; initially zero.
    Rotation(LayerId, Prop<f64>),
    /// Sets the x/y scale factors; initially (1, 1).
    Scale(LayerId, Prop<Vec2>),
    /// Sets x/y skew angles in radians; initially zero.
    Skew(LayerId, Prop<Vec2>),
    /// Sets the local pivot for rotation, skew and scale; initially zero.
    Pivot(LayerId, Prop<Vec2>),
    /// Sets the projection base of a projective layer; never animated.
    Projection(LayerId, Projective),
    /// Sets the X/Y depth-rotation angles in radians; initially zero.
    Tilt(LayerId, Prop<Vec2>),
    /// Sets the translation along Z; initially zero.
    Depth(LayerId, Prop<f64>),
    /// Removes projection, tilt and depth; the layer is affine again.
    ClearProjection(LayerId),
    /// Set the opacity.
    Opacity(LayerId, Prop<f32>),
    /// Set the scroll offset.
    ScrollOffset(LayerId, Prop<Vec2>),
    /// Set or clear the clip shape.
    Clip(LayerId, Option<ShapeData>),
    /// Set the blend mode.
    Blend(LayerId, BlendMode),
    /// Set or clear the filter.
    Filter(LayerId, Option<FilterId>),
    /// Set or clear the backdrop sample (group and optional per-member
    /// effect).
    Backdrop(LayerId, Option<BackdropSample>),
    /// Set the layer content, or clear it.
    Content(LayerId, Option<ContentOp>),
    /// Append a child.
    Push {
        /// The parent.
        parent: LayerId,
        /// The child.
        child: LayerId,
    },
    /// Insert a child at an index.
    Insert {
        /// The parent.
        parent: LayerId,
        /// Child index.
        index: usize,
        /// The child.
        child: LayerId,
    },
    /// Remove a child from a parent's child list.
    Detach {
        /// The parent.
        parent: LayerId,
        /// The child.
        child: LayerId,
    },
}

impl LayerOp {
    /// The layer the op edits: the op's own id, or the parent for child
    /// list ops.
    pub(crate) const fn layer(&self) -> LayerId {
        match *self {
            Self::Create(id)
            | Self::Remove(id)
            | Self::Transform(id, _)
            | Self::Translation(id, _)
            | Self::Rotation(id, _)
            | Self::Scale(id, _)
            | Self::Skew(id, _)
            | Self::Pivot(id, _)
            | Self::Projection(id, _)
            | Self::Tilt(id, _)
            | Self::Depth(id, _)
            | Self::ClearProjection(id)
            | Self::Opacity(id, _)
            | Self::ScrollOffset(id, _)
            | Self::Clip(id, _)
            | Self::Blend(id, _)
            | Self::Filter(id, _)
            | Self::Backdrop(id, _)
            | Self::Content(id, _)
            | Self::Push { parent: id, .. }
            | Self::Insert { parent: id, .. }
            | Self::Detach { parent: id, .. } => id,
        }
    }

    /// The animation slot of the op's animatable prop, if it carries
    /// one: [`LayerEdit::animation`] retargets the last queued op
    /// through it. `None` for the projection matrix and every
    /// non-animatable op.
    ///
    /// [`LayerEdit::animation`]: crate::LayerEdit::animation
    #[expect(
        clippy::match_same_arms,
        reason = "each animatable Prop is a different type; the arms cannot merge"
    )]
    pub(crate) const fn animation_mut(&mut self) -> Option<&mut Option<Animation>> {
        match self {
            Self::Transform(_, prop) => Some(&mut prop.animation),
            Self::Translation(_, prop) => Some(&mut prop.animation),
            Self::Rotation(_, prop) => Some(&mut prop.animation),
            Self::Scale(_, prop) => Some(&mut prop.animation),
            Self::Skew(_, prop) => Some(&mut prop.animation),
            Self::Pivot(_, prop) => Some(&mut prop.animation),
            Self::Tilt(_, prop) => Some(&mut prop.animation),
            Self::Depth(_, prop) => Some(&mut prop.animation),
            Self::Opacity(_, prop) => Some(&mut prop.animation),
            Self::ScrollOffset(_, prop) => Some(&mut prop.animation),
            _ => None,
        }
    }
}

/// A target's render-side install payload, sealed: only a [`GpuInstalls`]
/// target can wrap one, through [`LayerContent::install`]. The consumer
/// unwraps it with [`into_inner`](Install::into_inner) and applies it.
///
/// [`GpuInstalls`]: crate::GpuInstalls
/// [`LayerContent::install`]: crate::LayerContent::install
pub struct Install<T: Target> {
    inner: T::Install,
}

impl<T: Target> Install<T> {
    /// Wraps `inner`. `pub(crate)`: [`LayerContent::install`], gated on
    /// [`GpuInstalls`], is the only call site.
    ///
    /// [`GpuInstalls`]: crate::GpuInstalls
    /// [`LayerContent::install`]: crate::LayerContent::install
    pub(crate) const fn new(inner: T::Install) -> Self {
        Self { inner }
    }

    /// The payload to realise.
    #[must_use]
    pub fn into_inner(self) -> T::Install {
        self.inner
    }
}

impl<T: Target> std::fmt::Debug for Install<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The payload is the target's opaque render-side value.
        f.write_str("Install(..)")
    }
}

/// One committed op: a layer mutation, or an opaque render-side install a
/// capability method wrapped (GPU producers) travelling in order with the
/// layer ops.
pub enum Op<T: Target> {
    /// A layer-tree mutation.
    Layer(LayerOp),
    /// A render-side install on `layer`, applied in order: the reported
    /// declared alpha is noted on the layer — `None`, no frame landed
    /// yet, notes it not known opaque.
    Install(LayerId, Install<T>),
}

impl<T: Target> std::fmt::Debug for Op<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Install` carries an opaque render-side payload.
        match self {
            Self::Layer(op) => f.debug_tuple("Layer").field(op).finish(),
            Self::Install(..) => f.write_str("Install(..)"),
        }
    }
}

impl<T: Target> Op<T> {
    /// The layer the op edits.
    pub(crate) const fn layer(&self) -> LayerId {
        match *self {
            Self::Layer(ref op) => op.layer(),
            Self::Install(id, _) => id,
        }
    }
}

/// The committed change set for one surface.
#[derive(Debug)]
pub struct ChangeSet<T: Target> {
    /// New clear colour, when set this commit.
    pub clear: Option<WorkingColor>,
    /// The ops, in order.
    pub ops: Vec<Op<T>>,
    /// Replaced pictures, cleared on the consumer's side, whose storage
    /// returns to the queueing side.
    pub recycled: Vec<(LayerId, Picture)>,
    /// Whether recorded-content operands still animate: their tracks live
    /// on the queueing side and need the next frame's sample, at the fast
    /// rate class like a spring or curve on a layer.
    pub animating: bool,
}
