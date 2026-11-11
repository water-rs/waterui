//! A filter's stages run on the CPU: the composed program the GPU executor
//! runs, executed by `cherenkov-shader`'s lane executor.
//!
//! [`LaneProgram::new`] plans the filter exactly as the GPU executor does —
//! one composition, one pass per piece, the plain alternative of every
//! spatial stage — and compiles each pass's segment function for the lane
//! executor. [`LaneProgram::apply`] then runs the passes over a
//! [`CpuImage`] window under the [`CpuFilter`](filtrate_core::CpuFilter)
//! contract: a spatial pass reads the previous pass's output through a
//! sampler that clamps to the window, so rows at least the filter's
//! footprint away from a window edge inside the image match a full-image
//! application.
//!
//! A pass's input, its shape and the chain's auxiliary images are the
//! textures its segment binds. The shape is the chain input's coverage, or
//! its signed distance field, built over the window on the GPU executor's
//! jump-flooding schedule ([`crate::shape_field`]). A spatial pass runs 2×2
//! quads, so derivatives see the differences a GPU takes; lanes past the
//! window edge run as helpers and are not stored.

extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use std::collections::HashMap;

use cherenkov_shader::{
    SamplerFilter, SegmentArg,
    lanes::{Host, ImageOp, Interface, LANES, LaneError, Lanes, Mask, Program, Reg, Registers},
    naga::{Module, TypeInner},
    validate,
};
use filtrate_core::{AuxImage, CpuImage, Filter, ImageVisitor, ShapeInput, WorkingSpace};

use crate::plan::{PassAux, PassPlan, Plan};
use crate::shape_field;

/// Image handles a pass binds: its input, the chain input as its shape,
/// then auxiliary image `n` at `AUX + n`.
const INPUT: u32 = 0;
const SHAPE: u32 = 1;
const AUX: u32 = 2;
/// Sampler handles.
const LINEAR: u32 = 0;
const POINT: u32 = 1;

/// Lane columns of a spatial run: 8 quads side by side, two rows tall.
const RUN_COLUMNS: usize = LANES / 2;

/// Why a filter cannot run on the lane executor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LaneFilterError {
    /// The filter's stages do not plan: an empty chain, a stage that
    /// disagrees with its snippet, or a composition failure.
    Plan(String),
    /// A pass uses something the lane executor does not run.
    Unsupported {
        /// The pass's stage name.
        stage: &'static str,
        /// What it uses.
        reason: String,
    },
    /// An auxiliary image the filter binds has no CPU texels.
    GpuImage {
        /// The image's index among the filter's images.
        index: usize,
    },
}

impl core::fmt::Display for LaneFilterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Plan(message) => f.write_str(message),
            Self::Unsupported { stage, reason } => write!(f, "stage `{stage}`: {reason}"),
            Self::GpuImage { index } => write!(f, "auxiliary image {index} has no CPU texels"),
        }
    }
}

impl core::error::Error for LaneFilterError {}

/// An auxiliary image's decoded texels.
#[derive(Debug)]
struct Texels {
    width: usize,
    height: usize,
    data: Vec<[f32; 4]>,
}

/// One compiled pass: the program and the registers its arguments land in.
#[derive(Debug)]
struct Pass {
    program: Program,
    /// The colour argument of a colour pass.
    color: Option<[Reg; 4]>,
    /// The sample coordinate of a spatial pass.
    uv: Option<[Reg; 2]>,
    /// The `size` member of a spatial pass's uniform block.
    size: Option<[Reg; 2]>,
    /// Each dynamic uniform member's registers and the first flattened
    /// parameter it reads.
    uniforms: Vec<(Vec<Reg>, usize)>,
    /// The working-space constants' registers.
    space: Option<[Reg; 3]>,
    /// The handle each texture and sampler argument register holds.
    bindings: Vec<(Reg, u32)>,
    /// Which representation of the chain input's shape the pass reads.
    shape: Option<ShapeInput>,
    /// What auxiliary image `n` binds.
    aux: Vec<PassAux>,
}

impl Pass {
    const fn spatial(&self) -> bool {
        self.uv.is_some()
    }
}

/// A filter's passes compiled for the lane executor, with the auxiliary
/// images they read.
///
/// It holds no reference to the filter: the parameters arrive with each
/// [`LaneProgram::apply`], so one program serves every band of a frame at
/// once.
#[derive(Debug)]
pub struct LaneProgram {
    passes: Vec<Pass>,
    /// The filter's images, by index; `None` for one no pass binds.
    images: Vec<Option<Texels>>,
    /// Which passes' inputs a later pass reads.
    kept: Vec<bool>,
    /// Whether a pass reads the chain input's coverage as its shape.
    mask: bool,
    /// Whether a pass reads the chain input's signed distance field.
    sdf: bool,
}

