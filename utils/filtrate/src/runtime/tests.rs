use super::shader::{
    COLOR_CLAMP_MAX_TOKEN, F16_MAX_WGSL, PARAM_VEC4S_TOKEN, SPATIAL_OUTPUT_FORMAT_TOKEN,
    SPATIAL_WORKGROUP_X_TOKEN,
};
use super::uniform::FILTER_UNIFORM_WORDS;
use super::*;
use filtrate_core::{MAX_FILTER_PARAM_VEC4S, ParamArray, StageCollector};
use image::RgbaImage;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

struct TestGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_info: wgpu::AdapterInfo,
    rgba8_storage: bool,
    rgba16_storage: bool,
}

fn create_test_device() -> TestGpu {
    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    instance_descriptor.backends = wgpu::Backends::all();
    let instance = wgpu::Instance::new(instance_descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("filter GPU tests require a high-performance adapter");
    let adapter_info = adapter.get_info();
    let rgba8_storage = adapter
        .get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm)
        .allowed_usages
        .contains(wgpu::TextureUsages::STORAGE_BINDING);
    let rgba16_storage = adapter
        .get_texture_format_features(wgpu::TextureFormat::Rgba16Float)
        .allowed_usages
        .contains(wgpu::TextureUsages::STORAGE_BINDING);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: shaderloom::required_features(adapter.features()),
        ..Default::default()
    }))
    .expect("filter GPU tests require a working device");
    TestGpu {
        device,
        queue,
        adapter_info,
        rgba8_storage,
        rgba16_storage,
    }
}

fn assert_f32_eq(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= f32::EPSILON,
        "expected {expected}, got {actual}"
    );
}

