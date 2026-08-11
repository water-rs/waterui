#![doc = include_str!("../README.md")]

#[cfg(feature = "build")]
pub mod build;

use std::borrow::Cow;

/// Re-export used by generated shader layout expressions.
pub use wgpu;

/// One shader entry point and its backend-specific name or binary.
#[derive(Debug, Clone, Copy)]
pub struct CompiledEntryPoint {
    /// Original WGSL entry-point name.
    pub name: &'static str,
    /// Shader stage.
    pub stage: ShaderStage,
    /// Name emitted by Naga's Metal backend.
    pub metal_name: &'static str,
    /// Compute workgroup size. Non-compute entry points use zeroes.
    pub workgroup_size: (u32, u32, u32),
    /// Stage-specific DXIL bytecode.
    pub dxil: Option<&'static [u8]>,
}

/// Shader stage independent from backend-specific wgpu types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderStage {
    /// Vertex entry point.
    Vertex,
    /// Fragment entry point.
    Fragment,
    /// Compute entry point.
    Compute,
}

/// Returns the device features needed to load native shader artifacts.
///
/// GLES does not advertise passthrough shaders, so the intersection is empty
/// and the validated WGSL path remains active for that backend.
#[must_use]
pub fn required_features(adapter_features: wgpu::Features) -> wgpu::Features {
    adapter_features & wgpu::Features::PASSTHROUGH_SHADERS
}

/// One reflected bind group used both for offline translation and runtime layout creation.
#[derive(Debug, Clone, Copy)]
pub struct ReflectedBindGroup {
    /// Entries in binding-number order.
    pub entries: &'static [wgpu::BindGroupLayoutEntry],
}

/// A WGSL module with target-native artifacts produced by Shaderloom's build-script API.
#[derive(Debug, Clone, Copy)]
pub struct CompiledShader {
    label: &'static str,
    wgsl: &'static str,
    #[cfg_attr(
        any(target_arch = "wasm32", target_vendor = "apple"),
        expect(dead_code, reason = "Apple and WebGPU artifacts do not select SPIR-V")
    )]
    spirv: Option<&'static [u32]>,
    #[cfg_attr(
        not(target_vendor = "apple"),
        expect(dead_code, reason = "only Apple targets select MetalLib")
    )]
    metallib: Option<&'static [u8]>,
    entry_points: &'static [CompiledEntryPoint],
    bind_groups: &'static [ReflectedBindGroup],
}

impl CompiledShader {
    /// Creates an embedded shader from generated build artifacts.
    #[must_use]
    pub const fn new(
        label: &'static str,
        wgsl: &'static str,
        spirv: Option<&'static [u32]>,
        metallib: Option<&'static [u8]>,
        entry_points: &'static [CompiledEntryPoint],
        bind_groups: &'static [ReflectedBindGroup],
    ) -> Self {
        Self {
            label,
            wgsl,
            spirv,
            metallib,
            entry_points,
            bind_groups,
        }
    }