impl LaneProgram {
    /// Plans and compiles `filter`'s stages, and decodes the auxiliary
    /// images they bind.
    ///
    /// # Errors
    /// [`LaneFilterError::Plan`] when the stages do not plan,
    /// [`LaneFilterError::Unsupported`] for a pass using a construct the
    /// executor does not model, and [`LaneFilterError::GpuImage`] for an
    /// auxiliary image without CPU texels.
    pub fn new<F: Filter>(filter: &F) -> Result<Self, LaneFilterError> {
        let plan = Plan::new(filter, true, true, false, cherenkov_shader::Precision::F32)
            .map_err(|error| LaneFilterError::Plan(error.to_string()))?;
        let info = validate(&plan.module, plan.capabilities)
            .map_err(|error| LaneFilterError::Plan(error.to_string()))?;
        let mut decoded = Decode(Vec::new());
        filter.visit_images(&mut decoded);
        let mut available: Vec<Option<Texels>> = (0..F::IMAGES).map(|_| None).collect();
        for (index, texels) in decoded.0 {
            if let Some(slot) = available.get_mut(index) {
                *slot = texels;
            }
        }
        let mut images: Vec<Option<Texels>> = (0..F::IMAGES).map(|_| None).collect();
        let mut kept = alloc::vec![false; plan.passes.len()];
        let (mut mask, mut sdf) = (false, false);
        let mut passes = Vec::with_capacity(plan.passes.len());
        for pass in &plan.passes {
            mask |= pass.shape == Some(ShapeInput::Mask);
            sdf |= pass.shape == Some(ShapeInput::Sdf);
            for aux in &pass.aux {
                match *aux {
                    PassAux::Image(index) => {
                        let texels = available[index]
                            .take()
                            .or_else(|| images[index].take())
                            .ok_or(LaneFilterError::GpuImage { index })?;
                        images[index] = Some(texels);
                    }
                    PassAux::Texture(index) => return Err(LaneFilterError::GpuImage { index }),
                    PassAux::PassInput(input) => kept[input] = true,
                }
            }
            passes.push(compile(&plan.module, &info, pass)?);
        }
        Ok(Self {
            passes,
            images,
            kept,
            mask,
            sdf,
        })
    }

    /// Applies the passes to `image` in place.
    ///
    /// `params` are the filter's flattened parameters with every length
    /// already in device pixels, the values the GPU executor's uniforms
    /// hold. `image` holds premultiplied RGBA in the working space `space`.
    ///
    /// # Panics
    /// Panics when `params` holds fewer values than the filter's flattened
    /// parameters.
    #[expect(
        clippy::cast_precision_loss,
        reason = "image dimensions are bounded well below f32's exact integer range"
    )]
    pub fn apply(&self, params: &[f32], space: &WorkingSpace, image: &mut CpuImage<'_>) {
        let (width, height) = image.size;
        let Some(rows) = image.pixels.len().checked_div(width) else {
            return;
        };
        if rows == 0 {
            return;
        }
        let top = image.top;
        let chain_input = self.mask.then(|| image.pixels.to_vec());
        let field = self
            .sdf
            .then(|| shape_field::signed_distance(image.pixels, width, top, height));
        let mut inputs: Vec<Option<Vec<[f32; 4]>>> = (0..self.passes.len()).map(|_| None).collect();
        let size = [width as f32, height as f32];
        let window = Window {
            top,
            rows,
            width,
            height,
        };
        for (index, pass) in self.passes.iter().enumerate() {
            let input = (pass.spatial() || self.kept[index]).then(|| image.pixels.to_vec());
            let mut regs = pass.program.registers();
            pass.bind(&mut regs, params, space, size);
            let aux: Vec<View<'_>> = pass
                .aux
                .iter()
                .map(|aux| match *aux {
                    PassAux::Image(n) | PassAux::Texture(n) => {
                        let texels = self.images[n]
                            .as_ref()
                            .expect("LaneProgram::new decoded every image a pass binds");
                        View {
                            pixels: &texels.data,
                            top: 0,
                            rows: texels.height,
                            width: texels.width,
                            height: texels.height,
                        }
                    }
                    PassAux::PassInput(n) => window.view(
                        inputs[n]
                            .as_deref()
                            .expect("LaneProgram::new keeps every pass input a later pass reads"),
                    ),
                })
                .collect();
            let shape = match pass.shape {
                None => None,
                Some(ShapeInput::Mask) => chain_input.as_deref(),
                Some(ShapeInput::Sdf) => field.as_deref(),
            };
            let mut host = Images {
                input: input.as_deref().map(|pixels| window.view(pixels)),
                shape: shape.map(|pixels| window.view(pixels)),
                aux: &aux,
            };
            if pass.spatial() {
                pass.run_spatial(&mut regs, &mut host, image, rows);
            } else {
                pass.run_color(&mut regs, &mut host, image.pixels);
            }
            drop(aux);
            if self.kept[index] {
                inputs[index] = input;
            }
        }
    }
}

