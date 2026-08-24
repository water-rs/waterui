# shaderloom

`shaderloom` compiles static WGSL during a Rust package build, embeds the
target artifact in the final binary, and loads it through `wgpu` without a
runtime WGSL translation step on native backends.

The crate is independent from `WaterUI`. Its runtime dependency is only `wgpu`;
the optional `build` feature adds `naga` for build scripts.

## Backend artifacts

| Runtime backend | Build-time artifact | Runtime input |
| --- | --- | --- |
| Metal | MSL, AIR, then `MetalLib` | `ShaderModuleDescriptorPassthrough::metallib` |
| Direct3D 12 | Per-entry-point HLSL, then DXIL | `ShaderModuleDescriptorPassthrough::dxil` |
| Vulkan | SPIR-V | `ShaderModuleDescriptorPassthrough::spirv` |
| GLES | Validated WGSL | Standard `ShaderModuleDescriptor` |
| Browser WebGPU | Validated WGSL | Standard `ShaderModuleDescriptor` |

GLES and Browser WebGPU retain WGSL because `wgpu` does not expose a portable
offline binary shader input for those backends. They still benefit from
build-time parsing and validation.

## Package setup

Add the lightweight runtime dependency and the build-enabled build dependency:

```toml
[dependencies]
shaderloom = "0.1"

[build-dependencies]
shaderloom = { version = "0.1", features = ["build"] }
```

The snippets below are shown rather than compiled: they run in a `build.rs`,
`include!` a file that only exists in `OUT_DIR` after that build script has run,
or need a live `wgpu` adapter.

Compile a WGSL module from `build.rs`:

```rust,ignore
// build.rs, with `features = ["build"]` on the build-dependency.
shaderloom::build::compile_wgsl_shader("src/particles.wgsl", "particles");
```

Include the generated expression in library code:

```rust,ignore
use shaderloom::CompiledShader;

const PARTICLES: CompiledShader =
    include!(concat!(env!("OUT_DIR"), "/particles.rs"));
```

Request passthrough support when creating a native device:

```rust,ignore
let required_features = shaderloom::required_features(adapter.features());
let (device, queue) = adapter
    .request_device(&wgpu::DeviceDescriptor {
        required_features,
        ..Default::default()
    })
    .await?;
```

Create shared vertex and fragment modules for a render pipeline:

```rust,ignore
let (vertex, fragment) =
    PARTICLES.create_render_stages(&device, "vs_main", "fs_main");
```

For a module with several fragment passes, use `create_render_entry_points` to
load the shared native library once and select every required entry point.

Use `create_entry_point` for compute shaders. Use
`create_dynamic_wgsl_module` only when shader source is genuinely created at
runtime and therefore cannot be compiled by `build.rs`.

## Native toolchains

- Apple builds require Xcode's optional Metal Toolchain. Install it with
  `xcodebuild -downloadComponent MetalToolchain`.
- Windows builds require Microsoft's DirectX Shader Compiler executable,
  `dxc`, on `PATH`.
- Vulkan, GLES, and Browser WebGPU builds require no external shader compiler.

The build fails immediately when a required compiler is unavailable, WGSL is
invalid, a declared binding is unused, or a binding shape cannot be represented
by the reflected runtime layout.

Build-time shader compilation removes runtime source translation on Metal,
Direct3D 12, and Vulkan. GPU-specific pipeline-state creation can still perform
driver work the first time an application creates a render or compute pipeline.
