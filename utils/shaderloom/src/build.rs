//! Build-time compilation of WGSL into backend-native embedded artifacts.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use naga::back::{hlsl, msl, spv};
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use naga::{AddressSpace, Handle, ImageClass, ImageDimension, Module, ShaderStage, TypeInner};

/// Compiles a complete WGSL module read from `source_path`.
///
/// The generated Rust expression is written to `<OUT_DIR>/<artifact_name>.rs`.
/// Native artifacts are emitted for every backend selectable on the Cargo
/// target: `MetalLib` on Apple, DXIL and SPIR-V on Windows, and SPIR-V on other
/// non-Web targets. WebGPU and GLES use the validated WGSL emitted alongside
/// the native artifacts because those APIs have no portable offline binary.
///
/// # Panics
///
/// Panics when the source is outside the package, cannot be read, is invalid
/// WGSL, or cannot be compiled for one of the target's native backends.
pub fn compile_wgsl_shader(source_path: impl AsRef<Path>, artifact_name: &str) {
    let manifest_dir = PathBuf::from(required_env("CARGO_MANIFEST_DIR"));
    let source_path = if source_path.as_ref().is_absolute() {
        source_path.as_ref().to_path_buf()
    } else {
        manifest_dir.join(source_path)
    };
    writeln!(
        std::io::stdout().lock(),
        "cargo:rerun-if-changed={}",
        source_path.display()
    )
    .expect("failed to emit Cargo shader source tracking directive");
    let source = fs::read_to_string(&source_path).unwrap_or_else(|error| {
        panic!(
            "failed to read shader source {}: {error}",
            source_path.display()
        )
    });
    let label = source_path.strip_prefix(&manifest_dir).unwrap_or_else(|_| {
        panic!(
            "shader source {} must be inside the package at {}",
            source_path.display(),
            manifest_dir.display()
        )
    });
    compile_wgsl_source(
        label
            .to_str()
            .expect("shader source path must contain valid UTF-8"),
        &source,
        artifact_name,
    );
}

/// Compiles an already-composed complete WGSL module.
///
/// # Panics
///
/// Panics when the source is invalid WGSL, the artifact name is invalid, or
/// the shader cannot be compiled for one of the target's native backends.
pub fn compile_wgsl_source(label: &str, source: &str, artifact_name: &str) {
    assert_artifact_name(artifact_name);

    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("WGSL parse error in {label}: {error}"));
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|error| panic!("WGSL validation error in {label}: {error}"));
    let reflection = ShaderReflection::new(&module, &info);

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    fs::write(out_dir.join(format!("{artifact_name}.wgsl")), source)
        .unwrap_or_else(|error| panic!("failed to write generated WGSL for {label}: {error}"));

    let target_arch = required_env("CARGO_CFG_TARGET_ARCH");
    let target_os = required_env("CARGO_CFG_TARGET_OS");
    let target_vendor = required_env("CARGO_CFG_TARGET_VENDOR");

    if target_arch != "wasm32" && target_vendor != "apple" {
        compile_spirv(
            &module,
            &info,
            &out_dir.join(format!("{artifact_name}.spv")),
            label,
        );
    }
    if target_vendor == "apple" {
        compile_metallib(
            &module,
            &info,
            &reflection,
            &out_dir,
            artifact_name,
            label,
            &target_os,
        );
    }
    if target_os == "windows" {
        compile_dxil_entry_points(&module, &info, &reflection, &out_dir, artifact_name, label);
    }

    let rust = reflection.rust_expression(label, artifact_name);
    fs::write(out_dir.join(format!("{artifact_name}.rs")), rust).unwrap_or_else(|error| {
        panic!("failed to write generated Rust shader expression for {label}: {error}")
    });
}

fn assert_artifact_name(name: &str) {
    assert!(
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "shader artifact name must contain only ASCII letters, digits, or underscores"
    );
}

fn required_env(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} must be set for shader compilation"))
}

fn compile_spirv(module: &Module, info: &ModuleInfo, output: &Path, label: &str) {
    let words = spv::write_vec(module, info, &spv::Options::default(), None)
        .unwrap_or_else(|error| panic!("SPIR-V generation failed for {label}: {error}"));
    let bytes = words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<_>>();
    fs::write(output, bytes)
        .unwrap_or_else(|error| panic!("failed to write SPIR-V for {label}: {error}"));
}