impl Pass {
    /// Writes the pass's uniform values and argument handles.
    fn bind(&self, regs: &mut Registers, params: &[f32], space: &WorkingSpace, size: [f32; 2]) {
        for (members, first) in &self.uniforms {
            for (k, &reg) in members.iter().enumerate() {
                regs.splat(reg, params[first + k]);
            }
        }
        if let Some(regs_size) = self.size {
            regs.splat(regs_size[0], size[0]);
            regs.splat(regs_size[1], size[1]);
        }
        if let Some(luma) = self.space {
            for (&reg, &value) in luma.iter().zip(&space.luma) {
                regs.splat(reg, value);
            }
        }
        for &(reg, handle) in &self.bindings {
            regs.splat(reg, f32::from_bits(handle));
        }
    }

    /// A colour pass: every pixel's colour in, its result out, [`LANES`]
    /// pixels a run.
    fn run_color(&self, regs: &mut Registers, host: &mut Images<'_>, pixels: &mut [[f32; 4]]) {
        let color = self.color.expect("a colour pass takes a colour");
        for chunk in pixels.chunks_mut(LANES) {
            for (channel, &reg) in color.iter().enumerate() {
                let lanes = regs.get_mut(reg);
                for (lane, pixel) in chunk.iter().enumerate() {
                    lanes[lane] = pixel[channel];
                }
            }
            let active: Mask = core::array::from_fn(|lane| lane < chunk.len());
            let done = self.program.run(regs, active, host);
            for (lane, pixel) in chunk.iter_mut().enumerate() {
                *pixel = lane_result(regs, self.program.result(), &done, lane);
            }
        }
    }

    /// A spatial pass: two rows of 2×2 quads a run, sampling `host.input`.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel coordinates are bounded well below f32's exact integer range"
    )]
    fn run_spatial(
        &self,
        regs: &mut Registers,
        host: &mut Images<'_>,
        image: &mut CpuImage<'_>,
        rows: usize,
    ) {
        let uv = self.uv.expect("a spatial pass takes a coordinate");
        let (width, height) = image.size;
        let (scale_x, scale_y) = (1.0 / width as f32, 1.0 / height as f32);
        for y0 in (0..rows).step_by(2) {
            for x0 in (0..width).step_by(RUN_COLUMNS) {
                let place =
                    |lane: usize| (x0 + 2 * (lane >> 2) + (lane & 1), y0 + ((lane >> 1) & 1));
                for (lane, value) in regs.get_mut(uv[0]).iter_mut().enumerate() {
                    *value = (place(lane).0 as f32 + 0.5) * scale_x;
                }
                for (lane, value) in regs.get_mut(uv[1]).iter_mut().enumerate() {
                    *value = ((image.top + place(lane).1) as f32 + 0.5) * scale_y;
                }
                // Helper lanes past the window run so derivatives see
                // whole quads; only the window's pixels are stored.
                let done = self.program.run(regs, [true; LANES], host);
                for lane in 0..LANES {
                    let (x, y) = place(lane);
                    if x < width && y < rows {
                        image.pixels[y * width + x] =
                            lane_result(regs, self.program.result(), &done, lane);
                    }
                }
            }
        }
    }
}

/// One lane's result; a lane that discarded is transparent, as a discarded
/// fragment leaves a cleared target.
fn lane_result(regs: &Registers, result: &[Reg], done: &Mask, lane: usize) -> [f32; 4] {
    if done[lane] {
        core::array::from_fn(|channel| regs.get(result[channel])[lane])
    } else {
        [0.0; 4]
    }
}

