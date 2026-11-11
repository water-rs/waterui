//! naga IR to the lane program.
//!
//! Every value flattens to scalar registers ([`Ty`]). A composite built
//! from other values (`Compose`, `Splat`, `Swizzle`, a static member)
//! aliases their registers and costs nothing; ops are emitted only for
//! arithmetic, loads, conversions and host calls. Each function compiles
//! once into its own static frame — WGSL forbids recursion, so a function
//! is never active twice — and a call copies its arguments in and its
//! result out. Expressions naga leaves outside any `Emit` are constant
//! expressions; they compile into the program's constant prologue.

use std::collections::HashMap;
use std::rc::Rc;

use naga::valid::{FunctionInfo, ModuleInfo};
use naga::{
    AddressSpace, BinaryOperator, Block, DerivativeAxis, DerivativeControl, Expression, Function,
    Handle, Literal, MathFunction, Module, RelationalFunction, ScalarKind, Statement, SwitchValue,
    UnaryOperator,
};

use super::exec::{Case, Cast, Cmp, Deriv, F1, F2, F3, I1, I2, Node, Op};
use super::ty::{Kind, Ty};
use super::{Interface, LaneError, Reg};

/// A compiled value: its layout and registers.
#[derive(Clone, Debug)]
struct Val {
    ty: Ty,
    regs: Vec<Reg>,
}

/// A pointer into a register block, with its dynamic indices: the element
/// at `offset + Σ clamp(index) · stride`.
#[derive(Clone, Debug)]
struct Ptr {
    root: Rc<[Reg]>,
    ty: Ty,
    offset: usize,
    dynamic: Vec<Dynamic>,
}

#[derive(Clone, Copy, Debug)]
struct Dynamic {
    index: Reg,
    signed: bool,
    stride: usize,
    count: u32,
}

#[derive(Clone, Debug)]
enum Cx {
    Val(Val),
    Ptr(Ptr),
    /// A texture or sampler.
    Opaque,
}

/// The most candidates a dynamically indexed access may select among.
const MAX_CANDIDATES: usize = 4096;

/// A function's static frame.
#[derive(Clone)]
struct Frame {
    index: u32,
    args: Vec<Val>,
    result: Option<Val>,
}

struct FnCx<'f> {
    function: &'f Function,
    info: &'f FunctionInfo,
    exprs: Vec<Option<Cx>>,
    locals: Vec<Rc<[Reg]>>,
    args: Vec<Val>,
    result: Option<Val>,
}

/// What [`Compiler::compile`] produces.
pub struct Compiled {
    pub functions: Vec<Vec<Node>>,
    pub consts: Vec<Op>,
    pub registers: usize,
    pub arguments: Vec<Box<[Reg]>>,
    pub result: Box<[Reg]>,
    pub globals: Vec<(String, Box<[Reg]>)>,
}

pub struct Compiler<'m> {
    module: &'m Module,
    info: &'m ModuleInfo,
    interface: &'m Interface<'m>,
    next: Reg,
    consts: Vec<Op>,
    known: HashMap<Reg, u32>,
    splats: HashMap<u32, Reg>,
    global_exprs: HashMap<Handle<Expression>, Val>,
    functions: Vec<Vec<Node>>,
    frames: HashMap<Handle<Function>, Frame>,
    globals: HashMap<Handle<naga::GlobalVariable>, Rc<[Reg]>>,
    global_names: Vec<(String, Box<[Reg]>)>,
    private_init: Vec<Node>,
}

fn literal(literal: Literal) -> Result<(Kind, u32), LaneError> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "WGSL abstract literals concretize to 32 bits"
    )]
    Ok(match literal {
        Literal::F32(v) => (Kind::Float, v.to_bits()),
        Literal::F16(v) => (Kind::Float, v.to_f32().to_bits()),
        Literal::AbstractFloat(v) | Literal::F64(v) => (Kind::Float, (v as f32).to_bits()),
        Literal::I32(v) => (Kind::Sint, v as u32),
        Literal::I16(v) => (Kind::Sint, i32::from(v) as u32),
        Literal::U32(v) => (Kind::Uint, v),
        Literal::U16(v) => (Kind::Uint, u32::from(v)),
        Literal::AbstractInt(v) => (Kind::Sint, v as i32 as u32),
        Literal::Bool(v) => (Kind::Bool, u32::from(v)),
        other => return Err(LaneError::unsupported(format!("the literal {other:?}"))),
    })
}

impl<'m> Compiler<'m> {
    pub fn new(module: &'m Module, info: &'m ModuleInfo, interface: &'m Interface<'m>) -> Self {
        Self {
            module,
            info,
            interface,
            next: 0,
            consts: Vec::new(),
            known: HashMap::new(),
            splats: HashMap::new(),
            global_exprs: HashMap::new(),
            functions: Vec::new(),
            frames: HashMap::new(),
            globals: HashMap::new(),
            global_names: Vec::new(),
            private_init: Vec::new(),
        }
    }

    pub fn compile(mut self, entry: &str) -> Result<Compiled, LaneError> {
        self.allocate_globals()?;
        // Function 0 is the entry.
        self.functions.push(Vec::new());
        let (function, info) = self
            .module
            .entry_points
            .iter()
            .enumerate()
            .find(|(_, point)| point.name == entry)
            .map(|(index, point)| (&point.function, self.info.get_entry_point(index)))
            .or_else(|| {
                self.module
                    .functions
                    .iter()
                    .find(|(_, function)| function.name.as_deref() == Some(entry))
                    .map(|(handle, function)| (function, &self.info[handle]))
            })
            .ok_or_else(|| LaneError::Entry(entry.to_owned()))?;
        let (body, args, result) = self.function_body(function, info)?;
        let mut nodes = std::mem::take(&mut self.private_init);
        nodes.extend(body);
        self.functions[0] = nodes;
        Ok(Compiled {
            functions: self.functions,
            consts: self.consts,
            registers: self.next as usize,
            arguments: args.into_iter().map(|val| val.regs.into()).collect(),
            result: result.map_or_else(Box::default, |val| val.regs.into()),
            globals: self.global_names,
        })
    }

    fn alloc(&mut self, n: usize) -> Vec<Reg> {
        let start = self.next;
        let n = Reg::try_from(n).expect("register counts fit in u32");
        self.next = start.checked_add(n).expect("register counts fit in u32");
        (start..start + n).collect()
    }

    fn reg(&mut self) -> Reg {
        self.alloc(1)[0]
    }

    /// A register holding `bits` in every lane, from the prologue.
    fn konst(&mut self, bits: u32) -> Reg {
        if let Some(&reg) = self.splats.get(&bits) {
            return reg;
        }
        let dst = self.reg();
        self.consts.push(Op::Splat { dst, bits });
        self.known.insert(dst, bits);
        self.splats.insert(bits, dst);
        dst
    }