fn readback_rgba8_pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> [u8; 4] {
    const BYTES_PER_PIXEL: u32 = 4;
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let unpadded_bpr = width * BYTES_PER_PIXEL;
    let padded_bpr = unpadded_bpr.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
    let copy_size = u64::from(padded_bpr) * u64::from(height);

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("filter gpu test readback buffer"),
        size: copy_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("filter gpu test readback encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let map_result = rx
        .recv()
        .expect("map callback should return a completion result");
    map_result.expect("buffer mapping should succeed");

    let mapped = slice.get_mapped_range();
    let pixel = [mapped[0], mapped[1], mapped[2], mapped[3]];
    drop(mapped);
    buffer.unmap();
    pixel
}

fn readback_rgba8_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    const BYTES_PER_PIXEL: u32 = 4;
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let unpadded_bpr = width * BYTES_PER_PIXEL;
    let padded_bpr = unpadded_bpr.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
    let copy_size = u64::from(padded_bpr) * u64::from(height);

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("filter gpu test full readback buffer"),
        size: copy_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("filter gpu test full readback encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let map_result = rx
        .recv()
        .expect("map callback should return a completion result");
    map_result.expect("buffer mapping should succeed");

    let mapped = slice.get_mapped_range();
    let mut out = vec![0u8; (width * height * BYTES_PER_PIXEL) as usize];
    for row in 0..height as usize {
        let src_start = row * padded_bpr as usize;
        let src_end = src_start + unpadded_bpr as usize;
        let dst_start = row * unpadded_bpr as usize;
        let dst_end = dst_start + unpadded_bpr as usize;
        out[dst_start..dst_end].copy_from_slice(&mapped[src_start..src_end]);
    }
    drop(mapped);
    buffer.unmap();
    out
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let img = RgbaImage::from_raw(width, height, rgba.to_vec())
        .expect("rgba buffer length should match dimensions");
    img.save(path).expect("failed to save png");
}

#[derive(Clone, Copy)]
struct FilterReadbackSize {
    input: (u32, u32),
    output: (u32, u32),
}

fn rgba_len(width: u32, height: u32) -> usize {
    usize::try_from(u64::from(width) * u64::from(height) * 4).expect("rgba dimensions fit usize")
}

fn rgba_index(width: u32, x: u32, y: u32) -> usize {
    usize::try_from((u64::from(y) * u64::from(width) + u64::from(x)) * 4)
        .expect("rgba index fits usize")
}

fn scale_to_u8(numerator: u32, denominator: u32) -> u8 {
    let value = u64::from(numerator) * 255 / u64::from(denominator.max(1));
    u8::try_from(value).expect("scaled channel fits u8")
}

fn clamp_i32_to_u8(value: i32) -> u8 {
    u8::try_from(value.clamp(0, i32::from(u8::MAX))).expect("clamped channel fits u8")
}

fn create_test_input_rgba(width: u32, height: u32) -> Vec<u8> {
    let mut data = vec![0u8; rgba_len(width, height)];
    let max_x = width.saturating_sub(1).max(1);
    let max_y = height.saturating_sub(1).max(1);
    let min_dimension = i64::from(width.min(height));
    let inner_edge_radius = min_dimension * 28 / 100;
    let outer_edge_radius = min_dimension * 32 / 100;
    let inner_edge_radius_sq = inner_edge_radius * inner_edge_radius;
    let outer_edge_radius_sq = outer_edge_radius * outer_edge_radius;
    let center_x = i64::from(width) / 2;
    let center_y = i64::from(height) / 2;

    for y in 0..height {
        for x in 0..width {
            let idx = rgba_index(width, x, y);
            let x_gradient = scale_to_u8(x, max_x);
            let y_gradient = scale_to_u8(y, max_y);
            let checker = if ((x / 16) + (y / 16)) % 2 == 0 {
                32
            } else {
                -32
            };
            let dx = i64::from(x) - center_x;
            let dy = i64::from(y) - center_y;
            let ring_sq = dx * dx + dy * dy;
            let edge = if ring_sq > inner_edge_radius_sq && ring_sq < outer_edge_radius_sq {
                80
            } else {
                0
            };
            let inverse_gradient = u8::try_from(
                u64::from(max_x - x) * u64::from(max_y - y) * 255
                    / (u64::from(max_x) * u64::from(max_y)),
            )
            .expect("inverse channel fits u8");

            let r = clamp_i32_to_u8(i32::from(x_gradient) + checker + edge);
            let g = clamp_i32_to_u8(i32::from(y_gradient) - checker + edge);
            let b = clamp_i32_to_u8(i32::from(inverse_gradient) + edge);

            data[idx] = r;
            data[idx + 1] = g;
            data[idx + 2] = b;
            data[idx + 3] = 255;
        }
    }
    data
}

fn run_filter_and_readback<G: Effect>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    input_texture: &wgpu::Texture,
    size: FilterReadbackSize,
    mut filter: G,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let (input_width, input_height) = size.input;
    let (output_width, output_height) = size.output;
    let output_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter gallery output"),
        size: wgpu::Extent3d {
            width: output_width,
            height: output_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let ctx = EffectContext {
        device,
        queue,
        input_format: format,
        output_format: format,
    };
    pollster::block_on(filter.setup(&ctx)).expect("test filter setup should succeed");

    let input = EffectInput {
        device,
        queue,
        texture: input_texture,
        view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width: input_width,
        height: input_height,
        timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
    };
    let output = EffectOutput {
        device,
        queue,
        texture: &output_texture,
        view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width: output_width,
        height: output_height,
    };

    let _ = filter.render(&input, &output);
    readback_rgba8_image(device, queue, &output_texture, output_width, output_height)
}

#[test]
fn reject_empty_stage_graph() {
    let err = fuse_stages(&[]).expect_err("empty stage graph should fail");
    assert_eq!(err, EffectSetupError::EmptyGraph);
}

#[test]
fn fuse_adjacent_color_stages() {
    let filter = Chain {
        first: crate::filters::Brightness(0.2f32),
        second: Chain {
            first: crate::filters::Contrast(1.1f32),
            second: crate::filters::Invert,
        },
    };

    let stages = collect_filter_stages(&filter);
    let passes = fuse_stages(&stages).expect("fuse should succeed");

    assert_eq!(passes.len(), 1);
    assert_eq!(passes[0].param_offset, 0);
    assert_eq!(passes[0].param_count, 2);
    assert!(matches!(passes[0].kind, PlannedPassKind::Color { .. }));
}

#[test]
fn keep_spatial_boundaries() {
    let filter = Chain {
        first: crate::filters::Blur(2.0f32),
        second: Chain {
            first: crate::filters::Brightness(0.1f32),
            second: crate::filters::Sharpen(0.8f32),
        },
    };

    let stages = collect_filter_stages(&filter);
    let passes = fuse_stages(&stages).expect("fuse should succeed");

    assert_eq!(passes.len(), 4);
    assert!(matches!(passes[0].kind, PlannedPassKind::Spatial { .. }));
    assert!(matches!(passes[1].kind, PlannedPassKind::Spatial { .. }));
    assert!(matches!(passes[2].kind, PlannedPassKind::Color { .. }));
    assert!(matches!(passes[3].kind, PlannedPassKind::Spatial { .. }));
}

#[test]
fn preserve_param_offsets_across_fused_and_spatial_passes() {
    let filter = Chain {
        first: crate::filters::Brightness(0.2f32),
        second: Chain {
            first: crate::filters::Contrast(1.1f32),
            second: Chain {
                first: crate::filters::Blur(2.0f32),
                second: crate::filters::Sepia(0.7f32),
            },
        },
    };

    let stages = collect_filter_stages(&filter);
    let passes = fuse_stages(&stages).expect("fuse should succeed");

    assert_eq!(passes.len(), 4);
    assert_eq!(passes[0].param_offset, 0);
    assert_eq!(passes[0].param_count, 2);
    assert_eq!(passes[1].param_offset, 2);
    assert_eq!(passes[1].param_count, 1);
    assert_eq!(passes[2].param_offset, 3);
    assert_eq!(passes[2].param_count, 1);
    assert_eq!(passes[3].param_offset, 4);
    assert_eq!(passes[3].param_count, 1);
}

#[test]
fn runtime_binding_plan_tracks_scratch_ping_pong_and_blit_source() {
    let filter = Chain {
        first: crate::filters::Blur(2.0f32),
        second: Chain {
            first: crate::filters::Brightness(0.2f32),
            second: crate::filters::Sharpen(0.8f32),
        },
    };

    let stages = collect_filter_stages(&filter);
    let passes = fuse_stages(&stages).expect("fuse should succeed");

    let (plans, blit_source) =
        plan_runtime_bindings(&passes).expect("runtime binding planning should succeed");

    assert_eq!(plans.len(), 4);
    assert_eq!(
        plans[0],
        PassBindingPlan::Spatial {
            source: PassTextureSource::Input,
            target_scratch: 0,
            original: None
        }
    );
    assert_eq!(
        plans[1],
        PassBindingPlan::Spatial {
            source: PassTextureSource::Scratch(0),
            target_scratch: 1,
            original: None
        }
    );
    assert_eq!(
        plans[2],
        PassBindingPlan::Color {
            source: PassTextureSource::Scratch(1),
            target: ColorTarget::Scratch(0)
        }
    );
    assert_eq!(
        plans[3],
        PassBindingPlan::Spatial {
            source: PassTextureSource::Scratch(0),
            target_scratch: 1,
            original: None
        }
    );
    assert_eq!(blit_source, Some(1));
}

#[test]
fn runtime_binding_plan_for_fused_color_chain_uses_direct_output() {
    let filter = Chain {
        first: crate::filters::Brightness(0.1f32),
        second: Chain {
            first: crate::filters::Contrast(1.2f32),
            second: crate::filters::Invert,
        },
    };

    let stages = collect_filter_stages(&filter);
    let passes = fuse_stages(&stages).expect("fuse should succeed");
    let (plans, blit_source) =
        plan_runtime_bindings(&passes).expect("runtime binding planning should succeed");

    assert_eq!(
        plans,
        vec![PassBindingPlan::Color {
            source: PassTextureSource::Input,
            target: ColorTarget::Output
        }]
    );
    assert_eq!(blit_source, None);
}

type HugeParams = (
    (
        (
            (((([f32; 8], [f32; 8]), [f32; 8]), [f32; 8]), [f32; 8]),
            [f32; 8],
        ),
        [f32; 8],
    ),
    ([f32; 8], [f32; 8]),
);

#[derive(Debug, Clone, Copy)]
struct HugeFilter;

impl Filter for HugeFilter {
    const COLOR_ONLY: bool = true;
    type Params = HugeParams;

    fn params(&self) -> Self::Params {
        (
            (
                (
                    (((([0.0; 8], [0.0; 8]), [0.0; 8]), [0.0; 8]), [0.0; 8]),
                    [0.0; 8],
                ),
                [0.0; 8],
            ),
            ([0.0; 8], [0.0; 8]),
        )
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.color_fragment(
            include_str!("../shaders/color/adjustment/brightness.wgsl"),
            <Self::Params as ParamArray>::LEN,
        );
    }
}

#[test]
fn fast_fail_when_param_count_exceeds_uniform_limit() {
    const {
        assert!(<HugeParams as ParamArray>::LEN > MAX_FILTER_PARAMS);
    }

    let gpu = create_test_device();
    let mut adapter = FilterAdapter::new(HugeFilter);
    let ctx = EffectContext {
        device: &gpu.device,
        queue: &gpu.queue,
        input_format: wgpu::TextureFormat::Rgba8Unorm,
        output_format: wgpu::TextureFormat::Rgba8Unorm,
    };
    let Err(err) = adapter.plan_setup(&ctx) else {
        panic!("over-budget chain must fail setup");
    };
    assert_eq!(
        err,
        EffectSetupError::TooManyParams {
            declared: <HugeParams as ParamArray>::LEN,
            limit: MAX_FILTER_PARAMS,
        }
    );
}

#[test]
fn prefer_hdr_scratch_format_for_hdr_input_or_output() {
    assert_eq!(
        preferred_scratch_format(
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Bgra8Unorm
        ),
        wgpu::TextureFormat::Rgba16Float
    );
    assert_eq!(
        preferred_scratch_format(
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba16Float
        ),
        wgpu::TextureFormat::Rgba16Float
    );
    assert_eq!(
        preferred_scratch_format(
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Bgra8Unorm
        ),
        wgpu::TextureFormat::Rgba8Unorm
    );
}

#[test]
fn specialize_spatial_shader_rewrites_storage_format_token() {
    let src =
        "@group(0) @binding(2) var out_tex: texture_storage_2d<OUTPUT_STORAGE_FORMAT, write>;";
    let shader = specialize_spatial_shader(src, wgpu::TextureFormat::Rgba16Float)
        .expect("specialization should succeed");
    let shader_text = shader.as_str();
    assert!(shader_text.contains("texture_storage_2d<rgba16float, write>"));
    assert!(!shader_text.contains(SPATIAL_OUTPUT_FORMAT_TOKEN));
}

#[test]
fn specialize_spatial_shader_rewrites_workgroup_tokens() {
    let src = "@compute @workgroup_size(WORKGROUP_X, WORKGROUP_Y)";
    let shader = specialize_spatial_shader(src, wgpu::TextureFormat::Rgba8Unorm)
        .expect("specialization should succeed");
    let expected =
        alloc::format!("@compute @workgroup_size({SPATIAL_WORKGROUP_X}, {SPATIAL_WORKGROUP_Y})");
    // The specialized shader is preamble + fragment; every token must be
    // substituted throughout.
    assert!(shader.ends_with(&expected));
    assert!(!shader.contains(PARAM_VEC4S_TOKEN));
    assert!(!shader.contains(SPATIAL_WORKGROUP_X_TOKEN));
}

#[test]
fn specialize_spatial_shader_prepends_shared_preamble() {
    let shader = specialize_spatial_shader("// body", wgpu::TextureFormat::Rgba8Unorm)
        .expect("specialization should succeed");
    assert!(shader.contains("fn luminance"));
    assert!(shader.contains("original_dimensions: vec2<f32>"));
    assert!(shader.contains(&alloc::format!(
        "array<vec4<f32>, {MAX_FILTER_PARAM_VEC4S}>"
    )));
    assert!(shader.ends_with("// body"));
}

#[test]
fn specialize_color_shader_substitutes_clamp_bound_by_target_format() {
    let ldr = specialize_color_shader("// fragment", wgpu::TextureFormat::Rgba8Unorm);
    assert!(ldr.contains("const COLOR_CLAMP_MAX: f32 = 1.0;"));
    let hdr = specialize_color_shader("// fragment", wgpu::TextureFormat::Rgba16Float);
    assert!(hdr.contains(&alloc::format!(
        "const COLOR_CLAMP_MAX: f32 = {F16_MAX_WGSL};"
    )));
    assert!(!hdr.contains(COLOR_CLAMP_MAX_TOKEN));
}

#[test]
fn spatial_uniform_data_uses_vec4_packed_layout() {
    let data = build_spatial_uniform_data(320, 240, 640, 480, 640, 480, &[2.0, 3.0]);
    assert_eq!(data.len(), FILTER_UNIFORM_WORDS);
    assert_f32_eq(data[0], 320.0);
    assert_f32_eq(data[1], 240.0);
    assert_f32_eq(data[2], 640.0);
    assert_f32_eq(data[3], 480.0);
    assert_f32_eq(data[4], 640.0);
    assert_f32_eq(data[5], 480.0);
    assert_f32_eq(data[8], 2.0);
    assert_f32_eq(data[9], 3.0);
}

#[test]
fn color_uniform_data_uses_fixed_array_layout() {
    let data = build_color_uniform_data(800, 600, &[1.0, 2.0]);
    assert_eq!(data.len(), FILTER_UNIFORM_WORDS);
    assert_f32_eq(data[0], 800.0);
    assert_f32_eq(data[1], 600.0);
    assert_f32_eq(data[4], 1.0);
    assert_f32_eq(data[5], 2.0);
}

#[test]
fn hdr_color_fragments_do_not_clamp_to_unit_range() {
    let brightness = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/color/adjustment/brightness.wgsl"
    ));
    let contrast = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/color/adjustment/contrast.wgsl"
    ));
    let sharpen = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/image/convolution/sharpen.wgsl"
    ));

    assert!(!brightness.contains("clamp("));
    assert!(!contrast.contains("clamp("));
    assert!(!sharpen.contains("clamp(result.rgb"));
}

