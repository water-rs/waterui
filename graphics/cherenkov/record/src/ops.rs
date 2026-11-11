//! The committed layer ops a [`SurfaceTree`](crate::SurfaceTree) applies,
//! and the change sets a [`Shared`](crate::Shared) queue drains.
//!
//! Everything crossing to a consumer is owned; there are no locks anywhere
//! in the queue.

use kurbo::{Affine, Vec2};

use crate::Instant;
use crate::Target;
use crate::WorkingColor;
use crate::animation::{Animation, Keyframes, Repeat};
use crate::backdrop::BackdropSample;
use crate::display_list::{Edit, Picture, SlotUpdate};
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
    /// The instant the animation starts at, on the host's clock. `None`
    /// keeps the engine's own timing: the track starts at the first frame
    /// that samples it, and a retarget continues from the previous track's
    /// last sample. `Some(t)` starts the track at `t`, and a retarget
    /// continues from the previous track's value and velocity evaluated at
    /// `t`. A `start` without an `animation` is ignored.
    pub start: Option<Instant>,
}

/// One axis of a two-axis layer property — translation or scale — which
/// animates per axis, so x and y can run different animations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    /// The horizontal axis.
    X,
    /// The vertical axis.
    Y,
}

impl Axis {
    /// Both axes, x first: the order a [`Vec2`]'s lanes take.
    pub const BOTH: [Self; 2] = [Self::X, Self::Y];

    /// The axis's component of `value`.
    #[must_use]
    pub const fn of(self, value: Vec2) -> f64 {
        match self {
            Self::X => value.x,
            Self::Y => value.y,
        }
    }

    /// `value` with this axis's component replaced by `component`.
    #[must_use]
    pub const fn with(self, value: Vec2, component: f64) -> Vec2 {
        match self {
            Self::X => Vec2::new(component, value.y),
            Self::Y => Vec2::new(value.x, component),
        }
    }
}

/// A property's keyframe track: the value the property holds once a finite
/// repeat ends, and the track it runs until then.
#[derive(Clone, Debug)]
pub struct KeyframeProp<T> {
    /// The value the property holds after the last cycle.
    pub target: T,
    /// The track, starting at the first frame that samples it.
    pub keyframes: Keyframes<T>,
    /// How many cycles it runs.
    pub repeat: Repeat,
}

/// A keyframe track on one of the layer properties that take one.
#[derive(Clone, Debug)]
pub enum LayerKeyframes {
    /// The local transform, interpolated coefficient-wise.
    Transform(KeyframeProp<Affine>),
    /// The translation component.
    Translation(KeyframeProp<Vec2>),
    /// The unwrapped rotation angle in radians.
    Rotation(KeyframeProp<f64>),
    /// The x/y scale factors.
    Scale(KeyframeProp<Vec2>),
    /// The opacity.
    Opacity(KeyframeProp<f32>),
}

/// The instant a bound change's [`Animation`] starts at.
///
/// Carried in the change's `Context` metadata next to that animation when
/// the host drives animations from its own clock. Without it the track
/// starts at the first frame that samples it.
///
/// [`Animation`]: crate::Animation
#[derive(Clone, Copy, Debug)]
pub struct AnimationStart(pub Instant);

/// What a layer draws, crossing to the consumer.
#[derive(Clone, Debug)]
pub enum ContentOp {
    /// The whole display list of a live content, sent on first commit.
    Replace(Picture),
    /// New values for a live content's bound slots.
    Update(Vec<SlotUpdate>),
    /// A live content's regions edited in place, then slot updates: the
    /// target applies it to the list it holds ([`Picture::edit`]).
    ///
    /// Boxed like [`ContentChange::Edit`](crate::ContentChange::Edit), so
    /// the op keeps the size and layout of the frequent variants.
    Edit(Box<Edit>),
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
    /// Sets one axis of the translation with a track of its own: the other
    /// axis keeps whatever it runs. `Translation` sets both axes at once.
    TranslationAxis(LayerId, Axis, Prop<f64>),
    /// Sets the x/y scale factors; initially (1, 1).
    Scale(LayerId, Prop<Vec2>),
    /// Sets one axis's scale factor with a track of its own: the other
    /// axis keeps whatever it runs. `Scale` sets both axes at once.
    ScaleAxis(LayerId, Axis, Prop<f64>),
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
    /// Run a keyframe track on a property, replacing its track.
    Keyframes(LayerId, LayerKeyframes),
    /// Set or clear the clip shape.
    Clip(LayerId, Option<ShapeData>),
    /// Set the blend mode.
    Blend(LayerId, BlendMode),
    /// Set or clear the filter.
    Filter(LayerId, Option<FilterId>),
    /// Set or clear the backdrop sample (group and optional per-member
    /// effect).
    Backdrop(LayerId, Option<BackdropSample>),
    /// Set or clear the layer's host-window marker: a marked layer shows
    /// no recorded content — an external host window occupies its frame
    /// — and every mark splits the surface's paint order into regions,
    /// each presented in its own band stacked above that window
    /// (water-rs/waterui#2225). The mark names the window by the host's
    /// own id for it, which is how the host finds the band to stack.
    HostWindow(LayerId, Option<u64>),
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
            | Self::TranslationAxis(id, ..)
            | Self::Rotation(id, _)
            | Self::Scale(id, _)
            | Self::ScaleAxis(id, ..)
            | Self::Skew(id, _)
            | Self::Pivot(id, _)
            | Self::Projection(id, _)
            | Self::Tilt(id, _)
            | Self::Depth(id, _)
            | Self::ClearProjection(id)
            | Self::Opacity(id, _)
            | Self::ScrollOffset(id, _)
            | Self::Keyframes(id, _)
            | Self::Clip(id, _)
            | Self::Blend(id, _)
            | Self::Filter(id, _)
            | Self::Backdrop(id, _)
            | Self::HostWindow(id, _)
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
            Self::Rotation(_, prop)
            | Self::TranslationAxis(_, _, prop)
            | Self::ScaleAxis(_, _, prop) => Some(&mut prop.animation),
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

/// A target's sealed render-side install payload.
///
/// Only a [`GpuInstalls`] target can wrap one, through
/// [`LayerContent::install`]. The consumer unwraps it with
/// [`into_inner`](Install::into_inner) and applies it.
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