/// The window [`LaneProgram::apply`] runs on: rows `top..top + rows` of a
/// `width` × `height` image.
#[derive(Clone, Copy, Debug)]
struct Window {
    top: usize,
    rows: usize,
    width: usize,
    height: usize,
}

impl Window {
    /// `pixels`, holding the window's rows, as an image a pass samples.
    const fn view(self, pixels: &[[f32; 4]]) -> View<'_> {
        View {
            pixels,
            top: self.top,
            rows: self.rows,
            width: self.width,
            height: self.height,
        }
    }
}

/// An image a pass samples: rows `top..top + rows` of a `width` × `height`
/// image. Every access clamps to those rows and to the image's columns.
#[derive(Clone, Copy, Debug)]
struct View<'a> {
    pixels: &'a [[f32; 4]],
    top: usize,
    rows: usize,
    width: usize,
    height: usize,
}

impl View<'_> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        reason = "texel coordinates are clamped into the bounded window first"
    )]
    fn texel(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let y = y.clamp(self.top as i64, (self.top + self.rows) as i64 - 1) as usize - self.top;
        self.pixels[y * self.width + x]
    }

    /// The sample at normalized `(u, v)` displaced by `offset` texels:
    /// the nearest texel through the point sampler, the bilinear blend of
    /// the four around it through the linear one.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "texel coordinates are bounded well below f32's exact integer range"
    )]
    fn sample(&self, (u, v): (f32, f32), offset: [i32; 2], point: bool) -> [f32; 4] {
        let (x, y) = (u * self.width as f32, v * self.height as f32);
        let (dx, dy) = (i64::from(offset[0]), i64::from(offset[1]));
        if point {
            return self.texel(x.floor() as i64 + dx, y.floor() as i64 + dy);
        }
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64 + dx, y0 as i64 + dy);
        let lerp = |p: [f32; 4], q: [f32; 4], t: f32| -> [f32; 4] {
            core::array::from_fn(|c| (q[c] - p[c]).mul_add(t, p[c]))
        };
        lerp(
            lerp(self.texel(x0, y0), self.texel(x0 + 1, y0), tx),
            lerp(self.texel(x0, y0 + 1), self.texel(x0 + 1, y0 + 1), tx),
            ty,
        )
    }
}

/// The images one pass binds, by handle.
struct Images<'a> {
    /// The pass's input; `None` for a colour pass, which reads its colour
    /// as an argument.
    input: Option<View<'a>>,
    shape: Option<View<'a>>,
    aux: &'a [View<'a>],
}

impl Images<'_> {
    fn view(&self, handle: u32) -> &View<'_> {
        let view = match handle {
            INPUT => self.input.as_ref(),
            SHAPE => self.shape.as_ref(),
            aux => self.aux.get((aux - AUX) as usize),
        };
        view.expect("a pass binds only the images it declares")
    }
}

impl Host for Images<'_> {
    fn call(&mut self, function: usize, _: &[Lanes], _: &mut [Lanes], _: &Mask) {
        unreachable!("a filter pass's interface names no host function, got {function}");
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "image dimensions are bounded well below u32::MAX"
    )]
    fn image(&mut self, op: ImageOp<'_>, out: &mut [Lanes; 4], active: &Mask) {
        let lanes = (0..LANES).filter(|&lane| active[lane]);
        match op {
            ImageOp::Sample {
                image,
                sampler,
                uv,
                offset,
            } => {
                let view = self.view(image);
                for lane in lanes {
                    let texel = view.sample((uv[0][lane], uv[1][lane]), offset, sampler == POINT);
                    for (channel, value) in texel.into_iter().enumerate() {
                        out[channel][lane] = value;
                    }
                }
            }
            ImageOp::Load { image, texel } => {
                let view = self.view(image);
                for lane in lanes {
                    let value = view.texel(i64::from(texel[0][lane]), i64::from(texel[1][lane]));
                    for (channel, value) in value.into_iter().enumerate() {
                        out[channel][lane] = value;
                    }
                }
            }
            ImageOp::Size { image } => {
                let view = self.view(image);
                let (width, height) = (
                    f32::from_bits(view.width as u32),
                    f32::from_bits(view.height as u32),
                );
                for lane in lanes {
                    out[0][lane] = width;
                    out[1][lane] = height;
                }
            }
        }
    }
}

/// Decodes each image with CPU texels.
struct Decode(Vec<(usize, Option<Texels>)>);