#[test]
fn spatial_shaders_rely_on_shared_preamble_for_bindings() {
    let blur_horizontal = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/image/blur/blur_horizontal.wgsl"
    ));
    let blur_vertical = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/image/blur/blur_vertical.wgsl"
    ));
    let sharpen = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/image/convolution/sharpen.wgsl"
    ));

    for shader in [blur_horizontal, blur_vertical, sharpen] {
        // Bindings and the uniform struct live in the shared preamble;
        // fragments must not redeclare them.
        assert!(!shader.contains("@group"));
        assert!(!shader.contains(SPATIAL_OUTPUT_FORMAT_TOKEN));
        assert!(!shader.contains("input_sampler"));
        let specialized = specialize_spatial_shader(shader, wgpu::TextureFormat::Rgba8Unorm)
            .expect("specialization should succeed");
        assert!(specialized.contains("texture_storage_2d<rgba8unorm, write>"));
        assert!(specialized.contains("textureLoad("));
    }
}

#[test]
fn hdr_policy_builders_update_adapter_policy() {
    let adapter = FilterAdapter::new(crate::filters::Blur(2.0f32));
    assert_eq!(adapter.hdr_policy, HdrPolicy::PreferHdr);

    let adapter = adapter.require_hdr();
    assert_eq!(adapter.hdr_policy, HdrPolicy::RequireHdr);

    let adapter = adapter.force_ldr();
    assert_eq!(adapter.hdr_policy, HdrPolicy::ForceLdr);
}

