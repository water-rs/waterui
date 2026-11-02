//! Precompiles the engine's fixed WGSL modules (issue #57): naga parses,
//! validates and translates them once at build time instead of wgpu doing it
//! per pipeline at runtime. Each module produces
//!
//! - `<name>.spv` — naga SPIR-V as emitted, on the non-Apple, non-wasm
//!   targets only (issue #241). spirv-opt ran here until #124 measured it
//!   on the Pixel 9 Pro (Mali-G715): ~0.64 s faster cold pipeline creation
//!   but the effects scene's GPU time ~78% slower at p50 and ~2.7x at p99;
//! - `<name>.metal` — naga MSL at wgpu-hal's argument slots;
//! - `<name>.metallib` — the `.metal` compiled by `xcrun metal`/`metallib`,
//!   on Apple targets only.
//!
//! The runtime embeds these with `include_bytes!` and hands them to
//! `Device::create_shader_module_passthrough`, so the binaries are trusted
//! inputs: SPIR-V and MSL are emitted with naga's bounds checks and loop
//! bounding off (passthrough carries no runtime checks — this supersedes
//! the #55 runtime-checks work).
//!
//! Metal needs two accommodations of wgpu-hal's slot protocol, both encoded
//! in `src/render/bindings.rs`, which this script shares with the crate:
//! argument slots are per-stage counters over the bind group layout, and a
//! passthrough module is never written a runtime-array-sizes buffer, so the
//! MSL must not take one — the engine shaders never call `arrayLength`, so
//! their `array<T>` storage buffers are pinned to `array<T, 1>` for the MSL
//! emission only, which keeps the signature free of `_mslBufferSizes`.

#[path = "src/render/bindings.rs"]
mod bindings;
#[path = "src/render/capabilities.rs"]
mod capabilities;
#[path = "build_dxil.rs"]
mod dxil;
/// The Apple Metal toolchain plumbing every in-tree build script shares —
/// SDK and `-target` selection, `metal -c`/`metallib` driving, the
/// deployment floor. `cocoa-ui`'s `build.rs` includes this same file by
/// path for its capture compositor (waterui#2827).
#[path = "build_metal.rs"]
mod metal;
#[path = "src/render/programs.rs"]
mod programs;
/// The stroke record ABI the compute stroker's kernels share with the CPU
/// encoder: rendered into the WGSL prelude of the `coverage` module.
#[path = "src/render/stroke_abi.rs"]
mod stroke_abi;

mod build_tile_spirv;
/// The SPIR-V writer flags `cherenkov_shader::emit::spirv` emits with,
/// pulled in the same way so the two emitters cannot drift
/// (water-rs/waterui#3079).
#[path = "../shader/src/spv_flags.rs"]
mod spv_flags;
#[path = "build_tile.rs"]
mod tile;
/// The union member cap the WGSL fold compiles against — the same
/// definition `cherenkov::BackdropUnion::MAX_MEMBERS` re-exports, kept
/// free of a crate dependency so this build script does not link the
/// engine.
#[path = "../record/src/union_cap.rs"]
mod union_cap;

use std::env;
use std::fmt::Write as _;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use metal::AppleTarget;
use naga::back::{msl, spv};
use naga::valid::{ValidationFlags, Validator};

/// One fixed engine module to compile.
struct Spec {
    /// Output file stem, e.g. `engine0`.
    name: String,
    /// WGSL text (`VARIANT` already prepended for the engine shader).
    source: String,
    /// The pipeline's bind group layouts, in group order.
    groups: &'static [&'static [bindings::Entry]],
    /// The module's programs; each module with programs is emitted as
    /// DXIL on Windows targets. Empty for the Vulkan-only native external
    /// and tile modules.
    programs: &'static [programs::Program],
    /// Emit the `.metal` artifact; false for the Vulkan-only native
    /// external module.
    metal: bool,
    /// The `(texture, sampler)` group-1 WGSL bindings the restricted
    /// lowering merges into one combined sampled image at the texture's
    /// binding (issue #166).
    merge_pair: Option<(u32, u32)>,
    /// The tile-interface module: `tile::attachment_inputs` rewrites the
    /// corresponding-pixel reads on either platform, the MSL emission
    /// uses language 3, and the Apple product compiles attachment.metal
    /// with this module's `.metal` preincluded via `-include`; on SPIR-V
    /// targets it additionally emits the `_self` input-attachment
    /// variant (issue #2191).
    attachment: bool,
}

/// Removes every `// <marker>`-bracketed span from `source` — the mirror
/// of the marker convention in `shaders.rs`: marker lines pair up, and
/// the text between an opening and closing marker is a specialization's
/// optional block (the union field, the long-gradient branch, the image
/// source conversion, the GPU coverage cell buffer), dropped wholesale by
/// variants that do not carry it.
///
/// Only a whole trimmed line equal to the marker toggles the span — a
/// marker inside other text is not a marker. An unpaired marker is a
/// build bug and panics.
fn drop_marked(source: &str, marker: &str) -> String {
    let tag = format!("// {marker}");
    let mut out = String::with_capacity(source.len());
    let mut dropping = false;
    for line in source.split_inclusive('\n') {
        if line.trim() == tag {
            dropping = !dropping;
        } else if !dropping {
            out.push_str(line);
        }
    }
    assert!(!dropping, "unpaired `{tag}` marker");
    out
}

/// What the `// union-stub` spans carry.
const UNION_NAMES: &[&str] = &["union_field", "member_field"];
/// What the `// image-source` spans carry.
const IMAGE_SOURCE_NAMES: &[&str] = &["image_decode", "image_source_texel"];
/// What the `// cell-buffer` spans carry: the GPU coverage arena reads.
const CELL_NAMES: &[&str] = &["coverage_arena"];

/// The invariant of every stripped product: none of `names` survives — a
/// marker span that drifted fails loudly here rather than leaving dead code
/// in a shipped module.
fn assert_absent(source: &str, product: &str, names: &[&str]) {
    for name in names {
        assert!(
            !source.contains(name),
            "stripping left `{name}` in {product}"
        );
    }
}

/// The marker-stripped products of `shader.wgsl` and `shared.wgsl` the
/// engine variants are composed from.
struct Stripped {
    shared_plain: String,
    shader_no_long: String,
    shader_cell: String,
    shader_cell_plain: String,
    shader_long: String,
    shader_image: String,
    shader_plain: String,
}

