//! DXIL for wgpu's DX12 backend (issue #2726).
//!
//! DX12 passthrough modules hold exactly one entry point, and naga
//! specializes a vertex stage's output linkage against the fragment stage
//! it links with, so every program in `src/render/programs.rs` is emitted
//! per stage: naga writes HLSL exactly as wgpu-hal 30's DX12 backend
//! would at pipeline creation (`dx12/device.rs` `load_shader`), and `dxc`
//! from `PATH` compiles it with wgpu's own arguments
//! (`dx12/shader_compilation.rs` `compile_dxc`) plus optimization at the
//! default level and stripped debug and reflection data — debug and
//! release builds embed the same optimized DXIL.
//!
//! The register assignment restates wgpu-hal 30's `create_pipeline_layout`
//! (`dx12/device.rs`): CBV, SRV and UAV registers each count from 0 in
//! space 0 over the whole layout, groups in order, entries in binding
//! order. Within a group, every non-sampler entry but a dynamic uniform
//! takes the next register of its class in the group's descriptor table;
//! samplers take `space 255` at their index among the group's samplers,
//! read through a sampler-index buffer at the group's next SRV; dynamic
//! uniforms then take the next CBV each as root descriptors, with
//! indexing restricted. After the last group, the special constants
//! (`first_vertex`, `num_workgroups`) take the next CBV — wgpu-core
//! always requests them. The sampler heaps sit at `s0` and `s2048`.

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use naga::back::hlsl;
use proc_macro2::{Literal, TokenStream};
use quote::{format_ident, quote};

use crate::bindings::{Entry, Kind};
use crate::programs::Program;

/// Whether the DXIL artifacts are emitted: wgpu's DX12 backend exists on
/// Windows targets only.
pub fn emits_dxil() -> bool {
    env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows"
}

const fn target(register: u32) -> hlsl::BindTarget {
    hlsl::BindTarget {
        space: 0,
        register,
        binding_array_size: None,
        dynamic_storage_buffer_offsets_index: None,
        restrict_indexing: false,
    }
}

/// wgpu-hal DX12's HLSL options for a pipeline layout of `groups`, with
/// the per-stage fields a trusted, unchecked module sets.
fn options(groups: &[&[Entry]]) -> hlsl::Options {
    let (mut cbv, mut srv, mut uav) = (0, 0, 0);
    let mut binding_map = hlsl::BindingMap::new();
    let mut sampler_buffer_binding_map = hlsl::SamplerIndexBufferBindingMap::new();
    for (group, entries) in groups.iter().enumerate() {
        for pair in entries.windows(2) {
            assert!(
                pair[0].binding < pair[1].binding,
                "group {group}: bindings {} and {} are not strictly increasing",
                pair[0].binding,
                pair[1].binding
            );
        }
        let group = u32::try_from(group).expect("few groups");
        let at = |binding| naga::ResourceBinding { group, binding };
        for entry in *entries {
            let counter = match entry.kind {
                Kind::Uniform if entry.dynamic_offset => continue,
                Kind::Sampler => continue,
                Kind::Uniform => &mut cbv,
                Kind::StorageRead | Kind::Texture { .. } | Kind::TextureUint => &mut srv,
                Kind::Storage => &mut uav,
            };
            assert!(
                !entry.dynamic_offset,
                "dynamic storage buffers need wgpu-hal's offset root constants, \
                 which this plan does not model"
            );
            binding_map.insert(at(entry.binding), target(*counter));
            *counter += 1;
        }
        let mut samplers = 0;
        for entry in entries.iter().filter(|e| matches!(e.kind, Kind::Sampler)) {
            binding_map.insert(
                at(entry.binding),
                hlsl::BindTarget {
                    space: 255,
                    ..target(samplers)
                },
            );
            samplers += 1;
        }
        if samplers > 0 {
            sampler_buffer_binding_map.insert(hlsl::SamplerIndexBufferKey { group }, target(srv));
            srv += 1;
        }
        for entry in entries
            .iter()
            .filter(|e| matches!(e.kind, Kind::Uniform) && e.dynamic_offset)
        {
            binding_map.insert(
                at(entry.binding),
                hlsl::BindTarget {
                    restrict_indexing: true,
                    ..target(cbv)
                },
            );
            cbv += 1;
        }
    }
    hlsl::Options {
        shader_model: hlsl::ShaderModel::V6_0,
        binding_map,
        fake_missing_bindings: false,
        special_constants_binding: Some(target(cbv)),
        immediates_target: None,
        sampler_heap_target: hlsl::SamplerHeapBindTargets {
            standard_samplers: target(0),
            comparison_samplers: target(2048),
        },
        sampler_buffer_binding_map,
        dynamic_storage_buffer_offsets_targets: BTreeMap::new(),
        external_texture_binding_map: hlsl::ExternalTextureBindingMap::new(),
        zero_initialize_workgroup_memory: true,
        restrict_indexing: false,
        force_loop_bounding: false,
        task_dispatch_limits: None,
        mesh_shader_primitive_indices_clamp: false,
        ray_query_initialization_tracking: false,
    }
}