fn compile_metallib(
    module: &Module,
    info: &ModuleInfo,
    reflection: &ShaderReflection,
    out_dir: &Path,
    artifact_name: &str,
    label: &str,
    target_os: &str,
) {
    let mut options = reflection.msl_options();
    let mut versions = MSL_LANGUAGE_VERSIONS.iter().copied();
    let (source, translation, language_version) = loop {
        let version = versions.next().unwrap_or_else(|| {
            panic!("MSL generation failed for {label}: no supported Metal language version")
        });
        options.lang_version = version;
        match msl::write_string(module, info, &options, &msl::PipelineOptions::default()) {
            Ok((source, translation)) => break (source, translation, version),
            Err(error) if is_msl_version_error(&error) => {}
            Err(error) => panic!("MSL generation failed for {label}: {error}"),
        }
    };

    let translated_names = translation
        .entry_point_names
        .into_iter()
        .map(|name| {
            name.unwrap_or_else(|error| {
                panic!("MSL entry-point translation failed for {label}: {error}")
            })
        })
        .collect::<Vec<_>>();
    reflection.set_metal_names(&translated_names);

    let source_path = out_dir.join(format!("{artifact_name}.metal"));
    let air_path = out_dir.join(format!("{artifact_name}.air"));
    let metallib_path = out_dir.join(format!("{artifact_name}.metallib"));
    fs::write(&source_path, source)
        .unwrap_or_else(|error| panic!("failed to write generated MSL for {label}: {error}"));

    let sdk = apple_sdk(target_os);
    let standard = metal_language_standard(target_os, language_version);
    run_tool(
        Command::new("xcrun")
            .args(["--sdk", sdk, "metal", "-c"])
            .arg(standard)
            .arg(&source_path)
            .arg("-o")
            .arg(&air_path),
        "Metal compilation",
        label,
    );
    run_tool(
        Command::new("xcrun")
            .args(["--sdk", sdk, "metallib"])
            .arg(&air_path)
            .arg("-o")
            .arg(&metallib_path),
        "Metal library linking",
        label,
    );
}

fn metal_language_standard(target_os: &str, version: (u8, u8)) -> String {
    let prefix = if version.0 >= 3 {
        "metal"
    } else if target_os == "macos" {
        "macos-metal"
    } else {
        "ios-metal"
    };
    format!("-std={prefix}{}.{}", version.0, version.1)
}

const MSL_LANGUAGE_VERSIONS: &[(u8, u8)] = &[
    (1, 0),
    (1, 1),
    (1, 2),
    (2, 0),
    (2, 1),
    (2, 2),
    (2, 3),
    (2, 4),
    (3, 0),
    (3, 1),
    (3, 2),
];

const fn is_msl_version_error(error: &msl::Error) -> bool {
    matches!(
        error,
        msl::Error::UnsupportedAttribute(_)
            | msl::Error::UnsupportedFunction(_)
            | msl::Error::UnsupportedWritableStorageBuffer
            | msl::Error::UnsupportedWritableStorageTexture(_)
            | msl::Error::UnsupportedRWStorageTexture
            | msl::Error::UnsupportedArrayOf(_)
            | msl::Error::UnsupportedRayTracing
            | msl::Error::UnsupportedCooperativeMatrix
    )
}

fn apple_sdk(target_os: &str) -> &'static str {
    let target = required_env("TARGET");
    match target_os {
        "macos" => "macosx",
        "ios" if target.contains("-sim") || target.starts_with("x86_64-") => "iphonesimulator",
        "ios" => "iphoneos",
        "tvos" if target.contains("-sim") || target.starts_with("x86_64-") => "appletvsimulator",
        "tvos" => "appletvos",
        "watchos" if target.contains("-sim") || target.starts_with("x86_64-") => "watchsimulator",
        "watchos" => "watchos",
        "visionos" if target.contains("-sim") => "xrsimulator",
        "visionos" => "xros",
        other => panic!("unsupported Apple shader target OS '{other}'"),
    }
}