#[test]
fn then_preserves_hdr_policy() {
    let adapter = FilterAdapter::new(crate::filters::Blur(2.0f32)).require_hdr();
    let chained = adapter.then(crate::filters::Sharpen(1.0f32));
    assert_eq!(chained.hdr_policy, HdrPolicy::RequireHdr);
}

#[test]
fn gpu_color_filter_executes_and_writes_output() {
    let gpu = create_test_device();
    let device = &gpu.device;
    let queue = &gpu.queue;

    let width = 8;
    let height = 8;
    let format = wgpu::TextureFormat::Rgba8Unorm;

    let input_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter gpu color input"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    // Opaque black: under the premultiplied-alpha contract a fully
    // transparent input stays transparent regardless of the filter, so
    // the brightness lift is only observable on opaque pixels.
    let input_data: Vec<u8> = core::iter::repeat_n([0u8, 0, 0, 255], (width * height) as usize)
        .flatten()
        .collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &input_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &input_data,
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

    let output_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter gpu color output"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let mut adapter = FilterAdapter::new(crate::filters::Brightness(0.25f32));
    let ctx = EffectContext {
        device,
        queue,
        input_format: format,
        output_format: format,
    };
    pollster::block_on(Effect::setup(&mut adapter, &ctx))
        .expect("test filter setup should succeed");

    let input = EffectInput {
        device,
        queue,
        texture: &input_texture,
        view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width,
        height,
        timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
    };
    let output = EffectOutput {
        device,
        queue,
        texture: &output_texture,
        view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width,
        height,
    };

    let needs_redraw = Effect::render(&mut adapter, &input, &output);
    assert_eq!(needs_redraw, Ok(false));

    let pixel = readback_rgba8_pixel(device, queue, &output_texture, width, height);
    assert!(pixel[0] > 0 || pixel[1] > 0 || pixel[2] > 0);
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "GPU integration test keeps setup, render, and readback assertions in one scenario"
)]
fn gpu_spatial_filter_supports_mismatched_input_output_sizes() {
    let gpu = create_test_device();
    let device = &gpu.device;
    let queue = &gpu.queue;

    let in_width = 6;
    let in_height = 4;
    let out_width = 11;
    let out_height = 7;
    let format = wgpu::TextureFormat::Rgba8Unorm;

    let input_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter gpu spatial input"),
        size: wgpu::Extent3d {
            width: in_width,
            height: in_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let input_data = vec![255u8; (in_width * in_height * 4) as usize];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &input_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &input_data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(in_width * 4),
            rows_per_image: Some(in_height),
        },
        wgpu::Extent3d {
            width: in_width,
            height: in_height,
            depth_or_array_layers: 1,
        },
    );

    let output_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter gpu spatial output"),
        size: wgpu::Extent3d {
            width: out_width,
            height: out_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let mut adapter = FilterAdapter::new(crate::filters::Blur(1.0f32));
    let ctx = EffectContext {
        device,
        queue,
        input_format: format,
        output_format: format,
    };
    pollster::block_on(Effect::setup(&mut adapter, &ctx))
        .expect("test filter setup should succeed");
    assert!(
        !adapter.has_setup_error(),
        "spatial filter setup failed on adapter {} ({:?}); rgba8_storage={}, rgba16f_storage={}, error={:?}",
        gpu.adapter_info.name,
        gpu.adapter_info.backend,
        gpu.rgba8_storage,
        gpu.rgba16_storage,
        adapter.setup_error,
    );
    assert!(
        !adapter.passes.is_empty(),
        "spatial passes should be compiled"
    );

    let input = EffectInput {
        device,
        queue,
        texture: &input_texture,
        view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width: in_width,
        height: in_height,
        timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
    };
    let output = EffectOutput {
        device,
        queue,
        texture: &output_texture,
        view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width: out_width,
        height: out_height,
    };

    let needs_redraw = Effect::render(&mut adapter, &input, &output);
    assert_eq!(needs_redraw, Ok(false));

    let pixel = readback_rgba8_pixel(device, queue, &output_texture, out_width, out_height);
    assert!(
        pixel.iter().any(|&c| c > 0),
        "spatial output should not be all zeros, got {pixel:?}"
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "GPU integration test keeps setup, render, and fallback assertions in one scenario"
)]
fn gpu_spatial_filter_uses_direct_output_when_storage_binding_is_available() {
    let gpu = create_test_device();
    let device = &gpu.device;
    let queue = &gpu.queue;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let width = 8;
    let height = 8;

    let input_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter direct output input"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let input_data = vec![255u8; (width * height * 4) as usize];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &input_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &input_data,
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

    let output_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter direct output texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let mut adapter = FilterAdapter::new(crate::filters::Blur(1.0f32));
    let ctx = EffectContext {
        device,
        queue,
        input_format: format,
        output_format: format,
    };
    pollster::block_on(Effect::setup(&mut adapter, &ctx))
        .expect("test filter setup should succeed");
    assert!(
        !adapter.has_setup_error(),
        "spatial filter setup failed: {:?}",
        adapter.setup_error
    );
    let expected_direct_output = adapter.passes.iter().any(|pass| {
        matches!(
            &pass.kind,
            CompiledPassKind::Spatial {
                direct_output: Some(_),
                ..
            }
        )
    });

    let input = EffectInput {
        device,
        queue,
        texture: &input_texture,
        view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width,
        height,
        timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
    };
    let output = EffectOutput {
        device,
        queue,
        texture: &output_texture,
        view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width,
        height,
    };

    let _ = Effect::render(&mut adapter, &input, &output);
    assert_eq!(
        adapter.last_render_used_direct_output(),
        expected_direct_output
    );
    if expected_direct_output {
        assert_eq!(
            adapter.allocated_scratch_slots(),
            [true, false, false],
            "direct output path should avoid allocating the final scratch slot"
        );
    } else {
        assert_eq!(
            adapter.allocated_scratch_slots(),
            [true, true, false],
            "fallback path should preserve both blur scratch slots"
        );
    }
}