    /// Creates the shader module for one entry point.
    ///
    /// Native Metal, Vulkan, and Direct3D 12 devices receive offline-compiled
    /// artifacts. GLES and WebGPU receive the build-validated WGSL module because
    /// their public wgpu backends do not accept a portable offline binary.
    ///
    /// # Panics
    ///
    /// Panics when the entry point is absent, a native artifact is missing, or a
    /// native device was created without `PASSTHROUGH_SHADERS`.
    #[must_use]
    pub fn create_entry_point(
        &self,
        device: &wgpu::Device,
        stage: ShaderStage,
        name: &str,
    ) -> CompiledShaderModule {
        let entry_point = self.entry_point(stage, name);

        match backend(device) {
            #[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "apple")))]
            Backend::Gles => CompiledShaderModule {
                module: create_wgsl_module(device, self.label, self.wgsl),
                entry_point: entry_point.name,
            },
            #[cfg(target_arch = "wasm32")]
            Backend::WebGpu => CompiledShaderModule {
                module: create_wgsl_module(device, self.label, self.wgsl),
                entry_point: entry_point.name,
            },
            #[cfg(target_vendor = "apple")]
            Backend::Metal => {
                require_passthrough(device, self.label);
                let metallib = self
                    .metallib
                    .unwrap_or_else(|| panic!("shader '{}' has no embedded MetalLib", self.label));
                CompiledShaderModule {
                    module: create_passthrough_module(
                        device,
                        wgpu::ShaderModuleDescriptorPassthrough {
                            label: Some(self.label),
                            metallib: Some(Cow::Borrowed(metallib)),
                            num_workgroups: entry_point.workgroup_size,
                            ..Default::default()
                        },
                    ),
                    entry_point: entry_point.metal_name,
                }
            }
            #[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "apple")))]
            Backend::Vulkan => {
                require_passthrough(device, self.label);
                let spirv = self
                    .spirv
                    .unwrap_or_else(|| panic!("shader '{}' has no embedded SPIR-V", self.label));
                CompiledShaderModule {
                    module: create_passthrough_module(
                        device,
                        wgpu::ShaderModuleDescriptorPassthrough {
                            label: Some(self.label),
                            spirv: Some(Cow::Borrowed(spirv)),
                            num_workgroups: entry_point.workgroup_size,
                            ..Default::default()
                        },
                    ),
                    entry_point: entry_point.name,
                }
            }
            #[cfg(target_os = "windows")]
            Backend::Dx12 => {
                require_passthrough(device, self.label);
                let dxil = entry_point.dxil.unwrap_or_else(|| {
                    panic!(
                        "shader '{}' entry point '{}' has no embedded DXIL",
                        self.label, entry_point.name
                    )
                });
                CompiledShaderModule {
                    module: create_passthrough_module(
                        device,
                        wgpu::ShaderModuleDescriptorPassthrough {
                            label: Some(self.label),
                            dxil: Some(Cow::Borrowed(dxil)),
                            num_workgroups: entry_point.workgroup_size,
                            ..Default::default()
                        },
                    ),
                    entry_point: entry_point.name,
                }
            }
        }
    }

    /// Creates vertex and fragment modules for one render pipeline.
    ///
    /// Backends whose native artifact can contain multiple entry points load
    /// that artifact once and share the resulting module between both stages.
    /// Direct3D 12 uses one stage-specific DXIL module per entry point.
    ///
    /// # Panics
    ///
    /// Panics under the same conditions as [`Self::create_entry_point`].
    #[must_use]
    pub fn create_render_stages(
        &self,
        device: &wgpu::Device,
        vertex_name: &str,
        fragment_name: &str,
    ) -> (CompiledShaderModule, CompiledShaderModule) {
        let (vertex, mut fragments) =
            self.create_render_entry_points(device, vertex_name, &[fragment_name]);
        let fragment = fragments
            .pop()
            .expect("one fragment entry point was requested");
        (vertex, fragment)
    }

    /// Creates one vertex module and any number of fragment modules.
    ///
    /// Metal, Vulkan, GLES, and WebGPU load the multi-entry artifact once and
    /// share the resulting module. Direct3D 12 loads the stage-specific DXIL
    /// artifact for each requested entry point.
    ///
    /// # Panics
    ///
    /// Panics under the same conditions as [`Self::create_entry_point`].
    #[must_use]
    pub fn create_render_entry_points(
        &self,
        device: &wgpu::Device,
        vertex_name: &str,
        fragment_names: &[&str],
    ) -> (CompiledShaderModule, Vec<CompiledShaderModule>) {
        let vertex = self.entry_point(ShaderStage::Vertex, vertex_name);
        let fragments = fragment_names
            .iter()
            .map(|name| self.entry_point(ShaderStage::Fragment, name))
            .collect::<Vec<_>>();

        match backend(device) {
            #[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "apple")))]
            Backend::Gles => shared_render_entry_points(
                create_wgsl_module(device, self.label, self.wgsl),
                vertex.name,
                fragments.iter().map(|entry| entry.name),
            ),
            #[cfg(target_arch = "wasm32")]
            Backend::WebGpu => shared_render_entry_points(
                create_wgsl_module(device, self.label, self.wgsl),
                vertex.name,
                fragments.iter().map(|entry| entry.name),
            ),
            #[cfg(target_vendor = "apple")]
            Backend::Metal => {
                require_passthrough(device, self.label);
                let metallib = self
                    .metallib
                    .unwrap_or_else(|| panic!("shader '{}' has no embedded MetalLib", self.label));
                let module = create_passthrough_module(
                    device,
                    wgpu::ShaderModuleDescriptorPassthrough {
                        label: Some(self.label),
                        metallib: Some(Cow::Borrowed(metallib)),
                        ..Default::default()
                    },
                );
                shared_render_entry_points(
                    module,
                    vertex.metal_name,
                    fragments.iter().map(|entry| entry.metal_name),
                )
            }
            #[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "apple")))]
            Backend::Vulkan => {
                require_passthrough(device, self.label);
                let spirv = self
                    .spirv
                    .unwrap_or_else(|| panic!("shader '{}' has no embedded SPIR-V", self.label));
                let module = create_passthrough_module(
                    device,
                    wgpu::ShaderModuleDescriptorPassthrough {
                        label: Some(self.label),
                        spirv: Some(Cow::Borrowed(spirv)),
                        ..Default::default()
                    },
                );
                shared_render_entry_points(
                    module,
                    vertex.name,
                    fragments.iter().map(|entry| entry.name),
                )
            }
            #[cfg(target_os = "windows")]
            Backend::Dx12 => {
                let vertex = self.create_entry_point(device, ShaderStage::Vertex, vertex_name);
                let fragments = fragment_names
                    .iter()
                    .map(|name| self.create_entry_point(device, ShaderStage::Fragment, name))
                    .collect();
                (vertex, fragments)
            }
        }
    }

