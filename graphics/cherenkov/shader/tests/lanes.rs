//! The lane executor against per-lane references and the scalar evaluator.

#![cfg(feature = "lanes")]

use cherenkov_shader::lanes::{Host, ImageOp, Interface, LANES, LaneError, Lanes, Mask, Program};

/// No host functions.
struct None_;

impl Host for None_ {
    fn call(&mut self, function: usize, _: &[Lanes], _: &mut [Lanes], _: &Mask) {
        panic!("unexpected host call {function}");
    }

    fn image(&mut self, op: ImageOp<'_>, _: &mut [Lanes; 4], _: &Mask) {
        panic!("unexpected image op {op:?}");
    }
}

/// Compiles `source`'s `entry`, binds scalar float arguments per lane, runs
/// every lane and returns the flattened result per lane.
fn run(source: &str, entry: &str, args: &[Lanes]) -> Vec<Lanes> {
    run_masked(source, entry, args, [true; LANES]).0
}

fn run_masked(source: &str, entry: &str, args: &[Lanes], active: Mask) -> (Vec<Lanes>, Mask) {
    let program = Program::wgsl(source, entry, &Interface::default()).expect("compiles");
    let mut regs = program.registers();
    let mut slot = 0;
    for index in 0.. {
        if slot >= args.len() {
            break;
        }
        for &reg in program.argument(index) {
            *regs.get_mut(reg) = args[slot];
            slot += 1;
        }
    }
    let done = program.run(&mut regs, active, &mut None_);
    (
        program.result().iter().map(|&reg| *regs.get(reg)).collect(),
        done,
    )
}

#[expect(clippy::cast_precision_loss, reason = "lane indices are small")]
fn x_lanes() -> Lanes {
    std::array::from_fn(|l| (l as f32).mul_add(0.37, -4.0))
}

fn assert_lanes(actual: &Lanes, expected: impl Fn(usize) -> f32, what: &str) {
    for (l, &got) in actual.iter().enumerate() {
        let want = expected(l);
        assert!(
            (got - want).abs() <= 1e-5 * want.abs().max(1.0),
            "{what}: lane {l} got {got}, want {want}"
        );
    }
}

#[test]
fn arithmetic_and_math_match_per_lane() {
    let source = "
        fn f(x: f32) -> vec4<f32> {
            let a = clamp(x * 0.5 + 1.0, -1.0, 1.5);
            let b = mix(x, 2.0 * x, 0.25) - floor(x) + fract(x);
            let c = smoothstep(-2.0, 2.0, x) + step(0.0, x) + sign(x) + abs(x) % 1.5;
            let d = sqrt(abs(x)) + exp2(x * 0.1) + sin(x) * cos(x) + max(x, min(x * x, 3.0));
            return vec4<f32>(a, b, c, d);
        }";
    let x = x_lanes();
    let out = run(source, "f", &[x]);
    let sign = |v: f32| {
        if v > 0.0 {
            1.0
        } else if v < 0.0 {
            -1.0
        } else {
            0.0
        }
    };
    assert_lanes(
        &out[0],
        |l| x[l].mul_add(0.5, 1.0).clamp(-1.0, 1.5),
        "clamp",
    );
    assert_lanes(
        &out[1],
        |l| {
            let v = x[l];
            (2.0 * v).mul_add(0.25, v * 0.75) - v.floor() + (v - v.floor())
        },
        "mix/floor/fract",
    );
    assert_lanes(
        &out[2],
        |l| {
            let v = x[l];
            let t = ((v + 2.0) / 4.0).clamp(0.0, 1.0);
            let a = v.abs();
            (t * t).mul_add(2.0f32.mul_add(-t, 3.0), if v >= 0.0 { 1.0 } else { 0.0 })
                + sign(v)
                + 1.5f32.mul_add(-(a / 1.5).trunc(), a)
        },
        "smoothstep/step/sign/rem",
    );
    assert_lanes(
        &out[3],
        |l| {
            let v = x[l];
            v.sin().mul_add(v.cos(), v.abs().sqrt() + (v * 0.1).exp2()) + v.max((v * v).min(3.0))
        },
        "sqrt/exp2/sin/cos/max",
    );
}