impl Stripped {
    fn new(shader: &str, shared: &str) -> Self {
        // The stripped products. `shader_no_long` feeds the union module
        // (variant 3) and `shader_cell` the cell module (variant 5); both keep
        // the `// image-source` spans, since a union member or a GPU-routed
        // draw may be an image paint with source flags. `shader_long` (variant
        // 4), `shader_image` (variant 6) and `shader_plain` (variants 0–2)
        // additionally drop every `// cell-buffer` span, from `shared.wgsl`
        // too, so only GPU-routed draws carry the cell-texel reads (issue
        // #2543); `shader_long` and `shader_plain` drop the source conversion
        // and `shader_image` and `shader_plain` the long-gradient branch.
        let shader_no_long = drop_marked(shader, "long-gradient-stub");
        let shader_cell = drop_marked(shader, "union-stub");
        let shader_cell_plain = drop_marked(
            &drop_marked(&shader_cell, "long-gradient-stub"),
            "image-source",
        );
        let shader_no_cell = drop_marked(&shader_cell, "cell-buffer");
        let shader_long = drop_marked(
            &drop_marked(
                &drop_marked(&shader_no_cell, "image-source"),
                "non-gradient-paint",
            ),
            "generic-composite",
        );
        let shader_image = drop_marked(&shader_no_cell, "long-gradient-stub");
        let shader_plain = drop_marked(&shader_image, "image-source");
        let shared_plain = drop_marked(shared, "cell-buffer");
        for (product, source) in [
            ("shader_cell", &shader_cell),
            ("shader_cell_plain", &shader_cell_plain),
            ("shader_long", &shader_long),
            ("shader_image", &shader_image),
            ("shader_plain", &shader_plain),
        ] {
            assert_absent(source, product, UNION_NAMES);
        }
        for (product, source) in [
            ("shader_long", &shader_long),
            ("shader_image", &shader_image),
            ("shader_plain", &shader_plain),
            ("shared_plain", &shared_plain),
        ] {
            assert_absent(source, product, CELL_NAMES);
        }
        for (product, source) in [
            ("shader_long", &shader_long),
            ("shader_plain", &shader_plain),
            ("shader_cell_plain", &shader_cell_plain),
        ] {
            assert_absent(source, product, IMAGE_SOURCE_NAMES);
        }
        Self {
            shared_plain,
            shader_no_long,
            shader_cell,
            shader_cell_plain,
            shader_long,
            shader_image,
            shader_plain,
        }
    }
}

/// Passthrough shaders carry no naga runtime checks: the source is
/// controlled and validated here at build time.
const UNCHECKED: naga::proc::BoundsCheckPolicies = naga::proc::BoundsCheckPolicies {
    index: naga::proc::BoundsCheckPolicy::Unchecked,
    buffer: naga::proc::BoundsCheckPolicy::Unchecked,
    image_load: naga::proc::BoundsCheckPolicy::Unchecked,
    binding_array: naga::proc::BoundsCheckPolicy::Unchecked,
};

fn read_shader(manifest: &Path, name: &str) -> String {
    let path = manifest.join(format!("src/render/{name}.wgsl"));
    println!("cargo::rerun-if-changed={}", path.display());
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    for shared in ["bindings", "programs", "stroke_abi"] {
        println!(
            "cargo::rerun-if-changed={}",
            manifest.join(format!("src/render/{shared}.rs")).display()
        );
    }
    let read = |name: &str| read_shader(&manifest, name);
    let [
        shared,
        shader,
        union,
        present,
        external,
        blend,
        projective,
        mip,
        resolve,
        reduce,
        headroom,
        shadow,
        coverage,
        stroke,
        native,
        tile,
        space_convert,
    ] = [
        "shared",
        "shader",
        "union",
        "present",
        "external",
        "blend",
        "projective",
        "mip",
        "resolve",
        "reduce",
        "headroom",
        "shadow",
        "coverage",
        "stroke",
        "external_native",
        "composite/tile",
        "space_convert",
    ]
    .map(read);

    // The working-space ↔ sRGB conversion every colour-converting module
    // shares: filtrate-core's `space::WGSL`, rendered from the same
    // constants as the CPU conversion. It precedes `shared.wgsl` (and so
    // every module built on it), `present.wgsl`, `space_convert.wgsl` and
    // the `color.wgsl` producer prelude. Every module spec lands in
    // `OUT_DIR` below; the `shared` prelude and the `color` prelude, which
    // the runtime and the tests compose with their own sources, land there
    // too, so nothing can drift from what this script compiles.
    let space = filtrate_core::space::WGSL;
    let shared = format!("{space}\n{shared}");
    let present = format!("{space}\n{present}");
    let space_convert = format!("{space}\n{space_convert}");
    let color = format!("{space}\n{}", read("color"));
    for (name, text) in [("shared", &shared), ("color", &color)] {
        std::fs::write(out_dir.join(format!("{name}.wgsl")), text).unwrap();
    }

    // The union member cap the WGSL fold is compiled against:
    // `union_cap::UNION_MAX_MEMBERS` prepended like `VARIANT`, so the
    // Rust constant stays the single definition. The runtime assembles
    // the union module from this fragment and `shader.wgsl`, the image
    // variant from `shader_image.wgsl` — `shader.wgsl` minus every
    // `// union-stub`-bracketed span — and the plain variants from
    // `shader_plain.wgsl`, which also drops every `// image-source` span.
    let union_max = format!(
        "const UNION_MAX_MEMBERS: u32 = {}u;\n",
        union_cap::UNION_MAX_MEMBERS
    );
    std::fs::write(out_dir.join("union_max_members.wgsl"), &union_max).unwrap();
    // The coverage module is the stroke ABI prelude, `coverage.wgsl` and the
    // compute stroker's kernels, in that order. Its spec lands in `OUT_DIR`
    // like every other, and the runtime's WGSL delivery reads that file.
    let coverage = format!("{}\n{coverage}\n{stroke}", stroke_prelude());
    let stripped = Stripped::new(&shader, &shared);
    let specialized = drop_marked(&stripped.shader_plain, "full-fragment");
    let glyph = format!("{specialized}\n{}", read("shader_glyph"));
    let cell_specialized = drop_marked(&stripped.shader_cell_plain, "full-fragment");
    let cell_glyph = format!("{cell_specialized}\n{}", read("shader_glyph"));
    let composite = format!("{specialized}\n{}", read("shader_composite"));
    let specs = module_specs(&Modules {
        simple_entry: &read("entry_simple"),
        shadow_entry: &read("entry_shadow"),
        full_entry: &read("entry_full"),
        shared: &shared,
        shared_plain: &stripped.shared_plain,
        shader_no_long: &stripped.shader_no_long,
        shader_cell: &stripped.shader_cell,
        shader_cell_plain: &stripped.shader_cell_plain,
        cell_glyph: &cell_glyph,
        shader_long: &stripped.shader_long,
        shader_image: &stripped.shader_image,
        shader_plain: &stripped.shader_plain,
        glyph: &glyph,
        composite: &composite,
        union: &union,
        union_max: &union_max,
        blend: &blend,
        present: &present,
        external: &external,
        native: &native,
        projective: &projective,
        mip: &mip,
        resolve: &resolve,
        reduce: &reduce,
        headroom: &headroom,
        shadow: &shadow,
        space_convert: &space_convert,
        coverage: &coverage,
        tile: &tile,
    });

    compile_specs(&out_dir, &specs);
}