impl ImageVisitor for Decode {
    fn visit<I: AuxImage + ?Sized>(&mut self, index: usize, image: &I) {
        let texels = image.data().map(|data| {
            let (width, height) = (image.width() as usize, image.height() as usize);
            Texels {
                width,
                height,
                data: (0..width * height)
                    .map(|texel| crate::cpu::aux_pixel(data, texel))
                    .collect(),
            }
        });
        self.0.push((index, texels));
    }
}

/// Compiles one pass's segment function and locates its arguments.
fn compile(
    module: &Module,
    info: &cherenkov_shader::naga::valid::ModuleInfo,
    plan: &PassPlan,
) -> Result<Pass, LaneFilterError> {
    let unsupported = |reason: String| LaneFilterError::Unsupported {
        stage: plan.name,
        reason,
    };
    let name = plan.segment.function();
    let program = Program::compile(module, info, name, &Interface::default())
        .map_err(|error: LaneError| unsupported(error.to_string()))?;
    let (_, function) = module
        .functions
        .iter()
        .find(|(_, function)| function.name.as_deref() == Some(name))
        .expect("the composition defines every segment function");
    let mut pass = Pass {
        program,
        color: None,
        uv: None,
        size: None,
        uniforms: Vec::new(),
        space: None,
        bindings: Vec::new(),
        shape: plan.shape,
        aux: plan.aux.clone(),
    };
    for (index, arg) in plan.segment.args().iter().enumerate() {
        let regs = pass.program.argument(index).to_vec();
        match *arg {
            SegmentArg::Color => pass.color = Some(fixed(&regs, &unsupported)?),
            SegmentArg::Uv => pass.uv = Some(fixed(&regs, &unsupported)?),
            SegmentArg::WorkingSpace => pass.space = Some(fixed(&regs, &unsupported)?),
            SegmentArg::Input => pass.bindings.push((regs[0], INPUT)),
            SegmentArg::Shape => pass.bindings.push((regs[0], SHAPE)),
            SegmentArg::Aux(n) => pass.bindings.push((regs[0], AUX + n)),
            SegmentArg::InputSampler => pass.bindings.push((
                regs[0],
                match plan.sampler {
                    Some(SamplerFilter::Point) => POINT,
                    _ => LINEAR,
                },
            )),
            SegmentArg::Params => {
                let ty = function.arguments[index].ty;
                let TypeInner::Struct { ref members, .. } = module.types[ty].inner else {
                    return Err(unsupported("a uniform block that is not a struct".into()));
                };
                // Byte offsets to register offsets: the lane executor
                // flattens members in declaration order.
                let mut slots = HashMap::new();
                let mut slot = 0;
                for member in members {
                    slots.insert(member.offset, slot);
                    slot += match module.types[member.ty].inner {
                        TypeInner::Scalar(_) => 1,
                        TypeInner::Vector { size, .. } => size as usize,
                        ref other => {
                            return Err(unsupported(format!("a uniform member of {other:?}")));
                        }
                    };
                }
                if let Some(offset) = plan.segment.uniform().input_size() {
                    let start = slots[&offset];
                    pass.size = Some([regs[start], regs[start + 1]]);
                }
                for uniform in &plan.uniforms {
                    let start = slots[&uniform.offset];
                    pass.uniforms.push((
                        regs[start..start + uniform.components].to_vec(),
                        uniform.param,
                    ));
                }
            }
        }
    }
    Ok(pass)
}

/// An argument's registers as a fixed-size array.
fn fixed<const N: usize>(
    regs: &[Reg],
    unsupported: &impl Fn(String) -> LaneFilterError,
) -> Result<[Reg; N], LaneFilterError> {
    regs.try_into().map_err(|_| {
        unsupported(format!(
            "an argument of {} registers where {N} were expected",
            regs.len()
        ))
    })
}

#[cfg(test)]
mod tests {
    use cherenkov_oracle::metrics;
    use filtrate_core::{CpuFilter, CpuImage, Filter, FilterExt, ParamArray, WorkingSpace};

    use super::{LaneFilterError, LaneProgram};
    use crate::filters::{
        BlendMode, BlendWithImage, Bloom, Blur, Brightness, ColorMatrix, Convolution3x3,
        GaussianBlur, Grayscale, HueRotation, LumaCurve, Median3x3, OverscrollStretch, Saturation,
    };
    use crate::test_filters::{
        Masked, SdfReadout, assert_field_close, disc_coverage, disc_distance,
    };
    use crate::{FilterImage, OperatingSpace};

    /// A 37 × 23 image: odd in both directions, so runs end in partial
    /// chunks and half quads.
    const SIZE: (usize, usize) = (37, 23);

