//! User WGSL on `cherenkov-shader`'s lane executor: backdrop effect
//! shaders, evaluated inside a member's composite 32 pixels at a time, and
//! shader paints, rendered into an image the paint then samples.
//!
//! A registered source is the backend's prelude followed by the user's
//! text — the same module shape the GPU backend compiles — compiled once
//! at registration into a lane [`Program`]. The raster pass lays each run's
//! lanes out as 2×2 pixel quads, so derivatives see the neighbours a GPU
//! quad sees.

use std::sync::Arc;

use cherenkov::{BackdropShaderSource, RenderError, ResourceError, ShaderSource};
use cherenkov_shader::lanes::{Host, ImageOp, Interface, LANES, Lanes, Mask, Program, Reg};
use cherenkov_shader::naga;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use super::image::CpuImage;

/// The backdrop prelude, after filtrate-core's working-space fragment.
const BACKDROP_PRELUDE: &str = include_str!("backdrop.wgsl");

/// `backdrop_sample(q)`'s index in [`BACKDROP`]'s host functions.
pub const SAMPLE: usize = 0;
/// `backdrop_sample_level(q, level)`'s index.
pub const SAMPLE_LEVEL: usize = 1;
/// `backdrop_field(q)`'s index.
pub const FIELD: usize = 2;

/// What the raster pass supplies a backdrop shader.
const BACKDROP: Interface<'static> = Interface {
    functions: &["backdrop_sample", "backdrop_sample_level", "backdrop_field"],
    inputs: &[
        "backdrop_origin",
        "backdrop_size",
        "backdrop_scale",
        "backdrop_levels",
    ],
};

/// `BackdropPixel`'s slots: `p`, `sdf`, `normal`, `own_sdf`, `size`,
/// `scale`, `local`.
pub const PIXEL_SLOTS: usize = 11;
/// The effect uniforms: `array<vec4<f32>, 16>`.
pub const PARAM_SLOTS: usize = 64;

/// A compiled backdrop effect shader and the registers its inputs live in.
#[derive(Debug)]
pub struct BackdropProgram {
    pub program: Program,
    /// `px`'s registers, in [`PIXEL_SLOTS`] order.
    pub pixel: Box<[Reg]>,
    /// `params`' registers.
    pub params: Box<[Reg]>,
    /// `backdrop_origin`, `backdrop_size`, `backdrop_scale`,
    /// `backdrop_levels`.
    pub origin: [Reg; 2],
    pub size: [Reg; 2],
    pub scale: Reg,
    pub levels: Reg,
}

impl BackdropProgram {
    /// Compiles `source` against the backdrop prelude.
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when the module fails to parse or
    /// validate, its `backdrop_effect` does not have the contract's
    /// signature, or it uses a construct the lane executor does not model.
    pub fn compile(source: &BackdropShaderSource) -> Result<Self, ResourceError> {
        // The GPU backend appends the user source to its module text the
        // same way (`backdrop_effect_text`): the source names the prelude's
        // types and functions, so it cannot be parsed apart from it.
        let text = format!(
            "{}\n{BACKDROP_PRELUDE}\n{}",
            filtrate_core::space::WGSL,
            source.source
        );
        let program = Program::wgsl(&text, "backdrop_effect", &BACKDROP)
            .map_err(|error| ResourceError::Shader(error.to_string()))?;
        let signature = || {
            ResourceError::Shader(
                "`backdrop_effect` must be `fn(BackdropPixel, array<vec4<f32>, 16>) -> vec4<f32>`"
                    .into(),
            )
        };
        if program.result().len() != 4 || program.arity() != 2 {
            return Err(signature());
        }
        let argument = |index: usize, slots: usize| -> Result<Box<[Reg]>, ResourceError> {
            let regs = program.argument(index);
            if regs.len() == slots {
                Ok(regs.into())
            } else {
                Err(signature())
            }
        };
        let pixel = argument(0, PIXEL_SLOTS)?;
        let params = argument(1, PARAM_SLOTS)?;
        let global = |name: &str| {
            program
                .global(name)
                .expect("the backdrop prelude declares every input")
                .to_vec()
        };
        let (origin, size) = (global("backdrop_origin"), global("backdrop_size"));
        Ok(Self {
            pixel,
            params,
            origin: [origin[0], origin[1]],
            size: [size[0], size[1]],
            scale: global("backdrop_scale")[0],
            levels: global("backdrop_levels")[0],
            program,
        })
    }
}

