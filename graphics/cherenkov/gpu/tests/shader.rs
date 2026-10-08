//! The WGSL source translates through every naga backend wgpu uses, for
//! each `VARIANT` pipeline, so a Metal or D3D regression shows on Linux.

use cherenkov::__engine_test as split_test;
use naga::back::{hlsl, msl, spv};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{Module, front::wgsl};

const SHADER: &str = include_str!("../src/render/shader.wgsl");
/// The build-time outputs the runtime assembles modules from: the
/// `// union-stub`-stripped shader and the `UNION_MAX_MEMBERS`
/// fragment — read from `OUT_DIR` so the test cannot drift from what
/// `build.rs` ships.
const SHADER_NO_UNION: &str = include_str!(concat!(env!("OUT_DIR"), "/shader_no_union.wgsl"));
const UNION_MAX: &str = include_str!(concat!(env!("OUT_DIR"), "/union_max_members.wgsl"));
const SHARED: &str = include_str!("../src/render/shared.wgsl");
const UNION: &str = include_str!("../src/render/union.wgsl");
const BLEND: &str = include_str!("../src/render/blend.wgsl");
const PROJECTIVE: &str = include_str!("../src/render/projective.wgsl");
const MIP: &str = include_str!("../src/render/mip.wgsl");
const RESOLVE: &str = include_str!("../src/render/resolve.wgsl");

/// The oldest Metal language version wgpu selects on a supported macOS
/// (10.13 → 2.0), which is where `instance_id` and friends became legal.
const MSL_VERSION: (u8, u8) = (2, 0);

fn composed(variant: u32) -> (Module, naga::valid::ModuleInfo) {
    // The engine module is `shared.wgsl`, `shader.wgsl` then `blend.wgsl`,
    // as in build.rs: the union variant additionally prepends
    // `UNION_MAX_MEMBERS` and links `union.wgsl`; the plain variants use
    // the shipped `shader_no_union.wgsl`.
    let (cap, shader, union) = if variant == 3 {
        (UNION_MAX, SHADER, UNION)
    } else {
        ("", SHADER_NO_UNION, "")
    };
    let source =
        format!("const VARIANT: u32 = {variant}u;\n{cap}{SHARED}\n{shader}\n{union}{BLEND}");
    let module = wgsl::parse_str(&source).unwrap_or_else(|e| panic!("variant {variant}: {e}"));
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("variant {variant}: {e:?}"));
    (module, info)
}

split_test! {
fn every_variant_emits_msl() {
    for variant in 0..4 {
        let (module, info) = composed(variant);
        let options = msl::Options {
            lang_version: MSL_VERSION,
            ..msl::Options::default()
        };
        msl::write_string(&module, &info, &options, &msl::PipelineOptions::default())
            .unwrap_or_else(|e| panic!("variant {variant}: msl: {e}"));
    }
}
}

split_test! {
fn every_variant_emits_spirv() {
    for variant in 0..4 {
        let (module, info) = composed(variant);
        let mut writer = spv::Writer::new(&spv::Options::default())
            .unwrap_or_else(|e| panic!("variant {variant}: spv: {e}"));
        let mut words = Vec::<u32>::new();
        writer
            .write(&module, &info, None, &None, &mut words)
            .unwrap_or_else(|e| panic!("variant {variant}: spv: {e}"));
        assert_ne!(words, [] as [u32; 0]);
    }
}
}

split_test! {
fn every_variant_emits_hlsl() {
    for variant in 0..4 {
        let (module, info) = composed(variant);
        let options = hlsl::Options::default();
        let mut out = String::new();
        let pipeline_options = hlsl::PipelineOptions::default();
        let mut writer = hlsl::Writer::new(&mut out, &options, &pipeline_options);
        writer
            .write(&module, &info, None)
            .unwrap_or_else(|e| panic!("variant {variant}: hlsl: {e}"));
        assert_ne!(out, "");
    }
}
}

