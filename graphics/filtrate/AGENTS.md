# AGENTS.md — filtrate

Filter library: typed `Filter` chains whose stages are WGSL functions for
the shared composer (`cherenkov-shader`), and a reference wgpu executor
that runs one fragment pass per composed piece.

## Commands

- `cargo test --workspace` — unit + GPU integration tests (needs a GPU
  adapter; CI runners and dev machines have one).
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all` (rustfmt via the workspace toolchain)
- `cargo check -p filtrate --target wasm32-unknown-unknown --features webgl`
  — the wasm/WebGL build leg. `webgl` is wasm-only: enabling it elsewhere is
  a `compile_error!`, so never run `--all-features` on a native target.
- `cargo bench -p filtrate --bench gpu_runtime` — GPU pass benchmarks.

## Testing guidance

- **Prefer visual tests.** `gpu_export_filter_gallery_images` renders the
  built-in filters into `/tmp/waterui_filter_gallery/` — run it and read the
  outputs when touching stages or the executor.
- **Stages are checked on the CPU too.** Every built-in is composed and its
  passes validated without a GPU, `LINEAR` filters are checked to be linear
  maps, and CPU kernels are cross-checked against their shaders with the
  composer's evaluator (`cherenkov-shader`'s `eval` feature).

## Structure

- `core/` — `filtrate-core`: the `Filter` trait and its kinds, stage
  declarations, params, chains (no GPU).
- `derive/` — `filtrate-derive`: the `#[derive(Filter)]` proc macro.
- `src/shaders/` — the stages' WGSL snippets, `include_str!`'d by the filters.
- `src/executor/` — composition, entry points, pipelines, command encoding.
- `src/cpu.rs` — the SIMD CPU kernels.
