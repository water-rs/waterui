//! Stage declarations: the shader functions a filter is made of.
//!
//! A stage is a snippet for the shared shader composer (`cherenkov-shader`):
//! WGSL source defining one function named `apply`.
//!
//! - A [`ColorStage`] maps a colour to a colour:
//!   `fn apply(color: vec4<f32>, params: Params, space: WorkingSpace) -> vec4<f32>`.
//! - A [`SpatialStage`] samples its input around a coordinate:
//!   `fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, …) -> vec4<f32>`.
//!   The sampler's name declares its filter mode: `input_point_sampler` when
//!   the stage only fetches exact texels, `input_sampler` when it may bind a
//!   filtering sampler.
//!
//! `params` and `space` are optional, as are a spatial stage's `shape` and
//! `aux0`, `aux1`, … images. Colours are premultiplied, in the stage's
//! [`OperatingSpace`]. The members of the snippet's `Params` struct are fed,
//! in declaration order, by the stage's [`ParamSource`]s.

use crate::OperatingSpace;

/// Where one member of a snippet's `Params` struct takes its value from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamSource {
    /// The filter parameters starting at this index of the flattened
    /// [`Filter::Params`](crate::Filter::Params): one for an `f32` member, and
    /// `N` consecutive ones for a `vecN<f32>` member.
    Param(usize),
    /// A constant. The composer specializes it into the shader, so it costs
    /// no uniform space. Its length is the member's component count.
    Constant(&'static [f32]),
}

/// Which representation of the clip shape a spatial stage reads through the
/// snippet's `shape` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShapeInput {
    /// The clip shape's signed distance field, in pixels: negative inside.
    Sdf,
    /// The clip shape's coverage mask, in the red channel.
    Mask,
}

/// What an auxiliary image argument (`aux0`, `aux1`, …) of a spatial stage
/// binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuxSource {
    /// Image `n` of the filter's own [`Filter::visit_images`](crate::Filter::visit_images):
    /// CPU data uploaded at its native precision, or a GPU texture bound in
    /// place.
    Image(usize),
    /// Image `n` of the filter's own [`Filter::visit_images`](crate::Filter::visit_images),
    /// which must be a caller-provided GPU texture — CPU data does not
    /// satisfy it. The texture is bound at its native format, which must
    /// sample as `texture_2d<f32>`.
    Texture(usize),
    /// The input of the immediately preceding stage, which must be a spatial
    /// stage of the same filter. A two-pass filter uses this to read the
    /// image its first pass started from (bloom compositing its glow onto the
    /// original, for example).
    PreviousStageInput,
}

/// A colour stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorStage {
    /// The snippet name, used in diagnostics and generated identifiers.
    pub name: &'static str,
    /// The WGSL source of the snippet (its `f32` variant).
    pub source: &'static str,
    /// One source per member of the snippet's `Params` struct, in order.
    pub params: &'static [ParamSource],
    /// The colour space the stage operates in.
    pub space: OperatingSpace,
}

/// A spatial stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialStage {
    /// The snippet name, used in diagnostics and generated identifiers.
    pub name: &'static str,
    /// The WGSL source of the snippet (its `f32` variant).
    pub source: &'static str,
    /// One source per member of the snippet's `Params` struct, in order.
    pub params: &'static [ParamSource],
    /// The colour space the stage operates in.
    pub space: OperatingSpace,
    /// The clip-shape representation the snippet's `shape` argument reads;
    /// `None` exactly when the snippet takes no `shape`.
    pub shape: Option<ShapeInput>,
    /// What each auxiliary image argument binds, `aux0` first.
    pub aux: &'static [AuxSource],
}

/// A stage reported by a filter, with the offsets that place its indices.
///
/// The offsets position the stage's [`ParamSource::Param`] and
/// [`AuxSource::Image`]/[`AuxSource::Texture`] indices within the flattened
/// parameters and images of the filter being collected.
#[derive(Debug)]
pub struct Placed<S: 'static> {
    /// The stage declaration.
    pub stage: &'static S,
    /// Added to every [`ParamSource::Param`] index.
    pub param_base: usize,
    /// Added to every [`AuxSource::Image`] and [`AuxSource::Texture`] index.
    pub image_base: usize,
}

impl<S: 'static> Placed<S> {
    /// A stage of the filter being collected itself, at offset zero.
    #[must_use]
    pub const fn new(stage: &'static S) -> Self {
        Self {
            stage,
            param_base: 0,
            image_base: 0,
        }
    }

    /// The same stage, shifted by further offsets.
    #[must_use]
    pub const fn shifted(self, params: usize, images: usize) -> Self {
        Self {
            stage: self.stage,
            param_base: self.param_base + params,
            image_base: self.image_base + images,
        }
    }
}

impl<S: 'static> Clone for Placed<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: 'static> Copy for Placed<S> {}

/// Sink for the stages reported by [`Filter::collect_stages`](crate::Filter::collect_stages).
pub trait StageCollector {
    /// Records a colour stage.
    fn color(&mut self, stage: Placed<ColorStage>);

    /// Records a spatial stage.
    fn spatial(&mut self, stage: Placed<SpatialStage>);
}
