#[cfg(all(feature = "c-api", feature = "gpu"))]
pub mod applied_filter;
#[cfg(feature = "gpu")]
pub mod gpu_runtime;
#[cfg(feature = "gpu")]
pub mod gpu_surface;
#[cfg(all(feature = "c-api", feature = "gpu"))]
pub mod view_effect;
pub mod view_renderer;

#[cfg(feature = "gpu")]
fn acquire_surface_texture(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    context: &'static str,
) -> Option<wgpu::SurfaceTexture> {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(output)
        | wgpu::CurrentSurfaceTexture::Suboptimal(output) => Some(output),
        wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
            tracing::debug!(context, "surface lost or outdated; reconfiguring");
            surface.configure(device, config);
            match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(output)
                | wgpu::CurrentSurfaceTexture::Suboptimal(output) => Some(output),
                wgpu::CurrentSurfaceTexture::Occluded => {
                    tracing::debug!(context, "surface is occluded; skipping frame");
                    None
                }
                status => panic!("{context}: acquire after reconfigure failed: {status:?}"),
            }
        }
        wgpu::CurrentSurfaceTexture::Timeout => panic!("{context}: surface timeout"),
        wgpu::CurrentSurfaceTexture::Occluded => {
            tracing::debug!(context, "surface is occluded; skipping frame");
            None
        }
        wgpu::CurrentSurfaceTexture::Validation => {
            panic!("{context}: surface acquisition failed validation")
        }
    }
}
