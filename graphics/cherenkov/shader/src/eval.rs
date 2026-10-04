//! A reference interpreter for the subset of naga IR that snippets use.
//!
//! It runs a snippet's `apply`, or a composed segment, on the CPU with exact
//! `f32` arithmetic, so a composed function can be checked against the
//! sequential application of its snippets, and a CPU kernel can be
//! cross-checked against the shader it replaces. It is an oracle, not an
//! executor: every construct it does not model panics with a message naming
//! it.

use std::collections::HashMap;

use naga::{
    Arena, BinaryOperator, Block, Expression, Function, Handle, ImageQuery, Literal, LocalVariable,
    MathFunction, Module, SampleLevel, ScalarKind, Statement, TypeInner, UnaryOperator,
};

use crate::SamplerFilter;

/// A runtime value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A float scalar (`f32` or `f16`, evaluated as `f32`).
    Float(f32),
    /// A float vector.
    Vector(Vec<f32>),
    /// An integer scalar.
    Int(i64),
    /// A boolean.
    Bool(bool),
    /// A struct, its members in order.
    Struct(Vec<Self>),
    /// A texture registered with [`Eval::texture`].
    Texture(usize),
    /// A sampler with its filter mode.
    Sampler(SamplerFilter),
    /// A pointer to a local variable.
    Pointer(Handle<LocalVariable>),
}

impl Value {
    /// A float vector.
    #[must_use]
    pub fn vec(components: &[f32]) -> Self {
        Self::Vector(components.to_vec())
    }

    /// The components of a float scalar or vector.
    ///
    /// # Panics
    ///
    /// Panics when the value is not a float scalar or vector.
    #[must_use]
    pub fn components(&self) -> Vec<f32> {
        match self {
            Self::Float(value) => vec![*value],
            Self::Vector(values) => values.clone(),
            other => panic!("not numeric: {other:?}"),
        }
    }
}

/// A texture: a grid of texels, so the evaluator can model point and
/// bilinear sampling and gathers.
pub struct Texture<'t> {
    width: u32,
    height: u32,
    texel: Box<dyn Fn(u32, u32) -> [f32; 4] + 't>,
}

impl std::fmt::Debug for Texture<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Texture")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl<'t> Texture<'t> {
    /// A texture of `width` × `height` texels.
    #[must_use]
    pub fn texels(width: u32, height: u32, texel: impl Fn(u32, u32) -> [f32; 4] + 't) -> Self {
        Self {
            width,
            height,
            texel: Box::new(texel),
        }
    }

    /// A texture of `width` × `height` texels that reads `source` at each
    /// texel's centre.
    #[must_use]
    pub fn continuous(width: u32, height: u32, source: impl Fn([f32; 2]) -> [f32; 4] + 't) -> Self {
        Self::texels(width, height, move |x, y| {
            #[allow(clippy::cast_precision_loss, reason = "evaluator textures are tiny")]
            let uv = [
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            ];
            source(uv)
        })
    }

    fn texel_at(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, i64::from(self.width) - 1);
        let y = y.clamp(0, i64::from(self.height) - 1);
        let clamped = |coord: i64| u32::try_from(coord).unwrap_or(0);
        (self.texel)(clamped(x), clamped(y))
    }

    /// The texel at normalized `uv` — nearest (point) sampling.
    fn nearest(&self, uv: [f32; 2]) -> [f32; 4] {
        let (x, y, ..) = self.footprint(uv);
        self.texel_at(x, y)
    }

    /// The bilinear sample at normalized `uv` — linear filtering.
    fn bilinear(&self, uv: [f32; 2]) -> [f32; 4] {
        let (x, y, fx, fy) = self.footprint(uv);
        let blend = |a: [f32; 4], b: [f32; 4], t: f32| -> [f32; 4] {
            [
                a[0].mul_add(1.0 - t, b[0] * t),
                a[1].mul_add(1.0 - t, b[1] * t),
                a[2].mul_add(1.0 - t, b[2] * t),
                a[3].mul_add(1.0 - t, b[3] * t),
            ]
        };
        let top = blend(self.texel_at(x, y), self.texel_at(x + 1, y), fx);
        let bottom = blend(self.texel_at(x, y + 1), self.texel_at(x + 1, y + 1), fx);
        blend(top, bottom, fy)
    }

    /// `textureGather`: component `component` of the four footprint texels,
    /// in WGSL order (top-left, top-right, bottom-right, bottom-left).
    fn gather(&self, uv: [f32; 2], component: usize) -> [f32; 4] {
        let (x, y, ..) = self.footprint(uv);
        [
            self.texel_at(x, y)[component],
            self.texel_at(x + 1, y)[component],
            self.texel_at(x + 1, y + 1)[component],
            self.texel_at(x, y + 1)[component],
        ]
    }

    /// The bilinear footprint at normalized `uv`: the top-left texel and
    /// the fractional position within it.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "evaluator textures are tiny"
    )]
    fn footprint(&self, uv: [f32; 2]) -> (i64, i64, f32, f32) {
        let at = |u: f32, dim: u32| {
            let scaled = u.mul_add(dim as f32, -0.5);
            (scaled.floor() as i64, scaled.fract())
        };
        let (x, fx) = at(uv[0], self.width);
        let (y, fy) = at(uv[1], self.height);
        (x, y, fx, fy)
    }
}