#[test]
#[expect(clippy::cast_precision_loss, reason = "small counts")]
fn divergent_loops_branches_and_returns() {
    let source = "
        fn f(x: f32) -> f32 {
            var acc = 0.0;
            let n = i32(x);
            for (var i = 0; i < 20; i += 1) {
                if i >= n { break; }
                if i % 3 == 1 { continue; }
                acc += f32(i);
            }
            if x > 25.0 { return -1.0; }
            switch n % 4 {
                case 0: { acc += 100.0; }
                case 1, 2: { acc += 200.0; }
                default: { acc -= 1.0; }
            }
            var k = 0;
            loop {
                k += 1;
                if k * k >= n { break; }
            }
            return acc + f32(k) * 1000.0;
        }";
    let x: Lanes = std::array::from_fn(|l| l as f32);
    let out = run(source, "f", &[x]);
    assert_lanes(
        &out[0],
        |l| {
            let n = i32::try_from(l).expect("a lane index fits i32");
            if l as f32 > 25.0 {
                return -1.0;
            }
            let mut acc = 0.0;
            for i in 0..20 {
                if i >= n {
                    break;
                }
                if i % 3 == 1 {
                    continue;
                }
                acc += i as f32;
            }
            acc += match n % 4 {
                0 => 100.0,
                1 | 2 => 200.0,
                _ => -1.0,
            };
            let mut k = 0;
            loop {
                k += 1;
                if k * k >= n {
                    break;
                }
            }
            (k as f32).mul_add(1000.0, acc)
        },
        "control flow",
    );
}

#[test]
#[expect(clippy::cast_precision_loss, reason = "small indices")]
fn dynamic_indexing_reads_and_writes_per_lane() {
    let source = "
        struct Pair { a: f32, b: vec2<f32> }
        fn f(x: f32, params: array<vec4<f32>, 4>) -> vec2<f32> {
            let i = u32(x) % 4u;
            var table = array<f32, 5>(1.0, 2.0, 3.0, 4.0, 5.0);
            table[(i + 1u) % 5u] = x;
            var p: Pair;
            p.b[i % 2u] = params[i].y;
            return vec2<f32>(table[i] + table[(i + 1u) % 5u], p.b.x + p.b.y + params[3 - i32(i)][i]);
        }";
    let x: Lanes = std::array::from_fn(|l| l as f32);
    let params: Vec<f32> = (0..16).map(|v| v as f32 * 10.0).collect();
    let mut args = vec![x];
    args.extend(params.iter().map(|&v| [v; LANES]));
    let out = run(source, "f", &args);
    let expect = |l: usize| {
        let i = l % 4;
        let mut table = [1.0, 2.0, 3.0, 4.0, 5.0];
        table[(i + 1) % 5] = l as f32;
        let mut b = [0.0f32; 2];
        b[i % 2] = params[i * 4 + 1];
        (
            table[i] + table[(i + 1) % 5],
            b[0] + b[1] + params[(3 - i) * 4 + i],
        )
    };
    assert_lanes(&out[0], |l| expect(l).0, "local array");
    assert_lanes(&out[1], |l| expect(l).1, "struct member and argument array");
}

#[test]
fn calls_keep_each_result_and_their_locals_reset() {
    let source = "
        struct S { v: vec2<f32>, w: f32 }
        fn gsat(x: f32) -> f32 { var t = x; if t < 0.0 { return 0.0; } t = min(t, 1.0); return t; }
        fn pack(a: f32, b: f32) -> S { var s: S; s.v = vec2<f32>(a, b); s.w = gsat(a + b); return s; }
        fn f(x: f32) -> vec4<f32> {
            let s = pack(gsat(x), gsat(x * 0.25));
            let t = pack(gsat(-x), 0.5);
            return vec4<f32>(s.v, s.w, t.w + t.v.y);
        }";
    let x = x_lanes();
    let out = run(source, "f", &[x]);
    let gsat = |v: f32| if v < 0.0 { 0.0 } else { v.min(1.0) };
    assert_lanes(&out[0], |l| gsat(x[l]), "first call");
    assert_lanes(&out[1], |l| gsat(x[l] * 0.25), "second call");
    assert_lanes(
        &out[2],
        |l| gsat(gsat(x[l]) + gsat(x[l] * 0.25)),
        "nested call",
    );
    assert_lanes(
        &out[3],
        |l| gsat(gsat(-x[l]) + 0.5) + 0.5,
        "another frame use",
    );
}