fn compile_dxil_entry_points(
    module: &Module,
    info: &ModuleInfo,
    reflection: &ShaderReflection,
    out_dir: &Path,
    artifact_name: &str,
    label: &str,
) {
    let options = reflection.hlsl_options();
    for entry in &reflection.entry_points {
        let (source, translated_name) =
            generate_hlsl_entry_point(module, info, &options, entry, label);

        let stem = entry.artifact_stem(artifact_name);
        let hlsl_path = out_dir.join(format!("{stem}.hlsl"));
        let dxil_path = out_dir.join(format!("{stem}.dxil"));
        fs::write(&hlsl_path, source)
            .unwrap_or_else(|error| panic!("failed to write generated HLSL for {label}: {error}"));

        run_tool(
            Command::new("dxc")
                .args(["-T", entry.dxil_profile(), "-E"])
                .arg(translated_name)
                .args(["-Qstrip_debug", "-Qstrip_reflect", "-Fo"])
                .arg(&dxil_path)
                .arg(&hlsl_path),
            "DXIL compilation",
            label,
        );
    }
}

fn generate_hlsl_entry_point(
    module: &Module,
    info: &ModuleInfo,
    options: &hlsl::Options,
    entry: &ReflectedEntryPoint,
    label: &str,
) -> (String, String) {
    let pipeline_options = hlsl::PipelineOptions {
        entry_point: Some((entry.stage, entry.name.clone())),
    };
    let mut source = String::new();
    let mut writer = hlsl::Writer::new(&mut source, options, &pipeline_options);
    let mut output = writer
        .write(module, info, None)
        .unwrap_or_else(|error| panic!("HLSL generation failed for {label}: {error}"));
    assert_eq!(
        output.entry_point_names.len(),
        1,
        "HLSL generation for {label} must emit exactly one entry point"
    );
    let translated_name = output
        .entry_point_names
        .pop()
        .expect("one HLSL entry point was asserted")
        .unwrap_or_else(|error| panic!("HLSL entry-point translation failed for {label}: {error}"));
    (source, translated_name)
}

fn run_tool(command: &mut Command, operation: &str, label: &str) {
    let executable = command.get_program().to_owned();
    let output = command.output().unwrap_or_else(|error| {
        panic!(
            "{operation} failed for {label}: unable to execute {}: {error}",
            executable.to_string_lossy()
        )
    });
    if !output.status.success() {
        panic_tool_failure(operation, label, &output);
    }
}

fn panic_tool_failure(operation: &str, label: &str, output: &Output) -> ! {
    panic!(
        "{operation} failed for {label} with status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingKind {
    UniformBuffer,
    StorageBuffer { read_only: bool },
    Sampler,
    Texture,
    StorageTexture { mutable: bool },
}

#[derive(Debug, Clone)]
struct ReflectedBinding {
    group: u32,
    binding: u32,
    visibility: u32,
    kind: BindingKind,
    rust_type: String,
}

#[derive(Debug, Clone)]
struct ReflectedEntryPoint {
    name: String,
    metal_name: std::cell::RefCell<String>,
    stage: ShaderStage,
    workgroup_size: [u32; 3],
}

impl ReflectedEntryPoint {
    fn artifact_stem(&self, shader_stem: &str) -> String {
        format!(
            "{shader_stem}_{}_{}",
            stage_token(self.stage),
            sanitize_identifier(&self.name)
        )
    }

    fn dxil_profile(&self) -> &'static str {
        match self.stage {
            ShaderStage::Vertex => "vs_6_0",
            ShaderStage::Fragment => "ps_6_0",
            ShaderStage::Compute => "cs_6_0",
            other => panic!("DXIL AOT does not support {other:?} shader stages"),
        }
    }
}

#[derive(Debug)]
struct ShaderReflection {
    bindings: Vec<ReflectedBinding>,
    entry_points: Vec<ReflectedEntryPoint>,
}

