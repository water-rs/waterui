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
mod deployment;

#[path = "build_tile.rs"]
mod tile;

use std::env;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::Command;

use naga::back::{msl, spv};
use naga::valid::{Capabilities, ValidationFlags, Validator};

/// One fixed engine module to compile.
struct Spec {
    /// Output file stem, e.g. `engine0`.
    name: String,
    /// WGSL text (`VARIANT` already prepended for the engine shader).
    source: String,
    /// The pipeline's bind group layouts, in group order.
    groups: &'static [&'static [bindings::Entry]],
    /// Emit the `.metal` artifact; false for the Vulkan-only native
    /// external module.
    metal: bool,
    /// The `(texture, sampler)` group-1 WGSL bindings the restricted
    /// lowering merges into one combined sampled image at the texture's
    /// binding (issue #166).
    merge_pair: Option<(u32, u32)>,
}

/// Apple shader tools and the deployment target resolved by rustc.
struct AppleTarget {
    sdk: &'static str,
    deployment_variable: &'static str,
    deployment_version: String,
}

/// Passthrough shaders carry no naga runtime checks: the source is
/// controlled and validated here at build time.
const UNCHECKED: naga::proc::BoundsCheckPolicies = naga::proc::BoundsCheckPolicies {
    index: naga::proc::BoundsCheckPolicy::Unchecked,
    buffer: naga::proc::BoundsCheckPolicy::Unchecked,
    image_load: naga::proc::BoundsCheckPolicy::Unchecked,
    binding_array: naga::proc::BoundsCheckPolicy::Unchecked,
};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let shader_path = manifest.join("src/render/shader.wgsl");
    let present_path = manifest.join("src/render/present.wgsl");
    let shared_path = manifest.join("src/render/shared.wgsl");
    let external_path = manifest.join("src/render/external.wgsl");
    let blend_path = manifest.join("src/render/blend.wgsl");
    let projective_path = manifest.join("src/render/projective.wgsl");
    let mip_path = manifest.join("src/render/mip.wgsl");
    for path in [
        &shader_path,
        &present_path,
        &shared_path,
        &external_path,
        &blend_path,
        &projective_path,
        &mip_path,
    ] {
        println!("cargo::rerun-if-changed={}", path.display());
    }
    println!(
        "cargo::rerun-if-changed={}",
        manifest.join("src/render/bindings.rs").display()
    );

    let shared = std::fs::read_to_string(&shared_path)
        .unwrap_or_else(|e| panic!("{}: {e}", shared_path.display()));
    let shader = std::fs::read_to_string(&shader_path)
        .unwrap_or_else(|e| panic!("{}: {e}", shader_path.display()));
    let present = std::fs::read_to_string(&present_path)
        .unwrap_or_else(|e| panic!("{}: {e}", present_path.display()));
    let external = std::fs::read_to_string(&external_path)
        .unwrap_or_else(|e| panic!("{}: {e}", external_path.display()));
    let blend = std::fs::read_to_string(&blend_path)
        .unwrap_or_else(|e| panic!("{}: {e}", blend_path.display()));
    let projective = std::fs::read_to_string(&projective_path)
        .unwrap_or_else(|e| panic!("{}: {e}", projective_path.display()));
    let mip = std::fs::read_to_string(&mip_path)
        .unwrap_or_else(|e| panic!("{}: {e}", mip_path.display()));
    let native_path = manifest.join("src/render/external_native.wgsl");
    println!("cargo::rerun-if-changed={}", native_path.display());
    let native = std::fs::read_to_string(&native_path)
        .unwrap_or_else(|e| panic!("{}: {e}", native_path.display()));

    // The three VARIANT specializations of shader.wgsl plus present.wgsl,
    // external.wgsl, projective.wgsl and mip.wgsl — the fixed module set
    // `render` creates. The engine, external and projective modules share
    // the `shared.wgsl` prelude; the engine and projective tails share the
    // `blend.wgsl` compositing helpers.
    let mut specs: Vec<Spec> = (0..3u32)
        .map(|variant| Spec {
            name: format!("engine{variant}"),
            source: format!("const VARIANT: u32 = {variant}u;\n{shared}\n{shader}\n{blend}"),
            groups: bindings::ENGINE_GROUPS,
            metal: true,
            merge_pair: None,
        })
        .collect();
    specs.push(Spec {
        name: "present".into(),
        source: present,
        groups: bindings::PRESENT_GROUPS,
        metal: true,
        merge_pair: None,
    });
    specs.push(Spec {
        name: "external".into(),
        source: format!("{shared}\n{external}"),
        groups: bindings::EXTERNAL_GROUPS,
        metal: true,
        merge_pair: None,
    });
    // The Vulkan native module (issue #166): the same `fs_external` plus
    // `fs_external_format`, whose `ext_image`/`ext_sampler` pair lowers to
    // one combined sampled image at binding 5 — the binding a
    // `VkSamplerYcbcrConversion` immutable sampler occupies.
    specs.push(Spec {
        name: "external_native".into(),
        source: format!("{shared}\n{external}\n{native}"),
        groups: bindings::NATIVE_EXTERNAL_GROUPS,
        metal: false,
        merge_pair: Some((5, 6)),
    });
    specs.push(Spec {
        name: "projective".into(),
        source: format!("{shared}\n{blend}\n{projective}"),
        groups: bindings::PROJECTIVE_GROUPS,
        metal: true,
        merge_pair: None,
    });
    specs.push(Spec {
        name: "mip".into(),
        source: mip,
        groups: bindings::MIP_GROUPS,
        metal: true,
        merge_pair: None,
    });

    compile_specs(&out_dir, &specs);
}