/// Parses and validates an effect module's text, panicking on the first
/// naga diagnostic. `backdrop_effect_text` is the same composition the
/// renderer feeds `create_shader_module`, so an invalid module surfaces
/// here before any device exists.
fn effect_module(name: &str, user: &str) -> (Module, naga::valid::ModuleInfo) {
    // A member may draw under a union: the union-shape module validates
    // on the same user source before the plain one is emitted below.
    let union_text = cherenkov_gpu::backdrop_effect_text(user, true);
    let union_module =
        wgsl::parse_str(&union_text).unwrap_or_else(|e| panic!("{name} (union): {e}"));
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&union_module)
        .unwrap_or_else(|e| panic!("{name} (union): {e:?}"));
    let text = cherenkov_gpu::backdrop_effect_text(user, false);
    let module = wgsl::parse_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    (module, info)
}

/// Every backend the engine emits through, run on one effect module.
fn effect_emits(name: &str, module: &Module, info: &naga::valid::ModuleInfo) {
    let options = msl::Options {
        lang_version: MSL_VERSION,
        ..msl::Options::default()
    };
    msl::write_string(module, info, &options, &msl::PipelineOptions::default())
        .unwrap_or_else(|e| panic!("{name}: msl: {e}"));
    let mut writer =
        spv::Writer::new(&spv::Options::default()).unwrap_or_else(|e| panic!("{name}: spv: {e}"));
    let mut words = Vec::<u32>::new();
    writer
        .write(module, info, None, &None, &mut words)
        .unwrap_or_else(|e| panic!("{name}: spv: {e}"));
    assert_ne!(words, [] as [u32; 0]);
    let options = hlsl::Options::default();
    let mut out = String::new();
    let pipeline_options = hlsl::PipelineOptions::default();
    let mut hlsl_writer = hlsl::Writer::new(&mut out, &options, &pipeline_options);
    hlsl_writer
        .write(module, info, None)
        .unwrap_or_else(|e| panic!("{name}: hlsl: {e}"));
    assert_ne!(out, "");
}

split_test! {
/// The registered-effect module parses, validates and translates through
/// every naga backend — for each built-in effect kind's semantics as a
/// user source would use them (sample, SDF normal and displacement), and
/// for a representative `BackdropEffect::Shader` source.
fn backdrop_effect_text_emits() {
    // Each case is a `fn backdrop_effect` body exercising the member-
    // effect machinery the built-ins rely on.
    const CASES: &[(&str, &str)] = &[
        // BackdropEffect::Color: a premultiplied matrix multiply.
        (
            "color-matrix",
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                let c = backdrop_sample(px.p);
                return vec4<f32>(
                    c.rgb * params[0].xyz + params[1].xyz * c.a,
                    c.a
                );
            }",
        ),
        // BackdropEffect::Refraction: displaced sample along the normal.
        (
            "refraction-displace",
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                let t = clamp(1.0 + px.sdf / params[0].y, 0.0, 1.0);
                return backdrop_sample(px.p - px.normal * params[0].x * t * t);
            }",
        ),
        // BackdropEffect::Rim: a highlight scaled by the inside distance.
        (
            "rim-light",
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                let rim = clamp(1.0 + px.sdf / 4.0, 0.0, 1.0);
                return vec4<f32>(backdrop_sample(px.p).rgb * (1.0 + params[0].x * rim), backdrop_sample(px.p).a);
            }",
        ),
        // A representative BackdropEffect::Shader source.
        (
            "user-size",
            "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> {
                return vec4<f32>(px.size.x / 256.0, px.size.y / 256.0, 0.0, 1.0);
            }",
        ),
    ];
    for (name, user) in CASES {
        let (module, info) = effect_module(name, user);
        effect_emits(name, &module, &info);
    }
}
}

split_test! {
/// The projective composite and mip modules (#84) and the backdrop
/// capture resolve module translate through every naga backend, composed
/// as in build.rs.
fn standalone_modules_emit() {
    for (name, source) in [
        ("projective", format!("{SHARED}\n{BLEND}\n{PROJECTIVE}")),
        ("mip", MIP.to_owned()),
        ("resolve", RESOLVE.to_owned()),
    ] {
        let module = wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
        let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        effect_emits(name, &module, &info);
    }
}
}
