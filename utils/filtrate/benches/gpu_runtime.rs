//! GPU runtime benchmarks for filter effects.
//!
//! These benches exercise real wgpu render submission and wait for GPU
//! completion.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p filtrate --bench gpu_runtime
//! ```

use divan::Bencher;
use filtrate::multi_input::{BlendMode, FilterImage, blend_with_image_filter};
use filtrate::{Effect, EffectContext, EffectInput, EffectOutput, ShaderCache};

fn main() {
    divan::main();
}

struct GpuBench {
    device: wgpu::Device,
    queue: wgpu::Queue,
    input_texture: wgpu::Texture,
    input_view: wgpu::TextureView,
    output_texture: wgpu::Texture,
    output_view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    shader_cache: ShaderCache,
}

impl GpuBench {
    fn new() -> Self {
        let width = 64;
        let height = 64;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("filtrate benchmark requires a high-performance GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("filtrate benchmark requires a working GPU device");

        let input_texture = create_texture(
            &device,
            "filtrate gpu bench input",
            width,
            height,
            format,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let output_texture = create_texture(
            &device,
            "filtrate gpu bench output",
            width,
            height,
            format,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let input_rgba = solid_rgba(width, height, [96, 128, 192, 255]);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &input_rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        Self {
            device,
            queue,
            input_view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            output_view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            input_texture,
            output_texture,
            format,
            width,
            height,
            shader_cache: ShaderCache::new(),
        }
    }

    fn setup_filter<F: Effect>(&self, filter: &mut F) {
        let ctx = EffectContext {
            device: &self.device,
            queue: &self.queue,
            input_format: self.format,
            output_format: self.format,
            pipeline_cache: None,
            shader_cache: &self.shader_cache,
        };
        pollster::block_on(filter.setup(&ctx)).expect("filter setup should succeed");
    }

    fn render_filter<F: Effect>(&self, filter: &mut F) {
        let input = EffectInput {
            device: &self.device,
            queue: &self.queue,
            texture: &self.input_texture,
            view: self.input_view.clone(),
            format: self.format,
            width: self.width,
            height: self.height,
        };
        let output = EffectOutput {
            device: &self.device,
            queue: &self.queue,
            texture: &self.output_texture,
            view: self.output_view.clone(),
            format: self.format,
            width: self.width,
            height: self.height,
        };

        filter
            .render(&input, &output)
            .expect("filter render should succeed");
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }
}

#[divan::bench]
fn blend_with_image_render_64x64(b: Bencher) {
    let gpu = GpuBench::new();
    let aux = FilterImage::from_rgba8(2, 2, solid_rgba(2, 2, [32, 16, 8, 255]));
    let mut filter = blend_with_image_filter(aux, 0.35, BlendMode::Overlay);
    gpu.setup_filter(&mut filter);
    b.bench_local(|| gpu.render_filter(&mut filter));
}

fn create_texture(
    device: &wgpu::Device,
    label: &'static str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn solid_rgba(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let pixel_count = (width * height) as usize;
    let mut out = Vec::with_capacity(pixel_count * 4);
    for _ in 0..pixel_count {
        out.extend_from_slice(&rgba);
    }
    out
}