    fn zero(&mut self, ty: Ty) -> Val {
        let zero = self.konst(0);
        Val {
            regs: vec![zero; ty.slots()],
            ty,
        }
    }

    fn ty(&self, inner: &naga::TypeInner) -> Result<Ty, LaneError> {
        Ty::of(self.module, inner)
    }

    fn allocate_globals(&mut self) -> Result<(), LaneError> {
        for (handle, global) in self.module.global_variables.iter() {
            let ty = self.ty(&self.module.types[global.ty].inner)?;
            match global.space {
                AddressSpace::Handle => continue,
                AddressSpace::Private | AddressSpace::Uniform => {}
                other => {
                    return Err(LaneError::unsupported(format!(
                        "a global in the {other:?} address space"
                    )));
                }
            }
            let regs: Rc<[Reg]> = self.alloc(ty.slots()).into();
            let name = global.name.clone().unwrap_or_default();
            let input = self.interface.inputs.contains(&name.as_str());
            if global.space == AddressSpace::Private && !input {
                // Private memory starts each invocation at its initializer,
                // or zero.
                let init = match global.init {
                    Some(init) => self.global_expr(init)?,
                    None => self.zero(ty),
                };
                for (&dst, &src) in regs.iter().zip(&init.regs) {
                    self.private_init.push(Node::Op(Op::Copy { dst, src }));
                }
            }
            self.global_names
                .push((name, regs.iter().copied().collect()));
            self.globals.insert(handle, regs);
        }
        Ok(())
    }

    /// A constant expression of the module's global arena.
    fn global_expr(&mut self, handle: Handle<Expression>) -> Result<Val, LaneError> {
        if let Some(val) = self.global_exprs.get(&handle) {
            return Ok(val.clone());
        }
        let module = self.module;
        let val = match module.global_expressions[handle] {
            Expression::Literal(value) => {
                let (kind, bits) = literal(value)?;
                Val {
                    ty: Ty::Scalar(kind),
                    regs: vec![self.konst(bits)],
                }
            }
            Expression::Constant(constant) => self.global_expr(module.constants[constant].init)?,
            Expression::ZeroValue(ty) => {
                let ty = self.ty(&module.types[ty].inner)?;
                self.zero(ty)
            }
            Expression::Compose { ty, ref components } => {
                let ty = self.ty(&module.types[ty].inner)?;
                let mut regs = Vec::with_capacity(ty.slots());
                for &component in components {
                    regs.extend(self.global_expr(component)?.regs);
                }
                Val { ty, regs }
            }
            Expression::Splat { size, value } => {
                let value = self.global_expr(value)?;
                let kind = value.ty.scalar()?;
                Val {
                    ty: Ty::Vector(size as u32, kind),
                    regs: vec![value.regs[0]; size as usize],
                }
            }
            ref other => {
                return Err(LaneError::unsupported(format!(
                    "the constant expression {other:?}"
                )));
            }
        };
        self.global_exprs.insert(handle, val.clone());
        Ok(val)
    }

    /// Compiles `function`'s body into a fresh frame; returns the body,
    /// its argument and its result registers.
    #[expect(clippy::type_complexity, reason = "the frame's three parts")]
    fn function_body(
        &mut self,
        function: &'m Function,
        info: &'m FunctionInfo,
    ) -> Result<(Vec<Node>, Vec<Val>, Option<Val>), LaneError> {
        let mut args = Vec::with_capacity(function.arguments.len());
        for argument in &function.arguments {
            let ty = self.ty(&self.module.types[argument.ty].inner)?;
            let regs = self.alloc(ty.slots());
            args.push(Val { ty, regs });
        }
        let result = match &function.result {
            Some(result) => {
                let ty = self.ty(&self.module.types[result.ty].inner)?;
                let regs = self.alloc(ty.slots());
                Some(Val { ty, regs })
            }
            None => None,
        };
        let mut fx = FnCx {
            function,
            info,
            exprs: vec![None; function.expressions.len()],
            locals: Vec::with_capacity(function.local_variables.len()),
            args: args.clone(),
            result: result.clone(),
        };
        let mut body = Vec::new();
        for (_, local) in function.local_variables.iter() {
            let ty = self.ty(&self.module.types[local.ty].inner)?;
            let regs: Rc<[Reg]> = self.alloc(ty.slots()).into();
            // Locals reset on every entry: frames are reused across calls.
            let init = match local.init {
                Some(init) => match self.operand(&mut fx, init)? {
                    Cx::Val(val) => val,
                    _ => return Err(LaneError::unsupported("a non-value local initializer")),
                },
                None => self.zero(ty),
            };
            for (&dst, &src) in regs.iter().zip(&init.regs) {
                body.push(Node::Op(Op::Copy { dst, src }));
            }
            fx.locals.push(regs);
        }
        body.extend(self.block(&mut fx, &function.body)?);
        Ok((body, args, result))
    }

    fn frame(&mut self, handle: Handle<Function>) -> Result<Frame, LaneError> {
        if let Some(frame) = self.frames.get(&handle) {
            return Ok(frame.clone());
        }
        let index = u32::try_from(self.functions.len()).expect("function counts fit in u32");
        self.functions.push(Vec::new());
        let (body, args, result) =
            self.function_body(&self.module.functions[handle], &self.info[handle])?;
        self.functions[index as usize] = body;
        let frame = Frame {
            index,
            args,
            result,
        };
        self.frames.insert(handle, frame.clone());
        Ok(frame)
    }

    fn block(&mut self, fx: &mut FnCx<'m>, block: &Block) -> Result<Vec<Node>, LaneError> {
        let mut out = Vec::new();
        for statement in block {
            self.statement(fx, statement, &mut out)?;
        }
        Ok(out)
    }

