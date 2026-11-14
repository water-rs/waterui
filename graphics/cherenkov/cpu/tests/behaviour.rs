//! The shared cross-backend behaviour suite against `cherenkov-cpu`.

#![cfg(not(target_arch = "wasm32"))]
cherenkov::behaviour_suite! {
    backend: cherenkov_cpu::Raster,
    config: cherenkov_cpu::RasterConfig::default,
    uploads: true,
}
