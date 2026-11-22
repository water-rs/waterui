//! User WGSL shaders, evaluated per pixel by `cherenkov-shader`'s reference
//! interpreter.
//!
//! Shader paints and backdrop shaders compose with the engine's contract
//! preludes (`cherenkov_record::wgsl`) exactly as a backend composes them.
//! The shader's own arithmetic is `f32`, as WGSL's is; the oracle reads its
//! result into `f64` and composites it like any other paint.

use cherenkov_record::wgsl;
use cherenkov_shader::eval::{Eval, Value};
use cherenkov_shader::naga::{self, Binding, BuiltIn, Module, TypeInner};

use crate::RenderError;

/// The number of `vec4` entries in the preludes' `params` array.
const PARAM_VECTORS: usize = cherenkov_scene::ShaderPaint::MAX_PARAMS / 4;

/// Parses `text`, naming the shader in the error.
fn parse(text: &str) -> Result<Module, RenderError> {
    naga::front::wgsl::parse_str(text)
        .map_err(|error| RenderError::Shader(error.emit_to_string(text)))
}

/// `params` packed four per `vec4`, zero-filled, as the preludes declare
/// them.
#[expect(
    clippy::cast_possible_truncation,
    reason = "shader params are f32 in WGSL"
)]
#[must_use]
pub fn params(values: &[f64]) -> Value {
    Value::Struct(
        (0..PARAM_VECTORS)
            .map(|vector| {
                let component =
                    |lane: usize| values.get(vector * 4 + lane).map_or(0.0, |&v| v as f32);
                Value::vec(&[component(0), component(1), component(2), component(3)])
            })
            .collect(),
    )
}

/// A parsed shader paint module.
#[derive(Debug)]
pub struct PaintModule(Module);

impl PaintModule {
    /// Composes and parses a shader paint's fragment `source`.
    ///
    /// # Errors
    /// [`RenderError::Shader`] when the composed module does not parse.
    pub fn new(source: &str) -> Result<Self, RenderError> {
        parse(&wgsl::paint_module(source)).map(Self)
    }

    /// An evaluator with the prelude's globals bound: `time` 0,
    /// `resolution` the texture size an engine renders the paint into, and
    /// `params`.
    #[must_use]
    pub fn eval(&self, params: &[f64], resolution: [f32; 2]) -> PaintEval<'_> {
        let mut eval = Eval::new(&self.0);
        eval.bind(
            "uniforms",
            Value::Struct(vec![
                Value::Float(0.0),
                Value::Float(0.0),
                Value::vec(&resolution),
            ]),
        );
        eval.bind("params", self::params(params));
        PaintEval {
            module: &self.0,
            eval,
        }
    }
}

/// A shader paint evaluator.
#[derive(Debug)]
pub struct PaintEval<'m> {
    module: &'m Module,
    eval: Eval<'m>,
}

impl PaintEval<'_> {
    /// The paint at `uv`, with the fragment at `position` (`uv` scaled by
    /// the resolution): premultiplied linear Display P3.
    ///
    /// # Panics
    /// When `main` takes an input the paint's vertex stage does not supply,
    /// or uses a construct the interpreter does not model.
    #[must_use]
    pub fn at(&self, uv: [f32; 2], position: [f32; 2]) -> [f64; 4] {
        let entry = self
            .module
            .entry_points
            .iter()
            .find(|entry| entry.name == wgsl::PAINT_ENTRY)
            .unwrap_or_else(|| panic!("a shader paint defines `{}`", wgsl::PAINT_ENTRY));
        let input = |ty, binding: Option<&Binding>| self.input(ty, binding, uv, position);
        let args = entry
            .function
            .arguments
            .iter()
            .map(|argument| input(argument.ty, argument.binding.as_ref()))
            .collect();
        let out = self.eval.call_entry(wgsl::PAINT_ENTRY, args).components();
        [out[0], out[1], out[2], out[3]].map(f64::from)
    }

    /// The value of a fragment input of type `ty` with `binding`: the
    /// fragment position, `uv` at location 0, or a struct of them.
    fn input(
        &self,
        ty: naga::Handle<naga::Type>,
        binding: Option<&Binding>,
        uv: [f32; 2],
        position: [f32; 2],
    ) -> Value {
        match binding {
            Some(Binding::BuiltIn(BuiltIn::Position { .. })) => {
                Value::vec(&[position[0], position[1], 0.0, 1.0])
            }
            Some(Binding::Location { location: 0, .. }) => Value::vec(&uv),
            None => match &self.module.types[ty].inner {
                TypeInner::Struct { members, .. } => Value::Struct(
                    members
                        .iter()
                        .map(|member| self.input(member.ty, member.binding.as_ref(), uv, position))
                        .collect(),
                ),
                other => panic!("a shader paint input without a binding: {other:?}"),
            },
            Some(other) => {
                panic!("a shader paint input the vertex stage does not supply: {other:?}")
            }
        }
    }
}