/// The stroke ABI as WGSL: every `stroke_abi` constant as a `u32` `const`,
/// and the 8-point Gauss-Legendre rule the dashed arc lengths integrate with.
/// The Rust constants are the only definition of the values.
fn stroke_prelude() -> String {
    macro_rules! abi {
        ($($name:ident),* $(,)?) => {
            [$((stringify!($name), stroke_abi::$name)),*]
        };
    }
    let constants = abi![
        RECORD_WORDS,
        PIECE_MASK,
        PIECE_LINE,
        PIECE_CURVE,
        PIECE_POINT_ARC,
        PIECE_JOIN_ONLY,
        JOIN_SHIFT,
        JOIN_MASK,
        JOIN_START,
        JOIN_SHARED,
        JOIN_BEVEL,
        JOIN_MITER,
        JOIN_ROUND,
        CAP_START,
        CAP_END,
        CLOSE,
        LINEAR_END,
        SLOTS,
        SLOT_FS,
        SLOT_BS,
        SLOT_FE,
        SLOT_BE,
        SLOT_WORDS,
        N_OFF_MAX,
        N_LEN_MAX,
        CAP_BUTT,
        CAP_SQUARE,
        CAP_ROUND,
        FRAG_JOIN,
        FRAG_START_CAP,
        FRAG_END_CAP,
        FRAG_RECORD_SHIFT,
        STAGE_WORDS,
        LIST_WORDS,
        SCAN_BLOCK,
    ];
    let mut prelude = String::from("// The stroke ABI, rendered from `stroke_abi.rs`.\n");
    for (name, value) in constants {
        writeln!(prelude, "const {name}: u32 = {value}u;").expect("write to a String");
    }
    let (nodes, weights): (Vec<_>, Vec<_>) = gauss_legendre_8().into_iter().unzip();
    let list = |values: Vec<f64>| {
        values
            .iter()
            .map(|v| format!("{v:.12}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    writeln!(
        prelude,
        "const GL_NODES: array<f32, 8> = array<f32, 8>({});\n\
         const GL_WEIGHTS: array<f32, 8> = array<f32, 8>({});",
        list(nodes),
        list(weights)
    )
    .expect("write to a String");
    prelude
}

/// The 8-point Gauss-Legendre nodes and weights on `[-1, 1]`, by Newton's
/// iteration on the Legendre polynomial from its Chebyshev-like starting
/// roots.
fn gauss_legendre_8() -> Vec<(f64, f64)> {
    const N: u32 = 8;
    let n = f64::from(N);
    let mut rule = Vec::with_capacity(N as usize);
    for i in 0..N / 2 {
        let mut z = (std::f64::consts::PI * (f64::from(i) + 0.75) / (n + 0.5)).cos();
        let derivative = loop {
            let (mut p1, mut p2) = (1.0_f64, 0.0_f64);
            for j in 1..=N {
                let p3 = p2;
                p2 = p1;
                let j = f64::from(j);
                p1 = (j - 1.0).mul_add(-p3, 2.0f64.mul_add(j, -1.0) * z * p2) / j;
            }
            let pp = n * z.mul_add(p1, -p2) / z.mul_add(z, -1.0);
            let next = z - p1 / pp;
            if (next - z).abs() < 1e-15 {
                break pp;
            }
            z = next;
        };
        let weight = 2.0 / (z.mul_add(-z, 1.0) * derivative * derivative);
        rule.push((-z, weight));
        rule.push((z, weight));
    }
    rule.sort_by(|a, b| a.0.total_cmp(&b.0));
    rule
}

/// The WGSL module sources `module_specs` splices into `Spec`s.
struct Modules<'a> {
    simple_entry: &'a str,
    shadow_entry: &'a str,
    full_entry: &'a str,
    shared: &'a str,
    /// `shared` minus its `// cell-buffer` spans.
    shared_plain: &'a str,
    /// `shader` minus its `// long-gradient-stub` spans.
    shader_no_long: &'a str,
    /// `shader` minus its `// union-stub` spans only.
    shader_cell: &'a str,
    shader_cell_plain: &'a str,
    cell_glyph: &'a str,
    /// `shader` minus its `// union-stub`, `// cell-buffer` and
    /// `// image-source` spans.
    shader_long: &'a str,
    /// `shader` minus its `// union-stub`, `// cell-buffer` and
    /// `// long-gradient-stub` spans.
    shader_image: &'a str,
    /// `shader` minus all four marker families.
    shader_plain: &'a str,
    glyph: &'a str,
    composite: &'a str,
    union: &'a str,
    /// The `UNION_MAX_MEMBERS` const fragment.
    union_max: &'a str,
    blend: &'a str,
    present: &'a str,
    external: &'a str,
    native: &'a str,
    projective: &'a str,
    mip: &'a str,
    resolve: &'a str,
    reduce: &'a str,
    headroom: &'a str,
    shadow: &'a str,
    /// `space_convert.wgsl` behind the space fragment.
    space_convert: &'a str,
    coverage: &'a str,
    /// The Vulkan tile stage interfaces around the fragment bodies.
    tile: &'a str,
}

/// The tile composition families, `(module stem, engine VARIANT index)`:
/// both executors' modules share the attachment interfaces and differ
/// only in the engine spec source they are generated from. Used by the
/// Vulkan `specs` entries and the Apple metallib loop alike.
const TILE_FAMILIES: [(&str, usize); 5] = [
    ("engine_tile", 2),
    ("engine_tile_long", 4),
    ("engine_tile_image", 6),
    ("engine_tile_glyph", 7),
    ("engine_tile_composite", 8),
];

/// The fixed module set `render` creates: twelve paint/coverage specializations
/// of `shader.wgsl` plus `present`, `external`, `external_native`,
/// `projective`, `mip`, `resolve`, `reduce`, `shadow`, `coverage` and
/// `space_convert`.
/// The engine, external and projective modules share the `shared.wgsl`
/// prelude; the engine and projective tails share the `blend.wgsl`
/// compositing helpers.
/// Variant 3 (Union) additionally prepends `UNION_MAX_MEMBERS` and links
/// `union.wgsl` over `shader_no_long`; variant 4 (`LongGradient`) keeps the
/// `// long-gradient-stub` spans and variant 6 (`Image`) the
/// `// image-source` spans, the per-tap source texel conversion, each
/// without union; the plain variants drop every marker family, so their
/// modules carry neither the union fold and member-cap arrays, the binary
/// stop search nor the source conversion. Variant 5 (`Cell`), which only
/// GPU-routed rich paints use, keeps every span but the union's. GPU
/// ordinary paints retain the same cell-buffer reads with their simple,
/// glyph or full paint specialization, without source conversion or long
/// gradient code. Other variants drop the cell-buffer spans.
fn module_specs(m: &Modules<'_>) -> Vec<Spec> {
    let mut specs: Vec<Spec> = (0..12u32)
        .map(|variant| {
            let (cap, shared, shader, union) = match variant {
                3 => (m.union_max, m.shared, m.shader_no_long, m.union),
                4 => ("", m.shared_plain, m.shader_long, ""),
                5 => ("", m.shared, m.shader_cell, ""),
                6 => ("", m.shared_plain, m.shader_image, ""),
                7 => ("", m.shared_plain, m.glyph, ""),
                8 => ("", m.shared_plain, m.composite, ""),
                9 | 11 => ("", m.shared, m.shader_cell_plain, ""),
                10 => ("", m.shared, m.cell_glyph, ""),
                _ => ("", m.shared_plain, m.shader_plain, ""),
            };
            let paint_variant = match variant {
                9 => 0,
                10 => 7,
                11 => 2,
                _ => variant,
            };
            let entry = match paint_variant {
                0 => m.simple_entry,
                1 => m.shadow_entry,
                _ => m.full_entry,
            };
            Spec {
                name: format!("engine{variant}"),
                source: format!(
                    "const VARIANT: u32 = {paint_variant}u;\n{cap}{shared}\n{shader}\n{union}{}\n{entry}",
                    m.blend
                ),
                groups: bindings::ENGINE_GROUPS,
                programs: programs::ENGINE,
                metal: true,
                merge_pair: None,
                attachment: false,
            }
        })
        .collect();
    specs.push(Spec {
        name: "present".into(),
        source: m.present.into(),
        groups: bindings::PRESENT_GROUPS,
        programs: programs::MAIN,
        metal: true,
        merge_pair: None,
        attachment: false,
    });
    specs.push(Spec {
        name: "external".into(),
        source: format!("{}\n{}", m.shared, m.external),
        groups: bindings::EXTERNAL_GROUPS,
        programs: programs::EXTERNAL,
        metal: true,
        merge_pair: None,
        attachment: false,
    });
    // The Vulkan native module (issue #166): the same `fs_external` plus
    // `fs_external_format`, whose `ext_image`/`ext_sampler` pair lowers to
    // one combined sampled image at binding 5 — the binding a
    // `VkSamplerYcbcrConversion` immutable sampler occupies.
    specs.push(Spec {
        name: "external_native".into(),
        source: format!("{}\n{}\n{}", m.shared, m.external, m.native),
        groups: bindings::NATIVE_EXTERNAL_GROUPS,
        programs: &[],
        metal: false,
        merge_pair: Some((5, 6)),
        attachment: false,
    });
    specs.push(Spec {
        name: "projective".into(),
        source: format!("{}\n{}\n{}", m.shared, m.blend, m.projective),
        groups: bindings::PROJECTIVE_GROUPS,
        programs: programs::PROJECTIVE,
        metal: true,
        merge_pair: None,
        attachment: false,
    });
    helper_specs(m, &mut specs);
    // The Vulkan tile executor's composition modules (issue #2191): the
    // no-union variants 2, 4 and 6, like the Apple `engine_tile*`
    // metallibs, plus the tile stage interfaces. A union member samples
    // its group's capture, a read `ExecutionPlan::epoch` never admits
    // into a tile epoch.
    for (name, index) in TILE_FAMILIES {
        specs.push(Spec {
            name: name.into(),
            source: format!("{}\n{}", specs[index].source, m.tile),
            groups: bindings::ENGINE_GROUPS,
            programs: &[],
            metal: false,
            merge_pair: None,
            attachment: true,
        });
    }
    specs.push(Spec {
        name: "engine_tile_simple".into(),
        source: format!("{}\n{}", specs[0].source, m.tile),
        groups: bindings::ENGINE_GROUPS,
        programs: &[],
        metal: false,
        merge_pair: None,
        attachment: true,
    });
    specs
}

/// The single-program helper modules' specs.
fn helper_specs(m: &Modules<'_>, specs: &mut Vec<Spec>) {
    for (name, source, groups, programs) in [
        ("mip", m.mip, bindings::MIP_GROUPS, programs::MAIN),
        (
            "resolve",
            m.resolve,
            bindings::RESOLVE_GROUPS,
            programs::MAIN,
        ),
        ("reduce", m.reduce, bindings::REDUCE_GROUPS, programs::MAIN),
        (
            "headroom",
            m.headroom,
            bindings::HEADROOM_GROUPS,
            programs::MAIN,
        ),
        (
            "shadow",
            m.shadow,
            bindings::SHADOW_GROUPS,
            programs::SHADOW,
        ),
        (
            "space_convert",
            m.space_convert,
            bindings::SPACE_CONVERT_GROUPS,
            programs::SPACE_CONVERT,
        ),
        (
            "coverage",
            m.coverage,
            bindings::COVERAGE_GROUPS,
            programs::COVERAGE,
        ),
    ] {
        if name == "headroom"
            && metal::apple_target().is_none()
            && std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android")
        {
            continue;
        }
        specs.push(Spec {
            name: name.into(),
            source: source.into(),
            groups,
            programs,
            metal: true,
            merge_pair: None,
            attachment: false,
        });
    }
}

fn compile_specs(out_dir: &Path, specs: &[Spec]) {
    // wasm builds embed no passthrough artifacts, and Apple builds embed
    // no `.spv` (issue #241), so `xcrun` is required only on the targets
    // that consume its artifacts; the WGSL is still parsed and validated
    // here.
    let wasm = env::var("CARGO_CFG_TARGET_ARCH").unwrap() == "wasm32";
    let apple = metal::apple_target();
    let spirv = emits_spirv();
    // The crate reads the same condition as the `cherenkov_spirv` cfg, so
    // `build.rs` is the single place that decides it.
    println!("cargo::rustc-check-cfg=cfg(cherenkov_spirv)");
    if spirv {
        println!("cargo::rustc-cfg=cherenkov_spirv");
    }
    // Likewise `cherenkov_dxil` (issue #2726).
    println!("cargo::rustc-check-cfg=cfg(cherenkov_dxil)");
    let mut entries = proc_macro2::TokenStream::new();
    let mut dxil = dxil::emits_dxil().then(dxil::Modules::default);
    if dxil.is_some() {
        println!("cargo::rustc-cfg=cherenkov_dxil");
    }
    for spec in specs {
        compile(
            out_dir,
            spec,
            apple.as_ref(),
            wasm,
            spirv,
            &mut entries,
            dxil.as_mut(),
        );
    }
    std::fs::write(out_dir.join("entries.rs"), entries.to_string())
        .expect("write shader entry-point tables");
    if let Some(modules) = dxil {
        let table = modules.emit();
        std::fs::write(out_dir.join("dxil.rs"), table.to_string()).expect("write DXIL table");
    }
    if let Some(apple) = apple.as_ref() {
        // The tile executor's composition shaders. Only the stripped
        // no-union variants: a union member samples its group's capture,
        // a read `ExecutionPlan::epoch` never admits into a tile epoch.
        // `attachment` selects the shared scaffold, so each family's
        // generated .metal reaches attachment.metal through -include.
        for (name, index) in TILE_FAMILIES {
            compile(
                out_dir,
                &Spec {
                    name: name.into(),
                    source: specs[index].source.clone(),
                    groups: bindings::ENGINE_GROUPS,
                    programs: &[],
                    metal: true,
                    merge_pair: None,
                    attachment: true,
                },
                Some(apple),
                false,
                false,
                &mut entries,
                None,
            );
        }
        let fixture = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("tests/shaders/attachment_read.metal");
        std::io::Write::write_fmt(
            &mut std::io::stdout(),
            format_args!("cargo::rerun-if-changed={}\n", fixture.display()),
        )
        .unwrap();
        metal::compile_metal(
            out_dir,
            "attachment_read",
            &fixture,
            apple,
            (3, 0),
            None,
            &[],
        );
    }
}

/// Whether the `.spv` artifacts are emitted.
///
/// SPIR-V is the passthrough format of wgpu's Vulkan backend only, and
/// wgpu compiles that backend on exactly the non-Apple, non-wasm targets
/// (`windows`, `linux`, `android`, `freebsd`, `netbsd` in its
/// `wgpu-core-deps-windows-linux-android` manifest entry): Apple targets
/// get `.metallib`s and wgpu has no Vulkan backend for them unless the
/// non-default `vulkan-portability` feature is on (issue #241). This
/// crate's `wgpu` dependency is default-features, so `.spv` bytes are
/// unreachable on Apple and wasm.
fn emits_spirv() -> bool {
    env::var("CARGO_CFG_TARGET_ARCH").unwrap() != "wasm32"
        && env::var("CARGO_CFG_TARGET_VENDOR").unwrap() != "apple"
}

/// Parses, validates and compiles one module; with `dxil`, emits its
/// programs' DXIL and appends their table.
fn compile(
    out_dir: &Path,
    spec: &Spec,
    apple: Option<&AppleTarget>,
    wasm: bool,
    spirv: bool,
    entries: &mut proc_macro2::TokenStream,
    dxil: Option<&mut dxil::Modules>,
) {
    // Every consumer embeds this exact composed text — the runtime and
    // the writer tests read `<name>.wgsl` instead of re-assembling it.
    std::fs::write(out_dir.join(format!("{}.wgsl", spec.name)), &spec.source)
        .expect("write composed WGSL");
    let mut module = naga::front::wgsl::parse_str(&spec.source).unwrap_or_else(|e| {
        panic!(
            "{}: WGSL parse failed:\n{}",
            spec.name,
            e.emit_to_string(&spec.source)
        )
    });
    if spec.attachment {
        tile::attachment_inputs(&mut module);
    }
    let validated = Validator::new(ValidationFlags::all(), capabilities::ENGINE_CAPABILITIES)
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}: WGSL validation failed: {e:?}", spec.name));
    if spec.attachment && spirv {
        let fragment = match spec.name.as_str() {
            "engine_tile_simple" => Some("tile_simple"),
            "engine_tile_glyph" | "engine_tile_composite" => Some("tile_full"),
            _ => None,
        };
        module.entry_points.retain(|entry| {
            entry.name == "vs_main"
                || fragment.map_or_else(
                    || entry.name.starts_with("tile_") && entry.name != "tile_simple",
                    |name| entry.name == name,
                )
        });
    }
    // Metal's attachment wrapper calls helper functions directly; its roots
    // are outside the generated module. Other modules expose only entry points.
    let info = if !spec.attachment || spirv {
        naga::compact::compact(&mut module, naga::compact::KeepUnused::No);
        Validator::new(ValidationFlags::all(), capabilities::ENGINE_CAPABILITIES)
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}: compacted WGSL validation failed: {e:?}", spec.name))
    } else {
        validated
    };
    check_programs(spec, &module);
    if !spec.programs.is_empty() {
        dxil::write_entries(&spec.name, &module, &info, entries);
    }
    if wasm {
        return;
    }
    if spirv {
        write_spirv(out_dir, spec, &module, &info);
    }
    if spec.metal {
        write_metal(out_dir, spec, &module, apple);
    }
    if let Some(modules) = dxil.filter(|_| !spec.programs.is_empty()) {
        modules.collect(
            out_dir,
            &spec.name,
            spec.groups,
            spec.programs,
            (&module, &info),
        );
    }
}