    /// Creates bind group layouts matching the bindings used for native AOT translation.
    #[must_use]
    pub fn create_bind_group_layouts(&self, device: &wgpu::Device) -> Vec<wgpu::BindGroupLayout> {
        self.bind_groups
            .iter()
            .enumerate()
            .map(|(index, group)| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(&format!("{} bind group {index}", self.label)),
                    entries: group.entries,
                })
            })
            .collect()
    }

    /// Returns the build-validated WGSL for dynamic composition paths.
    #[must_use]
    pub const fn wgsl(&self) -> &'static str {
        self.wgsl
    }

    fn entry_point(&self, stage: ShaderStage, name: &str) -> &CompiledEntryPoint {
        self.entry_points
            .iter()
            .find(|entry| entry.stage == stage && entry.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "shader '{}' has no {stage:?} entry point named '{name}'",
                    self.label
                )
            })
    }
}

/// One wgpu shader module paired with the backend-specific entry-point name.
#[derive(Debug)]
pub struct CompiledShaderModule {
    module: wgpu::ShaderModule,
    entry_point: &'static str,
}

impl CompiledShaderModule {
    /// Returns the wgpu module.
    #[must_use]
    pub const fn module(&self) -> &wgpu::ShaderModule {
        &self.module
    }

    /// Returns the entry-point name used by the selected backend artifact.
    #[must_use]
    pub const fn entry_point(&self) -> &'static str {
        self.entry_point
    }
}

fn shared_render_entry_points(
    module: wgpu::ShaderModule,
    vertex_entry_point: &'static str,
    fragment_entry_points: impl IntoIterator<Item = &'static str>,
) -> (CompiledShaderModule, Vec<CompiledShaderModule>) {
    let fragments = fragment_entry_points
        .into_iter()
        .map(|entry_point| CompiledShaderModule {
            module: module.clone(),
            entry_point,
        })
        .collect();
    (
        CompiledShaderModule {
            module,
            entry_point: vertex_entry_point,
        },
        fragments,
    )
}

/// Creates a WGSL shader module for a shader that is genuinely dynamic at runtime.
#[must_use]
pub fn create_dynamic_wgsl_module(
    device: &wgpu::Device,
    label: Option<&str>,
    source: &str,
) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label,
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
    })
}

/// Memoizes runtime-compiled WGSL modules for one device.
///
/// Runtime-assembled shaders (fused filter chains, specialized spatial
/// passes, user fragment surfaces) are compiled from a fully substituted
/// source string, so that string *is* the specialization key: two callers
/// that produce byte-identical WGSL must share one [`wgpu::ShaderModule`].
///
/// One cache belongs to one device. Sharing a cache across devices would hand
/// out modules the other device cannot use, so hosts own a cache next to the
/// device they created it for.
///
/// The map is behind a `Mutex` because this is pure memoization with no
/// cross-thread protocol to model as a channel: lookups happen during
/// pipeline setup, and a hit costs one hash of the source.
#[derive(Default)]
pub struct WgslModuleCache {
    modules: std::sync::Mutex<std::collections::HashMap<String, wgpu::ShaderModule>>,
}

impl core::fmt::Debug for WgslModuleCache {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WgslModuleCache")
            .finish_non_exhaustive()
    }
}

