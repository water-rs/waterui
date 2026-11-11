//! User WGSL on `cherenkov-shader`'s lane executor: backdrop effect
//! shaders, evaluated inside a member's composite 32 pixels at a time.
//!
//! A registered source is the backend's prelude followed by the user's
//! text — the same module shape the GPU backend compiles — compiled once
//! at registration into a lane [`Program`]. The raster pass lays each run's
//! lanes out as 2×2 pixel quads, so derivatives see the neighbours a GPU
//! quad sees.

use cherenkov::{BackdropShaderSource, ResourceError};
use cherenkov_shader::lanes::{Interface, Program, Reg};

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
