//! The shared cross-backend behaviour suite against `cherenkov-gpu`.
//! Run with the lavapipe environment (`VK_ICD_FILENAMES` +
//! `WGPU_BACKEND=vulkan`); without an adapter every test returns early.

cherenkov::behaviour_suite! {
    backend: cherenkov_gpu::Gpu,
    config: cherenkov_gpu::GpuConfig::default,
    uploads: true,
}