    #[expect(clippy::cast_precision_loss, reason = "small indices")]
    fn input() -> Vec<[f32; 4]> {
        (0..SIZE.0 * SIZE.1)
            .map(|index| {
                let alpha = ((index * 29 + 53) % 251) as f32 / 250.0;
                let channel = |salt| ((index * salt + 17) % 239) as f32 / 238.0 * alpha;
                [channel(31), channel(43), channel(67), alpha]
            })
            .collect()
    }

    fn flat<F: Filter>(filter: &F) -> Vec<f32> {
        let mut values = vec![0.0; <F::Params as ParamArray>::LEN];
        filter.params().write_to(&mut values);
        values
    }

    /// `filter` through its lane program on rows `top..bottom` of the
    /// [`SIZE`] image `pixels`.
    fn lanes<F: Filter>(
        filter: &F,
        pixels: &[[f32; 4]],
        top: usize,
        bottom: usize,
    ) -> Vec<[f32; 4]> {
        lanes_sized(filter, pixels, SIZE, top, bottom)
    }

    /// `filter` through its lane program on rows `top..bottom` of the
    /// `size` image `pixels`.
    fn lanes_sized<F: Filter>(
        filter: &F,
        pixels: &[[f32; 4]],
        size: (usize, usize),
        top: usize,
        bottom: usize,
    ) -> Vec<[f32; 4]> {
        let program = LaneProgram::new(filter).expect("the filter runs on the lane executor");
        let mut window = pixels[top * size.0..bottom * size.0].to_vec();
        program.apply(
            &flat(filter),
            &WorkingSpace::LINEAR_DISPLAY_P3,
            &mut CpuImage {
                pixels: &mut window,
                top,
                size,
            },
        );
        window
    }

    /// `filter`'s CPU kernel over the whole image.
    fn kernel<F: CpuFilter>(filter: &F, pixels: &[[f32; 4]]) -> Vec<[f32; 4]> {
        let mut full = pixels.to_vec();
        filter
            .apply_cpu_image(
                &filter.params(),
                &WorkingSpace::LINEAR_DISPLAY_P3,
                &mut CpuImage {
                    pixels: &mut full,
                    top: 0,
                    size: SIZE,
                },
            )
            .expect("the CPU kernel runs");
        full
    }

    fn assert_matches_kernel<F: CpuFilter>(name: &str, filter: &F) {
        let input = input();
        let expected = kernel(filter, &input);
        let actual = lanes(filter, &input, 0, SIZE.1);
        metrics::assert_close(actual.as_flattened(), expected.as_flattened());
        assert!(
            actual != input,
            "{name}: the lane program left the image unchanged"
        );
    }

    #[test]
    fn colour_stages_match_their_cpu_kernels() {
        assert_matches_kernel("brightness", &Brightness(0.2f32));
        assert_matches_kernel("saturation", &Saturation(0.4f32));
        assert_matches_kernel("grayscale", &Grayscale(0.7f32));
        assert_matches_kernel("hue rotation", &HueRotation(40.0f32));
        assert_matches_kernel(
            "colour matrix",
            &ColorMatrix([0.9, 0.1, 0.0, 0.0, 0.2, 0.7, 0.1, 0.0, 0.0, 0.3, 0.6, 0.05]),
        );
        assert_matches_kernel(
            "luma curve",
            &LumaCurve {
                curve: [0.1f32, 0.3, 0.55, 0.8],
                amount: 0.9,
                chroma: 1.2,
                offset: 0.02,
            },
        );
        assert_matches_kernel(
            "a colour chain",
            &Brightness(-0.1f32).then(Saturation(1.3f32)),
        );
    }

    /// A stage with no CPU kernel, against its definition: the overshoot a
    /// sharpen produces stays unclamped either way.
    #[test]
    fn a_convolution_matches_its_definition() {
        let weights = [0.0f32, -1.0, 0.0, -1.0, 5.0, -1.0, 0.0, -1.0, 0.0];
        let input = input();
        let (width, height) = SIZE;
        let at = |x: usize, y: usize, dx: isize, dy: isize| {
            let x = x.saturating_add_signed(dx).min(width - 1);
            let y = y.saturating_add_signed(dy).min(height - 1);
            input[y * width + x]
        };
        let expected: Vec<[f32; 4]> = (0..width * height)
            .map(|index| {
                let (x, y) = (index % width, index / width);
                let mut sum = [0.0f32; 4];
                for (tap, weight) in weights.iter().enumerate() {
                    let texel = at(x, y, tap.cast_signed() % 3 - 1, tap.cast_signed() / 3 - 1);
                    for (total, value) in sum.iter_mut().zip(texel) {
                        *total = weight.mul_add(value, *total);
                    }
                }
                sum
            })
            .collect();
        let actual = lanes(&Convolution3x3(weights), &input, 0, height);
        assert!(
            expected.as_flattened().iter().any(|&v| v < 0.0),
            "the test image makes the sharpen undershoot"
        );
        metrics::assert_close(actual.as_flattened(), expected.as_flattened());
    }