impl ShaderReflection {
    fn new(module: &Module, info: &ModuleInfo) -> Self {
        let mut bindings = Vec::new();
        for (handle, global) in module.global_variables.iter() {
            let Some(resource) = global.binding else {
                continue;
            };
            let visibility = module
                .entry_points
                .iter()
                .enumerate()
                .filter(|(index, _)| !info.get_entry_point(*index)[handle].is_empty())
                .fold(0, |bits, (_, entry)| bits | stage_visibility(entry.stage));
            assert_ne!(
                visibility, 0,
                "bound global group {} binding {} is unused by every entry point",
                resource.group, resource.binding
            );
            let (kind, rust_type) = reflect_binding_type(module, handle);
            bindings.push(ReflectedBinding {
                group: resource.group,
                binding: resource.binding,
                visibility,
                kind,
                rust_type,
            });
        }
        bindings.sort_by_key(|binding| (binding.group, binding.binding));

        let entry_points = module
            .entry_points
            .iter()
            .map(|entry| ReflectedEntryPoint {
                name: entry.name.clone(),
                metal_name: std::cell::RefCell::new(entry.name.clone()),
                stage: entry.stage,
                workgroup_size: entry.workgroup_size,
            })
            .collect();

        Self {
            bindings,
            entry_points,
        }
    }

    fn set_metal_names(&self, translated_names: &[String]) {
        assert_eq!(
            self.entry_points.len(),
            translated_names.len(),
            "MSL translation must report every shader entry point"
        );
        for (entry, translated) in self.entry_points.iter().zip(translated_names) {
            entry.metal_name.borrow_mut().clone_from(translated);
        }
    }

    fn msl_options(&self) -> msl::Options {
        let mut stage_resources = BTreeMap::new();
        for stage in [
            ShaderStage::Vertex,
            ShaderStage::Fragment,
            ShaderStage::Compute,
        ] {
            let visibility = stage_visibility(stage);
            let mut buffers = 0;
            let mut textures = 0;
            let mut samplers = 0;
            let needs_sizes_buffer = stage == ShaderStage::Vertex
                || self.bindings.iter().any(|binding| {
                    binding.visibility & visibility != 0
                        && matches!(binding.kind, BindingKind::StorageBuffer { .. })
                });
            let mut resources = BTreeMap::new();
            for binding in &self.bindings {
                if binding.visibility & visibility == 0 {
                    continue;
                }
                let mut target = msl::BindTarget::default();
                match binding.kind {
                    BindingKind::UniformBuffer => {
                        target.buffer = Some(buffers);
                        buffers += 1;
                    }
                    BindingKind::StorageBuffer { read_only } => {
                        target.buffer = Some(buffers);
                        target.mutable = !read_only;
                        buffers += 1;
                    }
                    BindingKind::Sampler => {
                        target.sampler = Some(msl::BindSamplerTarget::Resource(samplers));
                        samplers += 1;
                    }
                    BindingKind::Texture => {
                        target.texture = Some(textures);
                        textures += 1;
                    }
                    BindingKind::StorageTexture { mutable } => {
                        target.texture = Some(textures);
                        target.mutable = mutable;
                        textures += 1;
                    }
                }
                resources.insert(
                    naga::ResourceBinding {
                        group: binding.group,
                        binding: binding.binding,
                    },
                    target,
                );
            }
            stage_resources.insert(
                stage,
                msl::EntryPointResources {
                    resources,
                    sizes_buffer: needs_sizes_buffer.then_some(buffers),
                    ..Default::default()
                },
            );
        }

        let per_entry_point_map = self
            .entry_points
            .iter()
            .map(|entry| {
                (
                    entry.name.clone(),
                    stage_resources
                        .get(&entry.stage)
                        .expect("supported MSL shader stage")
                        .clone(),
                )
            })
            .collect();

        msl::Options {
            per_entry_point_map,
            fake_missing_bindings: false,
            bounds_check_policies: naga::proc::BoundsCheckPolicies {
                index: naga::proc::BoundsCheckPolicy::Unchecked,
                buffer: naga::proc::BoundsCheckPolicy::Unchecked,
                image_load: naga::proc::BoundsCheckPolicy::Unchecked,
                binding_array: naga::proc::BoundsCheckPolicy::Unchecked,
            },
            force_loop_bounding: false,
            ..Default::default()
        }
    }

