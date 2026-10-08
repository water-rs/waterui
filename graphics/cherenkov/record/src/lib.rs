//! The render-target-neutral recording layer of the Cherenkov 2D engine:
//! the drawing verbs ([`Draw`]), the recorders that accept them, the
//! [`DisplayList`] they produce, and the colour, shape, paint, style and
//! glyph types the commands carry.
//!
//! Nothing here knows an engine, a surface, a GPU or a shaper: a render
//! target that draws recorded content another way consumes a
//! [`DisplayList`] through its [`view`](DisplayList::view) and applies
//! [`SlotUpdate`]s as they arrive.
//!
//! - [`Picture::record`] records constants on any thread into an
//!   immutable, shareable [`Picture`].
//! - A live recorder — [`Recorder`], built by the installing target —
//!   accepts nami signals anywhere a value is accepted. A signal's later
//!   changes become [`SlotUpdate`]s that regenerate only the commands
//!   referencing it, and a change carrying an [`Animation`] animates the
//!   operand instead.
//! - A target drains [`Content::take_change`] for the [`ContentChange`] to
//!   apply: first the whole [`Picture`], then the changed slots.
//! - [`Content::record_layered`] opens a recording that may declare
//!   backdrop materials ([`material`]), which never enter a display list:
//!   the recording comes back split at each one for its host to realize.

pub mod animation;
pub mod backdrop;
pub mod color;
pub mod display_list;
pub mod error;
pub mod frame;
pub mod glyph;
pub mod image;
pub mod material;
pub mod ops;
pub mod paint;
pub mod projective;
pub mod record;
pub mod resource;
pub mod shape;
pub mod size;
pub mod style;
pub mod surface;
pub mod target;
pub mod text;
pub mod tree;

pub use kurbo;
/// Monotonic presentation clock: std on native, browser performance clock on wasm.
pub use web_time::Instant;

pub use crate::animation::{
    Animatable, Animation, AnimationTrack, Curve, Decay, Lanes, Spring, curve_value, decay_step,
    settled, spring_step,
};
pub use crate::backdrop::{
    BackdropEffect, BackdropSample, BackdropShaderEffect, BackdropShaderSource, BackdropSpec,
    CaptureLevels, CaptureLevelsError, CaptureScale, CaptureScaleError, ColorMatrix, LevelRamp,
    LevelRampError, Refraction, Rim,
};
pub use crate::color::{
    Color, ColorSpace, DisplayP3, DynColor, LinearDisplayP3, LinearSrgb, Rec2020, Srgb,
    WorkingColor,
};
pub use crate::display_list::{
    Command, Dirty, DisplayList, DisplayListView, Operand, OperandKind, OperandRef, Operands,
    Picture, ScopeError, Slot, SlotUpdate, blends_within, translucent_within,
};
pub use crate::error::ResourceError;
pub use crate::frame::RefreshRange;
pub use crate::glyph::{FontId, Glyph, GlyphRun, GlyphStyle};
pub use crate::image::{
    Astc4x4, Bc7, Etc2Rgba, Format, ImageColorSpace, ImageData, ImageFormat, ImageUpload, Rgba8,
    Rgba16F,
};
pub use crate::material::{
    BackdropMaterial, CaptureClass, LayeredContent, MaterialCapture, MaterialEffect,
    MaterialGrouping, MaterialRegistry, MaterialRun, MaterialScope, MaterialShader,
};
pub use crate::ops::{
    BackdropId, ChangeSet, ContentOp, Install, LayerId, LayerOp, Op, Prop, SurfaceId,
};
pub use crate::paint::{
    ColorStop, Extend, ImageId, ImagePattern, Interpolation, LinearGradient,
    MeshColorInterpolation, MeshGradient, MeshGradientError, Paint, RadialGradient, Sampling,
    ShaderId, ShaderPaint, SweepGradient, TransformedPaint,
};
pub use crate::projective::{Projective, ProjectiveError};
pub use crate::record::{
    Animating, Binding, Content, ContentChange, ContentSpare, Draw, Fixed, Live, LiveOwner,
    Recorder, SampleFlag, StaticRecorder,
};
pub use crate::resource::{BackdropShaderId, ImageLimits, ResourceId};
pub use crate::shape::{
    ContinuousRect, EvenOdd, FillRule, PATH_TOLERANCE, PathRef, Semantic, Shape, ShapeData,
};
pub use crate::size::LayoutSize;
pub use crate::style::{BlendMode, BlendSpace, FilterId, Group, Shadow};
pub use crate::surface::{Layer, LayerContent, LayerEdit, LayerOwner, Shared, Transaction};
pub use crate::target::{BackdropSampling, GpuInstalls, ProjectiveLayers, Queue, Target};
pub use crate::text::TextLayoutId;
pub use crate::tree::{LayerAnimations, LayerNode, Realize, SurfaceTree, snap_animating};
pub use kurbo::Stroke;
