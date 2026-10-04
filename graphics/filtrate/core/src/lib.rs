#![no_std]
#![cfg_attr(
    test,
    allow(
        clippy::float_cmp,
        reason = "tests assert exact filter parameter values"
    )
)]
//! Long-term stable abstractions for filter pipelines.
//!
//! `filtrate-core` describes filters as pure data. A [`Filter`] is a sequence
//! of stages, each a shader function (a snippet for the shared composer,
//! `cherenkov-shader`), plus the parameters that feed them. Executors — the
//! reference wgpu executor in `filtrate`, or the Cherenkov engine — compose
//! the stages and decide how to run them.
//!
//! This crate has no dependencies: no GPU, no shader compiler and no reactive
//! system. Reactive frontends provide their own [`FilterParam`]
//! implementations on top of these abstractions.
//!
//! # Filter kinds
//!
//! Every filter has a kind, and the kind is a type:
//!
//! - A [`ColorFilter`] maps each pixel's colour to a colour, independently of
//!   its neighbours. Its [`ColorFilter::LINEAR`] property says whether it is a
//!   linear map on premultiplied RGBA with an identity alpha row —
//!   offsets proportional to alpha (Brightness's `+amount·a`, for example)
//!   are matrix coefficients and commute with src-over, so they qualify;
//!   only constant, non-alpha-scaled offsets disqualify. It is the property
//!   an executor needs before pushing a filter down into the shading of
//!   each primitive.
//! - A [`SpatialFilter`] samples its input around each pixel. Its
//!   [`SpatialFilter::footprint`] is the largest distance of any sample it
//!   takes, in pixels plus a fraction of the image extent.
//!
//! [`Chain<A, B>`] is a colour filter exactly when both halves are, and a
//! spatial filter otherwise.
//!
//! # Example
//!
//! A hand-written colour filter: one stage, the WGSL function that applies
//! it, and the parameter that feeds it. The built-in filters in `filtrate` are
//! written with `#[derive(Filter)]`, which generates exactly this.
//!
//! ```rust
//! use filtrate_core::{
//!     ColorFilter, ColorStage, Filter, FilterExt, OperatingSpace, ParamSource, Placed,
//!     StageCollector, kind,
//! };
//!
//! struct Brightness(f32);
//!
//! impl Filter for Brightness {
//!     type Kind = kind::Color;
//!     type Params = [f32; 1];
//!
//!     fn params(&self) -> [f32; 1] {
//!         [self.0]
//!     }
//!
//!     fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
//!         const STAGE: ColorStage = ColorStage {
//!             name: "brightness",
//!             source: "struct Params { amount: f32 }
//! fn apply(color: vec4<f32>, params: Params) -> vec4<f32> {
//!     return vec4<f32>(color.rgb + params.amount * color.a, color.a);
//! }",
//!             params: &[ParamSource::Param(0)],
//!             space: OperatingSpace::Working,
//!         };
//!         collector.color(Placed::new(&STAGE));
//!     }
//! }
//!
//! impl ColorFilter for Brightness {
//!     // `rgb + amount * a` is linear in premultiplied RGBA.
//!     const LINEAR: bool = true;
//! }
//!
//! let chain = Brightness(0.2).then(Brightness(-0.1));
//! assert_eq!(chain.params(), ([0.2], [-0.1]));
//! const { assert!(<filtrate_core::Chain<Brightness, Brightness> as ColorFilter>::LINEAR) };
//! ```

mod animation;
mod filter;
mod image;
mod kernel;
pub mod kind;
mod param;
mod params;
mod space;
mod stage;
mod visitor;

pub use animation::AnimationTrack;
pub use filter::{Chain, ColorFilter, Filter, FilterExt, Footprint, SpatialFilter};
pub use image::{AuxData, AuxFormat, AuxImage, ImageVisitor};
pub use kernel::{CpuFilter, CpuFilterError, CpuImage, CpuKernel};
pub use param::{AnimatedCallback, AnimatedTarget, FilterParam, Interpolator, WatchGuard};
pub use params::ParamArray;
pub use space::{OperatingSpace, WorkingSpace};
pub use stage::{
    AuxSource, ColorStage, ParamSource, Placed, ShapeInput, SpatialStage, StageCollector,
};
pub use visitor::SignalVisitor;