struct Emitter<'a> {
    out_dir: &'a Path,
    module: &'a naga::Module,
    info: &'a naga::valid::ModuleInfo,
}

struct DxcJob {
    stem: String,
    entry: String,
    profile: &'static str,
    file: String,
    hlsl_path: PathBuf,
    dxil_path: PathBuf,
}

#[derive(Default)]
pub struct Modules {
    jobs: Vec<DxcJob>,
    modules: Vec<ModuleJobs>,
}

struct ModuleJobs {
    name: String,
    start: usize,
    end: usize,
    entry_points: Vec<TokenStream>,
}

impl Emitter<'_> {
    /// Writes `entry`'s HLSL as wgpu-hal would and returns its DXC job.
    fn stage(
        &self,
        options: &hlsl::Options,
        (stage, entry): (naga::ShaderStage, &str),
        fragment: Option<&str>,
        stem: &str,
    ) -> DxcJob {
        let out_dir = self.out_dir;
        let (module, info) = naga::back::pipeline_constants::process_overrides(
            self.module,
            self.info,
            Some((stage, entry)),
            &naga::back::PipelineConstants::default(),
        )
        .unwrap_or_else(|e| panic!("{stem}: override processing failed: {e:?}"));
        // wgpu-hal reads the fragment linkage from the fragment stage's
        // module as created, before override processing.
        let fragment = fragment.map(|name| {
            hlsl::FragmentEntryPoint::new(self.module, name)
                .unwrap_or_else(|| panic!("{stem}: no fragment entry `{name}`"))
        });
        let pipeline_options = hlsl::PipelineOptions {
            entry_point: Some((stage, entry.to_string())),
        };
        let mut source = String::new();
        let mut reflection = hlsl::Writer::new(&mut source, options, &pipeline_options)
            .write(&module, &info, fragment.as_ref())
            .unwrap_or_else(|e| panic!("{stem}: HLSL emission failed: {e:?}"));
        assert_eq!(reflection.entry_point_names.len(), 1, "{stem}: one entry");
        let name = reflection
            .entry_point_names
            .pop()
            .unwrap()
            .unwrap_or_else(|e| panic!("{stem}: HLSL entry point failed: {e}"));
        let hlsl_path = out_dir.join(format!("{stem}.hlsl"));
        std::fs::write(&hlsl_path, &source).expect("write HLSL");
        let profile = match stage {
            naga::ShaderStage::Vertex => "vs_6_0",
            naga::ShaderStage::Fragment => "ps_6_0",
            naga::ShaderStage::Compute => "cs_6_0",
            _ => unreachable!("{stem}: the engine has no {stage:?} stages"),
        };
        let file = format!("{stem}.dxil");
        let dxil_path = out_dir.join(&file);
        DxcJob {
            stem: stem.to_owned(),
            entry: name,
            profile,
            file,
            hlsl_path,
            dxil_path,
        }
    }
}

fn dxc_spawn_error(error: &std::io::Error) -> String {
    format!(
        "cannot run `dxc` ({error}): Windows targets embed DXIL compiled by \
         `dxc` from PATH (issue #2726). On Windows, `water doctor --fix` \
         installs it; elsewhere, put the release pinned in \
         `.github/actions/install-dxc/action.yml` on PATH. macOS hosts need a \
         self-built `dxc`: Microsoft publishes no macOS binary."
    )
}