#[test]
fn matrices_and_vectors() {
    let source = "
        fn f(x: f32) -> vec4<f32> {
            let m = mat2x2<f32>(vec2<f32>(cos(x), sin(x)), vec2<f32>(-sin(x), cos(x)));
            let v = m * vec2<f32>(1.0, 2.0);
            let w = vec2<f32>(3.0, -1.0) * m;
            let n = mat3x3<f32>(vec3<f32>(2.0, 0.0, 1.0), vec3<f32>(x, 1.0, 0.0), vec3<f32>(0.0, 3.0, 1.0));
            let c = cross(vec3<f32>(1.0, x, 0.0), vec3<f32>(0.0, 1.0, x));
            return vec4<f32>(v.x + v.y, w.x - w.y, determinant(n), dot(c, vec3<f32>(1.0, 2.0, 3.0)) + length(normalize(c)));
        }";
    let x = x_lanes();
    let out = run(source, "f", &[x]);
    assert_lanes(
        &out[0],
        |l| {
            let (c, s) = (x[l].cos(), x[l].sin());
            2.0f32.mul_add(-s, c) + 2.0f32.mul_add(c, s)
        },
        "mat·vec",
    );
    assert_lanes(
        &out[1],
        |l| {
            let (c, s) = (x[l].cos(), x[l].sin());
            3.0f32.mul_add(c, -s) - (-3.0f32).mul_add(s, -c)
        },
        "vec·mat",
    );
    assert_lanes(
        &out[2],
        |l| {
            // Columns (2,0,1), (x,1,0), (0,3,1).
            let x = x[l];
            x.mul_add(-(0.0 - 3.0), 2.0 * (1.0 - 0.0)) + 0.0
        },
        "determinant",
    );
    assert_lanes(
        &out[3],
        |l| {
            let x = x[l];
            let c = [x * x, -x, 1.0];
            3.0f32.mul_add(c[2], 2.0f32.mul_add(c[1], c[0])) + 1.0
        },
        "cross/normalize",
    );
}

#[test]
#[expect(clippy::cast_sign_loss, reason = "bit patterns")]
fn integers_and_conversions() {
    let source = "
        fn f(x: f32) -> vec4<f32> {
            let i = i32(x * 3.0);
            let u = u32(abs(x) * 5.0);
            let shifted = (u << 3u) ^ (u >> 1u) | 1u;
            let q = i / 3 + i % 3 + select(0, 7, i < 0);
            let zero = i / (i - i);
            return vec4<f32>(f32(shifted), f32(q), f32(zero), f32(countOneBits(u) + firstLeadingBit(u + 1u)));
        }";
    let x = x_lanes();
    let out = run(source, "f", &[x]);
    #[expect(clippy::cast_possible_truncation, reason = "WGSL truncation")]
    let ints = |l: usize| ((x[l] * 3.0) as i32, (x[l].abs() * 5.0) as u32);
    #[expect(clippy::cast_precision_loss, reason = "small integers")]
    {
        assert_lanes(
            &out[0],
            |l| {
                let u = ints(l).1;
                (((u << 3) ^ (u >> 1)) | 1) as f32
            },
            "bits",
        );
        assert_lanes(
            &out[1],
            |l| {
                let i = ints(l).0;
                (i / 3 + i % 3 + if i < 0 { 7 } else { 0 }) as f32
            },
            "signed division",
        );
        // WGSL: division by zero yields the dividend.
        assert_lanes(&out[2], |l| ints(l).0 as f32, "division by zero");
        assert_lanes(
            &out[3],
            |l| {
                let u = ints(l).1;
                (u.count_ones() + (u + 1).ilog2()) as f32
            },
            "bit counts",
        );
    }
}