/// Asserts `spec.programs` name exactly the module's entry points, each
/// with its stage.
fn check_programs(spec: &Spec, module: &naga::Module) {
    if spec.programs.is_empty() {
        return;
    }
    let mut named = Vec::new();
    for program in spec.programs {
        match *program {
            programs::Program::Render { vertex, fragment } => {
                named.push((vertex, naga::ShaderStage::Vertex));
                named.push((fragment, naga::ShaderStage::Fragment));
            }
            programs::Program::Compute { entry, .. } => {
                named.push((entry, naga::ShaderStage::Compute));
            }
        }
    }
    for &(name, stage) in &named {
        assert!(
            module
                .entry_points
                .iter()
                .any(|ep| ep.name == name && ep.stage == stage),
            "{}: no {stage:?} entry point `{name}`",
            spec.name
        );
    }
    for ep in &module.entry_points {
        assert!(
            named.contains(&(ep.name.as_str(), ep.stage)),
            "{}: entry point `{}` is in no program",
            spec.name,
            ep.name
        );
    }
}

/// wgpu-hal Vulkan maps a WGSL `@binding` to its entry's ordinal position
/// in the bind group layout; this SPIR-V must match.
fn binding_map(spec: &Spec, module: &naga::Module) -> spv::BindingMap {
    module
        .global_variables
        .iter()
        .filter_map(|(_, var)| var.binding)
        .map(|br| {
            let slot = bindings::vulkan_slot(spec.groups[br.group as usize], br.binding);
            (
                br,
                spv::BindingInfo {
                    descriptor_set: br.group,
                    binding: slot,
                    binding_array_size: None,
                },
            )
        })
        .collect()
}