    fn hlsl_options(&self) -> hlsl::Options {
        let mut binding_map = hlsl::BindingMap::default();
        let mut sampler_buffer_binding_map = hlsl::SamplerIndexBufferBindingMap::default();
        let mut cbv = hlsl::BindTarget::default();
        let mut srv = hlsl::BindTarget::default();
        let mut uav = hlsl::BindTarget::default();

        let groups = self
            .bindings
            .iter()
            .map(|binding| binding.group)
            .max()
            .map_or(0, |group| group + 1);
        for group in 0..groups {
            let mut sampler_index = 0;
            for binding in self
                .bindings
                .iter()
                .filter(|binding| binding.group == group)
            {
                let target = match binding.kind {
                    BindingKind::UniformBuffer => next_hlsl_target(&mut cbv),
                    BindingKind::StorageBuffer { read_only: true } | BindingKind::Texture => {
                        next_hlsl_target(&mut srv)
                    }
                    BindingKind::StorageBuffer { read_only: false }
                    | BindingKind::StorageTexture { mutable: true } => next_hlsl_target(&mut uav),
                    BindingKind::StorageTexture { mutable: false } => next_hlsl_target(&mut srv),
                    BindingKind::Sampler => {
                        let target = hlsl::BindTarget {
                            space: u8::MAX,
                            register: sampler_index,
                            ..Default::default()
                        };
                        sampler_index += 1;
                        target
                    }
                };
                binding_map.insert(
                    naga::ResourceBinding {
                        group: binding.group,
                        binding: binding.binding,
                    },
                    target,
                );
            }
            if sampler_index != 0 {
                sampler_buffer_binding_map.insert(
                    hlsl::SamplerIndexBufferKey { group },
                    next_hlsl_target(&mut srv),
                );
            }
        }

        let special_constants_binding = Some(next_hlsl_target(&mut cbv));
        hlsl::Options {
            shader_model: hlsl::ShaderModel::V6_0,
            binding_map,
            fake_missing_bindings: false,
            special_constants_binding,
            sampler_heap_target: hlsl::SamplerHeapBindTargets {
                standard_samplers: hlsl::BindTarget {
                    register: 0,
                    space: 0,
                    ..Default::default()
                },
                comparison_samplers: hlsl::BindTarget {
                    register: 2048,
                    space: 0,
                    ..Default::default()
                },
            },
            sampler_buffer_binding_map,
            zero_initialize_workgroup_memory: true,
            restrict_indexing: false,
            force_loop_bounding: false,
            ray_query_initialization_tracking: false,
            ..Default::default()
        }
    }

    fn rust_expression(&self, label: &str, artifact_name: &str) -> String {
        let mut rust = String::new();
        writeln!(rust, "::shaderloom::CompiledShader::new(").unwrap();
        writeln!(rust, "    {label:?},").unwrap();
        writeln!(
            rust,
            "    include_str!(concat!(env!(\"OUT_DIR\"), \"/{artifact_name}.wgsl\")),"
        )
        .unwrap();
        writeln!(
            rust,
            "    ::shaderloom::include_compiled_spirv!(\"{artifact_name}.spv\"),"
        )
        .unwrap();
        writeln!(
            rust,
            "    ::shaderloom::include_compiled_metallib!(\"{artifact_name}.metallib\"),"
        )
        .unwrap();
        writeln!(rust, "    &[").unwrap();
        for entry in &self.entry_points {
            let stem = entry.artifact_stem(artifact_name);
            writeln!(rust, "        ::shaderloom::CompiledEntryPoint {{").unwrap();
            writeln!(rust, "            name: {:?},", entry.name).unwrap();
            writeln!(rust, "            stage: {},", rust_stage(entry.stage)).unwrap();
            writeln!(
                rust,
                "            metal_name: {:?},",
                entry.metal_name.borrow()
            )
            .unwrap();
            writeln!(
                rust,
                "            workgroup_size: ({}, {}, {}),",
                entry.workgroup_size[0], entry.workgroup_size[1], entry.workgroup_size[2]
            )
            .unwrap();
            writeln!(
                rust,
                "            dxil: ::shaderloom::include_compiled_dxil!(\"{stem}.dxil\"),"
            )
            .unwrap();
            writeln!(rust, "        }},").unwrap();
        }
        writeln!(rust, "    ],").unwrap();
        writeln!(rust, "    &[").unwrap();

        let group_count = self
            .bindings
            .iter()
            .map(|binding| binding.group)
            .max()
            .map_or(0, |group| group + 1);
        for group in 0..group_count {
            writeln!(
                rust,
                "        ::shaderloom::ReflectedBindGroup {{ entries: &["
            )
            .unwrap();
            for binding in self
                .bindings
                .iter()
                .filter(|binding| binding.group == group)
            {
                writeln!(
                    rust,
                    "            ::shaderloom::wgpu::BindGroupLayoutEntry {{ binding: {}, visibility: ::shaderloom::wgpu::ShaderStages::from_bits_retain({}), ty: {}, count: None }},",
                    binding.binding, binding.visibility, binding.rust_type
                )
                .unwrap();
            }
            writeln!(rust, "        ] }},").unwrap();
        }
        writeln!(rust, "    ],").unwrap();
        writeln!(rust, ")").unwrap();
        rust
    }
}

