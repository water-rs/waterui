//! Real GPU filter runtime benchmarks.
//!
//! These benchmarks exercise `FilterAdapter` through the same `Effect` trait
//! used by Hydrolysis, at a 3840x2160 render target.

use divan::Bencher;
use filtrate::{ShaderCache, filters};
use waterui_graphics::filter_view::{
    Effect, EffectContext, EffectInput, EffectOutput, FilterAdapter,
};

const WIDTH: u32 = 3840;
const HEIGHT: u32 = 2160;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

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
    shader_cache: ShaderCache,
}

impl GpuBench {
    fn new() -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("filter benchmark requires a high-performance GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("filter benchmark requires a working GPU device");

        let input_texture = create_texture(
            &device,
            "waterui graphics filter bench input",
            WIDTH,
            HEIGHT,
            FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let output_texture = create_texture(
            &device,
            "waterui graphics filter bench output",
            WIDTH,
            HEIGHT,
            FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::STORAGE_BINDING,
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &input_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &gradient_rgba(WIDTH, HEIGHT),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
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
            shader_cache: ShaderCache::new(),
        }
    }

    fn setup_filter<F: Effect>(&self, filter: &mut F) {
        let ctx = EffectContext {
            device: &self.device,
            queue: &self.queue,
            input_format: FORMAT,
            output_format: FORMAT,
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
            format: FORMAT,
            width: WIDTH,
            height: HEIGHT,
            timing: filtrate::EffectFrameTiming::new(
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                0,
            ),
        };
        let output = EffectOutput {
            device: &self.device,
            queue: &self.queue,
            texture: &self.output_texture,
            view: self.output_view.clone(),
            format: FORMAT,
            width: WIDTH,
            height: HEIGHT,
        };

        filter
            .render(&input, &output)
            .expect("filter render should succeed");
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }
}

fn bench_filter<F: Effect>(b: Bencher, mut filter: F) {
    let gpu = GpuBench::new();
    gpu.setup_filter(&mut filter);
    b.bench_local(|| gpu.render_filter(&mut filter));
}

macro_rules! bench_filter {
    ($fn_name:ident, $filter:expr) => {
        #[divan::bench]
        fn $fn_name(b: Bencher) {
            bench_filter(b, $filter);
        }
    };
}

bench_filter!(brightness, FilterAdapter::new(filters::Brightness(0.2f32)));
bench_filter!(contrast, FilterAdapter::new(filters::Contrast(1.4f32)));
bench_filter!(exposure, FilterAdapter::new(filters::Exposure(0.5f32)));
bench_filter!(gamma, FilterAdapter::new(filters::Gamma(1.2f32)));
bench_filter!(
    highlights_shadows,
    FilterAdapter::new(filters::HighlightsShadows(0.25f32, -0.2f32))
);
bench_filter!(
    temperature_tint,
    FilterAdapter::new(filters::TemperatureTint(6500.0f32, 12.0f32))
);
bench_filter!(
    white_point,
    FilterAdapter::new(filters::WhitePoint(1.0f32, 0.96f32, 0.9f32))
);
bench_filter!(
    vignette,
    FilterAdapter::new(filters::Vignette(0.55f32, 0.75f32))
);
bench_filter!(
    color_matrix,
    FilterAdapter::new(filters::ColorMatrix([
        1.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 1.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 1.0f32,
        0.0f32,
    ]))
);
bench_filter!(grayscale, FilterAdapter::new(filters::Grayscale(1.0f32)));
bench_filter!(
    hue_rotation,
    FilterAdapter::new(filters::HueRotation(120.0f32))
);
bench_filter!(invert, FilterAdapter::new(filters::Invert));
bench_filter!(saturation, FilterAdapter::new(filters::Saturation(1.8f32)));
bench_filter!(sepia, FilterAdapter::new(filters::Sepia(1.0f32)));
bench_filter!(vibrance, FilterAdapter::new(filters::Vibrance(0.6f32)));

bench_filter!(blur, FilterAdapter::new(filters::Blur(3.0f32)));
bench_filter!(
    gaussian_blur,
    FilterAdapter::new(filters::GaussianBlur(3.0f32))
);
bench_filter!(
    motion_blur,
    FilterAdapter::new(filters::MotionBlur(8.0f32, 45.0f32))
);
bench_filter!(
    zoom_blur,
    FilterAdapter::new(filters::ZoomBlur(0.25f32, 0.5f32, 0.5f32))
);
bench_filter!(
    convolution3x3,
    FilterAdapter::new(filters::Convolution3x3([
        0.0f32, -1.0f32, 0.0f32, -1.0f32, 5.0f32, -1.0f32, 0.0f32, -1.0f32, 0.0f32,
    ]))
);
bench_filter!(
    convolution5x5,
    FilterAdapter::new(filters::Convolution5x5(identity_kernel_5x5()))
);
bench_filter!(
    edge_work,
    FilterAdapter::new(filters::EdgeWork([1.0f32, 0.5f32]))
);
bench_filter!(median3x3, FilterAdapter::new(filters::Median3x3));
bench_filter!(prewitt, FilterAdapter::new(filters::Prewitt));
bench_filter!(sharpen, FilterAdapter::new(filters::Sharpen(1.5f32)));
bench_filter!(sobel, FilterAdapter::new(filters::Sobel));
bench_filter!(
    unsharp_mask,
    FilterAdapter::new(filters::UnsharpMask([2.0f32, 0.6f32]))
);
bench_filter!(morphology_min, FilterAdapter::new(filters::MorphologyMin));
bench_filter!(morphology_max, FilterAdapter::new(filters::MorphologyMax));
bench_filter!(
    morphology_gradient,
    FilterAdapter::new(filters::MorphologyGradient)
);

bench_filter!(
    bump_distortion,
    FilterAdapter::new(filters::BumpDistortion([0.5f32, 0.5f32, 0.2f32, 0.35f32]))
);
bench_filter!(
    kaleidoscope,
    FilterAdapter::new(filters::Kaleidoscope([0.5f32, 0.5f32, 8.0f32, 0.15f32]))
);
bench_filter!(
    perspective_correction,
    FilterAdapter::new(filters::PerspectiveCorrection(identity_perspective()))
);
bench_filter!(
    perspective_transform,
    FilterAdapter::new(filters::PerspectiveTransform(identity_perspective()))
);
bench_filter!(
    pinch_distortion,
    FilterAdapter::new(filters::PinchDistortion([0.5f32, 0.5f32, 0.35f32, 0.6f32]))
);
bench_filter!(pixellate, FilterAdapter::new(filters::Pixellate(8.0f32)));
bench_filter!(
    twirl_distortion,
    FilterAdapter::new(filters::TwirlDistortion([0.5f32, 0.5f32, 0.4f32, 2.0f32]))
);
bench_filter!(
    vortex_distortion,
    FilterAdapter::new(filters::VortexDistortion([0.5f32, 0.5f32, 0.4f32, 2.0f32]))
);

bench_filter!(
    dot_halftone,
    FilterAdapter::new(filters::DotHalftone([0.5f32, 0.5f32, 0.0f32, 8.0f32]))
);
bench_filter!(
    line_halftone,
    FilterAdapter::new(filters::LineHalftone([0.5f32, 0.5f32, 0.4f32, 8.0f32]))
);
bench_filter!(
    bloom,
    FilterAdapter::new(filters::Bloom([8.0f32, 1.5f32, 0.7f32]))
);
bench_filter!(
    gloom,
    FilterAdapter::new(filters::Gloom([8.0f32, 1.5f32, 0.7f32]))
);
bench_filter!(
    crystallize,
    FilterAdapter::new(filters::Crystallize(24.0f32))
);
bench_filter!(
    mirror_tile,
    FilterAdapter::new(filters::MirrorTile([0.4f32, 0.6f32]))
);
bench_filter!(
    photo_effect_mono,
    FilterAdapter::new(filters::PhotoEffectMono)
);
bench_filter!(
    photo_effect_noir,
    FilterAdapter::new(filters::PhotoEffectNoir)
);
bench_filter!(
    photo_effect_chrome,
    FilterAdapter::new(filters::PhotoEffectChrome)
);
bench_filter!(
    photo_effect_instant,
    FilterAdapter::new(filters::PhotoEffectInstant)
);
bench_filter!(
    photo_effect_fade,
    FilterAdapter::new(filters::PhotoEffectFade)
);
bench_filter!(
    photo_effect_process,
    FilterAdapter::new(filters::PhotoEffectProcess)
);
bench_filter!(
    photo_effect_tonal,
    FilterAdapter::new(filters::PhotoEffectTonal)
);
bench_filter!(
    photo_effect_transfer,
    FilterAdapter::new(filters::PhotoEffectTransfer)
);

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

fn gradient_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    let max_x = width.saturating_sub(1).max(1);
    let max_y = height.saturating_sub(1).max(1);
    for y in 0..height {
        for x in 0..width {
            out.push(u8::try_from((x * 255) / max_x).expect("x channel fits u8"));
            out.push(u8::try_from((y * 255) / max_y).expect("y channel fits u8"));
            out.push(
                u8::try_from((x ^ y) & 0xff)
                    .expect("xor channel fits u8")
                    .saturating_add(32),
            );
            out.push(255);
        }
    }
    out
}

const fn identity_kernel_5x5() -> [f32; 25] {
    let mut kernel = [0.0; 25];
    kernel[12] = 1.0;
    kernel
}

const fn identity_perspective() -> [f32; 8] {
    [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]
}