/// A shader paint's fragment entry calls nothing the host supplies.
const PAINT: Interface<'static> = Interface {
    functions: &[],
    inputs: &[],
};

/// The host of a program whose interface names no function.
struct NoCalls;

impl Host for NoCalls {
    fn call(&mut self, function: usize, _: &[Lanes], _: &mut [Lanes], _: &Mask) {
        unreachable!("a shader paint's interface names no host function, got {function}");
    }

    fn image(&mut self, op: ImageOp<'_>, _: &mut [Lanes; 4], _: &Mask) {
        unreachable!("a shader paint's entry takes its varyings, no texture: {op:?}");
    }
}

/// Lane columns of one run: 8 quads side by side, two rows tall.
const RUN_COLUMNS: usize = LANES / 2;

/// A compiled shader paint: its `main` fragment entry and the registers
/// its inputs and uniforms live in.
#[derive(Debug)]
pub struct PaintProgram {
    program: Program,
    /// `@builtin(position)`'s registers, however many components the
    /// entry reads (none when it reads no position).
    position: Box<[Reg]>,
    /// `@location(0)`'s (the vertex stage's `uv`) registers.
    uv: Box<[Reg]>,
    /// `uniforms`: `time`, padding, `resolution`.
    uniforms: [Reg; 4],
    /// `params`: `array<vec4<f32>, 16>`.
    params: Box<[Reg]>,
    /// Whether the shader animates: rendered again every frame.
    pub animated: bool,
}

/// A fragment input the paint's vertex stage supplies.
#[derive(Clone, Copy)]
enum Varying {
    Position,
    Uv,
}

impl PaintProgram {
    /// Compiles `source` the way the GPU backend composes it
    /// ([`cherenkov::wgsl::paint_module`]).
    ///
    /// # Errors
    /// [`ResourceError::Shader`] when the module fails to parse or
    /// validate, has no `main` fragment entry point, reads an input the
    /// paint's vertex stage does not supply, does not return a `vec4<f32>`,
    /// or uses a construct the lane executor does not model.
    pub fn compile(source: &ShaderSource) -> Result<Self, ResourceError> {
        let text = cherenkov::wgsl::paint_module(source);
        let invalid = |error: &dyn std::fmt::Display| ResourceError::Shader(error.to_string());
        let module = naga::front::wgsl::parse_str(&text)
            .map_err(|error| ResourceError::Shader(error.emit_to_string(&text)))?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|error| ResourceError::Shader(error.emit_to_string(&text)))?;
        let entry = module
            .entry_points
            .iter()
            .find(|entry| {
                entry.stage == naga::ShaderStage::Fragment
                    && entry.name == cherenkov::wgsl::PAINT_ENTRY
            })
            .ok_or_else(|| {
                ResourceError::Shader(format!(
                    "the shader paint does not define its fragment entry point `{}`",
                    cherenkov::wgsl::PAINT_ENTRY
                ))
            })?;
        let varyings = entry_varyings(&module, &entry.function)?;
        let program = Program::compile(&module, &info, cherenkov::wgsl::PAINT_ENTRY, &PAINT)
            .map_err(|error| invalid(&error))?;
        if program.result().len() != 4 {
            return Err(ResourceError::Shader(
                "a shader paint's `main` must return `@location(0) vec4<f32>`".into(),
            ));
        }
        let (mut position, mut uv) = (Vec::new(), Vec::new());
        let mut regs = (0..program.arity()).flat_map(|index| program.argument(index).iter());
        for (varying, slots) in varyings {
            let target = match varying {
                Varying::Position => &mut position,
                Varying::Uv => &mut uv,
            };
            target.extend(regs.by_ref().take(slots));
        }
        let global = |name: &str| {
            program
                .global(name)
                .expect("the paint prelude declares `uniforms` and `params`")
                .to_vec()
        };
        let uniforms = global("uniforms");
        let params = global("params");
        Ok(Self {
            position: position.into(),
            uv: uv.into(),
            uniforms: [uniforms[0], uniforms[1], uniforms[2], uniforms[3]],
            params: params.into(),
            animated: source.animated,
            program,
        })
    }

    /// Renders the paint over a `size` texture at `time` seconds with
    /// `uniforms` (at most 64; the rest read zero), on the caller's pool:
    /// row pairs in parallel, each in runs of [`RUN_COLUMNS`] × 2 pixel
    /// quads. A pixel centre `(x + ½, y + ½)` reads the position and uv the
    /// GPU's full-surface triangle interpolates there; a discarded pixel
    /// stays transparent.
    #[expect(
        clippy::cast_precision_loss,
        reason = "texture coordinates fit f32 exactly"
    )]
    pub fn render(&self, size: (u32, u32), time: f32, uniforms: &[f32]) -> CpuImage {
        let (width, height) = (size.0 as usize, size.1 as usize);
        let mut pixels = vec![[0.0; 4]; width * height].into_boxed_slice();
        let (w, h) = (size.0 as f32, size.1 as f32);
        let setup = || {
            let mut regs = self.program.registers();
            for (&reg, value) in self.uniforms.iter().zip([time, 0.0, w, h]) {
                regs.splat(reg, value);
            }
            for (index, &reg) in self.params.iter().enumerate() {
                regs.splat(reg, uniforms.get(index).copied().unwrap_or(0.0));
            }
            regs
        };
        pixels
            .par_chunks_mut(width * 2)
            .enumerate()
            .for_each_init(setup, |regs, (pair, rows)| {
                let y0 = pair * 2;
                for x0 in (0..width).step_by(RUN_COLUMNS) {
                    // Lane `l` is quad `l / 4`'s pixel at column bit 0,
                    // row bit 1: helper lanes past the edge still run so
                    // derivatives see a full quad, and are not stored.
                    let column = |l: usize| x0 + 2 * (l >> 2) + (l & 1);
                    let row = |l: usize| y0 + ((l >> 1) & 1);
                    let px: Lanes = std::array::from_fn(|l| column(l) as f32 + 0.5);
                    let py: Lanes = std::array::from_fn(|l| row(l) as f32 + 0.5);
                    // The triangle sits at depth 0 with w = 1.
                    let position = [px, py, [0.0; LANES], [1.0; LANES]];
                    for (&reg, value) in self.position.iter().zip(position) {
                        *regs.get_mut(reg) = value;
                    }
                    let uv = [px.map(|x| x / w), py.map(|y| y / h)];
                    for (&reg, value) in self.uv.iter().zip(uv) {
                        *regs.get_mut(reg) = value;
                    }
                    let done = self.program.run(regs, [true; LANES], &mut NoCalls);
                    let result = self.program.result();
                    for (l, &live) in done.iter().enumerate() {
                        let (x, y) = (column(l), row(l) - y0);
                        if x >= width || y * width >= rows.len() || !live {
                            continue;
                        }
                        rows[y * width + x] = std::array::from_fn(|c| regs.get(result[c])[l]);
                    }
                }
            });
        CpuImage {
            width: size.0,
            height: size.1,
            pixels,
        }
    }
}