fn write_spirv(out_dir: &Path, spec: &Spec, module: &naga::Module, info: &naga::valid::ModuleInfo) {
    let options = spv::Options {
        // SPIR-V 1.0 is legal for every Vulkan 1.x driver.
        lang_version: (1, 0),
        // The attachment pass finds the reads by name; it strips the
        // names again, so the written module matches the others.
        flags: if spec.attachment {
            spv_flags::SPIRV_WRITER_FLAGS | spv::WriterFlags::DEBUG
        } else {
            spv_flags::SPIRV_WRITER_FLAGS
        },
        fake_missing_bindings: false,
        binding_map: binding_map(spec, module),
        capabilities: None,
        bounds_check_policies: UNCHECKED,
        zero_initialize_workgroup_memory: spv::ZeroInitializeWorkgroupMemoryMode::Native,
        force_loop_bounding: false,
        ray_query_initialization_tracking: false,
        trace_ray_argument_validation: false,
        // naga 29 guarded integer division unconditionally; preserve it.
        emit_int_div_checks: true,
        use_storage_input_output_16: false,
        debug_info: None,
        task_dispatch_limits: None,
        mesh_shader_primitive_indices_clamp: false,
    };
    let mut writer = spv::Writer::new(&options)
        .unwrap_or_else(|e| panic!("{}: SPIR-V writer failed: {e:?}", spec.name));
    let mut words = Vec::new();
    writer
        .write(module, info, None, &None, &mut words)
        .unwrap_or_else(|e| panic!("{}: SPIR-V emission failed: {e:?}", spec.name));
    if let Some((texture, sampler)) = spec.merge_pair {
        merge_sampled_pair(&mut words, texture, sampler, &spec.name);
    }
    let write = |suffix: &str, words: &[u32]| {
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        // naga's emission ships as-is: spirv-opt was measured a net loss on
        // the device (issue #124) and the driver's compiler optimizes itself.
        std::fs::write(out_dir.join(format!("{}{suffix}.spv", spec.name)), bytes).unwrap();
    };
    if spec.attachment {
        // One module per input attachment mapping: a draw that sources
        // another slot reads it at index 1, a draw that sources its own
        // reads index 0 twice over.
        for (suffix, source_index) in [("", bindings::TILE_SOURCE_INDEX), ("_self", 0)] {
            write(
                suffix,
                &build_tile_spirv::attachment_reads(
                    &words,
                    bindings::TILE_INPUT_GROUP,
                    bindings::TILE_INPUT_BINDINGS,
                    source_index,
                    match spec.name.as_str() {
                        "engine_tile_simple" | "engine_tile_glyph" | "engine_tile_long" => {
                            [false, false]
                        }
                        "engine_tile_composite" => [false, true],
                        _ => [true, true],
                    },
                ),
            );
        }
        return;
    }
    write("", &words);
}