    #[test]
    fn spatial_stages_match_their_cpu_kernels() {
        assert_matches_kernel("box blur", &Blur(2.5f32));
        assert_matches_kernel(
            "gaussian blur",
            &GaussianBlur {
                sigma: 1.5f32,
                space: OperatingSpace::Working,
            },
        );
        assert_matches_kernel(
            "gaussian blur in sRGB",
            &GaussianBlur {
                sigma: 1.0f32,
                space: OperatingSpace::Srgb,
            },
        );
        assert_matches_kernel(
            "overscroll stretch",
            &OverscrollStretch {
                frame: [1.0f32, 0.0, 0.0, 1.0, 2.0, 1.5],
                size: [30.0, 19.0],
                stretch: [0.3, -0.2],
            },
        );
        let image = FilterImage::from_rgba8(
            5,
            4,
            (0..5 * 4 * 4)
                .map(|byte| u8::try_from(byte * 13 % 256).expect("fits a byte"))
                .collect(),
        );
        assert_matches_kernel(
            "blend with an image",
            &BlendWithImage {
                image,
                amount: 0.6f32,
                mode: BlendMode::Multiply,
            },
        );
        assert_matches_kernel(
            "blur after a colour stage",
            &Brightness(0.1f32).then(Blur(1.5f32)),
        );
    }

    /// A window with an apron of the filter's footprint matches the
    /// full-image application on its central rows, the `CpuFilter`
    /// contract the raster's bands rely on.
    fn assert_window_matches_full<F: Filter>(filter: &F, apron: usize) {
        let input = input();
        let full = lanes(filter, &input, 0, SIZE.1);
        let (top, bottom) = (3, SIZE.1 - 4);
        let window = lanes(filter, &input, top, bottom);
        let band = (top + apron) * SIZE.0..(bottom - apron) * SIZE.0;
        assert!(!band.is_empty(), "the window keeps rows past its apron");
        metrics::assert_close(
            window[band.start - top * SIZE.0..band.end - top * SIZE.0].as_flattened(),
            full[band].as_flattened(),
        );
    }