/// The inputs `function` reads, in argument order, each with its slot
/// count: arguments and struct members bound to `@builtin(position)` or
/// `@location(0)`, the two the paint's vertex stage supplies.
fn entry_varyings(
    module: &naga::Module,
    function: &naga::Function,
) -> Result<Vec<(Varying, usize)>, ResourceError> {
    let unsupported = |what: &str| {
        ResourceError::Shader(format!(
            "a shader paint reads only `@builtin(position)` and `@location(0)`, got {what}"
        ))
    };
    let varying = |binding: &naga::Binding, ty: naga::Handle<naga::Type>| {
        let slots = match module.types[ty].inner {
            naga::TypeInner::Scalar(_) => 1,
            naga::TypeInner::Vector { size, .. } => size as usize,
            _ => return Err(unsupported("a non-vector input")),
        };
        match *binding {
            naga::Binding::BuiltIn(naga::BuiltIn::Position { .. }) => {
                Ok((Varying::Position, slots))
            }
            // The vertex stage writes `uv` as a `vec2<f32>`; the GPU's
            // pipeline creation rejects any other type at location 0.
            naga::Binding::Location { location: 0, .. } if slots == 2 => Ok((Varying::Uv, slots)),
            ref other => Err(unsupported(&format!("{other:?}"))),
        }
    };
    let mut varyings = Vec::new();
    for argument in &function.arguments {
        if let Some(binding) = &argument.binding {
            varyings.push(varying(binding, argument.ty)?);
        } else if let naga::TypeInner::Struct { members, .. } = &module.types[argument.ty].inner {
            for member in members {
                let binding = member
                    .binding
                    .as_ref()
                    .ok_or_else(|| unsupported("an unbound struct member"))?;
                varyings.push(varying(binding, member.ty)?);
            }
        } else {
            return Err(unsupported("an unbound argument"));
        }
    }
    Ok(varyings)
}

