//! Cherenkov is a GPU 2D rendering engine for modern hardware.
//!
//! The design of the public API is `docs/api.md`. This crate is the front
//! end: the vocabulary of what to draw (colour, shapes, paint, styles), the
//! recording of it into display lists, and the shared engine — [`Engine`],
//! [`Surface`]s, the layer tree, transactions, resource handles, animation
//! and the render-thread loop, plus the [`Backend`] contract a backend
//! crate implements. There is no GPU dependency here.
//!
//! - [`Picture::record`] records constants on any thread into an immutable,
//!   shareable [`Picture`].
//! - [`Surface::record`] records on the UI thread and accepts nami signals
//!   anywhere a value is accepted. A signal's later changes become
//!   [`SlotUpdate`]s that regenerate only the commands referencing it.
//! - [`Engine::render`] drains every surface's queued change set into one
//!   commit per frame, samples the animations at the frame time and renders
//!   on the render thread.

// The recording layer is `cherenkov-record`; these imports keep its
// modules at their old `crate::*` paths so engine code is unchanged.
use cherenkov_record::{animation, color, display_list, glyph, paint, record, shape, size, style};

mod backdrop;
mod backend;
mod capability;
mod config;
mod engine;
mod error;
mod frame;
mod image;
#[cfg(target_arch = "wasm32")]
mod local;
pub mod lowering;
mod message;
mod projective;
mod resource;
mod surface;
mod text;
mod tree;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use crate::backend::RenderTransfer;
pub use kurbo;
/// The shaping library [`TextLayout`] wraps, at the version the engine
/// lowers.
pub use parley;
/// Monotonic presentation clock: std on native, browser performance clock on wasm.
pub use web_time::Instant;

pub use crate::animation::{
    Animatable, Animation, AnimationTrack, Curve, Decay, Lanes, Spring, curve_value, decay_step,
    settled, spring_step,
};
pub use crate::backdrop::{
    BackdropEffect, BackdropShaderEffect, BackdropShaderSource, ColorMatrix, Refraction, Rim,
};
pub use crate::backend::{
    Backend, Display, Frame, Redraw, Renderer, SurfaceFrame, SurfaceInfo, Visibility,
};
pub use crate::capability::{
    Backdrop, BackdropChain, BackdropRuns, BackdropShaders, DrainedProducer, Effects, Filters,
    GpuContent, HdrOutput, Planes, ProjectiveLayers, Runs, ShaderPaint as ShaderPaintCapability,
    ShaderSource, Uploads,
};
pub use crate::color::{
    Color, ColorSpace, DisplayP3, DynColor, LinearDisplayP3, LinearSrgb, Rec2020, Srgb,
    WorkingColor,
};
pub use crate::config::{Budget, Bytes, MemoryUsage, Pressure};
pub use crate::display_list::{
    Command, Dirty, DisplayList, DisplayListView, Operand, OperandKind, OperandRef, Operands,
    Picture, ScopeError, Slot, SlotUpdate,
};
pub use crate::engine::Engine;
pub use crate::engine::{CompletionWaker, SurfaceVisibility, WakeGate};
pub use crate::error::{EngineError, RenderError, ResourceError, SurfaceError};
pub use crate::frame::{
    DEFAULT_REFRESH, FrameId, FrameStats, FrameTime, FrameTiming, Next, Offscreen, OffscreenFormat,
    PassTiming, Phases, Readback, RefreshRange,
};
pub use crate::glyph::{FontId, Glyph, GlyphRun, GlyphStyle};
pub use crate::image::{
    Astc4x4, Bc7, Etc2Rgba, Format, ImageColorSpace, ImageData, ImageFormat, ImageUpload, Rgba8,
    Rgba16F,
};
pub use crate::message::{BackdropId, ContentOp, FontData, LayerId, ProducerId, Prop, SurfaceId};
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
pub use crate::resource::{
    BackdropGroup, BackdropSample, BackdropShader, Filter, Font, FontSource, FrameSink,
    GpuProducer, Image, Shader,
};
pub use crate::shape::{
    ContinuousRect, EvenOdd, FillRule, PATH_TOLERANCE, PathRef, Semantic, Shape, ShapeData,
};
pub use crate::size::LayoutSize;
pub use crate::style::{BlendMode, BlendSpace, FilterId, Group, Shadow};
pub use crate::surface::{Layer, LayerContent, LayerEdit, Surface, Transaction};
pub use crate::text::{TextLayout, draw_text};
pub use crate::tree::{LayerAnimations, LayerNode, SurfaceTree, snap_animating};
pub use cherenkov_record::BackdropShaderId;
pub use cherenkov_record::ResourceId;
pub use kurbo::Stroke;