/// Evaluates functions of one module.
#[derive(Debug)]
pub struct Eval<'m> {
    module: &'m Module,
    textures: Vec<Texture<'m>>,
}

struct Frame<'f> {
    function: &'f Function,
    args: Vec<Value>,
    values: HashMap<Handle<Expression>, Value>,
    locals: HashMap<Handle<LocalVariable>, Value>,
}

impl<'m> Eval<'m> {
    /// An evaluator for `module`'s functions.
    #[must_use]
    pub const fn new(module: &'m Module) -> Self {
        Self {
            module,
            textures: Vec::new(),
        }
    }

    /// Registers a texture and returns the value that passes it as an
    /// argument.
    pub fn texture(&mut self, texture: Texture<'m>) -> Value {
        self.textures.push(texture);
        Value::Texture(self.textures.len() - 1)
    }

    /// The texture object a `Value::Texture` refers to.
    fn texture_at(&self, value: &Value) -> &Texture<'m> {
        let Value::Texture(index) = value else {
            panic!("expected a texture, got {value:?}");
        };
        &self.textures[*index]
    }

    /// The function named `name`.
    ///
    /// # Panics
    ///
    /// Panics when the module has no function of that name.
    #[must_use]
    pub fn function(&self, name: &str) -> Handle<Function> {
        self.module
            .functions
            .iter()
            .find(|(_, function)| function.name.as_deref() == Some(name))
            .map_or_else(|| panic!("no function `{name}`"), |(handle, _)| handle)
    }

    /// Calls `function` with `args` and returns its result.
    ///
    /// # Panics
    ///
    /// Panics when the function uses a construct the evaluator does not
    /// model, when an argument has the wrong shape, or when the function
    /// returns no value.
    #[must_use]
    pub fn call(&self, function: Handle<Function>, args: Vec<Value>) -> Value {
        let function = &self.module.functions[function];
        let mut frame = Frame {
            function,
            args,
            values: HashMap::new(),
            locals: HashMap::new(),
        };
        for (handle, local) in function.local_variables.iter() {
            let value = local.init.map_or_else(
                || self.zero(local.ty),
                |init| self.expression(&function.expressions, init, &mut frame),
            );
            frame.locals.insert(handle, value);
        }
        self.block(&function.body, &mut frame)
            .expect("the function returns a value")
    }

    fn block(&self, block: &Block, frame: &mut Frame<'_>) -> Option<Value> {
        for statement in block {
            match statement {
                Statement::Emit(range) => {
                    for handle in range.clone() {
                        let value = self.expression(&frame.function.expressions, handle, frame);
                        frame.values.insert(handle, value);
                    }
                }
                Statement::Block(inner) => {
                    if let Some(value) = self.block(inner, frame) {
                        return Some(value);
                    }
                }
                Statement::If {
                    condition,
                    accept,
                    reject,
                } => {
                    let taken =
                        match self.expression(&frame.function.expressions, *condition, frame) {
                            Value::Bool(value) => value,
                            other => panic!("condition is {other:?}"),
                        };
                    let branch = if taken { accept } else { reject };
                    if let Some(value) = self.block(branch, frame) {
                        return Some(value);
                    }
                }
                Statement::Store { pointer, value } => {
                    let Value::Pointer(local) =
                        self.expression(&frame.function.expressions, *pointer, frame)
                    else {
                        panic!("store through a non-local pointer");
                    };
                    let value = self.expression(&frame.function.expressions, *value, frame);
                    frame.locals.insert(local, value);
                }
                Statement::Call {
                    function,
                    arguments,
                    result,
                } => {
                    let args = arguments
                        .iter()
                        .map(|&argument| {
                            self.expression(&frame.function.expressions, argument, frame)
                        })
                        .collect();
                    let value = self.call(*function, args);
                    if let Some(result) = result {
                        frame.values.insert(*result, value);
                    }
                }
                Statement::Return { value } => {
                    return value
                        .map(|value| self.expression(&frame.function.expressions, value, frame));
                }
                other => panic!("the evaluator does not support {other:?}"),
            }
        }
        None
    }

    fn global(&self, handle: Handle<Expression>) -> Value {
        let mut frame = Frame {
            function: self.module.functions.iter().next().expect("a function").1,
            args: Vec::new(),
            values: HashMap::new(),
            locals: HashMap::new(),
        };
        self.expression(&self.module.global_expressions, handle, &mut frame)
    }

    fn zero(&self, ty: Handle<naga::Type>) -> Value {
        match &self.module.types[ty].inner {
            TypeInner::Scalar(scalar) if scalar.kind == ScalarKind::Float => Value::Float(0.0),
            TypeInner::Scalar(scalar) if scalar.kind == ScalarKind::Bool => Value::Bool(false),
            TypeInner::Scalar(_) => Value::Int(0),
            TypeInner::Vector { size, .. } => Value::Vector(vec![0.0; *size as usize]),
            TypeInner::Struct { members, .. } => {
                Value::Struct(members.iter().map(|member| self.zero(member.ty)).collect())
            }
            other => panic!("no zero value for {other:?}"),
        }
    }

    #[allow(clippy::too_many_lines, reason = "one arm per supported expression")]
    fn expression(
        &self,
        arena: &Arena<Expression>,
        handle: Handle<Expression>,
        frame: &mut Frame<'_>,
    ) -> Value {
        if std::ptr::eq(arena, &raw const frame.function.expressions)
            && let Some(value) = frame.values.get(&handle)
        {
            return value.clone();
        }
        let sub = |operand: Handle<Expression>, frame: &mut Frame<'_>| {
            self.expression(arena, operand, frame)
        };
        match arena[handle] {
            Expression::Literal(literal) => match literal {
                Literal::F32(value) => Value::Float(value),
                Literal::F16(value) => Value::Float(value.to_f32()),
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "evaluated literals are small"
                )]
                Literal::AbstractFloat(value) | Literal::F64(value) => Value::Float(value as f32),
                Literal::I16(value) => Value::Int(i64::from(value)),
                Literal::U16(value) => Value::Int(i64::from(value)),
                Literal::I32(value) => Value::Int(i64::from(value)),
                Literal::U32(value) => Value::Int(i64::from(value)),
                Literal::AbstractInt(value) | Literal::I64(value) => Value::Int(value),
                #[allow(clippy::cast_possible_wrap, reason = "evaluated literals are small")]
                Literal::U64(value) => Value::Int(value as i64),
                Literal::Bool(value) => Value::Bool(value),
            },
            Expression::Constant(constant) => self.global(self.module.constants[constant].init),
            Expression::ZeroValue(ty) => self.zero(ty),
            Expression::Compose { ty, ref components } => {
                let values: Vec<Value> = components
                    .iter()
                    .map(|&component| sub(component, frame))
                    .collect();
                match self.module.types[ty].inner {
                    TypeInner::Struct { .. } => Value::Struct(values),
                    _ => Value::Vector(values.iter().flat_map(Value::components).collect()),
                }
            }
            Expression::AccessIndex { base, index } => match sub(base, frame) {
                Value::Vector(values) => Value::Float(values[index as usize]),
                Value::Struct(fields) => fields[index as usize].clone(),
                other => panic!("cannot index {other:?}"),
            },
            Expression::Splat { size, value } => {
                let Value::Float(value) = sub(value, frame) else {
                    panic!("splat of a non-scalar");
                };
                Value::Vector(vec![value; size as usize])
            }
            Expression::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let values = sub(vector, frame).components();
                Value::Vector(
                    pattern[..size as usize]
                        .iter()
                        .map(|&component| values[component as usize])
                        .collect(),
                )
            }
            Expression::FunctionArgument(index) => frame.args[index as usize].clone(),
            Expression::LocalVariable(local) => Value::Pointer(local),
            Expression::Load { pointer } => {
                let Value::Pointer(local) = sub(pointer, frame) else {
                    panic!("load through a non-local pointer");
                };
                frame.locals[&local].clone()
            }
            Expression::ImageSample {
                image,
                sampler,
                gather,
                coordinate,
                level,
                depth_ref,
                ..
            } => {
                assert!(
                    matches!(
                        level,
                        SampleLevel::Exact(_) | SampleLevel::Zero | SampleLevel::Auto
                    ),
                    "unsupported sample level"
                );
                assert!(depth_ref.is_none(), "depth-comparison sampling");
                let uv = sub(coordinate, frame).components();
                let uv = [uv[0], uv[1]];
                let texture = self.texture_at(&sub(image, frame));
                if let Some(component) = gather {
                    return Value::vec(&texture.gather(uv, component as usize));
                }
                let Value::Sampler(filter) = sub(sampler, frame) else {
                    panic!("sampling through a non-sampler");
                };
                Value::vec(&match filter {
                    SamplerFilter::Point => texture.nearest(uv),
                    SamplerFilter::Filtered => texture.bilinear(uv),
                })
            }
            Expression::ImageLoad {
                image, coordinate, ..
            } => {
                let at = sub(coordinate, frame).components();
                let texture = self.texture_at(&sub(image, frame));
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "evaluated loads use small coordinates"
                )]
                let texel = texture.texel_at(at[0] as i64, at[1] as i64);
                Value::vec(&texel)
            }
            Expression::ImageQuery { image, query } => {
                let texture = self.texture_at(&sub(image, frame));
                match query {
                    ImageQuery::Size { .. } =>
                    {
                        #[allow(
                            clippy::cast_precision_loss,
                            reason = "evaluator textures are tiny"
                        )]
                        Value::vec(&[texture.width as f32, texture.height as f32])
                    }
                    other => panic!("the evaluator does not support {other:?}"),
                }
            }
            Expression::Unary { op, expr } => match (op, sub(expr, frame)) {
                (UnaryOperator::Negate, value) => numeric(
                    &value,
                    value
                        .components()
                        .iter()
                        .map(|component| -component)
                        .collect(),
                ),
                (UnaryOperator::LogicalNot, Value::Bool(value)) => Value::Bool(!value),
                (op, value) => panic!("unsupported {op:?} on {value:?}"),
            },
            Expression::Binary { op, left, right } => {
                let left = sub(left, frame);
                let right = sub(right, frame);
                binary(op, &left, &right)
            }
            Expression::Select {
                condition,
                accept,
                reject,
            } => match sub(condition, frame) {
                Value::Bool(true) => sub(accept, frame),
                Value::Bool(false) => sub(reject, frame),
                other => panic!("select on {other:?}"),
            },
            Expression::Math {
                fun,
                arg,
                arg1,
                arg2,
                ..
            } => {
                let first = sub(arg, frame);
                let second = arg1.map(|handle| sub(handle, frame));
                let third = arg2.map(|handle| sub(handle, frame));
                math(fun, &first, second.as_ref(), third.as_ref())
            }
            Expression::As {
                expr,
                kind: ScalarKind::Float,
                convert,
            } => {
                let value = sub(expr, frame);
                match convert {
                    Some(2) => numeric(
                        &value,
                        value
                            .components()
                            .iter()
                            .map(|&component| half::f16::from_f32(component).to_f32())
                            .collect(),
                    ),
                    _ => value,
                }
            }
            Expression::CallResult(_) => panic!("call result read before its call"),
            ref other => panic!("the evaluator does not support {other:?}"),
        }
    }
}

