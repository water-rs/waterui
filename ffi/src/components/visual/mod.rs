#[cfg(feature = "c-api")]
pub mod applied_filter;
pub mod gpu_runtime;
pub mod gpu_surface;
#[cfg(feature = "c-api")]
pub mod view_effect;
pub mod view_renderer;

fn acquire_surface_texture(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    context: &'static str,
) -> wgpu::SurfaceTexture {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(output)
        | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
        wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
            tracing::debug!(context, "surface lost or outdated; reconfiguring");
            surface.configure(device, config);
            match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(output)
                | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
                status => panic!("{context}: acquire after reconfigure failed: {status:?}"),
            }
        }
        wgpu::CurrentSurfaceTexture::Timeout => panic!("{context}: surface timeout"),
        wgpu::CurrentSurfaceTexture::Occluded => panic!("{context}: surface is occluded"),
        wgpu::CurrentSurfaceTexture::Validation => {
            panic!("{context}: surface acquisition failed validation")
        }
    }
}