/// A host function and a host-written private input.
struct Doubler;

impl Host for Doubler {
    fn call(&mut self, function: usize, args: &[Lanes], out: &mut [Lanes], active: &Mask) {
        assert_eq!(function, 0);
        for l in 0..LANES {
            if active[l] {
                out[0][l] = args[0][l] * 2.0;
                out[1][l] = args[1][l] + 1.0;
            }
        }
    }

    fn image(&mut self, op: ImageOp<'_>, _: &mut [Lanes; 4], _: &Mask) {
        panic!("unexpected image op {op:?}");
    }
}

/// A host binding one 4×3 image, handle 7, whose texel `(x, y)` is
/// `(x, y, 10x + y, 1)`, and one sampler, handle 2, that returns the
/// coordinate it was given.
struct Images;

impl Host for Images {
    fn call(&mut self, function: usize, _: &[Lanes], _: &mut [Lanes], _: &Mask) {
        panic!("unexpected host call {function}");
    }

    #[expect(clippy::cast_precision_loss, reason = "texel coordinates are small")]
    fn image(&mut self, op: ImageOp<'_>, out: &mut [Lanes; 4], active: &Mask) {
        for l in (0..LANES).filter(|&l| active[l]) {
            match op {
                ImageOp::Sample {
                    image,
                    sampler,
                    uv,
                    offset,
                } => {
                    assert_eq!((image, sampler), (7, 2));
                    out[0][l] = uv[0][l];
                    out[1][l] = uv[1][l];
                    out[2][l] = offset[0] as f32;
                    out[3][l] = offset[1] as f32;
                }
                ImageOp::Load { image, texel } => {
                    assert_eq!(image, 7);
                    let (x, y) = (texel[0][l].clamp(0, 3), texel[1][l].clamp(0, 2));
                    out[0][l] = x as f32;
                    out[1][l] = y as f32;
                    out[2][l] = (10 * x + y) as f32;
                    out[3][l] = 1.0;
                }
                ImageOp::Size { image } => {
                    assert_eq!(image, 7);
                    out[0][l] = f32::from_bits(4);
                    out[1][l] = f32::from_bits(3);
                }
            }
        }
    }
}

#[test]
#[expect(clippy::cast_precision_loss, reason = "lane indices are small")]
fn image_ops_reach_the_host_through_helpers() {
    let source = "
        fn texel(t: texture_2d<f32>, p: vec2<i32>) -> vec4<f32> {
            return textureLoad(t, p, 0);
        }
        fn f(t: texture_2d<f32>, s: sampler, uv: vec2<f32>) -> vec4<f32> {
            let size = vec2<f32>(textureDimensions(t, 0));
            let sampled = textureSampleLevel(t, s, uv, 0.0, vec2<i32>(1, -2));
            let loaded = texel(t, vec2<i32>(uv * size));
            return vec4<f32>(sampled.xy * size, sampled.z + sampled.w * 10.0, loaded.z);
        }";
    let program = Program::wgsl(source, "f", &Interface::default()).expect("compiles");
    let mut regs = program.registers();
    regs.splat(program.argument(0)[0], f32::from_bits(7));
    regs.splat(program.argument(1)[0], f32::from_bits(2));
    let u: Lanes = std::array::from_fn(|l| (l % 4) as f32 / 4.0 + 0.1);
    let v: Lanes = std::array::from_fn(|l| (l / 8) as f32 / 3.0 + 0.1);
    *regs.get_mut(program.argument(2)[0]) = u;
    *regs.get_mut(program.argument(2)[1]) = v;
    program.run(&mut regs, [true; LANES], &mut Images);
    let out: Vec<Lanes> = program.result().iter().map(|&r| *regs.get(r)).collect();
    assert_lanes(&out[0], |l| u[l] * 4.0, "sampled u in texels");
    assert_lanes(&out[1], |l| v[l] * 3.0, "sampled v in texels");
    assert_lanes(&out[2], |_| -19.0, "the constant offset");
    assert_lanes(
        &out[3],
        |l| (10 * (l % 4) + (l / 8).min(2)) as f32,
        "the loaded texel",
    );
}