/// Emits the Metal source and, on Apple targets, the compiled library.
///
/// wgpu-hal passes a runtime-array-sizes buffer to naga-compiled modules,
/// but writes none for passthrough modules — and never calls `arrayLength`
/// sites exist in these shaders anyway. Emitting the storage arrays as
/// `array<T, 1>` keeps naga from putting a `_mslBufferSizes` argument in
/// the entry point signature, which would otherwise be left unbound.
/// The restricted lowering of `external_native` (issue #166): the
/// designated `(texture, sampler)` variable pair — `@binding(5)` and
/// `@binding(6)` in group 1 — merges into one `OpTypeSampledImage`
/// variable occupying the texture's descriptor slot, so a
/// `VkSamplerYcbcrConversion` immutable sampler can bind it.
///
/// Restricted means the pass rejects anything it does not precisely
/// model: a use of the pair's variables outside the `OpLoad` →
/// `OpSampledImage` → `OpImageSample*` chain (a fetch, an `OpImage`
/// disassembly, the sampler bound to a different texture) panics the
/// build rather than silently lowering wrong.
#[expect(clippy::too_many_lines, reason = "one SPIR-V rewriting pass")]
fn merge_sampled_pair(words: &mut Vec<u32>, binding_tex: u32, binding_smp: u32, name: &str) {
    use std::collections::{HashMap, HashSet};
    const OP_NAME: u32 = 5;
    const OP_ENTRY_POINT: u32 = 15;
    const OP_DECORATE: u32 = 71;
    const OP_TYPE_IMAGE: u32 = 25;
    const OP_TYPE_SAMPLER: u32 = 26;
    const OP_TYPE_SAMPLED_IMAGE: u32 = 27;
    const OP_TYPE_POINTER: u32 = 32;
    const OP_VARIABLE: u32 = 59;
    const OP_LOAD: u32 = 61;
    const OP_SAMPLED_IMAGE: u32 = 86;
    // Result-producing opcodes `uses_of` scans as consumers; other uses
    // start operand scanning after the result id anyway.
    const OP_FUNCTION_CALL: u32 = 57;
    const OP_STORE: u32 = 62;
    const OP_ACCESS_CHAIN: u32 = 65;
    const OP_IN_BOUNDS_ACCESS_CHAIN: u32 = 66;
    const OP_BITCAST: u32 = 124;
    const DECORATION_DESCRIPTOR_SET: u32 = 34;
    const DECORATION_BINDING: u32 = 33;
    const UNIFORM_CONSTANT: u32 = 0;

    // Pass 1: decorations and variables.
    let mut set_of: HashMap<u32, u32> = HashMap::new();
    let mut binding_of: HashMap<u32, u32> = HashMap::new();
    let mut variables: HashMap<u32, usize> = HashMap::new();
    let mut ty_of: HashMap<u32, u32> = HashMap::new();
    let mut i = 5usize;
    while i < words.len() {
        let wc = (words[i] >> 16) as usize;
        assert!(wc > 0 && i + wc <= words.len(), "{name}: truncated SPIR-V");
        let opcode = words[i] & 0xFFFF;
        if opcode == OP_DECORATE && words[i + 2] == DECORATION_BINDING {
            binding_of.insert(words[i + 1], words[i + 3]);
        } else if opcode == OP_DECORATE && words[i + 2] == DECORATION_DESCRIPTOR_SET {
            set_of.insert(words[i + 1], words[i + 3]);
        } else if opcode == OP_VARIABLE {
            variables.insert(words[i + 2], i);
        } else if matches!(
            opcode,
            OP_TYPE_POINTER | OP_TYPE_IMAGE | OP_TYPE_SAMPLER | OP_TYPE_SAMPLED_IMAGE
        ) {
            ty_of.insert(words[i + 1], opcode);
        }
        i += wc;
    }
    let var_at = |binding: u32, what: &str| -> usize {
        let mut found = None;
        for (&id, &at) in &variables {
            if binding_of.get(&id) == Some(&binding) && set_of.get(&id) == Some(&1) {
                assert!(
                    found.is_none(),
                    "{name}: more than one {what} at binding {binding}"
                );
                found = Some(at);
            }
        }
        found.unwrap_or_else(|| panic!("{name}: no {what} at binding {binding}"))
    };
    let tex_at = var_at(binding_tex, "texture");
    let smp_at = var_at(binding_smp, "sampler");
    let tex_var = words[tex_at + 2];
    let smp_var = words[smp_at + 2];

    // Pointee type of each variable's pointer type.
    let pointee = |at: usize| -> u32 {
        let ptr = words[at + 1];
        let ty = *ty_of.get(&ptr).expect("variable without a pointer type");
        assert!(
            ty == OP_TYPE_POINTER,
            "{name}: variable type is not a pointer"
        );
        let mut j = 5usize;
        while j < words.len() {
            let wc = (words[j] >> 16) as usize;
            if words[j] & 0xFFFF == OP_TYPE_POINTER && words[j + 1] == ptr {
                assert!(
                    words[j + 2] == UNIFORM_CONSTANT,
                    "{name}: the pair is not UniformConstant"
                );
                return words[j + 3];
            }
            j += wc;
        }
        panic!("{name}: unresolved pointer type");
    };
    let image_ty = pointee(tex_at);
    let smp_ty = pointee(smp_at);
    assert_eq!(
        ty_of.get(&image_ty),
        Some(&OP_TYPE_IMAGE),
        "{name}: binding {binding_tex} is not a texture"
    );
    assert_eq!(
        ty_of.get(&smp_ty),
        Some(&OP_TYPE_SAMPLER),
        "{name}: binding {binding_smp} is not a sampler"
    );

    // Pass 2: loads of the two variables, keyed by result id.
    let mut tex_loads: HashSet<u32> = HashSet::new();
    let mut smp_loads: HashSet<u32> = HashSet::new();
    i = 5;
    while i < words.len() {
        let wc = (words[i] >> 16) as usize;
        if words[i] & 0xFFFF == OP_LOAD && words[i + 3] == tex_var {
            tex_loads.insert(words[i + 2]);
        } else if words[i] & 0xFFFF == OP_LOAD && words[i + 3] == smp_var {
            smp_loads.insert(words[i + 2]);
        }
        i += wc;
    }

    // Pass 3: every use must sit inside `OpLoad → OpSampledImage`; the
    // loads themselves must feed only an OpSampledImage. The scan only
    // inspects opcodes that can consume a resource id — decorations and
    // entry points carry operands that are literals or the ids' own
    // declarations, not uses.
    let uses_of = |id: u32| -> Vec<u32> {
        let mut out = Vec::new();
        let mut j = 5usize;
        while j < words.len() {
            let wc = (words[j] >> 16) as usize;
            let opcode = words[j] & 0xFFFF;
            let consumes = matches!(
                opcode,
                OP_FUNCTION_CALL
                    | OP_VARIABLE
                    | OP_LOAD
                    | OP_STORE
                    | OP_ACCESS_CHAIN
                    | OP_IN_BOUNDS_ACCESS_CHAIN
                    | OP_BITCAST
                    | OP_SAMPLED_IMAGE
                    | 55 // OpArrayLength
                    | 81 // OpGenericCastToPtr
                    | 82 // OpGenericCastToPtrExplicit
                    | 87..=104 // OpImageSample* / OpImage* reads
            );
            // Operand start: OpStore carries no result, every other
            // consumer above writes result-type + result-id first.
            let start = j + if opcode == OP_STORE { 1 } else { 3 };
            if consumes && start <= j + wc && words[start..j + wc].contains(&id) {
                out.push(opcode);
            }
            j += wc;
        }
        out
    };
    for (var, what) in [(tex_var, "texture"), (smp_var, "sampler")] {
        for opcode in uses_of(var) {
            assert!(
                opcode == OP_LOAD || opcode == OP_VARIABLE,
                "{name}: unexpected use of the {what} variable (opcode {opcode})"
            );
        }
    }
    for &load in tex_loads.iter().chain(smp_loads.iter()) {
        for opcode in uses_of(load) {
            assert!(
                opcode == OP_SAMPLED_IMAGE,
                "{name}: a load of the designated pair feeds opcode {opcode}"
            );
        }
    }
    // Every OpSampledImage must pair a tex load with an smp load.
    i = 5;
    while i < words.len() {
        let wc = (words[i] >> 16) as usize;
        if words[i] & 0xFFFF == OP_SAMPLED_IMAGE {
            let (a, b) = (words[i + 3], words[i + 4]);
            assert!(
                (tex_loads.contains(&a) && smp_loads.contains(&b))
                    || (tex_loads.contains(&b) && smp_loads.contains(&a)),
                "{name}: OpSampledImage does not combine the designated pair"
            );
        }
        i += wc;
    }

    // Reuse an existing `OpTypeSampledImage %image_ty` when naga already
    // emitted one; fresh ids come from the bound otherwise. naga may
    // declare the sampled-image type after the global variables; the
    // rewritten variable needs its pointer type defined first, so the
    // existing declaration is moved up to just before the first variable
    // and the pointer declared with it.
    let mut ty_si = 0u32;
    let mut si_at = usize::MAX;
    let mut ty_ptr = 0u32;
    let mut first_var = usize::MAX;
    let mut j = 5usize;
    while j < words.len() {
        let wc = (words[j] >> 16) as usize;
        let opcode = words[j] & 0xFFFF;
        if opcode == OP_VARIABLE && first_var == usize::MAX {
            first_var = j;
        }
        if opcode == OP_TYPE_SAMPLED_IMAGE && words[j + 2] == image_ty {
            ty_si = words[j + 1];
            si_at = j;
        }
        if opcode == OP_TYPE_POINTER
            && words[j + 2] == UNIFORM_CONSTANT
            && ty_si != 0
            && words[j + 3] == ty_si
        {
            ty_ptr = words[j + 1];
        }
        j += wc;
    }
    let bound = words[3];
    let mut next = bound;
    let mut new_types: Vec<u32> = Vec::new();
    if ty_si == 0 {
        ty_si = next;
        next += 1;
        new_types.extend_from_slice(&[(3 << 16) | OP_TYPE_SAMPLED_IMAGE, ty_si, image_ty]);
    } else {
        new_types.extend_from_slice(&words[si_at..si_at + 3]);
    }
    if ty_ptr == 0 {
        ty_ptr = next;
        next += 1;
        new_types.extend_from_slice(&[
            (4 << 16) | OP_TYPE_POINTER,
            ty_ptr,
            UNIFORM_CONSTANT,
            ty_si,
        ]);
    }
    words[3] = next;

    // Rebuild the stream.
    let mut out = Vec::with_capacity(words.len() + 8);
    i = 0;
    while i < words.len() {
        let wc = (words[i] >> 16) as usize;
        let opcode = words[i] & 0xFFFF;
        let wc = if i == 0 { 5 } else { wc };
        // The five header words are copied verbatim.
        if i < 5 {
            out.extend_from_slice(&words[i..5]);
            i = 5;
            continue;
        }
        if opcode == OP_SAMPLED_IMAGE {
            let (img, smp) = (words[i + 3], words[i + 4]);
            assert!(
                tex_loads.contains(&img) && smp_loads.contains(&smp),
                "{name}: a sampled image does not use the designated pair"
            );
            out.push((4 << 16) | OP_LOAD);
            out.push(words[i + 1]);
            out.push(words[i + 2]);
            out.push(tex_var);
            i += wc;
            continue;
        }
        if opcode == OP_LOAD
            && (smp_loads.contains(&words[i + 2]) || tex_loads.contains(&words[i + 2]))
        {
            // Both loads of the pair die with the OpSampledImage they fed.
            i += wc;
            continue;
        }
        if opcode == OP_VARIABLE && words[i + 2] == smp_var {
            i += wc;
            continue;
        }
        if (opcode == OP_DECORATE || opcode == OP_NAME) && words[i + 1] == smp_var {
            i += wc;
            continue;
        }
        if opcode == OP_ENTRY_POINT && words[i + 1..i + wc].contains(&smp_var) {
            // Drop the deleted sampler variable from the entry point's
            // interface list; the literal string may share its value.
            let mut end = i + 3;
            while !words[end].to_le_bytes().contains(&0) {
                end += 1;
            }
            let interface: Vec<u32> = words[end + 1..i + wc]
                .iter()
                .copied()
                .filter(|&id| id != smp_var)
                .collect();
            out.push(
                u32::try_from(3 + (end - i - 2) + interface.len()).expect("op word count") << 16
                    | opcode,
            );
            out.extend_from_slice(&words[i + 1..=end]);
            out.extend_from_slice(&interface);
            i += wc;
            continue;
        }
        if i == tex_at {
            out.push(words[i]);
            out.push(ty_ptr);
            out.extend_from_slice(&words[i + 2..i + wc]);
            i += wc;
            continue;
        }
        if i == si_at {
            i += wc;
            continue;
        }
        if i == first_var && !new_types.is_empty() {
            out.extend_from_slice(&new_types);
        }
        out.extend_from_slice(&words[i..i + wc]);
        i += wc;
    }
    *words = out;
}

