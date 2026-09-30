//! The shared runtime's device contract: a device created by
//! [`GpuRuntime::new`] must be accepted by `cherenkov::Engine<Gpu>` — which
//! requires `Features::PASSTHROUGH_SHADERS` on Vulkan and Metal for its
//! precompiled fixed shaders (cherenkov issue #57).
#![cfg(feature = "gpu")]

use waterui_graphics::gpu::GpuRuntime;

#[test]
fn shared_runtime_device_is_accepted_by_cherenkov_engine() {
    let runtime =
        pollster::block_on(GpuRuntime::new()).expect("a GPU adapter is required on test hardware");
    runtime
        .engine()
        .expect("cherenkov engine must accept the shared runtime's device");
}