/// A parsed backdrop shader module.
#[derive(Debug)]
pub struct BackdropModule(Module);

/// The `BackdropPixel` a backdrop shader receives, in the contract's member
/// order.
#[derive(Clone, Copy, Debug)]
pub struct BackdropPixel {
    /// The member pixel centre in device space.
    pub p: [f64; 2],
    /// The signed distance to the field's edge, negative inside.
    pub sdf: f64,
    /// The field's unit outward normal.
    pub normal: [f64; 2],
    /// The signed distance to the member's own clip edge.
    pub own_sdf: f64,
    /// The member's device bounds size.
    pub size: [f64; 2],
    /// Device pixels per logical pixel of the member's recording.
    pub scale: f64,
    /// The member pixel in the member layer's own space.
    pub local: [f64; 2],
}

/// The capture reads a backdrop shader calls: the hooks a backend binds.
pub trait BackdropReads {
    /// `backdrop_sample(q)`.
    fn sample(&self, q: [f64; 2]) -> [f64; 4];
    /// `backdrop_sample_level(q, level)`.
    fn sample_level(&self, q: [f64; 2], level: f64) -> [f64; 4];
    /// `backdrop_field(q)`: `(sdf, normal, weight)`.
    fn field(&self, q: [f64; 2]) -> (f64, [f64; 2], f64);
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the shader's values are f32 in WGSL"
)]
fn vec_value(values: &[f64]) -> Value {
    Value::Vector(values.iter().map(|&v| v as f32).collect())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the shader's values are f32 in WGSL"
)]
const fn float_value(value: f64) -> Value {
    Value::Float(value as f32)
}

fn point(value: &Value) -> [f64; 2] {
    let components = value.components();
    [f64::from(components[0]), f64::from(components[1])]
}

impl BackdropModule {
    /// Composes and parses a backdrop shader's `source`.
    ///
    /// # Errors
    /// [`RenderError::Shader`] when the composed module does not parse.
    pub fn new(source: &str) -> Result<Self, RenderError> {
        parse(&wgsl::backdrop_module(source)).map(Self)
    }

    /// An evaluator whose capture reads are bound to `reads`.
    #[must_use]
    pub fn eval<'m>(&'m self, reads: &'m impl BackdropReads) -> BackdropEval<'m> {
        let mut eval = Eval::new(&self.0);
        eval.host("backdrop_sample", |args| {
            vec_value(&reads.sample(point(&args[0])))
        });
        eval.host("backdrop_sample_level", |args| {
            let level = f64::from(args[1].components()[0]);
            vec_value(&reads.sample_level(point(&args[0]), level))
        });
        eval.host("backdrop_field", |args| {
            let (sdf, normal, weight) = reads.field(point(&args[0]));
            Value::Struct(vec![
                float_value(sdf),
                vec_value(&normal),
                float_value(weight),
            ])
        });
        BackdropEval { eval }
    }
}

/// A backdrop shader evaluator.
#[derive(Debug)]
pub struct BackdropEval<'m> {
    eval: Eval<'m>,
}

impl BackdropEval<'_> {
    /// `backdrop_effect(px, params)`: premultiplied, unclamped.
    ///
    /// # Panics
    /// When the shader uses a construct the interpreter does not model.
    #[must_use]
    pub fn at(&self, px: &BackdropPixel, params: &Value) -> [f64; 4] {
        let pixel = Value::Struct(vec![
            vec_value(&px.p),
            float_value(px.sdf),
            vec_value(&px.normal),
            float_value(px.own_sdf),
            vec_value(&px.size),
            float_value(px.scale),
            vec_value(&px.local),
        ]);
        let effect = self.eval.function(wgsl::BACKDROP_ENTRY);
        let out = self
            .eval
            .call(effect, vec![pixel, params.clone()])
            .components();
        [out[0], out[1], out[2], out[3]].map(f64::from)
    }
}