fn compile_specs(out_dir: &Path, specs: &[Spec]) {
    // wasm builds embed no passthrough artifacts, and Apple builds embed
    // no `.spv` (issue #241), so `xcrun` is required only on the targets
    // that consume its artifacts; the WGSL is still parsed and validated
    // here.
    let wasm = env::var("CARGO_CFG_TARGET_ARCH").unwrap() == "wasm32";
    let apple = apple_target();
    let spirv = emits_spirv();
    // The crate reads the same condition as the `cherenkov_spirv` cfg, so
    // `build.rs` is the single place that decides it.
    println!("cargo::rustc-check-cfg=cfg(cherenkov_spirv)");
    if spirv {
        println!("cargo::rustc-cfg=cherenkov_spirv");
    }
    for spec in specs {
        compile(out_dir, spec, apple.as_ref(), wasm, spirv);
    }
    if let Some(apple) = apple.as_ref() {
        let spec = Spec {
            name: "engine_tile".into(),
            source: specs[2].source.clone(),
            groups: bindings::ENGINE_GROUPS,
            metal: true,
            merge_pair: None,
        };
        compile(out_dir, &spec, Some(apple), false, false);
        let fixture = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("tests/shaders/attachment_read.metal");
        std::io::Write::write_fmt(
            &mut std::io::stdout(),
            format_args!("cargo::rerun-if-changed={}\n", fixture.display()),
        )
        .unwrap();
        compile_metal(out_dir, "attachment_read", &fixture, apple, (3, 0));
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

/// Parses, validates and compiles one module.
fn compile(out_dir: &Path, spec: &Spec, apple: Option<&AppleTarget>, wasm: bool, spirv: bool) {
    let mut module = naga::front::wgsl::parse_str(&spec.source).unwrap_or_else(|e| {
        panic!(
            "{}: WGSL parse failed:\n{}",
            spec.name,
            e.emit_to_string(&spec.source)
        )
    });
    if spec.name == "engine_tile" {
        tile::attachment_inputs(&mut module);
    }
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}: WGSL validation failed: {e:?}", spec.name));
    if wasm {
        return;
    }
    if spirv {
        write_spirv(out_dir, spec, &module, &info);
    }
    if spec.metal {
        write_metal(out_dir, spec, &module, apple);
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
        flags: spv::WriterFlags::empty(),
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
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    // naga's emission ships as-is: spirv-opt was measured a net loss on the
    // device (issue #124) and the driver's compiler optimizes itself.
    std::fs::write(out_dir.join(format!("{}.spv", spec.name)), bytes).unwrap();
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
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}: WGSL validation failed: {e:?}", spec.name));
    let options = msl::Options {
        // The floor Metal language version wgpu selects on supported
        // hardware; the shader test covers the same value.
        lang_version: if spec.name == "engine_tile" {
            (3, 0)
        } else {
            (2, 0)
        },
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
        let input = if spec.name == "engine_tile" {
            let scaffold = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("src/render/composite/attachment.metal");
            println!("cargo::rerun-if-changed={}", scaffold.display());
            scaffold
        } else {
            metal
        };
        compile_metal(out_dir, &spec.name, &input, apple, options.lang_version);
    }
}

fn compile_metal(out_dir: &Path, name: &str, input: &Path, apple: &AppleTarget, version: (u8, u8)) {
    let sdk = apple.sdk;
    let air = out_dir.join(format!("{name}.air"));
    let metallib = out_dir.join(format!("{name}.metallib"));
    // Metal 3 unified the platform-specific language dialects.
    let dialect = if version >= (3, 0) {
        "metal"
    } else if sdk == "macosx" {
        "macos-metal"
    } else {
        "ios-metal"
    };
    run(
        Command::new("xcrun")
            .env(apple.deployment_variable, &apple.deployment_version)
            .args(["-sdk", sdk, "metal", "-c", "-o"])
            .arg(&air)
            // Match the language naga emitted instead of inheriting
            // the build SDK's newest version, which older supported
            // operating systems cannot load.
            .arg(format!("-std={dialect}{}.{}", version.0, version.1))
            .arg("-I")
            .arg(out_dir)
            .arg(input),
        name,
        "xcrun metal is required to build cherenkov-gpu for Apple \
             targets: engine shaders are precompiled (issue #57).",
    );
    run(
        Command::new("xcrun")
            .env(apple.deployment_variable, &apple.deployment_version)
            .args(["-sdk", sdk, "metallib", "-o"])
            .arg(&metallib)
            .arg(&air),
        name,
        "xcrun metallib is required to build cherenkov-gpu for Apple \
             targets: engine shaders are precompiled (issue #57).",
    );
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

/// The Metal tool configuration for an Apple target, or `None` for other targets. A
/// non-Apple host cannot produce a `.metallib`, so building for Apple there
/// is an explicit error — never a silent WGSL fallback.
fn apple_target() -> Option<AppleTarget> {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let (sdk, deployment_variable) = match target_os.as_str() {
        "macos" => ("macosx", "MACOSX_DEPLOYMENT_TARGET"),
        "ios" if target_env == "sim" => ("iphonesimulator", "IPHONEOS_DEPLOYMENT_TARGET"),
        "ios" => ("iphoneos", "IPHONEOS_DEPLOYMENT_TARGET"),
        "tvos" | "watchos" | "visionos" => panic!(
            "cherenkov-gpu precompiles Metal shaders for Apple targets (issue \
             #57): no Metal SDK mapping exists for target-os {target_os}"
        ),
        _ => return None,
    };
    let target = env::var("TARGET").unwrap();
    let host = env::var("HOST").unwrap();
    assert!(
        host.ends_with("apple-darwin"),
        "cherenkov-gpu precompiles Metal shaders with xcrun (issue #57): \
         building for {target} needs an Apple host, but the host is {host}. \
         Build for Apple targets on macOS."
    );
    println!("cargo::rerun-if-env-changed={deployment_variable}");
    let output = Command::new(env::var_os("RUSTC").expect("Cargo provides RUSTC"))
        .args(["--print", "deployment-target", "--target", &target])
        .output()
        .expect("rustc must report the Apple deployment target");
    assert!(
        output.status.success(),
        "rustc could not resolve the deployment target for {target}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let resolved = String::from_utf8(output.stdout).expect("rustc deployment target is UTF-8");
    let deployment_version = resolved
        .trim()
        .strip_prefix(&format!("{deployment_variable}="))
        .expect("rustc reports the target platform's deployment variable")
        .to_owned();
    deployment::require_floor(&deployment_version);
    Some(AppleTarget {
        sdk,
        deployment_variable,
        deployment_version,
    })
}

/// Runs `command`, failing the build with `hint` when the tool is missing
/// and with the tool's own output when it fails.
fn run(command: &mut Command, name: &str, hint: &str) {
    let output = match command.output() {
        Ok(output) => output,
        Err(e) => panic!(
            "{name}: {hint}\nspawning {}: {e}",
            command.get_program().display()
        ),
    };
    assert!(
        output.status.success(),
        "{name}: {} failed ({})\nstdout:\n{}\nstderr:\n{}",
        command.get_program().display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