const fn next_hlsl_target(counter: &mut hlsl::BindTarget) -> hlsl::BindTarget {
    let target = *counter;
    counter.register += 1;
    target
}

fn reflect_binding_type(
    module: &Module,
    handle: Handle<naga::GlobalVariable>,
) -> (BindingKind, String) {
    let global = &module.global_variables[handle];
    match global.space {
        AddressSpace::Uniform => (
            BindingKind::UniformBuffer,
            "::shaderloom::wgpu::BindingType::Buffer { ty: ::shaderloom::wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }".to_owned(),
        ),
        AddressSpace::Storage { access } => {
            let read_only = !access.intersects(naga::StorageAccess::STORE | naga::StorageAccess::ATOMIC);
            (
                BindingKind::StorageBuffer { read_only },
                format!(
                    "::shaderloom::wgpu::BindingType::Buffer {{ ty: ::shaderloom::wgpu::BufferBindingType::Storage {{ read_only: {read_only} }}, has_dynamic_offset: false, min_binding_size: None }}"
                ),
            )
        }
        AddressSpace::Handle => reflect_handle_type(module, global.ty),
        other => panic!("unsupported bound shader address space {other:?}"),
    }
}

fn reflect_handle_type(module: &Module, ty: Handle<naga::Type>) -> (BindingKind, String) {
    match module.types[ty].inner {
        TypeInner::Sampler { comparison } => (
            BindingKind::Sampler,
            format!(
                "::shaderloom::wgpu::BindingType::Sampler({})",
                if comparison {
                    "::shaderloom::wgpu::SamplerBindingType::Comparison"
                } else {
                    "::shaderloom::wgpu::SamplerBindingType::Filtering"
                }
            ),
        ),
        TypeInner::Image {
            dim,
            arrayed,
            class,
        } => reflect_image_type(dim, arrayed, class),
        ref other => panic!("unsupported bound handle type {other:?}"),
    }
}

fn reflect_image_type(
    dimension: ImageDimension,
    arrayed: bool,
    class: ImageClass,
) -> (BindingKind, String) {
    let view_dimension = rust_view_dimension(dimension, arrayed);
    match class {
        ImageClass::Sampled { kind, multi } => {
            let sample_type = match kind {
                naga::ScalarKind::Float | naga::ScalarKind::AbstractFloat => {
                    "::shaderloom::wgpu::TextureSampleType::Float { filterable: true }"
                }
                naga::ScalarKind::Sint | naga::ScalarKind::AbstractInt => {
                    "::shaderloom::wgpu::TextureSampleType::Sint"
                }
                naga::ScalarKind::Uint => "::shaderloom::wgpu::TextureSampleType::Uint",
                naga::ScalarKind::Bool => {
                    panic!("boolean sampled textures are not supported")
                }
            };
            (
                BindingKind::Texture,
                format!(
                    "::shaderloom::wgpu::BindingType::Texture {{ sample_type: {sample_type}, view_dimension: {view_dimension}, multisampled: {multi} }}"
                ),
            )
        }
        ImageClass::Depth { multi } => (
            BindingKind::Texture,
            format!(
                "::shaderloom::wgpu::BindingType::Texture {{ sample_type: ::shaderloom::wgpu::TextureSampleType::Depth, view_dimension: {view_dimension}, multisampled: {multi} }}"
            ),
        ),
        ImageClass::Storage { format, access } => {
            let mutable =
                access.intersects(naga::StorageAccess::STORE | naga::StorageAccess::ATOMIC);
            let rust_access = if access.contains(naga::StorageAccess::ATOMIC) {
                "::shaderloom::wgpu::StorageTextureAccess::Atomic"
            } else if access.contains(naga::StorageAccess::LOAD)
                && access.contains(naga::StorageAccess::STORE)
            {
                "::shaderloom::wgpu::StorageTextureAccess::ReadWrite"
            } else if access.contains(naga::StorageAccess::STORE) {
                "::shaderloom::wgpu::StorageTextureAccess::WriteOnly"
            } else {
                "::shaderloom::wgpu::StorageTextureAccess::ReadOnly"
            };
            (
                BindingKind::StorageTexture { mutable },
                format!(
                    "::shaderloom::wgpu::BindingType::StorageTexture {{ access: {rust_access}, format: ::shaderloom::wgpu::TextureFormat::{format:?}, view_dimension: {view_dimension} }}"
                ),
            )
        }
        ImageClass::External => panic!("external textures are not supported by shader AOT"),
    }
}