/// A value of `like`'s shape holding `components`.
fn numeric(like: &Value, components: Vec<f32>) -> Value {
    match like {
        Value::Float(_) => Value::Float(components[0]),
        _ => Value::Vector(components),
    }
}

fn zip(left: &Value, right: &Value, combine: impl Fn(f32, f32) -> f32) -> Value {
    let lefts = left.components();
    let rights = right.components();
    let count = lefts.len().max(rights.len());
    let at = |values: &[f32], index: usize| {
        if values.len() == 1 {
            values[0]
        } else {
            values[index]
        }
    };
    let components: Vec<f32> = (0..count)
        .map(|index| combine(at(&lefts, index), at(&rights, index)))
        .collect();
    if count == 1 {
        Value::Float(components[0])
    } else {
        Value::Vector(components)
    }
}

fn binary(op: BinaryOperator, left: &Value, right: &Value) -> Value {
    match op {
        BinaryOperator::Add => zip(left, right, |lhs, rhs| lhs + rhs),
        BinaryOperator::Subtract => zip(left, right, |lhs, rhs| lhs - rhs),
        BinaryOperator::Multiply => zip(left, right, |lhs, rhs| lhs * rhs),
        BinaryOperator::Divide => zip(left, right, |lhs, rhs| lhs / rhs),
        BinaryOperator::Less => Value::Bool(left.components()[0] < right.components()[0]),
        BinaryOperator::Greater => Value::Bool(left.components()[0] > right.components()[0]),
        other => panic!("the evaluator does not support {other:?}"),
    }
}

