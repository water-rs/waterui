//! The SPIR-V writer flags every Cherenkov SPIR-V emitter shares.
//!
//! `cherenkov-gpu`'s build script pulls this file in through `#[path]` —
//! the same mechanism it shares `../record/src/union_cap.rs` by — so the
//! choice cannot drift between emitters and the build script links no
//! crate for it. The file must stay usable outside this crate: no
//! `crate::` references.

/// The `naga::back::spv::Writer` flags a Cherenkov SPIR-V module is emitted
/// with.
///
/// `naga::back::spv::Options::default()` includes
/// `ADJUST_COORDINATE_SPACE` (and `DEBUG` under `debug_assertions`).
/// `ADJUST_COORDINATE_SPACE` appends an `OpFNegate` epilogue that flips
/// `Position.y` in every vertex entry point, but wgpu's Vulkan backend
/// performs that flip itself with a negative-height viewport, so a module
/// written with the defaults renders upside down when it is loaded as a
/// passthrough shader (water-rs/waterui#3079). wgpu-hal compiles its own
/// runtime modules with `WriterFlags::empty()` plus per-module flags;
/// Cherenkov emits with that same base.
pub const SPIRV_WRITER_FLAGS: naga::back::spv::WriterFlags = naga::back::spv::WriterFlags::empty();