    #[expect(clippy::too_many_lines, reason = "one arm per statement")]
    fn statement(
        &mut self,
        fx: &mut FnCx<'m>,
        statement: &Statement,
        out: &mut Vec<Node>,
    ) -> Result<(), LaneError> {
        match *statement {
            Statement::Emit(ref range) => {
                for handle in range.clone() {
                    let cx = self.expr(fx, handle, out)?;
                    fx.exprs[handle.index()] = Some(cx);
                }
            }
            Statement::Block(ref inner) => out.extend(self.block(fx, inner)?),
            Statement::If {
                condition,
                ref accept,
                ref reject,
            } => {
                let cond = self.value(fx, condition)?.regs[0];
                let accept = self.block(fx, accept)?;
                let reject = self.block(fx, reject)?;
                out.push(Node::If {
                    cond,
                    accept,
                    reject,
                });
            }
            Statement::Switch {
                selector,
                ref cases,
            } => {
                let selector = self.value(fx, selector)?.regs[0];
                let mut compiled = Vec::with_capacity(cases.len());
                for case in cases {
                    #[expect(clippy::cast_sign_loss, reason = "selectors compare as bits")]
                    let (values, default) = match case.value {
                        SwitchValue::I32(v) => (vec![v as u32], false),
                        SwitchValue::U32(v) => (vec![v], false),
                        SwitchValue::Default => (Vec::new(), true),
                    };
                    compiled.push(Case {
                        values,
                        default,
                        body: self.block(fx, &case.body)?,
                        fall_through: case.fall_through,
                    });
                }
                out.push(Node::Switch {
                    selector,
                    cases: compiled,
                });
            }
            Statement::Loop {
                ref body,
                ref continuing,
                break_if,
            } => {
                let body = self.block(fx, body)?;
                let continuing = self.block(fx, continuing)?;
                let break_if = match break_if {
                    Some(condition) => Some(self.value(fx, condition)?.regs[0]),
                    None => None,
                };
                out.push(Node::Loop {
                    body,
                    continuing,
                    break_if,
                });
            }
            Statement::Break => out.push(Node::Break),
            Statement::Continue => out.push(Node::Continue),
            Statement::Kill => out.push(Node::Kill),
            Statement::Return { value } => {
                let (dst, src) = match (value, &fx.result) {
                    (Some(value), Some(result)) => {
                        let dst = result.regs.clone();
                        (dst, self.value(fx, value)?.regs)
                    }
                    _ => (Vec::new(), Vec::new()),
                };
                out.push(Node::Return {
                    dst: dst.into(),
                    src: src.into(),
                });
            }
            Statement::Store { pointer, value } => {
                let Cx::Ptr(ptr) = self.operand(fx, pointer)? else {
                    return Err(LaneError::unsupported("a store through a non-pointer"));
                };
                let value = self.value(fx, value)?;
                self.store(&ptr, &value, out)?;
            }
            Statement::Call {
                function,
                ref arguments,
                result,
            } => {
                let mut args = Vec::with_capacity(arguments.len());
                for &argument in arguments {
                    args.push(self.operand(fx, argument)?);
                }
                let callee = &self.module.functions[function];
                let host = callee.name.as_deref().and_then(|name| {
                    self.interface
                        .functions
                        .iter()
                        .position(|candidate| *candidate == name)
                });
                let value = if let Some(host) = host {
                    let mut regs = Vec::new();
                    for arg in &args {
                        if let Cx::Val(val) = arg {
                            regs.extend(&val.regs);
                        }
                    }
                    let ty = match &callee.result {
                        Some(result) => Some(self.ty(&self.module.types[result.ty].inner)?),
                        None => None,
                    };
                    let outs = ty
                        .as_ref()
                        .map_or_else(Vec::new, |ty| self.alloc(ty.slots()));
                    out.push(Node::Op(Op::Host {
                        function: u32::try_from(host).expect("host function counts fit in u32"),
                        args: regs.into(),
                        outs: outs.clone().into(),
                    }));
                    ty.map(|ty| Val { ty, regs: outs })
                } else {
                    let frame = self.frame(function)?;
                    for (arg, param) in args.iter().zip(&frame.args) {
                        if let Cx::Val(val) = arg {
                            for (&dst, &src) in param.regs.iter().zip(&val.regs) {
                                out.push(Node::Op(Op::Copy { dst, src }));
                            }
                        }
                    }
                    out.push(Node::Call(frame.index));
                    match frame.result {
                        Some(result) => {
                            let regs = self.alloc(result.regs.len());
                            for (&dst, &src) in regs.iter().zip(&result.regs) {
                                out.push(Node::Op(Op::Copy { dst, src }));
                            }
                            Some(Val {
                                ty: result.ty,
                                regs,
                            })
                        }
                        None => None,
                    }
                };
                if let (Some(result), Some(value)) = (result, value) {
                    fx.exprs[result.index()] = Some(Cx::Val(value));
                }
            }
            ref other => {
                return Err(LaneError::unsupported(format!("the statement {other:?}")));
            }
        }
        Ok(())
    }

    /// An already compiled expression, or a constant one compiled into the
    /// prologue.
    fn operand(&mut self, fx: &mut FnCx<'m>, handle: Handle<Expression>) -> Result<Cx, LaneError> {
        if let Some(cx) = &fx.exprs[handle.index()] {
            return Ok(cx.clone());
        }
        let mut prologue = Vec::new();
        let cx = self.expr(fx, handle, &mut prologue)?;
        for node in prologue {
            let Node::Op(op) = node else {
                unreachable!("expressions compile to straight-line ops")
            };
            self.consts.push(op);
        }
        fx.exprs[handle.index()] = Some(cx.clone());
        Ok(cx)
    }

    fn value(&mut self, fx: &mut FnCx<'m>, handle: Handle<Expression>) -> Result<Val, LaneError> {
        match self.operand(fx, handle)? {
            Cx::Val(val) => Ok(val),
            other => Err(LaneError::unsupported(format!("{other:?} used as a value"))),
        }
    }

    fn result_ty(&self, fx: &FnCx<'m>, handle: Handle<Expression>) -> Result<Ty, LaneError> {
        self.ty(fx.info[handle].ty.inner_with(&self.module.types))
    }

    fn emit(out: &mut Vec<Node>, op: Op) {
        out.push(Node::Op(op));
    }

    fn f1(&mut self, out: &mut Vec<Node>, f: F1, a: Reg) -> Reg {
        let dst = self.reg();
        Self::emit(out, Op::F1 { f, dst, a });
        dst
    }

    fn f2(&mut self, out: &mut Vec<Node>, f: F2, a: Reg, b: Reg) -> Reg {
        let dst = self.reg();
        Self::emit(out, Op::F2 { f, dst, a, b });
        dst
    }

    fn f3(&mut self, out: &mut Vec<Node>, f: F3, a: Reg, b: Reg, c: Reg) -> Reg {
        let dst = self.reg();
        Self::emit(out, Op::F3 { f, dst, a, b, c });
        dst
    }

    fn i1(&mut self, out: &mut Vec<Node>, f: I1, a: Reg) -> Reg {
        let dst = self.reg();
        Self::emit(out, Op::I1 { f, dst, a });
        dst
    }

    fn i2(&mut self, out: &mut Vec<Node>, f: I2, a: Reg, b: Reg) -> Reg {
        let dst = self.reg();
        Self::emit(out, Op::I2 { f, dst, a, b });
        dst
    }

    fn cmp(&mut self, out: &mut Vec<Node>, f: Cmp, a: Reg, b: Reg) -> Reg {
        let dst = self.reg();
        Self::emit(out, Op::Cmp { f, dst, a, b });
        dst
    }

    fn cast(&mut self, out: &mut Vec<Node>, f: Cast, a: Reg) -> Reg {
        let dst = self.reg();
        Self::emit(out, Op::Cast { f, dst, a });
        dst
    }

    fn select(&mut self, out: &mut Vec<Node>, cond: Reg, accept: Reg, reject: Reg) -> Reg {
        let dst = self.reg();
        Self::emit(
            out,
            Op::Select {
                dst,
                cond,
                accept,
                reject,
            },
        );
        dst
    }