/// One use of a shader paint: uniforms by their exact IEEE
/// representation, as the GPU backend keys its textures.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct PaintKey {
    shader: u64,
    uniforms: Box<[u32]>,
    size: (u32, u32),
}

/// A rendered use and the shader time it was rendered at.
struct Rendered {
    image: Arc<CpuImage>,
    time: f32,
}

/// The registered shader paints and the images this frame's uses
/// rendered.
pub struct ShaderPaints {
    /// The largest texture side and bytes one use may render: the CPU
    /// image limits.
    limits: (u32, u64),
    programs: FxHashMap<u64, Arc<PaintProgram>>,
    images: FxHashMap<PaintKey, Rendered>,
    used: FxHashSet<PaintKey>,
    /// The first rendered frame's time: shader time counts from it, as on
    /// the GPU backend.
    origin: Option<cherenkov::Instant>,
    time: f32,
}

impl ShaderPaints {
    /// An empty registry whose uses render at most `max_dimension` texels
    /// a side and `max_bytes` each.
    pub fn new(max_dimension: u32, max_bytes: u64) -> Self {
        Self {
            limits: (max_dimension, max_bytes),
            programs: FxHashMap::default(),
            images: FxHashMap::default(),
            used: FxHashSet::default(),
            origin: None,
            time: 0.0,
        }
    }

    /// Registers `program` as shader `id`.
    pub fn add(&mut self, id: u64, program: PaintProgram) {
        self.programs.insert(id, Arc::new(program));
        self.images.retain(|key, _| key.shader != id);
    }

    /// Unregisters shader `id` and drops its rendered uses.
    pub fn remove(&mut self, id: u64) {
        self.programs.remove(&id);
        self.images.retain(|key, _| key.shader != id);
    }

    /// Starts a frame at `time`.
    pub fn begin_frame(&mut self, time: cherenkov::FrameTime) {
        let instant = time.instant();
        let origin = *self.origin.get_or_insert(instant);
        self.time = instant.saturating_duration_since(origin).as_secs_f32();
    }

    /// Drops the rendered uses no draw of this frame asked for.
    pub fn finish_frame(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.images.retain(|key, _| used.contains(key));
    }

    /// The rendered images' heap bytes.
    pub fn bytes(&self) -> u64 {
        self.images
            .values()
            .map(|rendered| rendered.image.bytes())
            .sum()
    }

    /// Shader `shader`'s image over a `size` texture with `uniforms`, and
    /// whether the shader animates. A static use renders once; an animated
    /// one again, on `pool`, whenever the frame's shader time moved.
    ///
    /// # Errors
    /// [`RenderError::Render`] for an unregistered shader, more than 64
    /// uniforms, or a texture beyond the registry's limits.
    pub fn image(
        &mut self,
        shader: u64,
        uniforms: &[f32],
        size: (u32, u32),
        pool: &rayon::ThreadPool,
    ) -> Result<(Arc<CpuImage>, bool), RenderError> {
        let (max_dimension, max_bytes) = self.limits;
        let program = self
            .programs
            .get(&shader)
            .ok_or_else(|| RenderError::Render(format!("unregistered shader {shader}")))?;
        if uniforms.len() > PARAM_SLOTS {
            return Err(RenderError::Render(format!(
                "shader paint accepts at most {PARAM_SLOTS} uniform floats"
            )));
        }
        // Sixteen bytes a texel: f32 RGBA, as every CPU image.
        let bytes = u64::from(size.0) * u64::from(size.1) * 16;
        if size.0 > max_dimension || size.1 > max_dimension || bytes > max_bytes {
            return Err(RenderError::Render(format!(
                "shader texture {size:?} exceeds the CPU image limits ({max_dimension} texels a \
                 side, {max_bytes} bytes)"
            )));
        }
        let key = PaintKey {
            shader,
            uniforms: uniforms.iter().map(|v| v.to_bits()).collect(),
            size,
        };
        let animated = program.animated;
        let image = match self.images.get(&key) {
            Some(rendered) if !animated || rendered.time.to_bits() == self.time.to_bits() => {
                Arc::clone(&rendered.image)
            }
            _ => {
                let time = self.time;
                let image = Arc::new(pool.install(|| program.render(size, time, uniforms)));
                self.images.insert(
                    key.clone(),
                    Rendered {
                        image: Arc::clone(&image),
                        time: self.time,
                    },
                );
                image
            }
        };
        self.used.insert(key);
        Ok((image, animated))
    }
}