fn rust_view_dimension(dimension: ImageDimension, arrayed: bool) -> &'static str {
    match (dimension, arrayed) {
        (ImageDimension::D1, false) => "::shaderloom::wgpu::TextureViewDimension::D1",
        (ImageDimension::D2, false) => "::shaderloom::wgpu::TextureViewDimension::D2",
        (ImageDimension::D2, true) => "::shaderloom::wgpu::TextureViewDimension::D2Array",
        (ImageDimension::D3, false) => "::shaderloom::wgpu::TextureViewDimension::D3",
        (ImageDimension::Cube, false) => "::shaderloom::wgpu::TextureViewDimension::Cube",
        (ImageDimension::Cube, true) => "::shaderloom::wgpu::TextureViewDimension::CubeArray",
        (ImageDimension::D1 | ImageDimension::D3, true) => {
            panic!("wgpu does not support arrayed {dimension:?} texture bindings")
        }
    }
}

fn stage_visibility(stage: ShaderStage) -> u32 {
    match stage {
        ShaderStage::Vertex => 1,
        ShaderStage::Fragment => 2,
        ShaderStage::Compute => 4,
        other => panic!("shader AOT does not support {other:?} stages"),
    }
}

fn stage_token(stage: ShaderStage) -> &'static str {
    match stage {
        ShaderStage::Vertex => "vertex",
        ShaderStage::Fragment => "fragment",
        ShaderStage::Compute => "compute",
        other => panic!("shader AOT does not support {other:?} stages"),
    }
}

fn rust_stage(stage: ShaderStage) -> &'static str {
    match stage {
        ShaderStage::Vertex => "::shaderloom::ShaderStage::Vertex",
        ShaderStage::Fragment => "::shaderloom::ShaderStage::Fragment",
        ShaderStage::Compute => "::shaderloom::ShaderStage::Compute",
        other => panic!("shader AOT does not support {other:?} stages"),
    }
}

fn sanitize_identifier(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SHADER: &str = include_str!("shader_test.wgsl");

    #[test]
    fn reflection_drives_native_binding_maps_and_rust_layout() {
        let module = naga::front::wgsl::parse_str(TEST_SHADER).expect("test WGSL must parse");
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .expect("test WGSL must validate");
        let reflection = ShaderReflection::new(&module, &info);

        assert_eq!(reflection.bindings.len(), 1);
        assert_eq!(reflection.bindings[0].visibility, 3);
        assert!(
            reflection
                .msl_options()
                .per_entry_point_map
                .contains_key("vs_main")
        );
        assert!(
            reflection
                .hlsl_options()
                .binding_map
                .contains_key(&naga::ResourceBinding {
                    group: 0,
                    binding: 0,
                })
        );
        let hlsl_options = reflection.hlsl_options();
        for entry in &reflection.entry_points {
            let (source, translated_name) =
                generate_hlsl_entry_point(&module, &info, &hlsl_options, entry, "test WGSL");
            assert!(!source.is_empty());
            assert!(source.contains(&translated_name));
        }
        let rust = reflection.rust_expression("test.wgsl", "test_shader");
        assert!(rust.contains("CompiledShader::new"));
        assert!(rust.contains("ShaderStages::from_bits_retain(3)"));
    }
}