fn compile_dxc_job(job: &DxcJob) -> Result<Vec<u8>, String> {
    let output = Command::new("dxc")
        .args([
            "-E",
            job.entry.as_str(),
            "-T",
            job.profile,
            "-HV",
            "2018",
            "-no-warnings",
            "-Ges",
        ])
        .args(["-Qstrip_debug", "-Qstrip_reflect", "-Fo"])
        .arg(&job.dxil_path)
        .arg(&job.hlsl_path)
        .output()
        .map_err(|error| format!("{}: {}", job.stem, dxc_spawn_error(&error)))?;
    if !output.status.success() {
        return Err(format!(
            "{}: `dxc` failed ({}):\n{}",
            job.stem,
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let dxil = std::fs::read(&job.dxil_path)
        .map_err(|error| format!("{}: cannot read DXIL: {error}", job.stem))?;
    if dxil.len() <= 20 || &dxil[..4] != b"DXBC" {
        return Err(format!("{}: `dxc` wrote no DXIL container", job.stem));
    }
    if !dxil[4..20].iter().any(|&byte| byte != 0) {
        return Err(format!(
            "{}: the DXIL is unsigned; `dxc` needs `dxil.dll` beside it",
            job.stem
        ));
    }
    Ok(dxil)
}

fn compile_dxc_jobs(jobs: &[DxcJob]) -> Vec<Vec<u8>> {
    if jobs.is_empty() {
        return Vec::new();
    }
    let worker_count = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(jobs.len());
    let next_job = AtomicUsize::new(0);
    let results = Mutex::new(
        (0..jobs.len())
            .map(|_| None)
            .collect::<Vec<Option<Result<Vec<u8>, String>>>>(),
    );
    std::thread::scope(|scope| {
        let next_job = &next_job;
        let results = &results;
        for _ in 0..worker_count {
            scope.spawn(move || {
                loop {
                    let index = next_job.fetch_add(1, Ordering::Relaxed);
                    if index >= jobs.len() {
                        break;
                    }
                    let result = compile_dxc_job(&jobs[index]);
                    results.lock().expect("DXC result mutex").as_mut_slice()[index] = Some(result);
                }
            });
        }
    });
    let mut compiled = Vec::with_capacity(jobs.len());
    let mut failures = Vec::new();
    for (index, result) in results
        .into_inner()
        .expect("DXC result mutex")
        .into_iter()
        .enumerate()
    {
        match result {
            Some(Ok(bytes)) => compiled.push(bytes),
            Some(Err(failure)) => failures.push(failure),
            None => failures.push(format!(
                "{}: DXC worker returned no result",
                jobs[index].stem
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{} DXC stage failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
    compiled
}

pub fn entry_point_tokens(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    index: usize,
) -> TokenStream {
    let entry = &module.entry_points[index];
    let name = &entry.name;
    let workgroup_size = if entry.stage == naga::ShaderStage::Compute {
        entry.workgroup_size
    } else {
        [0, 0, 0]
    };
    let [x, y, z] = workgroup_size;
    let workgroup_memory_sizes = if entry.stage == naga::ShaderStage::Compute {
        module
            .global_variables
            .iter()
            .filter(|(handle, variable)| {
                variable.space == naga::AddressSpace::WorkGroup
                    && !info.get_entry_point(index)[*handle].is_empty()
            })
            .map(|(_, variable)| module.types[variable.ty].inner.size(module.to_ctx()))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let workgroup_memory_sizes = workgroup_memory_sizes
        .into_iter()
        .map(Literal::u32_unsuffixed);
    quote! {
        wgpu::PassthroughShaderEntryPoint {
            name: ::std::borrow::Cow::Borrowed(#name),
            workgroup_size: (#x, #y, #z),
            workgroup_memory_sizes: ::std::borrow::Cow::Borrowed(&[#(#workgroup_memory_sizes),*]),
        }
    }
}

pub fn write_entries(
    name: &str,
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    table: &mut TokenStream,
) {
    let constant = format_ident!("{}", name.to_uppercase());
    let entries = (0..module.entry_points.len())
        .map(|index| entry_point_tokens(module, info, index))
        .collect::<Vec<_>>();
    table.extend(quote! {
        pub const #constant: &[wgpu::PassthroughShaderEntryPoint<'static>] = &[#(#entries),*];
    });
}

fn entry_point_for(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: naga::ShaderStage,
    name: &str,
) -> TokenStream {
    let index = module
        .entry_points
        .iter()
        .position(|entry| entry.name == name && entry.stage == stage)
        .unwrap_or_else(|| panic!("no {stage:?} entry point `{name}`"));
    entry_point_tokens(module, info, index)
}

impl Modules {
    /// Emits `name`'s HLSL and records its stages for the shared DXC pool.
    pub fn collect(
        &mut self,
        out_dir: &Path,
        name: &str,
        groups: &[&[Entry]],
        programs: &[Program],
        (module, info): (&naga::Module, &naga::valid::ModuleInfo),
    ) {
        let m = Emitter {
            out_dir,
            module,
            info,
        };
        let render_options = options(groups);
        let mut jobs = Vec::new();
        let mut entry_points = Vec::new();
        for program in programs {
            match *program {
                Program::Render { vertex, fragment } => {
                    jobs.push(m.stage(
                        &render_options,
                        (naga::ShaderStage::Vertex, vertex),
                        Some(fragment),
                        &format!("{name}.{vertex}.{fragment}.vs"),
                    ));
                    entry_points.push(entry_point_for(
                        module,
                        info,
                        naga::ShaderStage::Vertex,
                        vertex,
                    ));
                    jobs.push(m.stage(
                        &render_options,
                        (naga::ShaderStage::Fragment, fragment),
                        None,
                        &format!("{name}.{fragment}.ps"),
                    ));
                    entry_points.push(entry_point_for(
                        module,
                        info,
                        naga::ShaderStage::Fragment,
                        fragment,
                    ));
                }
                Program::Compute { entry, layout } => {
                    jobs.push(m.stage(
                        &options(layout.groups()),
                        (naga::ShaderStage::Compute, entry),
                        None,
                        &format!("{name}.{entry}.cs"),
                    ));
                    entry_points.push(entry_point_for(
                        module,
                        info,
                        naga::ShaderStage::Compute,
                        entry,
                    ));
                }
            }
        }
        let start = self.jobs.len();
        self.jobs.extend(jobs);
        let end = self.jobs.len();
        self.modules.push(ModuleJobs {
            name: name.to_owned(),
            start,
            end,
            entry_points,
        });
    }

    /// Compiles every collected stage in one pool and emits tables by module.
    pub fn emit(self) -> TokenStream {
        let compiled = compile_dxc_jobs(&self.jobs);
        let mut compiled = compiled.into_iter();
        let mut table = TokenStream::new();
        for module in self.modules {
            let jobs = &self.jobs[module.start..module.end];
            let module_compiled = compiled.by_ref().take(jobs.len()).collect();
            write_module(
                &module.name,
                jobs,
                module_compiled,
                module.entry_points,
                &mut table,
            );
        }
        table
    }
}

/// Appends a module's per-stage DXIL table, deduplicating identical blobs
/// within the module.
fn write_module(
    name: &str,
    jobs: &[DxcJob],
    compiled: Vec<Vec<u8>>,
    entry_points: Vec<TokenStream>,
    table: &mut TokenStream,
) {
    let mut definitions = TokenStream::new();
    let mut blobs: Vec<(Vec<u8>, proc_macro2::Ident)> = Vec::new();
    let mut stages = Vec::with_capacity(jobs.len());
    for ((job, bytes), entry) in jobs.iter().zip(compiled).zip(entry_points) {
        let blob = if let Some((_, existing)) = blobs
            .iter()
            .find(|(existing, _)| existing.as_slice() == bytes.as_slice())
        {
            existing.clone()
        } else {
            let blob = format_ident!("{}_{}", name.to_uppercase(), blobs.len());
            let path = Literal::string(&format!("/{}", job.file));
            definitions.extend(quote! {
                const #blob: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), #path));
            });
            blobs.push((bytes, blob.clone()));
            blob
        };
        stages.push(quote! {
            super::DxilStage {
                dxil: #blob,
                entry: #entry,
            }
        });
    }
    let constant = format_ident!("{}", name.to_uppercase());
    table.extend(quote! {
        #definitions
        pub const #constant: &[super::DxilStage] = &[#(#stages),*];
    });
}