#[test]
fn gpu_spatial_filter_falls_back_when_output_lacks_storage_binding_usage() {
    let gpu = create_test_device();
    let device = &gpu.device;
    let queue = &gpu.queue;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let width = 8;
    let height = 8;

    let input_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter fallback output input"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let input_data = vec![255u8; (width * height * 4) as usize];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &input_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &input_data,
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

    let output_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter fallback output texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let mut adapter = FilterAdapter::new(crate::filters::Blur(1.0f32));
    let ctx = EffectContext {
        device,
        queue,
        input_format: format,
        output_format: format,
    };
    pollster::block_on(Effect::setup(&mut adapter, &ctx))
        .expect("test filter setup should succeed");
    assert!(
        !adapter.has_setup_error(),
        "spatial filter setup failed: {:?}",
        adapter.setup_error
    );

    let input = EffectInput {
        device,
        queue,
        texture: &input_texture,
        view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width,
        height,
        timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
    };
    let output = EffectOutput {
        device,
        queue,
        texture: &output_texture,
        view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width,
        height,
    };

    let _ = Effect::render(&mut adapter, &input, &output);
    assert!(!adapter.last_render_used_direct_output());
    assert_eq!(
        adapter.allocated_scratch_slots(),
        [true, true, false],
        "non-storage output must keep both blur scratch slots for fallback blit"
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "gallery export intentionally enumerates every filter case in one visual artifact generator"
)]
fn gpu_export_filter_gallery_images() {
    let gpu = create_test_device();
    let device = &gpu.device;
    let queue = &gpu.queue;

    let input_width = 256;
    let input_height = 256;
    let output_dir = PathBuf::from("/tmp/waterui_filter_gallery");
    fs::create_dir_all(&output_dir).expect("failed to create output directory");

    let input_rgba = create_test_input_rgba(input_width, input_height);
    write_png(
        &output_dir.join("input.png"),
        input_width,
        input_height,
        &input_rgba,
    );

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let input_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filter gallery input"),
        size: wgpu::Extent3d {
            width: input_width,
            height: input_height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
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
            bytes_per_row: Some(input_width * 4),
            rows_per_image: Some(input_height),
        },
        wgpu::Extent3d {
            width: input_width,
            height: input_height,
            depth_or_array_layers: 1,
        },
    );

    macro_rules! export_filter {
        ($name:literal, $ow:expr, $oh:expr, $filter:expr) => {{
            let result = run_filter_and_readback(
                device,
                queue,
                &input_texture,
                FilterReadbackSize {
                    input: (input_width, input_height),
                    output: ($ow, $oh),
                },
                $filter,
            );
            write_png(&output_dir.join($name), $ow, $oh, &result);
        }};
    }

    export_filter!(
        "brightness.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Brightness(0.2f32))
    );
    export_filter!(
        "contrast.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Contrast(1.4f32))
    );
    export_filter!(
        "saturation.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Saturation(1.8f32))
    );
    export_filter!(
        "grayscale.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Grayscale(1.0f32))
    );
    export_filter!(
        "hue_rotation.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::HueRotation(120.0f32))
    );
    export_filter!(
        "sepia.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Sepia(1.0f32))
    );
    export_filter!(
        "invert.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Invert)
    );
    export_filter!(
        "blur.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Blur(3.0f32))
    );
    export_filter!(
        "sharpen.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Sharpen(1.5f32))
    );
    export_filter!(
        "chain_blur_brightness.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Blur(2.0f32))
            .then(crate::filters::Brightness(0.15f32))
            .then(crate::filters::Contrast(1.2f32))
    );
    export_filter!(
        "blur_resized_384x216.png",
        384,
        216,
        FilterAdapter::new(crate::filters::Blur(2.0f32))
    );

    // P9 additions — verify each new spatial / preset filter actually
    // round-trips through the wgpu pipeline end-to-end.
    export_filter!(
        "sobel.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Sobel)
    );
    export_filter!(
        "prewitt.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Prewitt)
    );
    export_filter!(
        "median3x3.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Median3x3)
    );
    export_filter!(
        "morphology_min.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::MorphologyMin)
    );
    export_filter!(
        "morphology_max.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::MorphologyMax)
    );
    export_filter!(
        "morphology_gradient.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::MorphologyGradient)
    );
    // 3x3 sharpen kernel: identity * 5 minus the four neighbours.
    export_filter!(
        "convolution3x3_sharpen.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Convolution3x3([
            0.0f32, -1.0, 0.0, -1.0, 5.0, -1.0, 0.0, -1.0, 0.0,
        ]))
    );
    // 5x5 identity (centre = 1, rest = 0). Output should match input.
    export_filter!("convolution5x5_identity.png", input_width, input_height, {
        let mut kernel = [0.0f32; 25];
        kernel[12] = 1.0;
        FilterAdapter::new(crate::filters::Convolution5x5(kernel))
    });
    export_filter!(
        "photo_effect_mono.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::PhotoEffectMono)
    );
    export_filter!(
        "photo_effect_noir.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::PhotoEffectNoir)
    );
    export_filter!(
        "photo_effect_chrome.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::PhotoEffectChrome)
    );
    export_filter!(
        "photo_effect_instant.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::PhotoEffectInstant)
    );
    export_filter!(
        "photo_effect_fade.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::PhotoEffectFade)
    );
    export_filter!(
        "photo_effect_process.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::PhotoEffectProcess)
    );
    export_filter!(
        "photo_effect_tonal.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::PhotoEffectTonal)
    );
    export_filter!(
        "photo_effect_transfer.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::PhotoEffectTransfer)
    );
    // Mixed chain: photo preset chained with tunable color filters.
    export_filter!(
        "chain_chrome_brightness_contrast.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::PhotoEffectChrome)
            .then(crate::filters::Brightness(0.05f32))
            .then(crate::filters::Contrast(1.1f32))
    );
    // Mixed chain: edge detection feeding a tonal preset.
    export_filter!(
        "chain_sobel_then_tonal.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Sobel).then(crate::filters::PhotoEffectTonal)
    );
    export_filter!(
        "vibrance.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Vibrance(0.8f32))
    );
    export_filter!(
        "vignette.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Vignette(0.55f32, 0.35f32))
    );
    // Non-square target: the vignette must stay circular, not elliptical.
    export_filter!(
        "vignette_wide_384x216.png",
        384,
        216,
        FilterAdapter::new(crate::filters::Vignette(0.55f32, 0.35f32))
    );
    export_filter!(
        "bloom.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Bloom {
            radius: 8.0f32,
            intensity: 1.2,
            threshold: 0.6,
        })
    );
    export_filter!(
        "gloom.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Gloom {
            radius: 8.0f32,
            intensity: 0.8,
            threshold: 0.4,
        })
    );
    export_filter!(
        "unsharp_mask.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::UnsharpMask {
            radius: 4.0f32,
            intensity: 1.5,
        })
    );
    // A spatial pass ahead of the with-original pair: bloom's "original"
    // must resolve to the blurred intermediate, not the pipeline input.
    export_filter!(
        "chain_blur_then_bloom.png",
        input_width,
        input_height,
        FilterAdapter::new(crate::filters::Blur(2.0f32)).then(crate::filters::Bloom {
            radius: 8.0f32,
            intensity: 1.2,
            threshold: 0.6,
        })
    );
}
