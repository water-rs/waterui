//! The lane runtime: straight-line ops over whole registers, structured
//! control flow over per-lane masks.
//!
//! Every op computes all [`LANES`] lanes whatever the mask: expressions are
//! side-effect free, and a lane a branch left inactive never reads what an
//! op computed for it. Only state that outlives an expression — locals,
//! private globals, results — is written under the mask.

use super::{Host, LANES, Lanes, Mask, Reg};

/// A one-operand float function.
#[derive(Clone, Copy, Debug)]
pub enum F1 {
    Neg,
    Abs,
    Floor,
    Ceil,
    Round,
    Trunc,
    Fract,
    Sqrt,
    InverseSqrt,
    Exp,
    Exp2,
    Log,
    Log2,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    Sign,
    Saturate,
    Radians,
    Degrees,
    QuantizeF16,
}

/// A two-operand float function.
#[derive(Clone, Copy, Debug)]
pub enum F2 {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Min,
    Max,
    Pow,
    Atan2,
    /// `step(edge, x)`.
    Step,
    /// `ldexp(x, i32)`: the right operand holds `i32` bits.
    Ldexp,
}

/// A three-operand float function.
#[derive(Clone, Copy, Debug)]
pub enum F3 {
    Fma,
    Clamp,
    Mix,
    SmoothStep,
}

/// A one-operand integer function on 32-bit patterns.
#[derive(Clone, Copy, Debug)]
pub enum I1 {
    Neg,
    Not,
    /// `bool` negation of a `0`/`1` slot.
    BoolNot,
    Abs,
    CountOnes,
    ReverseBits,
    LeadingZeros,
    TrailingZeros,
    FirstLeadingU,
    FirstLeadingS,
    FirstTrailing,
    SignS,
}

/// A two-operand integer function on 32-bit patterns.
#[derive(Clone, Copy, Debug)]
pub enum I2 {
    Add,
    Sub,
    Mul,
    DivS,
    DivU,
    RemS,
    RemU,
    And,
    Or,
    Xor,
    Shl,
    ShrS,
    ShrU,
    MinS,
    MaxS,
    MinU,
    MaxU,
}

/// A comparison producing a `bool` slot.
#[derive(Clone, Copy, Debug)]
pub enum Cmp {
    FEq,
    FNe,
    FLt,
    FLe,
    FGt,
    FGe,
    Eq,
    Ne,
    SLt,
    SLe,
    SGt,
    SGe,
    ULt,
    ULe,
    UGt,
    UGe,
}

/// A numeric conversion.
#[derive(Clone, Copy, Debug)]
pub enum Cast {
    FloatToSint,
    FloatToUint,
    SintToFloat,
    UintToFloat,
    BoolToFloat,
    FloatToBool,
    BitsToBool,
}

/// A screen-space derivative over a 2×2 quad of lanes.
#[derive(Clone, Copy, Debug)]
pub enum Deriv {
    X,
    Y,
    Width,
    XCoarse,
    YCoarse,
    WidthCoarse,
}

