# Precompiled engine shaders

Issue #57. The engine's fixed WGSL modules — the three `VARIANT`
specializations of `shader.wgsl` plus `present.wgsl` — are parsed, validated
and translated by naga once in `cherenkov-gpu`'s `build.rs`, instead of wgpu
running naga per pipeline at device and pipeline creation. The runtime embeds
the products with `include_bytes!` and loads them through
`Device::create_shader_module_passthrough`.

Delivery by backend (a property of the selected backend, never a runtime
fallback):

| Backend | Loaded as | Artifact |
|---|---|---|
| Vulkan | SPIR-V passthrough | `<name>.spv`: naga emission as-is. A `spirv-opt -O` step ran here until issue #124 measured it on the Pixel 9 Pro (Mali-G715): ~0.64 s faster cold pipeline creation, but the effects scene's GPU time ~78% slower at p50 and ~2.7x at p99 — so the driver's own compiler does the optimizing |
| Metal | `.metallib` passthrough | `<name>.metallib`: naga MSL at wgpu-hal's argument slots, compiled by `xcrun -sdk <sdk> metal` + `metallib` during the build |
| WebGPU (wasm32) | WGSL | The original source, via `create_shader_module_trusted` with unchecked runtime checks |
| DX12, GL, BrowserWebGpu native | WGSL | Same trusted-WGSL path; wgpu 29 passthrough has no GLSL producer and this build produces no DXIL/HLSL — WGSL is the declared delivery for those backends |

Dynamic shaders — `color.wgsl`/`paint.wgsl` composites, user shader-paint
sources and `interop::shader_module` — are unaffected: they keep the checked
`create_shader_module` path and full naga validation.

## Runtime checks

Passthrough modules carry no naga bounds checks or loop bounding; the SPIR-V
and MSL are emitted with `BoundsCheckPolicy::Unchecked` and
`force_loop_bounding: false`. That is safe here because the sources are fixed
strings shipped with the crate and validated at build time — the same trust
level the passthrough API documents. This supersedes the runtime-checks work
tracked as #55; there was no #55 scaffolding in the tree to remove.

## Layout contract

A passthrough shader must match the bind group layouts the device builds at
runtime — the layout no longer adapts to reflected bindings. The engine's
layouts therefore live in `gpu/src/render/bindings.rs` as plain data shared
between the crate and `build.rs`:

- Vulkan: a WGSL `@binding` maps to its entry's ordinal position within the
  group (`vulkan/device.rs` `create_bind_group_layout`); `build.rs` builds a
  naga `BindingMap` from the same table.
- Metal: buffers, textures and samplers each get a per-stage ordinal counted
  through the groups in declaration order (`metal/device.rs`
  `create_pipeline_layout`); a stage that can see a storage buffer reserves a
  trailing runtime-array-sizes buffer, and the vertex stage always reserves
  one for vertex pulling. `bindings::metal_plan` reproduces that assignment
  into naga's `EntryPointResourceMap`.
- The engine pipelines declare no vertex buffers, so vertex pulling never
  activates and no vertex-buffer sizes are bound.

For Metal the storage buffers are emitted as `array<T, 1>` for the MSL
translation only (the WGSL stays runtime-sized). naga's own backend emits
runtime arrays in exactly that form plus a `_mslBufferSizes` argument; a
passthrough module gets no sizes buffer written, so the argument must not
exist. None of the fixed shaders call `arrayLength`, so the pin is exact.

A backend that takes passthrough shaders but a device without
`Features::PASSTHROUGH_SHADERS` — possible only with a host-supplied
`interop::SharedDevice` — is an explicit `EngineError::Backend` at engine
creation. Engine-created devices request the feature on Vulkan and Metal,
which wgpu-hal advertises unconditionally there.

## Build requirements

- Apple targets run `xcrun -sdk <macosx|iphoneos|iphonesimulator>
  metal`/`metallib`, so building for Apple requires an Apple host — an
  explicit error otherwise. The Metal compiler's `-std` matches naga's
  emitted language version. It must not inherit the build SDK's newest
  language version: a newer SDK can otherwise produce libraries that
  supported older operating systems reject at load time. Metal 1.x/2.x use
  the platform-specific `macos-metal` or `ios-metal` dialect; Metal 3 and
  newer use the unified `metal` dialect. Both shader tools receive the
  deployment target reported by `rustc --print deployment-target` for the
  Cargo target, so the metallib has the same OS floor as the Rust binary.
  Explicit `MACOSX_DEPLOYMENT_TARGET` and `IPHONEOS_DEPLOYMENT_TARGET`
  settings participate in that resolution and invalidate the build script.
  The workspace defaults both to 26.0, matching the native Apple contract
  in `docs/api.md` and both iOS host manifests.
- wasm32 targets skip the toolchain entirely (nothing embeds the artifacts);
  the WGSL is still parsed and validated.

## macOS verification

The Metal path cannot compile on Linux (no `xcrun`); it was verified by
reading the emitted MSL signatures and slot assignments against wgpu-hal 29's
`create_pipeline_layout`. On a macOS host, run:

```sh
git clone https://github.com/water-rs/cherenkov && cd cherenkov
rustup default stable

# 1. Build for macOS — compiles .metal -> .air -> .metallib via xcrun.
cargo build -p cherenkov-gpu
ls target/debug/build/cherenkov-gpu-*/out/*.metallib   # 4 files expected

# 2. Lint + test on Metal hardware.
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo nextest run --locked --workspace
cargo test --locked --workspace --doc

# 3. iOS and simulator targets.
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo check -p cherenkov-gpu --target aarch64-apple-ios
cargo check -p cherenkov-gpu --target aarch64-apple-ios-sim

# 4. Corpus on Metal — must be bit-identical to origin/dev.
cargo build --locked --release -p cherenkov-bench --features cherenkov
./target/release/cherenkov-bench render --engine cherenkov \
    --corpus scenes/corpus --out-dir out/metal-corpus
# Compare against an origin/dev build's output directory: every
# render-*.json metrics object and every *.engine.png must be identical.

# 5. Steady-frame perf + memory on the perf scenes (Metal device numbers are
#    the authoritative wall-clock evidence per AGENTS.md):
./target/release/cherenkov-bench measure --engine cherenkov \
    --scene scenes/perf/map --warmup 5 --frames 60 --out out/measure-metal.json
```

Expected: build emits `engine{0,1,2}.metallib` + `present.metallib`; tests
pass; corpus renders byte-identical to dev; `wgpu::Backend::Metal` selects
`ShaderDelivery::Metallib` (visible via `RUST_LOG=cherenkov_gpu=debug` or the
`shader_delivery` return on a shared device).
