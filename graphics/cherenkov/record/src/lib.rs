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

pub mod animation;
pub mod color;
pub mod display_list;
pub mod glyph;
pub mod paint;
pub mod record;
pub mod resource;
pub mod shape;
pub mod size;
pub mod style;

pub use kurbo;
/// Monotonic presentation clock: std on native, browser performance clock on wasm.
pub use web_time::Instant;

pub use crate::animation::{
    Animatable, Animation, Curve, Decay, Lanes, Spring, curve_value, decay_step, settled,
    spring_step,
};
pub use crate::color::{
    Color, ColorSpace, DisplayP3, DynColor, LinearDisplayP3, LinearSrgb, Rec2020, Srgb,
    WorkingColor,
};
pub use crate::display_list::{
    Command, Dirty, DisplayList, DisplayListView, Operand, OperandKind, OperandRef, Operands,
    Picture, ScopeError, Slot, SlotUpdate,
};
pub use crate::glyph::{FontId, Glyph, GlyphRun, GlyphStyle};
pub use crate::paint::{
    ColorStop, Extend, ImageId, ImagePattern, Interpolation, LinearGradient,
    MeshColorInterpolation, MeshGradient, MeshGradientError, Paint, RadialGradient, Sampling,
    ShaderId, ShaderPaint, SweepGradient, TransformedPaint,
};
pub use crate::record::{
    Animating, Binding, Content, ContentChange, ContentSpare, Draw, Fixed, Live, LiveOwner,
    Recorder, SampleFlag, StaticRecorder,
};
pub use crate::resource::{BackdropShaderId, ResourceId};
pub use crate::shape::{
    ContinuousRect, EvenOdd, FillRule, PATH_TOLERANCE, PathRef, Semantic, Shape, ShapeData,
};
pub use crate::size::LayoutSize;
pub use crate::style::{BlendMode, BlendSpace, FilterId, Group, Shadow};
pub use kurbo::Stroke;