/// One straight-line op.
#[derive(Clone, Debug)]
pub enum Op {
    Copy {
        dst: Reg,
        src: Reg,
    },
    Splat {
        dst: Reg,
        bits: u32,
    },
    F1 {
        f: F1,
        dst: Reg,
        a: Reg,
    },
    F2 {
        f: F2,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    F3 {
        f: F3,
        dst: Reg,
        a: Reg,
        b: Reg,
        c: Reg,
    },
    I1 {
        f: I1,
        dst: Reg,
        a: Reg,
    },
    I2 {
        f: I2,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Cmp {
        f: Cmp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Cast {
        f: Cast,
        dst: Reg,
        a: Reg,
    },
    Select {
        dst: Reg,
        cond: Reg,
        accept: Reg,
        reject: Reg,
    },
    Deriv {
        f: Deriv,
        dst: Reg,
        a: Reg,
    },
    /// `dst = acc + clamp(index, 0, count - 1) * mult`, as `u32`.
    Index {
        dst: Reg,
        acc: Option<Reg>,
        index: Reg,
        signed: bool,
        count: u32,
        mult: u32,
    },
    /// `dst[s] = table[sel * width + s]`, per lane.
    Gather {
        dst: Box<[Reg]>,
        sel: Reg,
        table: Box<[Reg]>,
    },
    /// A host function: `args` are copied out, `outs` written back.
    Host {
        function: u32,
        args: Box<[Reg]>,
        outs: Box<[Reg]>,
    },
}

/// A switch case.
#[derive(Clone, Debug)]
pub struct Case {
    /// The selector values (`u32` bits) that enter here; empty for
    /// `default`.
    pub values: Vec<u32>,
    pub default: bool,
    pub body: Vec<Node>,
    pub fall_through: bool,
}

/// A structured node.
#[derive(Clone, Debug)]
pub enum Node {
    Op(Op),
    /// Masked copies `dst[i] = src[i]`.
    Store {
        dst: Box<[Reg]>,
        src: Box<[Reg]>,
    },
    /// Masked `table[sel * width + s] = src[s]`.
    Scatter {
        sel: Reg,
        table: Box<[Reg]>,
        src: Box<[Reg]>,
    },
    If {
        cond: Reg,
        accept: Vec<Self>,
        reject: Vec<Self>,
    },
    Loop {
        body: Vec<Self>,
        continuing: Vec<Self>,
        break_if: Option<Reg>,
    },
    Switch {
        selector: Reg,
        cases: Vec<Case>,
    },
    Break,
    Continue,
    /// Masked result copies, then the lanes leave the function.
    Return {
        dst: Box<[Reg]>,
        src: Box<[Reg]>,
    },
    Kill,
    /// Runs function `0`'s body; the lanes that entered continue after it.
    Call(u32),
}

/// A break target: a loop collects its continued lanes too.
#[derive(Clone, Copy)]
struct Target {
    is_loop: bool,
    broken: Mask,
    continued: Mask,
}

/// The register file and scratch a run needs; reused across runs.
pub struct State {
    pub(super) regs: Vec<Lanes>,
    targets: Vec<Target>,
    args: Vec<Lanes>,
    outs: Vec<Lanes>,
    pub(super) killed: Mask,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registers")
            .field("registers", &self.regs.len())
            .finish_non_exhaustive()
    }
}

impl State {
    pub(super) fn new(count: usize) -> Self {
        Self {
            regs: vec![[0.0; LANES]; count],
            targets: Vec::new(),
            args: Vec::new(),
            outs: Vec::new(),
            killed: [false; LANES],
        }
    }
}

const fn none(mask: &Mask) -> bool {
    let mut i = 0;
    while i < LANES {
        if mask[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn and(a: &Mask, b: &Mask) -> Mask {
    std::array::from_fn(|l| a[l] && b[l])
}

fn and_not(a: &Mask, b: &Mask) -> Mask {
    std::array::from_fn(|l| a[l] && !b[l])
}

fn or(a: &Mask, b: &Mask) -> Mask {
    std::array::from_fn(|l| a[l] || b[l])
}

fn truth(lanes: &Lanes) -> Mask {
    std::array::from_fn(|l| lanes[l].to_bits() != 0)
}

const fn from_bool(v: bool) -> f32 {
    f32::from_bits(v as u32)
}

const fn u(v: f32) -> u32 {
    v.to_bits()
}

#[expect(clippy::cast_possible_wrap, reason = "WGSL reinterprets the bits")]
const fn s(v: f32) -> i32 {
    v.to_bits() as i32
}

const fn fu(v: u32) -> f32 {
    f32::from_bits(v)
}

#[expect(clippy::cast_sign_loss, reason = "WGSL reinterprets the bits")]
const fn fs(v: i32) -> f32 {
    f32::from_bits(v as u32)
}

/// WGSL `round`: ties to even.
const fn round_even(x: f32) -> f32 {
    x.round_ties_even()
}

/// WGSL `sign`: zero at zero, unlike `f32::signum`.
fn sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

fn f1(f: F1, x: f32) -> f32 {
    match f {
        F1::Neg => -x,
        F1::Abs => x.abs(),
        F1::Floor => x.floor(),
        F1::Ceil => x.ceil(),
        F1::Round => round_even(x),
        F1::Trunc => x.trunc(),
        F1::Fract => x - x.floor(),
        F1::Sqrt => x.sqrt(),
        F1::InverseSqrt => x.sqrt().recip(),
        F1::Exp => x.exp(),
        F1::Exp2 => x.exp2(),
        F1::Log => x.ln(),
        F1::Log2 => x.log2(),
        F1::Sin => x.sin(),
        F1::Cos => x.cos(),
        F1::Tan => x.tan(),
        F1::Asin => x.asin(),
        F1::Acos => x.acos(),
        F1::Atan => x.atan(),
        F1::Sinh => x.sinh(),
        F1::Cosh => x.cosh(),
        F1::Tanh => x.tanh(),
        F1::Asinh => x.asinh(),
        F1::Acosh => x.acosh(),
        F1::Atanh => x.atanh(),
        F1::Sign => sign(x),
        F1::Saturate => x.clamp(0.0, 1.0),
        F1::Radians => x.to_radians(),
        F1::Degrees => x.to_degrees(),
        F1::QuantizeF16 => half::f16::from_f32(x).to_f32(),
    }
}

fn f2(f: F2, x: f32, y: f32) -> f32 {
    match f {
        F2::Add => x + y,
        F2::Sub => x - y,
        F2::Mul => x * y,
        F2::Div => x / y,
        // WGSL `%` on floats truncates: `x - y * trunc(x / y)`.
        F2::Rem => y.mul_add(-(x / y).trunc(), x),
        F2::Min => x.min(y),
        F2::Max => x.max(y),
        F2::Pow => x.powf(y),
        F2::Atan2 => x.atan2(y),
        F2::Step => {
            if y >= x {
                1.0
            } else {
                0.0
            }
        }
        F2::Ldexp => x * s(y).clamp(-300, 300).cast_exp2(),
    }
}

/// `2^e` for an exponent the caller clamped.
trait CastExp2 {
    fn cast_exp2(self) -> f32;
}

impl CastExp2 for i32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "the exponent is clamped to ±300"
    )]
    fn cast_exp2(self) -> f32 {
        (self as f32).exp2()
    }
}

#[expect(
    clippy::many_single_char_names,
    reason = "x, y and z are the operand names of the WGSL builtins"
)]
fn f3(f: F3, x: f32, y: f32, z: f32) -> f32 {
    match f {
        F3::Fma => x.mul_add(y, z),
        F3::Clamp => x.max(y).min(z),
        // WGSL defines `mix` as `x * (1 - z) + y * z`.
        F3::Mix => y.mul_add(z, x * (1.0 - z)),
        F3::SmoothStep => {
            let t = ((z - x) / (y - x)).clamp(0.0, 1.0);
            t * t * 2.0f32.mul_add(-t, 3.0)
        }
    }
}