fn math(fun: MathFunction, first: &Value, second: Option<&Value>, third: Option<&Value>) -> Value {
    let unary =
        |apply: fn(f32) -> f32| numeric(first, first.components().into_iter().map(apply).collect());
    match fun {
        MathFunction::Abs => unary(f32::abs),
        MathFunction::Exp2 => unary(f32::exp2),
        MathFunction::Sqrt => unary(f32::sqrt),
        MathFunction::Floor => unary(f32::floor),
        MathFunction::Fract => unary(f32::fract),
        MathFunction::Saturate => unary(|value| value.clamp(0.0, 1.0)),
        MathFunction::Cos => unary(f32::cos),
        MathFunction::Sin => unary(f32::sin),
        MathFunction::Min => zip(first, second.expect("min takes two"), f32::min),
        MathFunction::Max => zip(first, second.expect("max takes two"), f32::max),
        MathFunction::Pow => zip(first, second.expect("pow takes two"), f32::powf),
        MathFunction::Clamp => {
            let low = zip(first, second.expect("clamp takes three"), f32::max);
            zip(&low, third.expect("clamp takes three"), f32::min)
        }
        MathFunction::Mix => {
            let end = second.expect("mix takes three");
            let weight = third.expect("mix takes three");
            let difference = zip(end, first, |to, from| to - from);
            let scaled = zip(&difference, weight, |delta, amount| delta * amount);
            zip(first, &scaled, |from, step| from + step)
        }
        MathFunction::Dot => {
            let products = zip(first, second.expect("dot takes two"), |lhs, rhs| lhs * rhs);
            Value::Float(products.components().iter().sum())
        }
        other => panic!("the evaluator does not support {other:?}"),
    }
}

/// Asserts two numeric values agree within `tolerance` per component.
///
/// # Panics
///
/// Panics when they do not, or when they have different component counts.
pub fn assert_close(actual: &Value, expected: &Value, tolerance: f32) {
    let actuals = actual.components();
    let expecteds = expected.components();
    assert_eq!(actuals.len(), expecteds.len(), "{actual:?} vs {expected:?}");
    for (got, want) in actuals.iter().zip(&expecteds) {
        assert!(
            (got - want).abs() <= tolerance,
            "{actual:?} vs {expected:?}"
        );
    }
}
