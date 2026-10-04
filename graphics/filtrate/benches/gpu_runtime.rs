//! GPU benchmarks for the reference executor.
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
use filtrate::filters::{BlendMode, BlendWithImage, Bloom, Blur, Brightness, Saturation};
use filtrate::{
    Effect, EffectContext, EffectInput, EffectOutput, Executor, FilterExt, FilterImage,
    ShapeTextures,
};

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
}

impl GpuBench {
    fn new(width: u32, height: u32) -> Self {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
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
        }
    }

    fn setup_filter<F: Effect>(&self, filter: &mut F) {
        let ctx = EffectContext {
            device: &self.device,
            queue: &self.queue,
            input_format: self.format,
            output_format: self.format,
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
            timing: filtrate::EffectFrameTiming::new(
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                0,
            ),
            shape: ShapeTextures::default(),
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
    let gpu = GpuBench::new(64, 64);
    let mut filter = Executor::new(BlendWithImage {
        image: FilterImage::from_rgba8(2, 2, solid_rgba(2, 2, [32, 16, 8, 255])),
        amount: 0.35_f32,
        mode: BlendMode::Overlay,
    });
    gpu.setup_filter(&mut filter);
    b.bench_local(|| gpu.render_filter(&mut filter));
}

// ----------------------------------------------------------------------------
// Pass structure: a colour segment is one pass; a separable blur is two
// materialized passes; bloom adds a composite that reads its first pass's
// input.
// ----------------------------------------------------------------------------

#[divan::bench(args = [256, 1024, 2048])]
fn colour_chain(b: Bencher, size: u32) {
    let gpu = GpuBench::new(size, size);
    let mut filter = Executor::new(Saturation(1.3_f32).then(Brightness(0.05_f32)));
    gpu.setup_filter(&mut filter);
    b.bench_local(|| gpu.render_filter(&mut filter));
}

#[divan::bench(args = [256, 1024, 2048])]
fn blur(b: Bencher, size: u32) {
    let gpu = GpuBench::new(size, size);
    let mut filter = Executor::new(Blur(4.0_f32));
    gpu.setup_filter(&mut filter);
    b.bench_local(|| gpu.render_filter(&mut filter));
}

#[divan::bench(args = [1024])]
fn bloom(b: Bencher, size: u32) {
    let gpu = GpuBench::new(size, size);
    let mut filter = Executor::new(Bloom {
        radius: 8.0_f32,
        intensity: 1.2,
        threshold: 0.6,
    });
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