fn write_metal(out_dir: &Path, spec: &Spec, module: &naga::Module, apple: Option<&AppleTarget>) {
    let module = pin_runtime_arrays(module);
    let info = Validator::new(ValidationFlags::all(), capabilities::ENGINE_CAPABILITIES)
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}: WGSL validation failed: {e:?}", spec.name));
    let options = msl::Options {
        // Metal 2.3 is the first language version that takes
        // `-ftexture-write-rounding-mode` (see `compile_metal`); every
        // supported deployment target (macOS and iOS 26) runs it. The
        // shader test covers the same value.
        lang_version: if spec.attachment { (3, 0) } else { (2, 3) },
        per_entry_point_map: resource_map(spec, &module),
        inline_samplers: Vec::new(),
        spirv_cross_compatibility: false,
        fake_missing_bindings: false,
        bounds_check_policies: UNCHECKED,
        zero_initialize_workgroup_memory: false,
        force_loop_bounding: false,
        task_dispatch_limits: None,
        mesh_shader_primitive_indices_clamp: false,
        ray_query_initialization_tracking: false,
        // Preserve naga 29's integer division semantics.
        emit_int_div_checks: true,
    };
    // The engine's pipelines declare no vertex buffers, so wgpu-hal's
    // `vertex_pulling_transform` never applies; it is still passed so the
    // emission matches the hal's option set.
    let pipeline_options = msl::PipelineOptions {
        entry_point: None,
        allow_and_force_point_size: false,
        vertex_pulling_transform: true,
        vertex_buffer_mappings: Vec::new(),
        binding_array_length_map: naga::FastHashMap::default(),
    };
    let (source, translation_info) = msl::write_string(&module, &info, &options, &pipeline_options)
        .unwrap_or_else(|e| panic!("{}: MSL emission failed: {e:?}", spec.name));
    // wgpu-hal looks up pipeline functions by `entry_point` name verbatim,
    // so the emitted names must be the WGSL names.
    let emitted: Vec<&str> = translation_info
        .entry_point_names
        .iter()
        .map(|name| name.as_deref().unwrap_or("<error>"))
        .collect();
    let expected: Vec<&str> = module
        .entry_points
        .iter()
        .map(|ep| ep.name.as_str())
        .collect();
    assert_eq!(
        emitted, expected,
        "{}: MSL entry point names differ from the WGSL names",
        spec.name
    );

    let metal = out_dir.join(format!("{}.metal", spec.name));
    std::fs::write(&metal, &source).unwrap();

    if let Some(apple) = apple {
        // Attachment-interface modules compile the shared scaffold with
        // their own generated body preincluded; every other module
        // compiles its own emission.
        let (input, preinclude) = if spec.attachment {
            let scaffold = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("src/render/composite/attachment.metal");
            println!("cargo::rerun-if-changed={}", scaffold.display());
            (scaffold, Some(metal))
        } else {
            (metal, None)
        };
        // Only a family whose module keeps the `// image-source` spans has
        // the decode table among its `fs_full` parameters; the scaffold
        // declares and forwards it under this define, at the fragment
        // buffer index the binding plan gives the table.
        let mut defines = if spec.attachment && spec.source.contains(IMAGE_SOURCE_NAMES[0]) {
            let plan = bindings::metal_plan(spec.groups, bindings::FRAGMENT);
            let buffer = plan
                .targets
                .iter()
                .find(|&&(group, binding, _)| {
                    group == 0 && binding == bindings::IMAGE_DECODE_BINDING
                })
                .and_then(|&(_, _, target)| target.buffer)
                .unwrap_or_else(|| panic!("{}: the image decode table has no buffer", spec.name));
            vec![
                "TILE_IMAGE_SOURCE".to_owned(),
                format!("TILE_IMAGE_DECODE_BUFFER={buffer}"),
            ]
        } else {
            Vec::new()
        };
        if spec.attachment {
            match spec.name.as_str() {
                "engine_tile_long" => defines.push("TILE_LONG_GRADIENT".to_owned()),
                "engine_tile_glyph" => defines.push("TILE_GLYPH".to_owned()),
                "engine_tile_composite" => defines.push("TILE_COMPOSITE_SOURCE_OVER".to_owned()),
                _ => {}
            }
        }
        metal::compile_metal(
            out_dir,
            &spec.name,
            &input,
            apple,
            options.lang_version,
            preinclude.as_deref(),
            &defines,
        );
    }
}