    /// Applies `make` lane-wise across `a` and `b`, broadcasting a scalar.
    fn zip(
        &mut self,
        out: &mut Vec<Node>,
        left: &[Reg],
        right: &[Reg],
        mut make: impl FnMut(&mut Self, &mut Vec<Node>, Reg, Reg) -> Reg,
    ) -> Vec<Reg> {
        let width = left.len().max(right.len());
        (0..width)
            .map(|slot| {
                let lhs = left[if left.len() == 1 { 0 } else { slot }];
                let rhs = right[if right.len() == 1 { 0 } else { slot }];
                make(self, out, lhs, rhs)
            })
            .collect()
    }

    fn dot(&mut self, out: &mut Vec<Node>, a: &[Reg], b: &[Reg], kind: Kind) -> Reg {
        let (mul, add) = if kind == Kind::Float {
            (true, true)
        } else {
            (false, false)
        };
        let product = |c: &mut Self, out: &mut Vec<Node>, x, y| {
            if mul {
                c.f2(out, F2::Mul, x, y)
            } else {
                c.i2(out, I2::Mul, x, y)
            }
        };
        let mut acc = product(self, out, a[0], b[0]);
        for i in 1..a.len() {
            let p = product(self, out, a[i], b[i]);
            acc = if add {
                self.f2(out, F2::Add, acc, p)
            } else {
                self.i2(out, I2::Add, acc, p)
            };
        }
        acc
    }

    fn load(&mut self, ptr: &Ptr, out: &mut Vec<Node>) -> Result<Val, LaneError> {
        let n = ptr.ty.slots();
        let dst = self.alloc(n);
        if ptr.dynamic.is_empty() {
            for (i, &dst) in dst.iter().enumerate() {
                Self::emit(
                    out,
                    Op::Copy {
                        dst,
                        src: ptr.root[ptr.offset + i],
                    },
                );
            }
        } else {
            let (sel, table) = self.candidates(ptr, out)?;
            Self::emit(
                out,
                Op::Gather {
                    dst: dst.clone().into(),
                    sel,
                    table: table.into(),
                },
            );
        }
        Ok(Val {
            ty: ptr.ty.clone(),
            regs: dst,
        })
    }

    fn store(&mut self, ptr: &Ptr, value: &Val, out: &mut Vec<Node>) -> Result<(), LaneError> {
        if ptr.dynamic.is_empty() {
            let dst: Box<[Reg]> = ptr.root[ptr.offset..ptr.offset + value.regs.len()].into();
            out.push(Node::Store {
                dst,
                src: value.regs.clone().into(),
            });
        } else {
            let (sel, table) = self.candidates(ptr, out)?;
            out.push(Node::Scatter {
                sel,
                table: table.into(),
                src: value.regs.clone().into(),
            });
        }
        Ok(())
    }

    /// The selector register and candidate table of a dynamic access.
    fn candidates(&mut self, ptr: &Ptr, out: &mut Vec<Node>) -> Result<(Reg, Vec<Reg>), LaneError> {
        let total = ptr
            .dynamic
            .iter()
            .try_fold(1usize, |acc, level| acc.checked_mul(level.count as usize))
            .filter(|&total| total <= MAX_CANDIDATES)
            .ok_or_else(|| {
                LaneError::unsupported(format!(
                    "a dynamic access selecting among more than {MAX_CANDIDATES} elements"
                ))
            })?;
        let width = ptr.ty.slots();
        let mut table = Vec::with_capacity(total * width);
        for candidate in 0..total {
            let mut rest = candidate;
            let mut offset = ptr.offset;
            for level in ptr.dynamic.iter().rev() {
                offset += (rest % level.count as usize) * level.stride;
                rest /= level.count as usize;
            }
            table.extend_from_slice(&ptr.root[offset..offset + width]);
        }
        let mut acc = None;
        let mut mult = u32::try_from(total).expect("bounded by MAX_CANDIDATES");
        for level in &ptr.dynamic {
            mult /= level.count;
            let dst = self.reg();
            Self::emit(
                out,
                Op::Index {
                    dst,
                    acc,
                    index: level.index,
                    signed: level.signed,
                    count: level.count,
                    mult,
                },
            );
            acc = Some(dst);
        }
        Ok((acc.expect("a dynamic access has a level"), table))
    }

    /// The constant value of a register filled by the prologue.
    fn known(&self, reg: Reg) -> Option<u32> {
        self.known.get(&reg).copied()
    }