#[expect(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "WGSL bit patterns"
)]
const fn i1(f: I1, x: u32) -> u32 {
    let xs = x as i32;
    match f {
        I1::Neg => xs.wrapping_neg() as u32,
        I1::Not => !x,
        I1::BoolNot => x ^ 1,
        I1::Abs => xs.wrapping_abs() as u32,
        I1::CountOnes => x.count_ones(),
        I1::ReverseBits => x.reverse_bits(),
        I1::LeadingZeros => x.leading_zeros(),
        I1::TrailingZeros => x.trailing_zeros(),
        I1::FirstLeadingU => {
            if x == 0 {
                u32::MAX
            } else {
                x.ilog2()
            }
        }
        I1::FirstLeadingS => {
            let y = if xs < 0 { !x } else { x };
            if y == 0 { u32::MAX } else { y.ilog2() }
        }
        I1::FirstTrailing => {
            if x == 0 {
                u32::MAX
            } else {
                x.trailing_zeros()
            }
        }
        I1::SignS => xs.signum() as u32,
    }
}

#[expect(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "WGSL bit patterns"
)]
fn i2(f: I2, x: u32, y: u32) -> u32 {
    let (xs, ys) = (x as i32, y as i32);
    match f {
        I2::Add => x.wrapping_add(y),
        I2::Sub => x.wrapping_sub(y),
        I2::Mul => x.wrapping_mul(y),
        // WGSL: a zero divisor, or `MIN / -1`, yields the dividend.
        I2::DivS => {
            if ys == 0 || (xs == i32::MIN && ys == -1) {
                x
            } else {
                (xs / ys) as u32
            }
        }
        I2::DivU => x.checked_div(y).unwrap_or(x),
        // WGSL: those divisors yield a zero remainder.
        I2::RemS => {
            if ys == 0 || (xs == i32::MIN && ys == -1) {
                0
            } else {
                (xs % ys) as u32
            }
        }
        I2::RemU => x.checked_rem(y).unwrap_or(0),
        I2::And => x & y,
        I2::Or => x | y,
        I2::Xor => x ^ y,
        // WGSL masks the shift amount to the bit width.
        I2::Shl => x << (y & 31),
        I2::ShrS => (xs >> (y & 31)) as u32,
        I2::ShrU => x >> (y & 31),
        I2::MinS => xs.min(ys) as u32,
        I2::MaxS => xs.max(ys) as u32,
        I2::MinU => x.min(y),
        I2::MaxU => x.max(y),
    }
}