#[test]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the absolute lane value is small"
)]
#[expect(clippy::suboptimal_flops, reason = "mirrors the shader's expression")]
fn pointer_arguments_write_back_for_the_lanes_that_call() {
    let source = "
        fn bump(p: ptr<function, vec2<f32>>, k: f32) {
            (*p).x += k;
            *p = *p * 2.0;
        }
        fn add(p: ptr<function, f32>, k: f32) { *p += k; }
        fn f(x: f32) -> vec3<f32> {
            var v = vec2<f32>(x, 1.0);
            if x > 0.0 { bump(&v, 3.0); }
            var a = array<f32, 3>(10.0, 20.0, 30.0);
            add(&a[u32(abs(x)) % 3u], x);
            return vec3<f32>(v, a[0] + 2.0 * a[1] + 4.0 * a[2]);
        }";
    let x = x_lanes();
    let out = run(source, "f", &[x]);
    assert_lanes(
        &out[0],
        |l| if x[l] > 0.0 { (x[l] + 3.0) * 2.0 } else { x[l] },
        "pointee x",
    );
    assert_lanes(&out[1], |l| if x[l] > 0.0 { 2.0 } else { 1.0 }, "pointee y");
    assert_lanes(
        &out[2],
        |l| {
            let mut a = [10.0, 20.0, 30.0];
            a[(x[l].abs() as usize) % 3] += x[l];
            a[0] + 2.0 * a[1] + 4.0 * a[2]
        },
        "an element pointer",
    );
}

#[test]
fn a_texture_global_is_unsupported() {
    let source = "
        @group(0) @binding(0) var t: texture_2d<f32>;
        fn f(p: vec2<i32>) -> vec4<f32> { return textureLoad(t, p, 0); }";
    assert!(matches!(
        Program::wgsl(source, "f", &Interface::default()),
        Err(LaneError::Unsupported(_))
    ));
}

#[test]
fn host_functions_and_inputs() {
    let source = "
        var<private> gain: f32;
        var<private> scratch: f32 = 3.0;
        fn twice(q: vec2<f32>) -> vec2<f32> { return vec2<f32>(); }
        fn f(x: f32) -> vec2<f32> {
            scratch += x;
            var r = vec2<f32>(0.0);
            if x > 0.0 { r = twice(vec2<f32>(x, x * 3.0)); }
            return r * gain + vec2<f32>(scratch);
        }";
    let program = Program::wgsl(
        source,
        "f",
        &Interface {
            functions: &["twice"],
            inputs: &["gain"],
        },
    )
    .expect("compiles");
    let mut regs = program.registers();
    let gain = program.global("gain").expect("an input")[0];
    regs.splat(gain, 0.5);
    let x = x_lanes();
    *regs.get_mut(program.argument(0)[0]) = x;
    // Two runs: the private resets, the input persists.
    for _ in 0..2 {
        program.run(&mut regs, [true; LANES], &mut Doubler);
    }
    let out: Vec<Lanes> = program.result().iter().map(|&r| *regs.get(r)).collect();
    assert_lanes(
        &out[0],
        |l| {
            let v = x[l];
            f32::mul_add(if v > 0.0 { v * 2.0 } else { 0.0 }, 0.5, 3.0) + v
        },
        "host x",
    );
    assert_lanes(
        &out[1],
        |l| {
            let v = x[l];
            f32::mul_add(if v > 0.0 { v.mul_add(3.0, 1.0) } else { 0.0 }, 0.5, 3.0) + v
        },
        "host y",
    );
}