impl WgslModuleCache {
    /// Creates an empty cache for one device.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the module compiled from `source`, compiling it on first use.
    ///
    /// # Panics
    ///
    /// Panics when a previous caller panicked while holding the cache lock.
    #[must_use]
    pub fn module(
        &self,
        device: &wgpu::Device,
        label: Option<&str>,
        source: &str,
    ) -> wgpu::ShaderModule {
        let mut modules = self
            .modules
            .lock()
            .expect("WGSL module cache poisoned by a panicking compile");
        if let Some(module) = modules.get(source) {
            return module.clone();
        }
        let module = create_dynamic_wgsl_module(device, label, source);
        modules.insert(source.to_owned(), module.clone());
        module
    }

    /// Number of distinct sources compiled through this cache.
    ///
    /// # Panics
    ///
    /// Panics when a previous caller panicked while holding the cache lock.
    #[must_use]
    pub fn len(&self) -> usize {
        self.modules
            .lock()
            .expect("WGSL module cache poisoned by a panicking compile")
            .len()
    }

    /// Whether this cache has compiled anything yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(any(target_arch = "wasm32", not(target_vendor = "apple")))]
fn create_wgsl_module(
    device: &wgpu::Device,
    label: &'static str,
    source: &'static str,
) -> wgpu::ShaderModule {
    create_dynamic_wgsl_module(device, Some(label), source)
}

#[cfg(not(target_arch = "wasm32"))]
fn require_passthrough(device: &wgpu::Device, label: &str) {
    assert!(
        device
            .features()
            .contains(wgpu::Features::PASSTHROUGH_SHADERS),
        "native shader '{label}' requires a device created with PASSTHROUGH_SHADERS"
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn create_passthrough_module(
    device: &wgpu::Device,
    descriptor: wgpu::ShaderModuleDescriptorPassthrough<'_>,
) -> wgpu::ShaderModule {
    // SAFETY: Every binary is generated from the validated WGSL module together
    // with the reflected bind-group layout embedded in the same CompiledShader.
    unsafe { device.create_shader_module_passthrough(descriptor) }
}

#[derive(Debug, Clone, Copy)]
enum Backend {
    #[cfg(target_os = "windows")]
    Dx12,
    #[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "apple")))]
    Gles,
    #[cfg(target_vendor = "apple")]
    Metal,
    #[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "apple")))]
    Vulkan,
    #[cfg(target_arch = "wasm32")]
    WebGpu,
}

#[cfg(target_arch = "wasm32")]
const fn backend(_device: &wgpu::Device) -> Backend {
    Backend::WebGpu
}

#[cfg(not(target_arch = "wasm32"))]
fn backend(device: &wgpu::Device) -> Backend {
    #[cfg(target_vendor = "apple")]
    // SAFETY: `as_hal` only exposes the handle when the device really is that backend,
    // and this only reads whether it matched — the handle never escapes.
    if unsafe { device.as_hal::<wgpu::hal::api::Metal>() }.is_some() {
        return Backend::Metal;
    }

    #[cfg(target_os = "windows")]
    if unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }.is_some() {
        return Backend::Dx12;
    }

    #[cfg(not(target_vendor = "apple"))]
    if unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.is_some() {
        return Backend::Vulkan;
    }

    #[cfg(not(target_vendor = "apple"))]
    if unsafe { device.as_hal::<wgpu::hal::api::Gles>() }.is_some() {
        return Backend::Gles;
    }

    panic!("WaterUI embedded shaders do not support this wgpu backend")
}

/// Includes a generated SPIR-V artifact on targets that can select Vulkan.
#[macro_export]
macro_rules! include_compiled_spirv {
    ($file:literal) => {{
        #[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "apple")))]
        {
            Some(wgpu::include_spirv_source!(concat!(
                env!("OUT_DIR"),
                "/",
                $file
            )))
        }
        #[cfg(any(target_arch = "wasm32", target_vendor = "apple"))]
        {
            None
        }
    }};
}

/// Includes a generated `MetalLib` artifact on Apple targets.
#[macro_export]
macro_rules! include_compiled_metallib {
    ($file:literal) => {{
        #[cfg(target_vendor = "apple")]
        {
            Some(include_bytes!(concat!(env!("OUT_DIR"), "/", $file)).as_slice())
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            None
        }
    }};
}

/// Includes generated DXIL for one entry point on Windows.
#[macro_export]
macro_rules! include_compiled_dxil {
    ($file:literal) => {{
        #[cfg(target_os = "windows")]
        {
            Some(include_bytes!(concat!(env!("OUT_DIR"), "/", $file)).as_slice())
        }
        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }};
}