#[expect(clippy::float_cmp, reason = "WGSL comparisons are exact")]
fn cmp(f: Cmp, x: f32, y: f32) -> bool {
    match f {
        Cmp::FEq => x == y,
        Cmp::FNe => x != y,
        Cmp::FLt => x < y,
        Cmp::FLe => x <= y,
        Cmp::FGt => x > y,
        Cmp::FGe => x >= y,
        Cmp::Eq => u(x) == u(y),
        Cmp::Ne => u(x) != u(y),
        Cmp::SLt => s(x) < s(y),
        Cmp::SLe => s(x) <= s(y),
        Cmp::SGt => s(x) > s(y),
        Cmp::SGe => s(x) >= s(y),
        Cmp::ULt => u(x) < u(y),
        Cmp::ULe => u(x) <= u(y),
        Cmp::UGt => u(x) > u(y),
        Cmp::UGe => u(x) >= u(y),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "WGSL conversions saturate, as Rust's `as` does"
)]
fn cast(f: Cast, x: f32) -> f32 {
    match f {
        Cast::FloatToSint => fs(x as i32),
        Cast::FloatToUint => fu(x as u32),
        Cast::SintToFloat => s(x) as f32,
        Cast::UintToFloat => u(x) as f32,
        Cast::BoolToFloat => {
            if u(x) == 0 {
                0.0
            } else {
                1.0
            }
        }
        Cast::FloatToBool => from_bool(x != 0.0),
        Cast::BitsToBool => from_bool(u(x) != 0),
    }
}

/// The derivative of `v` at lane `l`: lanes are 2×2 quads, lane bit 0 the
/// column and bit 1 the row.
fn deriv(f: Deriv, v: &Lanes, l: usize) -> f32 {
    let dx = |l: usize| v[l | 1] - v[l & !1];
    let dy = |l: usize| v[l | 2] - v[l & !2];
    let quad = l & !3;
    match f {
        Deriv::X => dx(l),
        Deriv::Y => dy(l),
        Deriv::Width => dx(l).abs() + dy(l).abs(),
        Deriv::XCoarse => dx(quad),
        Deriv::YCoarse => dy(quad),
        Deriv::WidthCoarse => dx(quad).abs() + dy(quad).abs(),
    }
}