    #[test]
    fn a_window_matches_the_full_image_past_its_footprint() {
        assert_window_matches_full(&Blur(2.0f32), 3);
        // Helpers taking a pointer to a local array.
        assert_window_matches_full(&Median3x3, 1);
        assert_window_matches_full(
            &GaussianBlur {
                sigma: 1.0f32,
                space: OperatingSpace::Working,
            },
            4,
        );
        // Two spatial passes, the second reading the first one's input.
        let bloom = Bloom {
            radius: 1.5f32,
            intensity: 0.8,
            threshold: 0.3,
        };
        let reach = <Bloom<f32> as crate::SpatialFilter>::footprint_of(&bloom.params());
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a small positive footprint"
        )]
        let apron = reach.resolve((37.0, 23.0)).ceil() as usize;
        assert_window_matches_full(&bloom, apron);
    }

    /// The size the shape field tests run at, the GPU executor's.
    const FIELD_SIZE: (u32, u32) = (48, 40);

    /// [`SdfReadout`] on rows `top..bottom` of premultiplied white at
    /// `coverage`: the field the stage read.
    fn shape_sdf(coverage: &[f32], top: usize, bottom: usize) -> Vec<f32> {
        let texels: Vec<[f32; 4]> = coverage.iter().map(|&alpha| [alpha; 4]).collect();
        let size = (FIELD_SIZE.0 as usize, FIELD_SIZE.1 as usize);
        lanes_sized(&SdfReadout, &texels, size, top, bottom)
            .into_iter()
            .map(|texel| texel[0])
            .collect()
    }

    /// The field the GPU executor's test holds its passes to, at the same
    /// tolerance.
    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "test coordinates are small integers"
    )]
    fn a_shape_field_is_the_distance_to_the_half_coverage_contour() {
        let size = FIELD_SIZE;
        let rows = size.1 as usize;
        let disc = disc_coverage(size);
        assert_field_close(size, &shape_sdf(&disc, 0, rows), 0.25, disc_distance);

        // Opaque content ends at the image's edges.
        let (width, height) = (size.0 as f32, size.1 as f32);
        let opaque = vec![1.0; (size.0 * size.1) as usize];
        assert_field_close(size, &shape_sdf(&opaque, 0, rows), 0.25, |x, y| {
            -x.min(width - x).min(y).min(height - y)
        });

        // Nothing covered is farther than any distance the GPU's field
        // holds, the largest finite f16.
        let empty = vec![0.0; (size.0 * size.1) as usize];
        assert!(
            shape_sdf(&empty, 0, rows)
                .iter()
                .all(|&distance| distance == 65504.0),
            "an empty shape is at the field's largest distance everywhere"
        );
    }

    /// A band of the disc whose edges cross its interior: a window edge
    /// inside the image adds no contour, so rows an apron away from it hold
    /// the disc's distance, to the field's accuracy, wherever it is within
    /// the apron, and no nearer one elsewhere.
    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "test coordinates are small integers"
    )]
    fn a_band_sees_the_contour_within_its_apron() {
        const APRON: usize = 6;
        const TOLERANCE: f32 = 0.25;
        let reach = APRON as f32;
        let size = FIELD_SIZE;
        let width = size.0 as usize;
        let (top, bottom) = (12, 30);
        let band = shape_sdf(&disc_coverage(size), top, bottom);
        let mut within = 0;
        for y in top + APRON..bottom - APRON {
            for x in 0..width {
                let expected = disc_distance(x as f32 + 0.5, y as f32 + 0.5);
                let got = band[(y - top) * width + x];
                assert_eq!(got < 0.0, expected < 0.0, "pixel ({x}, {y}): the sign");
                if expected.abs() <= reach {
                    assert!(
                        (got - expected).abs() <= TOLERANCE,
                        "pixel ({x}, {y}): distance {got}, expected {expected}"
                    );
                    within += 1;
                } else {
                    assert!(
                        got.abs() >= reach - TOLERANCE,
                        "pixel ({x}, {y}): distance {got} is within the apron, expected {expected}"
                    );
                }
            }
        }
        assert!(within > 0, "the band holds pixels near the contour");
    }

    /// A stage reading the mask and one reading the field, chained: both
    /// read the chain input's shape, not the previous stage's output.
    #[test]
    fn chained_stages_read_the_chain_inputs_shape() {
        let size = FIELD_SIZE;
        let rows = size.1 as usize;
        let disc = disc_coverage(size);
        let texels: Vec<[f32; 4]> = disc.iter().map(|&alpha| [alpha; 4]).collect();
        let sized = (size.0 as usize, rows);
        let chained: Vec<f32> = lanes_sized(&Masked.then(SdfReadout), &texels, sized, 0, rows)
            .into_iter()
            .map(|texel| texel[0])
            .collect();
        metrics::assert_close(&chained, &shape_sdf(&disc, 0, rows));
    }

    #[test]
    fn a_gpu_texture_image_is_refused() {
        struct Texture;
        impl filtrate_core::AuxImage for Texture {
            fn width(&self) -> u32 {
                1
            }
            fn height(&self) -> u32 {
                1
            }
            fn data(&self) -> Option<filtrate_core::AuxData<'_>> {
                None
            }
        }
        struct WithTexture(BlendWithImage<f32>);
        impl Filter for WithTexture {
            type Kind = <BlendWithImage<f32> as Filter>::Kind;
            type Params = <BlendWithImage<f32> as Filter>::Params;
            const IMAGES: usize = 1;
            fn params(&self) -> Self::Params {
                self.0.params()
            }
            fn collect_stages<C: filtrate_core::StageCollector>(&self, collector: &mut C) {
                self.0.collect_stages(collector);
            }
            fn visit_images<V: filtrate_core::ImageVisitor>(&self, visitor: &mut V) {
                visitor.visit(0, &Texture);
            }
        }
        let filter = WithTexture(BlendWithImage {
            image: FilterImage::from_rgba8(1, 1, vec![0; 4]),
            amount: 1.0,
            mode: BlendMode::Normal,
        });
        assert_eq!(
            LaneProgram::new(&filter).map(|_| ()),
            Err(LaneFilterError::GpuImage { index: 0 })
        );
    }
}