#[test]
#[expect(clippy::cast_precision_loss, reason = "quad coordinates are small")]
fn derivatives_take_quad_differences() {
    let source = "
        fn f(x: f32, y: f32) -> vec3<f32> {
            let v = x * x + 3.0 * y;
            return vec3<f32>(dpdx(v), dpdy(v), fwidth(v));
        }";
    // Lane bit 0 is the column, bit 1 the row, the rest the quad.
    let px: Lanes = std::array::from_fn(|l| ((l >> 2) * 2 + (l & 1)) as f32 + 0.5);
    let py: Lanes = std::array::from_fn(|l| ((l >> 1) & 1) as f32 + 0.5);
    let out = run(source, "f", &[px, py]);
    let v = |l: usize| 3.0f32.mul_add(py[l], px[l] * px[l]);
    let dx = |l: usize| v(l | 1) - v(l & !1);
    assert_lanes(&out[0], dx, "dpdx");
    assert_lanes(&out[1], |_| 3.0, "dpdy");
    assert_lanes(&out[2], |l| dx(l).abs() + 3.0, "fwidth");
}

#[test]
fn discard_retires_lanes() {
    let source = "
        @fragment fn main(@location(0) x: f32) -> @location(0) vec4<f32> {
            if x < 0.0 { discard; }
            return vec4<f32>(x);
        }";
    let x = x_lanes();
    let mut active = [true; LANES];
    active[LANES - 1] = false;
    let (out, done) = run_masked(source, "main", &[x], active);
    for l in 0..LANES {
        assert_eq!(done[l], active[l] && x[l] >= 0.0, "lane {l}");
        if done[l] {
            assert!((out[0][l] - x[l]).abs() < 1e-6);
        }
    }
}

#[test]
fn unsupported_constructs_fail_at_compile_time() {
    let source = "
        @group(0) @binding(0) var tex: texture_2d<f32>;
        @group(0) @binding(1) var smp: sampler;
        fn f(uv: vec2<f32>) -> vec4<f32> { return textureSampleLevel(tex, smp, uv, 0.0); }";
    let error = Program::wgsl(source, "f", &Interface::default()).expect_err("no image host");
    assert!(matches!(error, LaneError::Unsupported(_)), "{error}");
    let error = Program::wgsl("fn f() -> f32 { return 1.0; }", "g", &Interface::default())
        .expect_err("no such entry");
    assert_eq!(error, LaneError::Entry("g".into()));
}

#[cfg(feature = "eval")]
#[test]
fn agrees_with_the_scalar_evaluator() {
    use cherenkov_shader::eval::{Eval, Value};
    let source = "
        fn f(c: vec4<f32>, a: f32) -> vec4<f32> {
            var acc = vec4<f32>(0.0);
            for (var i = 0; i < 4; i++) {
                acc = acc + mix(c, vec4<f32>(a), 0.25) * 0.5;
                if acc.x > 2.0 { break; }
            }
            return clamp(acc, vec4<f32>(-1.0), vec4<f32>(3.0)) + vec4<f32>(dot(c.xyz, c.zyx));
        }";
    let module = naga::front::wgsl::parse_str(source).expect("parses");
    let eval = Eval::new(&module);
    let function = eval.function("f");
    let x = x_lanes();
    let cs: [Lanes; 4] = [0.3, 0.6, 0.9, 1.2].map(|scale| std::array::from_fn(|l| x[l] * scale));
    let a: Lanes = std::array::from_fn(|l| x[l].mul_add(-0.2, 1.0));
    let out = run(source, "f", &[cs[0], cs[1], cs[2], cs[3], a]);
    for l in 0..LANES {
        let want = eval
            .call(
                function,
                vec![
                    Value::vec(&[cs[0][l], cs[1][l], cs[2][l], cs[3][l]]),
                    Value::Float(a[l]),
                ],
            )
            .components();
        for k in 0..4 {
            assert!(
                (out[k][l] - want[k]).abs() <= 1e-5,
                "lane {l} component {k}: {} vs {}",
                out[k][l],
                want[k]
            );
        }
    }
}