impl Op {
    #[expect(clippy::too_many_lines, reason = "one arm per op")]
    #[expect(
        clippy::many_single_char_names,
        reason = "x, y, z and d are the operand and destination lanes of the WGSL builtins"
    )]
    fn run(&self, st: &mut State, mask: &Mask, host: &mut impl Host) {
        let regs = &mut st.regs;
        macro_rules! map1 {
            ($dst:expr, $a:expr, |$x:ident| $body:expr) => {{
                let a = regs[$a as usize];
                let d = &mut regs[$dst as usize];
                for l in 0..LANES {
                    let $x = a[l];
                    d[l] = $body;
                }
            }};
        }
        macro_rules! map2 {
            ($dst:expr, $a:expr, $b:expr, |$x:ident, $y:ident| $body:expr) => {{
                let a = regs[$a as usize];
                let bb = regs[$b as usize];
                let d = &mut regs[$dst as usize];
                for l in 0..LANES {
                    let ($x, $y) = (a[l], bb[l]);
                    d[l] = $body;
                }
            }};
        }
        match *self {
            Self::Copy { dst, src } => regs[dst as usize] = regs[src as usize],
            Self::Splat { dst, bits } => regs[dst as usize] = [fu(bits); LANES],
            Self::F1 { f, dst, a } => match f {
                // The hot elementwise forms get their own loops so they
                // vectorize; the rest call through `f1`.
                F1::Neg => map1!(dst, a, |x| -x),
                F1::Abs => map1!(dst, a, |x| x.abs()),
                F1::Floor => map1!(dst, a, |x| x.floor()),
                F1::Fract => map1!(dst, a, |x| x - x.floor()),
                F1::Sqrt => map1!(dst, a, |x| x.sqrt()),
                F1::Saturate => map1!(dst, a, |x| x.clamp(0.0, 1.0)),
                f => map1!(dst, a, |x| f1(f, x)),
            },
            Self::F2 { f, dst, a, b } => match f {
                F2::Add => map2!(dst, a, b, |x, y| x + y),
                F2::Sub => map2!(dst, a, b, |x, y| x - y),
                F2::Mul => map2!(dst, a, b, |x, y| x * y),
                F2::Div => map2!(dst, a, b, |x, y| x / y),
                F2::Min => map2!(dst, a, b, |x, y| x.min(y)),
                F2::Max => map2!(dst, a, b, |x, y| x.max(y)),
                f => map2!(dst, a, b, |x, y| f2(f, x, y)),
            },
            Self::F3 { f, dst, a, b, c } => {
                let (x, y, z) = (regs[a as usize], regs[b as usize], regs[c as usize]);
                let d = &mut regs[dst as usize];
                match f {
                    F3::Mix => {
                        for l in 0..LANES {
                            d[l] = f32::mul_add(y[l], z[l], x[l] * (1.0 - z[l]));
                        }
                    }
                    F3::Clamp => {
                        for l in 0..LANES {
                            d[l] = x[l].max(y[l]).min(z[l]);
                        }
                    }
                    f => {
                        for l in 0..LANES {
                            d[l] = f3(f, x[l], y[l], z[l]);
                        }
                    }
                }
            }
            Self::I1 { f, dst, a } => map1!(dst, a, |x| fu(i1(f, u(x)))),
            Self::I2 { f, dst, a, b } => map2!(dst, a, b, |x, y| fu(i2(f, u(x), u(y)))),
            Self::Cmp { f, dst, a, b } => map2!(dst, a, b, |x, y| from_bool(cmp(f, x, y))),
            Self::Cast { f, dst, a } => map1!(dst, a, |x| cast(f, x)),
            Self::Select {
                dst,
                cond,
                accept,
                reject,
            } => {
                let (c, t, e) = (
                    regs[cond as usize],
                    regs[accept as usize],
                    regs[reject as usize],
                );
                let d = &mut regs[dst as usize];
                for l in 0..LANES {
                    d[l] = if u(c[l]) == 0 { e[l] } else { t[l] };
                }
            }
            Self::Deriv { f, dst, a } => {
                let v = regs[a as usize];
                let d = &mut regs[dst as usize];
                for (l, out) in d.iter_mut().enumerate() {
                    *out = deriv(f, &v, l);
                }
            }
            Self::Index {
                dst,
                acc,
                index,
                signed,
                count,
                mult,
            } => {
                let i = regs[index as usize];
                let base = acc.map_or([0.0; LANES], |acc| regs[acc as usize]);
                let d = &mut regs[dst as usize];
                for l in 0..LANES {
                    let raw = u(i[l]);
                    #[expect(clippy::cast_sign_loss, reason = "clamped at zero first")]
                    let clamped = if signed { s(i[l]).max(0) as u32 } else { raw }.min(count - 1);
                    d[l] = fu(u(base[l]).wrapping_add(clamped * mult));
                }
            }
            Self::Gather {
                ref dst,
                sel,
                ref table,
            } => {
                let sel = regs[sel as usize];
                let width = dst.len();
                for l in 0..LANES {
                    let row = u(sel[l]) as usize * width;
                    for (slot, &out) in dst.iter().enumerate() {
                        regs[out as usize][l] = regs[table[row + slot] as usize][l];
                    }
                }
            }
            Self::Host {
                function,
                ref args,
                ref outs,
            } => {
                st.args.clear();
                st.args
                    .extend(args.iter().map(|&reg| st.regs[reg as usize]));
                st.outs.clear();
                st.outs.resize(outs.len(), [0.0; LANES]);
                host.call(function as usize, &st.args, &mut st.outs, mask);
                for (&reg, value) in outs.iter().zip(&st.outs) {
                    st.regs[reg as usize] = *value;
                }
            }
        }
    }
}

/// A compiled program's functions; `functions[0]` is the entry.
pub struct Code<'p> {
    pub functions: &'p [Vec<Node>],
}

