//! Pipeline creation that never waits on the GPU host thread.
//!
//! `wgpu` creates pipelines synchronously, and creation is where the backend
//! compiles shaders: on Metal it is `newLibraryWithSource` and
//! `newRenderPipelineState`, which block their caller on the system shader
//! compiler for as long as compilation takes — seconds on a cold shader cache
//! or a virtualized GPU. Effect setup runs on the GPU host thread, which in a
//! windowed host is the UI main thread, so natively the creation runs on a
//! blocking-pool thread and setup awaits it. `wgpu` error scopes are
//! per-thread, so the scope that validates the creation is pushed and popped
//! on the thread that creates.
//!
//! A WebGPU or WebGL page has one thread and no blocking pool, so there the
//! creation stays on the calling thread and only the scope's pop is awaited.

#[cfg(target_family = "wasm")]
use core::{future::Future, pin::Pin};

/// Runs `create` on a blocking-pool thread under a validation error scope,
/// resolving to its product or the validation error the scope caught.
///
/// Dropping the returned task abandons the result; a creation already
/// running finishes on its thread and its product is dropped there.
#[cfg(not(target_family = "wasm"))]
pub fn validated<T: Send + 'static>(
    device: &wgpu::Device,
    create: impl FnOnce(&wgpu::Device) -> T + Send + 'static,
) -> blocking::Task<Result<T, wgpu::Error>> {
    let device = device.clone();
    blocking::unblock(move || {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let product = create(&device);
        pollster::block_on(scope.pop()).map_or(Ok(product), Err)
    })
}

/// Runs `create` under a validation error scope, resolving to its product
/// or the validation error the scope caught.
#[cfg(target_family = "wasm")]
pub fn validated<T: 'static>(
    device: &wgpu::Device,
    create: impl FnOnce(&wgpu::Device) -> T,
) -> Pin<Box<dyn Future<Output = Result<T, wgpu::Error>>>> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let product = create(device);
    let pop = scope.pop();
    Box::pin(async move { pop.await.map_or(Ok(product), Err) })
}