    #[expect(clippy::too_many_lines, reason = "one arm per expression")]
    fn expr(
        &mut self,
        fx: &mut FnCx<'m>,
        handle: Handle<Expression>,
        out: &mut Vec<Node>,
    ) -> Result<Cx, LaneError> {
        let module = self.module;
        Ok(match fx.function.expressions[handle] {
            Expression::Literal(value) => {
                let (kind, bits) = literal(value)?;
                Cx::Val(Val {
                    ty: Ty::Scalar(kind),
                    regs: vec![self.konst(bits)],
                })
            }
            Expression::Constant(constant) => {
                Cx::Val(self.global_expr(module.constants[constant].init)?)
            }
            Expression::ZeroValue(ty) => {
                let ty = self.ty(&module.types[ty].inner)?;
                Cx::Val(self.zero(ty))
            }
            Expression::Compose { ty, ref components } => {
                let ty = self.ty(&module.types[ty].inner)?;
                let mut regs = Vec::with_capacity(ty.slots());
                for &component in components {
                    regs.extend(self.value(fx, component)?.regs);
                }
                Cx::Val(Val { ty, regs })
            }
            Expression::Access { base, index } => {
                let index_val = self.value(fx, index)?;
                let index_reg = index_val.regs[0];
                let signed = index_val.ty.scalar()? == Kind::Sint;
                match self.operand(fx, base)? {
                    Cx::Ptr(mut ptr) => {
                        let (element, stride, count) = ptr.ty.indexed()?;
                        match self.known(index_reg) {
                            Some(bits) => {
                                #[expect(clippy::cast_possible_wrap, reason = "index bits")]
                                let i = if signed {
                                    u32::try_from((bits as i32).max(0)).unwrap_or(0)
                                } else {
                                    bits
                                };
                                ptr.offset += i.min(count - 1) as usize * stride;
                            }
                            None => ptr.dynamic.push(Dynamic {
                                index: index_reg,
                                signed,
                                stride,
                                count,
                            }),
                        }
                        ptr.ty = element;
                        Cx::Ptr(ptr)
                    }
                    Cx::Val(val) => {
                        let (element, stride, count) = val.ty.indexed()?;
                        let root: Rc<[Reg]> = val.regs.into();
                        let ptr = Ptr {
                            root,
                            ty: element,
                            offset: 0,
                            dynamic: vec![Dynamic {
                                index: index_reg,
                                signed,
                                stride,
                                count,
                            }],
                        };
                        if let Some(bits) = self.known(index_reg) {
                            #[expect(clippy::cast_possible_wrap, reason = "index bits")]
                            let i = if signed {
                                u32::try_from((bits as i32).max(0)).unwrap_or(0)
                            } else {
                                bits
                            };
                            let start = i.min(count - 1) as usize * stride;
                            Cx::Val(Val {
                                regs: ptr.root[start..start + ptr.ty.slots()].to_vec(),
                                ty: ptr.ty,
                            })
                        } else {
                            Cx::Val(self.load(&ptr, out)?)
                        }
                    }
                    Cx::Opaque => return Err(LaneError::unsupported("an index into a texture")),
                }
            }
            Expression::AccessIndex { base, index } => match self.operand(fx, base)? {
                Cx::Ptr(mut ptr) => {
                    let (offset, ty) = ptr.ty.element(index)?;
                    ptr.offset += offset;
                    ptr.ty = ty;
                    Cx::Ptr(ptr)
                }
                Cx::Val(val) => {
                    let (offset, ty) = val.ty.element(index)?;
                    Cx::Val(Val {
                        regs: val.regs[offset..offset + ty.slots()].to_vec(),
                        ty,
                    })
                }
                Cx::Opaque => return Err(LaneError::unsupported("a member of a texture")),
            },
            Expression::Splat { size, value } => {
                let value = self.value(fx, value)?;
                Cx::Val(Val {
                    ty: Ty::Vector(size as u32, value.ty.scalar()?),
                    regs: vec![value.regs[0]; size as usize],
                })
            }
            Expression::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let value = self.value(fx, vector)?;
                Cx::Val(Val {
                    ty: Ty::Vector(size as u32, value.ty.scalar()?),
                    regs: pattern[..size as usize]
                        .iter()
                        .map(|&component| value.regs[component as usize])
                        .collect(),
                })
            }
            Expression::FunctionArgument(index) => {
                let val = fx.args[index as usize].clone();
                if val.ty == Ty::Opaque {
                    Cx::Opaque
                } else {
                    Cx::Val(val)
                }
            }
            Expression::GlobalVariable(global) => match self.globals.get(&global) {
                Some(root) => Cx::Ptr(Ptr {
                    root: Rc::clone(root),
                    ty: self.ty(&module.types[module.global_variables[global].ty].inner)?,
                    offset: 0,
                    dynamic: Vec::new(),
                }),
                None => Cx::Opaque,
            },
            Expression::LocalVariable(local) => Cx::Ptr(Ptr {
                root: Rc::clone(&fx.locals[local.index()]),
                ty: self.ty(&module.types[fx.function.local_variables[local].ty].inner)?,
                offset: 0,
                dynamic: Vec::new(),
            }),
            Expression::Load { pointer } => match self.operand(fx, pointer)? {
                Cx::Ptr(ptr) => Cx::Val(self.load(&ptr, out)?),
                Cx::Opaque => Cx::Opaque,
                Cx::Val(_) => return Err(LaneError::unsupported("a load of a value")),
            },
            Expression::Unary { op, expr } => {
                let value = self.value(fx, expr)?;
                let kind = value.ty.scalar()?;
                let regs = value
                    .regs
                    .iter()
                    .map(|&a| match (op, kind) {
                        (UnaryOperator::Negate, Kind::Float) => self.f1(out, F1::Neg, a),
                        (UnaryOperator::Negate, _) => self.i1(out, I1::Neg, a),
                        (UnaryOperator::LogicalNot, _)
                        | (UnaryOperator::BitwiseNot, Kind::Bool) => self.i1(out, I1::BoolNot, a),
                        (UnaryOperator::BitwiseNot, _) => self.i1(out, I1::Not, a),
                    })
                    .collect();
                Cx::Val(Val { ty: value.ty, regs })
            }
            Expression::Binary { op, left, right } => {
                let left = self.value(fx, left)?;
                let right = self.value(fx, right)?;
                let ty = self.result_ty(fx, handle)?;
                Cx::Val(self.binary(out, op, &left, &right, ty)?)
            }
            Expression::Select {
                condition,
                accept,
                reject,
            } => {
                let cond = self.value(fx, condition)?;
                let accept = self.value(fx, accept)?;
                let reject = self.value(fx, reject)?;
                let regs = (0..accept.regs.len())
                    .map(|i| {
                        let c = cond.regs[if cond.regs.len() == 1 { 0 } else { i }];
                        self.select(out, c, accept.regs[i], reject.regs[i])
                    })
                    .collect();
                Cx::Val(Val {
                    ty: accept.ty,
                    regs,
                })
            }
            Expression::Derivative { axis, ctrl, expr } => {
                let value = self.value(fx, expr)?;
                let coarse = ctrl == DerivativeControl::Coarse;
                let f = match (axis, coarse) {
                    (DerivativeAxis::X, false) => Deriv::X,
                    (DerivativeAxis::Y, false) => Deriv::Y,
                    (DerivativeAxis::Width, false) => Deriv::Width,
                    (DerivativeAxis::X, true) => Deriv::XCoarse,
                    (DerivativeAxis::Y, true) => Deriv::YCoarse,
                    (DerivativeAxis::Width, true) => Deriv::WidthCoarse,
                };
                let regs = value
                    .regs
                    .iter()
                    .map(|&a| {
                        let dst = self.reg();
                        Self::emit(out, Op::Deriv { f, dst, a });
                        dst
                    })
                    .collect();
                Cx::Val(Val { ty: value.ty, regs })
            }
            Expression::Relational { fun, argument } => {
                let value = self.value(fx, argument)?;
                let f = match fun {
                    RelationalFunction::All => I2::And,
                    RelationalFunction::Any => I2::Or,
                    other => {
                        return Err(LaneError::unsupported(format!("the relational {other:?}")));
                    }
                };
                let mut acc = value.regs[0];
                for &next in &value.regs[1..] {
                    acc = self.i2(out, f, acc, next);
                }
                Cx::Val(Val {
                    ty: Ty::Scalar(Kind::Bool),
                    regs: vec![acc],
                })
            }
            Expression::Math {
                fun,
                arg,
                arg1,
                arg2,
                arg3,
            } => {
                if arg3.is_some() {
                    return Err(LaneError::unsupported(format!("the math function {fun:?}")));
                }
                let a = self.value(fx, arg)?;
                let b = match arg1 {
                    Some(h) => Some(self.value(fx, h)?),
                    None => None,
                };
                let c = match arg2 {
                    Some(h) => Some(self.value(fx, h)?),
                    None => None,
                };
                let ty = self.result_ty(fx, handle)?;
                Cx::Val(self.math(out, fun, &a, b.as_ref(), c.as_ref(), ty)?)
            }
            Expression::As {
                expr,
                kind,
                convert,
            } => {
                let value = self.value(fx, expr)?;
                let ty = self.result_ty(fx, handle)?;
                let from = value.ty.scalar()?;
                let regs = match convert {
                    None => value.regs,
                    Some(width) => value
                        .regs
                        .iter()
                        .map(|&a| match (kind, from) {
                            (ScalarKind::Float, Kind::Float) if width == 2 => {
                                self.f1(out, F1::QuantizeF16, a)
                            }
                            (ScalarKind::Float, Kind::Sint) => self.cast(out, Cast::SintToFloat, a),
                            (ScalarKind::Float, Kind::Uint) => self.cast(out, Cast::UintToFloat, a),
                            (ScalarKind::Float, Kind::Bool) => self.cast(out, Cast::BoolToFloat, a),
                            (ScalarKind::Sint, Kind::Float) => self.cast(out, Cast::FloatToSint, a),
                            (ScalarKind::Uint, Kind::Float) => self.cast(out, Cast::FloatToUint, a),
                            (ScalarKind::Bool, Kind::Float) => self.cast(out, Cast::FloatToBool, a),
                            (ScalarKind::Bool, Kind::Sint | Kind::Uint) => {
                                self.cast(out, Cast::BitsToBool, a)
                            }
                            // Same-width integer, bool-to-integer and
                            // float-to-float conversions keep the bits.
                            _ => a,
                        })
                        .collect(),
                };
                Cx::Val(Val { ty, regs })
            }
            Expression::CallResult(_) => {
                return Err(LaneError::unsupported("a call result read before its call"));
            }
            ref other => {
                return Err(LaneError::unsupported(format!("the expression {other:?}")));
            }
        })
    }

    fn binary(
        &mut self,
        out: &mut Vec<Node>,
        op: BinaryOperator,
        left: &Val,
        right: &Val,
        ty: Ty,
    ) -> Result<Val, LaneError> {
        use BinaryOperator as B;
        if matches!(left.ty, Ty::Matrix { .. }) || matches!(right.ty, Ty::Matrix { .. }) {
            return self.matrix(out, op, left, right, ty);
        }
        let kind = left.ty.scalar()?;
        let regs = self.zip(out, &left.regs, &right.regs, |c, out, x, y| {
            match (op, kind) {
                (B::Add, Kind::Float) => c.f2(out, F2::Add, x, y),
                (B::Subtract, Kind::Float) => c.f2(out, F2::Sub, x, y),
                (B::Multiply, Kind::Float) => c.f2(out, F2::Mul, x, y),
                (B::Divide, Kind::Float) => c.f2(out, F2::Div, x, y),
                (B::Modulo, Kind::Float) => c.f2(out, F2::Rem, x, y),
                (B::Add, _) => c.i2(out, I2::Add, x, y),
                (B::Subtract, _) => c.i2(out, I2::Sub, x, y),
                (B::Multiply, _) => c.i2(out, I2::Mul, x, y),
                (B::Divide, Kind::Sint) => c.i2(out, I2::DivS, x, y),
                (B::Divide, _) => c.i2(out, I2::DivU, x, y),
                (B::Modulo, Kind::Sint) => c.i2(out, I2::RemS, x, y),
                (B::Modulo, _) => c.i2(out, I2::RemU, x, y),
                (B::Equal, Kind::Float) => c.cmp(out, Cmp::FEq, x, y),
                (B::NotEqual, Kind::Float) => c.cmp(out, Cmp::FNe, x, y),
                (B::Less, Kind::Float) => c.cmp(out, Cmp::FLt, x, y),
                (B::LessEqual, Kind::Float) => c.cmp(out, Cmp::FLe, x, y),
                (B::Greater, Kind::Float) => c.cmp(out, Cmp::FGt, x, y),
                (B::GreaterEqual, Kind::Float) => c.cmp(out, Cmp::FGe, x, y),
                (B::Equal, _) => c.cmp(out, Cmp::Eq, x, y),
                (B::NotEqual, _) => c.cmp(out, Cmp::Ne, x, y),
                (B::Less, Kind::Sint) => c.cmp(out, Cmp::SLt, x, y),
                (B::LessEqual, Kind::Sint) => c.cmp(out, Cmp::SLe, x, y),
                (B::Greater, Kind::Sint) => c.cmp(out, Cmp::SGt, x, y),
                (B::GreaterEqual, Kind::Sint) => c.cmp(out, Cmp::SGe, x, y),
                (B::Less, _) => c.cmp(out, Cmp::ULt, x, y),
                (B::LessEqual, _) => c.cmp(out, Cmp::ULe, x, y),
                (B::Greater, _) => c.cmp(out, Cmp::UGt, x, y),
                (B::GreaterEqual, _) => c.cmp(out, Cmp::UGe, x, y),
                (B::And | B::LogicalAnd, _) => c.i2(out, I2::And, x, y),
                (B::InclusiveOr | B::LogicalOr, _) => c.i2(out, I2::Or, x, y),
                (B::ExclusiveOr, _) => c.i2(out, I2::Xor, x, y),
                (B::ShiftLeft, _) => c.i2(out, I2::Shl, x, y),
                (B::ShiftRight, Kind::Sint) => c.i2(out, I2::ShrS, x, y),
                (B::ShiftRight, _) => c.i2(out, I2::ShrU, x, y),
            }
        });
        Ok(Val { ty, regs })
    }

    fn matrix(
        &mut self,
        out: &mut Vec<Node>,
        op: BinaryOperator,
        left: &Val,
        right: &Val,
        ty: Ty,
    ) -> Result<Val, LaneError> {
        let column = |val: &Val, rows: u32, j: u32| -> Vec<Reg> {
            val.regs[(j * rows) as usize..((j + 1) * rows) as usize].to_vec()
        };
        let regs = match (op, &left.ty, &right.ty) {
            (BinaryOperator::Add | BinaryOperator::Subtract, _, _) => {
                let f = if op == BinaryOperator::Add {
                    F2::Add
                } else {
                    F2::Sub
                };
                self.zip(out, &left.regs, &right.regs, |c, out, x, y| {
                    c.f2(out, f, x, y)
                })
            }
            (BinaryOperator::Multiply, Ty::Matrix { columns, rows }, Ty::Vector(..)) => {
                // mat · vec: Σ_k column_k · v_k.
                (0..*rows)
                    .map(|i| {
                        let row: Vec<Reg> = (0..*columns)
                            .map(|k| left.regs[(k * rows + i) as usize])
                            .collect();
                        self.dot(out, &row, &right.regs, Kind::Float)
                    })
                    .collect()
            }
            (BinaryOperator::Multiply, Ty::Vector(..), Ty::Matrix { columns, rows }) => (0
                ..*columns)
                .map(|j| {
                    let col = column(right, *rows, j);
                    self.dot(out, &left.regs, &col, Kind::Float)
                })
                .collect(),
            (
                BinaryOperator::Multiply,
                Ty::Matrix {
                    columns: inner,
                    rows,
                },
                Ty::Matrix { columns, .. },
            ) => {
                let mut regs = Vec::with_capacity((columns * rows) as usize);
                for j in 0..*columns {
                    let col = column(right, *inner, j);
                    for i in 0..*rows {
                        let row: Vec<Reg> = (0..*inner)
                            .map(|k| left.regs[(k * rows + i) as usize])
                            .collect();
                        regs.push(self.dot(out, &row, &col, Kind::Float));
                    }
                }
                regs
            }
            (BinaryOperator::Multiply, _, _) => {
                self.zip(out, &left.regs, &right.regs, |c, out, x, y| {
                    c.f2(out, F2::Mul, x, y)
                })
            }
            (op, l, r) => {
                return Err(LaneError::unsupported(format!(
                    "the matrix operation {op:?} on {l:?} and {r:?}"
                )));
            }
        };
        Ok(Val { ty, regs })
    }

    #[expect(clippy::too_many_lines, reason = "one arm per math function")]
    #[expect(
        clippy::many_single_char_names,
        reason = "a, b, c and x, y, z are the operand names of the WGSL builtins"
    )]
    fn math(
        &mut self,
        out: &mut Vec<Node>,
        fun: MathFunction,
        a: &Val,
        b: Option<&Val>,
        c: Option<&Val>,
        ty: Ty,
    ) -> Result<Val, LaneError> {
        use MathFunction as M;
        let kind = a.ty.scalar()?;
        let need = |v: Option<&Val>| {
            v.cloned()
                .ok_or_else(|| LaneError::unsupported(format!("{fun:?} without its operands")))
        };
        let float1 = |c: &mut Self, out: &mut Vec<Node>, f: F1| -> Vec<Reg> {
            a.regs.iter().map(|&x| c.f1(out, f, x)).collect()
        };
        let regs = match fun {
            M::Abs => match kind {
                Kind::Float => float1(self, out, F1::Abs),
                Kind::Sint => a.regs.iter().map(|&x| self.i1(out, I1::Abs, x)).collect(),
                _ => a.regs.clone(),
            },
            M::Min | M::Max => {
                let b = need(b)?;
                let f = match (fun, kind) {
                    (M::Min, Kind::Float) => Op::F2 {
                        f: F2::Min,
                        dst: 0,
                        a: 0,
                        b: 0,
                    },
                    (M::Max, Kind::Float) => Op::F2 {
                        f: F2::Max,
                        dst: 0,
                        a: 0,
                        b: 0,
                    },
                    (M::Min, Kind::Sint) => Op::I2 {
                        f: I2::MinS,
                        dst: 0,
                        a: 0,
                        b: 0,
                    },
                    (M::Max, Kind::Sint) => Op::I2 {
                        f: I2::MaxS,
                        dst: 0,
                        a: 0,
                        b: 0,
                    },
                    (M::Min, _) => Op::I2 {
                        f: I2::MinU,
                        dst: 0,
                        a: 0,
                        b: 0,
                    },
                    _ => Op::I2 {
                        f: I2::MaxU,
                        dst: 0,
                        a: 0,
                        b: 0,
                    },
                };
                self.zip(out, &a.regs, &b.regs, |c, out, x, y| match f {
                    Op::F2 { f, .. } => c.f2(out, f, x, y),
                    Op::I2 { f, .. } => c.i2(out, f, x, y),
                    _ => unreachable!("min and max are two-operand ops"),
                })
            }
            M::Clamp => {
                let (lo, hi) = (need(b)?, need(c)?);
                if kind == Kind::Float {
                    (0..a.regs.len())
                        .map(|i| {
                            let at = |v: &Val| v.regs[if v.regs.len() == 1 { 0 } else { i }];
                            self.f3(out, F3::Clamp, a.regs[i], at(&lo), at(&hi))
                        })
                        .collect()
                } else {
                    let (max, min) = if kind == Kind::Sint {
                        (I2::MaxS, I2::MinS)
                    } else {
                        (I2::MaxU, I2::MinU)
                    };
                    let raised =
                        self.zip(out, &a.regs, &lo.regs, |c, out, x, y| c.i2(out, max, x, y));
                    self.zip(out, &raised, &hi.regs, |c, out, x, y| c.i2(out, min, x, y))
                }
            }
            M::Saturate => float1(self, out, F1::Saturate),
            M::Cos => float1(self, out, F1::Cos),
            M::Cosh => float1(self, out, F1::Cosh),
            M::Sin => float1(self, out, F1::Sin),
            M::Sinh => float1(self, out, F1::Sinh),
            M::Tan => float1(self, out, F1::Tan),
            M::Tanh => float1(self, out, F1::Tanh),
            M::Acos => float1(self, out, F1::Acos),
            M::Asin => float1(self, out, F1::Asin),
            M::Atan => float1(self, out, F1::Atan),
            M::Asinh => float1(self, out, F1::Asinh),
            M::Acosh => float1(self, out, F1::Acosh),
            M::Atanh => float1(self, out, F1::Atanh),
            M::Radians => float1(self, out, F1::Radians),
            M::Degrees => float1(self, out, F1::Degrees),
            M::Ceil => float1(self, out, F1::Ceil),
            M::Floor => float1(self, out, F1::Floor),
            M::Round => float1(self, out, F1::Round),
            M::Fract => float1(self, out, F1::Fract),
            M::Trunc => float1(self, out, F1::Trunc),
            M::Exp => float1(self, out, F1::Exp),
            M::Exp2 => float1(self, out, F1::Exp2),
            M::Log => float1(self, out, F1::Log),
            M::Log2 => float1(self, out, F1::Log2),
            M::Sqrt => float1(self, out, F1::Sqrt),
            M::InverseSqrt => float1(self, out, F1::InverseSqrt),
            M::QuantizeToF16 => float1(self, out, F1::QuantizeF16),
            M::Atan2 | M::Pow | M::Step | M::Ldexp => {
                let b = need(b)?;
                let f = match fun {
                    M::Atan2 => F2::Atan2,
                    M::Pow => F2::Pow,
                    M::Step => F2::Step,
                    _ => F2::Ldexp,
                };
                self.zip(out, &a.regs, &b.regs, |c, out, x, y| c.f2(out, f, x, y))
            }
            M::Dot => {
                let b = need(b)?;
                vec![self.dot(out, &a.regs, &b.regs, kind)]
            }
            M::Cross => {
                let b = need(b)?;
                let (x, y) = (&a.regs, &b.regs);
                (0..3)
                    .map(|i| {
                        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
                        let p = self.f2(out, F2::Mul, x[j], y[k]);
                        let q = self.f2(out, F2::Mul, x[k], y[j]);
                        self.f2(out, F2::Sub, p, q)
                    })
                    .collect()
            }
            M::Length => vec![self.length(out, &a.regs)],
            M::Distance => {
                let b = need(b)?;
                let d = self.zip(out, &a.regs, &b.regs, |c, out, x, y| {
                    c.f2(out, F2::Sub, x, y)
                });
                vec![self.length(out, &d)]
            }
            M::Normalize => {
                let len = self.length(out, &a.regs);
                a.regs
                    .iter()
                    .map(|&x| self.f2(out, F2::Div, x, len))
                    .collect()
            }
            M::FaceForward => {
                let (i, nref) = (need(b)?, need(c)?);
                let d = self.dot(out, &nref.regs, &i.regs, Kind::Float);
                let zero = self.konst(0);
                let negative = self.cmp(out, Cmp::FLt, d, zero);
                a.regs
                    .iter()
                    .map(|&n| {
                        let flipped = self.f1(out, F1::Neg, n);
                        self.select(out, negative, n, flipped)
                    })
                    .collect()
            }
            M::Reflect => {
                let n = need(b)?;
                let d = self.dot(out, &n.regs, &a.regs, Kind::Float);
                let two = self.konst(2.0f32.to_bits());
                let k = self.f2(out, F2::Mul, d, two);
                (0..a.regs.len())
                    .map(|i| {
                        let p = self.f2(out, F2::Mul, k, n.regs[i]);
                        self.f2(out, F2::Sub, a.regs[i], p)
                    })
                    .collect()
            }
            M::Refract => {
                let (n, eta) = (need(b)?, need(c)?);
                let eta = eta.regs[0];
                let d = self.dot(out, &n.regs, &a.regs, Kind::Float);
                let one = self.konst(1.0f32.to_bits());
                let zero = self.konst(0);
                let dd = self.f2(out, F2::Mul, d, d);
                let rest = self.f2(out, F2::Sub, one, dd);
                let ee = self.f2(out, F2::Mul, eta, eta);
                let scaled = self.f2(out, F2::Mul, ee, rest);
                let k = self.f2(out, F2::Sub, one, scaled);
                let negative = self.cmp(out, Cmp::FLt, k, zero);
                let root = self.f1(out, F1::Sqrt, k);
                let ed = self.f2(out, F2::Mul, eta, d);
                let m = self.f2(out, F2::Add, ed, root);
                (0..a.regs.len())
                    .map(|i| {
                        let p = self.f2(out, F2::Mul, eta, a.regs[i]);
                        let q = self.f2(out, F2::Mul, m, n.regs[i]);
                        let r = self.f2(out, F2::Sub, p, q);
                        self.select(out, negative, zero, r)
                    })
                    .collect()
            }
            M::Sign => match kind {
                Kind::Float => float1(self, out, F1::Sign),
                _ => a.regs.iter().map(|&x| self.i1(out, I1::SignS, x)).collect(),
            },
            M::Fma | M::Mix | M::SmoothStep => {
                let (b, c) = (need(b)?, need(c)?);
                let f = match fun {
                    M::Fma => F3::Fma,
                    M::Mix => F3::Mix,
                    _ => F3::SmoothStep,
                };
                let n = a.regs.len().max(b.regs.len()).max(c.regs.len());
                (0..n)
                    .map(|i| {
                        let at = |v: &Val| v.regs[if v.regs.len() == 1 { 0 } else { i }];
                        self.f3(out, f, at(a), at(&b), at(&c))
                    })
                    .collect()
            }
            M::Transpose => {
                let Ty::Matrix { columns, rows } = a.ty else {
                    return Err(LaneError::unsupported("transpose of a non-matrix"));
                };
                (0..rows)
                    .flat_map(|i| (0..columns).map(move |j| (i, j)))
                    .map(|(i, j)| a.regs[(j * rows + i) as usize])
                    .collect()
            }
            M::Determinant => {
                let m = |c: u32, r: u32, n: u32| a.regs[(c * n + r) as usize];
                match a.ty {
                    Ty::Matrix {
                        columns: 2,
                        rows: 2,
                    } => {
                        let p = self.f2(out, F2::Mul, m(0, 0, 2), m(1, 1, 2));
                        let q = self.f2(out, F2::Mul, m(1, 0, 2), m(0, 1, 2));
                        vec![self.f2(out, F2::Sub, p, q)]
                    }
                    Ty::Matrix {
                        columns: 3,
                        rows: 3,
                    } => {
                        // Expansion along the first column.
                        let minor = |c: &mut Self, out: &mut Vec<Node>, r0: u32, r1: u32| {
                            let p = c.f2(out, F2::Mul, m(1, r0, 3), m(2, r1, 3));
                            let q = c.f2(out, F2::Mul, m(2, r0, 3), m(1, r1, 3));
                            c.f2(out, F2::Sub, p, q)
                        };
                        let m0 = minor(self, out, 1, 2);
                        let m1 = minor(self, out, 0, 2);
                        let m2 = minor(self, out, 0, 1);
                        let t0 = self.f2(out, F2::Mul, m(0, 0, 3), m0);
                        let t1 = self.f2(out, F2::Mul, m(0, 1, 3), m1);
                        let t2 = self.f2(out, F2::Mul, m(0, 2, 3), m2);
                        let s = self.f2(out, F2::Sub, t0, t1);
                        vec![self.f2(out, F2::Add, s, t2)]
                    }
                    ref other => {
                        return Err(LaneError::unsupported(format!(
                            "the determinant of {other:?}"
                        )));
                    }
                }
            }
            M::CountOneBits => a
                .regs
                .iter()
                .map(|&x| self.i1(out, I1::CountOnes, x))
                .collect(),
            M::ReverseBits => a
                .regs
                .iter()
                .map(|&x| self.i1(out, I1::ReverseBits, x))
                .collect(),
            M::CountLeadingZeros => a
                .regs
                .iter()
                .map(|&x| self.i1(out, I1::LeadingZeros, x))
                .collect(),
            M::CountTrailingZeros => a
                .regs
                .iter()
                .map(|&x| self.i1(out, I1::TrailingZeros, x))
                .collect(),
            M::FirstLeadingBit => {
                let f = if kind == Kind::Sint {
                    I1::FirstLeadingS
                } else {
                    I1::FirstLeadingU
                };
                a.regs.iter().map(|&x| self.i1(out, f, x)).collect()
            }
            M::FirstTrailingBit => a
                .regs
                .iter()
                .map(|&x| self.i1(out, I1::FirstTrailing, x))
                .collect(),
            other => {
                return Err(LaneError::unsupported(format!(
                    "the math function {other:?}"
                )));
            }
        };
        Ok(Val { ty, regs })
    }

    fn length(&mut self, out: &mut Vec<Node>, v: &[Reg]) -> Reg {
        if v.len() == 1 {
            return self.f1(out, F1::Abs, v[0]);
        }
        let d = self.dot(out, v, v, Kind::Float);
        self.f1(out, F1::Sqrt, d)
    }
}