impl Code<'_> {
    /// Runs `nodes` for the lanes of `mask`; returns the lanes that fall
    /// out of the end (the rest broke, continued, returned or were killed).
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per structured control-flow node"
    )]
    pub fn block(
        &self,
        nodes: &[Node],
        mut mask: Mask,
        st: &mut State,
        host: &mut impl Host,
    ) -> Mask {
        for node in nodes {
            if none(&mask) {
                break;
            }
            match node {
                Node::Op(op) => op.run(st, &mask, host),
                Node::Store { dst, src } => {
                    for (&d, &s) in dst.iter().zip(src.iter()) {
                        let value = st.regs[s as usize];
                        let out = &mut st.regs[d as usize];
                        for l in 0..LANES {
                            if mask[l] {
                                out[l] = value[l];
                            }
                        }
                    }
                }
                Node::Scatter { sel, table, src } => {
                    let sel = st.regs[*sel as usize];
                    let width = src.len();
                    for l in 0..LANES {
                        if !mask[l] {
                            continue;
                        }
                        let row = u(sel[l]) as usize * width;
                        for (slot, &from) in src.iter().enumerate() {
                            let value = st.regs[from as usize][l];
                            st.regs[table[row + slot] as usize][l] = value;
                        }
                    }
                }
                Node::If {
                    cond,
                    accept,
                    reject,
                } => {
                    let c = truth(&st.regs[*cond as usize]);
                    let yes = self.block(accept, and(&mask, &c), st, host);
                    let no = self.block(reject, and_not(&mask, &c), st, host);
                    mask = or(&yes, &no);
                }
                Node::Loop {
                    body,
                    continuing,
                    break_if,
                } => {
                    st.targets.push(Target {
                        is_loop: true,
                        broken: [false; LANES],
                        continued: [false; LANES],
                    });
                    let mut live = mask;
                    while !none(&live) {
                        let fell = self.block(body, live, st, host);
                        let target = st.targets.last_mut().expect("the loop's target");
                        let next = or(&fell, &target.continued);
                        target.continued = [false; LANES];
                        live = self.block(continuing, next, st, host);
                        if let Some(cond) = break_if {
                            let c = truth(&st.regs[*cond as usize]);
                            let target = st.targets.last_mut().expect("the loop's target");
                            target.broken = or(&target.broken, &and(&live, &c));
                            live = and_not(&live, &c);
                        }
                    }
                    mask = st.targets.pop().expect("the loop's target").broken;
                }
                Node::Switch { selector, cases } => {
                    mask = self.switch(*selector, cases, mask, st, host);
                }
                Node::Break => {
                    let target = st.targets.last_mut().expect("a break target");
                    target.broken = or(&target.broken, &mask);
                    mask = [false; LANES];
                }
                Node::Continue => {
                    let target = st
                        .targets
                        .iter_mut()
                        .rev()
                        .find(|target| target.is_loop)
                        .expect("a loop to continue");
                    target.continued = or(&target.continued, &mask);
                    mask = [false; LANES];
                }
                Node::Return { dst, src } => {
                    for (&d, &s) in dst.iter().zip(src.iter()) {
                        let value = st.regs[s as usize];
                        let out = &mut st.regs[d as usize];
                        for l in 0..LANES {
                            if mask[l] {
                                out[l] = value[l];
                            }
                        }
                    }
                    mask = [false; LANES];
                }
                Node::Kill => {
                    st.killed = or(&st.killed, &mask);
                    mask = [false; LANES];
                }
                Node::Call(function) => {
                    // Break targets never cross a call.
                    let depth = st.targets.len();
                    self.block(&self.functions[*function as usize], mask, st, host);
                    debug_assert_eq!(st.targets.len(), depth);
                    // Killed lanes stop for good.
                    mask = and_not(&mask, &st.killed);
                }
            }
        }
        mask
    }

    fn switch(
        &self,
        selector: Reg,
        cases: &[Case],
        mask: Mask,
        st: &mut State,
        host: &mut impl Host,
    ) -> Mask {
        let sel = st.regs[selector as usize];
        // Each lane enters the first case listing its value, else default.
        let mut taken = [false; LANES];
        let mut entry: Vec<Mask> = cases
            .iter()
            .map(|case| {
                let enters: Mask = std::array::from_fn(|l| {
                    mask[l] && !taken[l] && case.values.contains(&u(sel[l]))
                });
                taken = or(&taken, &enters);
                enters
            })
            .collect();
        let rest = and_not(&mask, &taken);
        if let Some(default) = cases.iter().position(|case| case.default) {
            entry[default] = or(&entry[default], &rest);
        }
        st.targets.push(Target {
            is_loop: false,
            broken: [false; LANES],
            continued: [false; LANES],
        });
        let mut carry = [false; LANES];
        let mut done = [false; LANES];
        for (case, enters) in cases.iter().zip(&entry) {
            let fell = self.block(&case.body, or(&carry, enters), st, host);
            if case.fall_through {
                carry = fell;
            } else {
                done = or(&done, &fell);
                carry = [false; LANES];
            }
        }
        let target = st.targets.pop().expect("the switch's target");
        or(&or(&done, &carry), &target.broken)
    }
}