/// Pins each runtime-sized `array<T>` to `array<T, 1>` in a cloned module.
/// Metal cannot express runtime array types; naga's own backend uses the
/// bound-1 form plus a sizes argument, and the pinned form alone is what a
/// passthrough signature needs.
fn pin_runtime_arrays(module: &naga::Module) -> naga::Module {
    let mut module = module.clone();
    let dynamic: Vec<_> = module
        .types
        .iter()
        .filter(|(_, ty)| {
            matches!(
                ty.inner,
                naga::TypeInner::Array {
                    size: naga::ArraySize::Dynamic,
                    ..
                }
            )
        })
        .map(|(handle, _)| handle)
        .collect();
    for handle in dynamic {
        let mut ty = module.types.get_handle(handle).unwrap().clone();
        // `base` and `stride` are preserved; only the bound is pinned.
        if let naga::TypeInner::Array { ref mut size, .. } = ty.inner {
            *size = naga::ArraySize::Constant(NonZeroU32::new(1).unwrap());
        }
        module.types.replace(handle, ty);
    }
    module
}

/// The argument slots naga must emit, reproducing wgpu-hal's
/// `create_pipeline_layout` assignment: per-stage counters over the groups
/// in order.
fn resource_map(spec: &Spec, module: &naga::Module) -> msl::EntryPointResourceMap {
    let mut map = msl::EntryPointResourceMap::new();
    for ep in &module.entry_points {
        let stage = match ep.stage {
            naga::ShaderStage::Vertex => bindings::VERTEX,
            naga::ShaderStage::Fragment => bindings::FRAGMENT,
            naga::ShaderStage::Compute => bindings::COMPUTE,
            other => panic!("{}: unsupported shader stage {other:?}", spec.name),
        };
        let plan = bindings::metal_plan(spec.groups, stage);
        let resources = plan
            .targets
            .iter()
            .map(|&(group, binding, target)| {
                (
                    naga::ResourceBinding { group, binding },
                    msl::BindTarget {
                        buffer: target.buffer,
                        texture: target.texture,
                        sampler: target.sampler.map(msl::BindSamplerTarget::Resource),
                        external_texture: None,
                        mutable: target.mutable,
                    },
                )
            })
            .collect();
        map.insert(
            ep.name.clone(),
            msl::EntryPointResources {
                resources,
                immediates_buffer: None,
                sizes_buffer: plan.sizes_buffer,
            },
        );
    }
    map
}
