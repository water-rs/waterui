use std::time::Instant;

use waterui::graphics::{GpuContext, GpuFrame, GpuRenderer, GpuSurface, bytemuck, wgpu};
use waterui::prelude::*;

pub fn init() -> Environment {
    Environment::new()
}

pub fn main() -> impl View {
    vstack((
        text("Cinematic HDR Flame (GpuSurface)").size(24),
        text("HDR film buffer + bloom + ACES tonemap").size(14),
        GpuSurface::new(FlameRenderer::default()).size(400.0, 500.0),
        text("Rendered at 120fps").size(12),
    ))
    .padding()
}

const FILM_WGSL: &str = r#"
// Cinematic flame (HDR) with simple film pipeline:
// 1) Render procedural flame to HDR film buffer
// 2) Threshold + downsample to bloom buffer
// 3) Blur bloom (separable)
// 4) Composite + ACES tonemap + vignette + grain

struct Globals {
    time: f32,
    exposure: f32,
    bloom_threshold: f32,
    bloom_intensity: f32,
    edr_gain: f32,
    bloom_radius: f32,
    wind: f32,
    flame_strength: f32,
    resolution: vec2<f32>,
    inv_resolution: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;

// Shared texture/sampler set: film + bloom (bloom may be unused in some passes).
@group(1) @binding(0) var t_film: texture_2d<f32>;
@group(1) @binding(1) var t_bloom: texture_2d<f32>;
@group(1) @binding(2) var s_linear: sampler;

struct BlurParams {
    texel_size: vec2<f32>,
    direction: vec2<f32>,
}

@group(2) @binding(0) var<uniform> blur: BlurParams;
@group(2) @binding(1) var t_source: texture_2d<f32>;
@group(2) @binding(2) var s_source: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
    );

    let pos = positions[vertex_index];
    var output: VertexOutput;
    output.position = vec4<f32>(pos, 0.0, 1.0);
    output.uv = (pos + 1.0) * 0.5; // (0,0) bottom-left
    return output;
}

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, -s, s, c);
}

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);

    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));

    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var a = 0.55;
    var s = 0.0;
    for (var i: i32 = 0; i < 6; i = i + 1) {
        s += a * noise(p);
        p = (rot(0.35) * p) * 2.0 + vec2<f32>(17.0, 23.0);
        a *= 0.5;
    }
    return s;
}

fn ridge(n: f32) -> f32 {
    return 1.0 - abs(2.0 * n - 1.0);
}

fn rfbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var a = 0.60;
    var s = 0.0;
    for (var i: i32 = 0; i < 5; i = i + 1) {
        s += a * ridge(noise(p));
        p = (rot(0.62) * p) * 2.1 + vec2<f32>(9.2, 7.7);
        a *= 0.5;
    }
    return s;
}

fn fire_palette(x: f32) -> vec3<f32> {
    // x: 0..1 (cool -> hot). Return HDR-ish linear RGB.
    let c0 = vec3<f32>(0.02, 0.005, 0.002);  // ember
    let c1 = vec3<f32>(0.85, 0.10, 0.015);  // red
    let c2 = vec3<f32>(1.75, 0.55, 0.08);   // orange
    let c3 = vec3<f32>(2.60, 1.80, 0.55);   // yellow-hot (less white)

    let t1 = smoothstep(0.00, 0.55, x);
    let t2 = smoothstep(0.55, 0.85, x);
    let t3 = smoothstep(0.85, 1.00, x);

    let a = mix(c0, c1, t1);
    let b = mix(c1, c2, t2);
    let c = mix(c2, c3, t3);
    return mix(mix(a, b, t2), c, t3);
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn vignette(uv: vec2<f32>, aspect: f32) -> f32 {
    let q = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);
    let r = length(q);
    return smoothstep(0.95, 0.30, r);
}

// Render-target textures in wgpu/WebGPU use a top-left origin when sampled.
// Our `uv` is bottom-left origin, so flip Y for all texture sampling to keep
// multi-pass render->sample pipelines consistent (avoids vertical mirroring).
fn tex_uv(uv: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(uv.x, 1.0 - uv.y);
}

fn sample_film(uv: vec2<f32>) -> vec3<f32> {
    return textureSample(t_film, s_linear, tex_uv(uv)).rgb;
}

fn sample_bloom(uv: vec2<f32>) -> vec3<f32> {
    return textureSample(t_bloom, s_linear, tex_uv(uv)).rgb;
}

fn sample_source(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_source, s_source, tex_uv(uv));
}

@fragment
fn fs_flame(input: VertexOutput) -> @location(0) vec4<f32> {
    let t = globals.time;
    let res = max(globals.resolution, vec2<f32>(1.0));
    let aspect = res.x / res.y;

    // Aspect-correct flame space; base anchored at bottom.
    // `uv` is bottom-left origin; keep p.y in 0..1 so the tip never hard-clips.
    var p = vec2<f32>((input.uv.x - 0.5) * aspect, input.uv.y);
    p.x *= 1.15;

    // Scale Y so the flame fades out *before* the top edge.
    let y = max(p.y, 0.0) * 1.25;
    let y01 = clamp(y, 0.0, 1.0);
    let wind = globals.wind;

    // Bend + sway (stronger towards the top), plus a gusty drift.
    let sway = 0.10 * sin(t * 0.90 + y01 * 2.4) + 0.06 * sin(t * 1.70 + y01 * 4.8);
    let gust = (fbm(vec2<f32>(y01 * 1.20, t * 0.25)) - 0.5) * 0.12;
    let center = (sway + gust) * (0.20 + 0.80 * y01) + wind * y01 * y01;

    // Flow field for turbulence (rising motion).
    let rise = t * 2.2;
    let q = vec2<f32>((p.x - center) * 2.4, y * 3.8 - rise);

    let n = fbm(q + vec2<f32>(0.0, t * 0.25));
    let r = rfbm(q * 1.6 + vec2<f32>(2.3, -t * 0.7));
    let tongue_n = rfbm(vec2<f32>((p.x - center) * 4.2, y * 7.5 - t * 4.5));
    let tongues = smoothstep(0.35, 0.98, tongue_n);

    // Width profile: wide base -> thin tip.
    let base_w = 0.18;
    let tip_w = 0.006;
    let w = mix(base_w, tip_w, pow(y01, 1.55));
    let wv = w * (0.65 + 0.50 * n) * (0.80 + 0.55 * tongues);

    // Lateral turbulence (adds tongues and breaks symmetry).
    let x_turb =
        (fbm(q * 2.3 + vec2<f32>(12.0, t * 1.6)) - 0.5) * 0.12 * (0.15 + 0.85 * y01);
    let d = abs((p.x - center) + x_turb);

    // Main body mask with soft edge.
    var mask = 1.0 - smoothstep(wv * 0.85, wv, d);

    // Add flame tongues (mostly in the upper half).
    let tongue_halo = 1.0 - smoothstep(wv * 0.45, wv * 2.4, d);
    mask = clamp(mask + tongues * tongue_halo * (0.10 + 0.55 * y01), 0.0, 1.0);

    // Streaky breakup.
    let breakup = smoothstep(0.15, 0.90, r);
    mask *= mix(0.30, 1.0, breakup);

    // Fade-in at the base (avoid hard clip on the bottom edge).
    mask *= smoothstep(0.00, 0.03, y01);

    // Soft tip fade (prevents the “black bar” truncation).
    let tip_fade = 1.0 - smoothstep(0.95, 1.30, y + (n - 0.5) * 0.12);
    mask *= clamp(tip_fade, 0.0, 1.0);

    // Halo glow around the flame.
    let halo = (1.0 - smoothstep(wv * 0.8, wv * 3.2, d)) * (0.22 + 0.35 * y01);

    // Core is hottest near the centerline.
    let core = exp(-d * d / (wv * wv * 0.08 + 1e-4));

    // Flicker
    let flicker = 0.82 + 0.18 * sin(t * 11.0 + (n + tongues) * 6.28318);
    let intensity = (mask * 0.75 + halo) * flicker;

    // Temperature: hot core + hot base, cooler top/edges.
    let heat = clamp(core * 0.85 + (1.0 - y01) * 0.30 + tongues * 0.10, 0.0, 1.0);

    // HDR emission (scaled so bloom can do the heavy lifting).
    let strength = globals.flame_strength;
    let outer_col = vec3<f32>(1.60, 0.34, 0.03);
    let inner_col = vec3<f32>(2.80, 1.90, 0.55);
    let mixv = clamp(pow(heat, 1.35) * 0.90 + core * 0.20, 0.0, 1.0);
    var col = mix(outer_col, inner_col, mixv) * intensity;
    col *= (0.65 + 4.8 * pow(heat, 1.6)) * strength;

    // Slight soot/dimming near edges in the upper flame.
    let soot = smoothstep(0.25, 0.95, y01) * smoothstep(wv * 0.35, wv * 2.2, d);
    col *= 1.0 - 0.45 * soot;

    // Background (subtle warm base).
    var bg = vec3<f32>(0.0015, 0.0018, 0.0025);
    bg += vec3<f32>(0.010, 0.004, 0.002) * exp(-y01 * 4.0);

    return vec4<f32>(bg + col, 1.0);
}

@fragment
fn fs_downsample(input: VertexOutput) -> @location(0) vec4<f32> {
    // 2x2 box filter + soft threshold
    let uv = input.uv;
    let texel = globals.inv_resolution;

    let c0 = sample_film(uv + vec2<f32>(-0.5 * texel.x, -0.5 * texel.y));
    let c1 = sample_film(uv + vec2<f32>( 0.5 * texel.x, -0.5 * texel.y));
    let c2 = sample_film(uv + vec2<f32>(-0.5 * texel.x,  0.5 * texel.y));
    let c3 = sample_film(uv + vec2<f32>( 0.5 * texel.x,  0.5 * texel.y));

    let col = (c0 + c1 + c2 + c3) * 0.25;
    let lum = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));

    // Soft-knee bloom extraction (keeps bloom mostly on highlights).
    let thr = globals.bloom_threshold;
    let knee = thr * 0.55;
    let soft = clamp((lum - thr + knee) / (2.0 * knee), 0.0, 1.0);
    let contrib = max(lum - thr, 0.0) + soft * soft * knee;
    let scale = contrib / max(lum, 1e-4);
    return vec4<f32>(col * scale, 1.0);
}

@fragment
fn fs_blur(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv = input.uv;
    let off = blur.texel_size * blur.direction * globals.bloom_radius;

    // 5-tap Gaussian-ish blur (separable)
    var c = sample_source(uv) * 0.227027;
    c += sample_source(uv + off * 1.384615) * 0.316216;
    c += sample_source(uv - off * 1.384615) * 0.316216;
    c += sample_source(uv + off * 3.230769) * 0.070270;
    c += sample_source(uv - off * 3.230769) * 0.070270;

    return c;
}

@fragment
fn fs_final(input: VertexOutput) -> @location(0) vec4<f32> {
    let t = globals.time;
    let res = max(globals.resolution, vec2<f32>(1.0));
    let aspect = res.x / res.y;

    let film = sample_film(input.uv);
    let bloom = sample_bloom(input.uv);

    var col = film + bloom * globals.bloom_intensity;
    col *= globals.exposure;

    // subtle vignette before tonemap
    col *= 0.55 + 0.45 * vignette(input.uv, aspect);

    // Tonemap: SDR uses ACES; HDR uses a white-point rolloff that can exceed 1.0.
    let edr = globals.edr_gain;
    if (edr > 1.0) {
        col = max(col, vec3<f32>(0.0));
        col = col / (vec3<f32>(1.0) + col / edr);
    } else {
        col = aces(col);
    }

    // film grain (tiny, post-tonemap) to avoid banding
    let px = floor(input.uv * res);
    let g = hash21(px + vec2<f32>(t * 60.0, t * 13.0));
    col += (g - 0.5) * (1.0 / 255.0) * 6.0;

    return vec4<f32>(col, 1.0);
}
"#;

struct FlameRenderer {
    last_tick: Instant,
    sim_time: f32,

    globals_buffer: Option<wgpu::Buffer>,
    globals_bind_group: Option<wgpu::BindGroup>,

    sample_layout: Option<wgpu::BindGroupLayout>,
    blur_layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,

    flame_pipeline: Option<wgpu::RenderPipeline>,
    downsample_pipeline: Option<wgpu::RenderPipeline>,
    blur_pipeline: Option<wgpu::RenderPipeline>,
    final_pipeline: Option<wgpu::RenderPipeline>,
    final_format: Option<wgpu::TextureFormat>,

    film_view: Option<wgpu::TextureView>,
    bloom_down_view: Option<wgpu::TextureView>,
    bloom_temp_view: Option<wgpu::TextureView>,
    bloom_blur_view: Option<wgpu::TextureView>,

    sample_bind_group: Option<wgpu::BindGroup>,
    final_bind_group: Option<wgpu::BindGroup>,
    blur_x_bind_group: Option<wgpu::BindGroup>,
    blur_y_bind_group: Option<wgpu::BindGroup>,

    size: (u32, u32),
}

impl Default for FlameRenderer {
    fn default() -> Self {
        Self {
            last_tick: Instant::now(),
            sim_time: 0.0,

            globals_buffer: None,
            globals_bind_group: None,

            sample_layout: None,
            blur_layout: None,
            sampler: None,

            flame_pipeline: None,
            downsample_pipeline: None,
            blur_pipeline: None,
            final_pipeline: None,
            final_format: None,

            film_view: None,
            bloom_down_view: None,
            bloom_temp_view: None,
            bloom_blur_view: None,

            sample_bind_group: None,
            final_bind_group: None,
            blur_x_bind_group: None,
            blur_y_bind_group: None,

            size: (0, 0),
        }
    }
}

impl FlameRenderer {
    const FILM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
    const GLOBALS_SIZE: u64 = std::mem::size_of::<[f32; 12]>() as u64;
    const BLUR_PARAMS_SIZE: u64 = std::mem::size_of::<[f32; 4]>() as u64;

    fn ensure_targets(&mut self, frame: &GpuFrame) {
        if self.size == (frame.width, frame.height)
            && self.film_view.is_some()
            && self.bloom_down_view.is_some()
            && self.bloom_temp_view.is_some()
            && self.bloom_blur_view.is_some()
            && self.sample_bind_group.is_some()
            && self.final_bind_group.is_some()
            && self.blur_x_bind_group.is_some()
            && self.blur_y_bind_group.is_some()
        {
            return;
        }

        self.size = (frame.width, frame.height);

        let bloom_w = (frame.width / 2).max(1);
        let bloom_h = (frame.height / 2).max(1);

        let film = frame.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Film HDR"),
            size: wgpu::Extent3d {
                width: frame.width.max(1),
                height: frame.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FILM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let film_view = film.create_view(&wgpu::TextureViewDescriptor::default());

        let bloom_down = frame.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Bloom Downsample"),
            size: wgpu::Extent3d {
                width: bloom_w,
                height: bloom_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FILM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_down_view = bloom_down.create_view(&wgpu::TextureViewDescriptor::default());

        let bloom_temp = frame.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Bloom Blur Temp"),
            size: wgpu::Extent3d {
                width: bloom_w,
                height: bloom_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FILM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_temp_view = bloom_temp.create_view(&wgpu::TextureViewDescriptor::default());

        let bloom_blur = frame.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Bloom Blurred"),
            size: wgpu::Extent3d {
                width: bloom_w,
                height: bloom_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FILM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_blur_view = bloom_blur.create_view(&wgpu::TextureViewDescriptor::default());

        let Some(sample_layout) = &self.sample_layout else {
            return;
        };
        let Some(blur_layout) = &self.blur_layout else {
            return;
        };
        let Some(sampler) = &self.sampler else {
            return;
        };

        // Bind film + blurred bloom (bloom is ignored by the downsample pass).
        let sample_bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Sample Bind Group"),
            layout: sample_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&film_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    // Dummy: bind the film here so intermediate passes never sample the same
                    // texture they are writing to (wgpu exclusive COLOR_TARGET usage).
                    resource: wgpu::BindingResource::TextureView(&film_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        // Final composite needs the blurred bloom.
        let final_bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Final Bind Group"),
            layout: sample_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&film_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom_blur_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        // Blur params (two bind groups with fixed directions).
        let blur_x_buffer = frame.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flame Blur Params X"),
            size: Self::BLUR_PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blur_y_buffer = frame.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flame Blur Params Y"),
            size: Self::BLUR_PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        #[allow(clippy::cast_precision_loss)]
        let texel_size = (1.0 / bloom_w as f32, 1.0 / bloom_h as f32);
        let blur_x: [f32; 4] = [texel_size.0, texel_size.1, 1.0, 0.0];
        let blur_y: [f32; 4] = [texel_size.0, texel_size.1, 0.0, 1.0];
        frame
            .queue
            .write_buffer(&blur_x_buffer, 0, bytemuck::bytes_of(&blur_x));
        frame
            .queue
            .write_buffer(&blur_y_buffer, 0, bytemuck::bytes_of(&blur_y));

        let blur_x_bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Blur X Bind Group"),
            layout: blur_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: blur_x_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom_down_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        let blur_y_bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Blur Y Bind Group"),
            layout: blur_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: blur_y_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom_temp_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        // Update stored views + bind groups.
        self.film_view = Some(film_view);
        self.bloom_down_view = Some(bloom_down_view);
        self.bloom_temp_view = Some(bloom_temp_view);
        self.bloom_blur_view = Some(bloom_blur_view);

        self.sample_bind_group = Some(sample_bind_group);
        self.final_bind_group = Some(final_bind_group);
        self.blur_x_bind_group = Some(blur_x_bind_group);
        self.blur_y_bind_group = Some(blur_y_bind_group);
    }
}

impl GpuRenderer for FlameRenderer {
    fn setup(&mut self, ctx: &GpuContext) {
        self.last_tick = Instant::now();
        self.sim_time = 0.0;

        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Flame Film Shader"),
                source: wgpu::ShaderSource::Wgsl(FILM_WGSL.into()),
            });

        let globals_size = Self::GLOBALS_SIZE;
        let globals_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Flame Globals"),
            size: globals_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Flame Globals Layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: core::num::NonZeroU64::new(globals_size),
                        },
                        count: None,
                    }],
                });

        let globals_bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Globals Bind Group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Flame Linear Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let sample_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Flame Sample Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let blur_size = Self::BLUR_PARAMS_SIZE;
        let blur_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Flame Blur Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: core::num::NonZeroU64::new(blur_size),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let flame_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Flame Pipeline Layout"),
                    bind_group_layouts: &[&globals_layout],
                    push_constant_ranges: &[],
                });

        let composite_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Flame Composite Layout"),
                    bind_group_layouts: &[&globals_layout, &sample_layout],
                    push_constant_ranges: &[],
                });

        // Blur shader uses @group(2), so we must provide layouts for groups 0..=2.
        let blur_pipeline_layout =
            ctx.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Flame Blur Pipeline Layout"),
                    bind_group_layouts: &[&globals_layout, &sample_layout, &blur_layout],
                    push_constant_ranges: &[],
                });

        let flame_pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Flame Pass Pipeline"),
                layout: Some(&flame_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_flame"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: Self::FILM_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        let downsample_pipeline =
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Flame Bloom Downsample Pipeline"),
                    layout: Some(&composite_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_downsample"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: Self::FILM_FORMAT,
                            blend: Some(wgpu::BlendState::REPLACE),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                    cache: None,
                });

        let blur_pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Flame Bloom Blur Pipeline"),
                layout: Some(&blur_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_blur"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: Self::FILM_FORMAT,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        let final_pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Flame Final Pipeline"),
                layout: Some(&composite_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_final"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        self.globals_buffer = Some(globals_buffer);
        self.globals_bind_group = Some(globals_bind_group);

        self.sample_layout = Some(sample_layout);
        self.blur_layout = Some(blur_layout);
        self.sampler = Some(sampler);

        self.flame_pipeline = Some(flame_pipeline);
        self.downsample_pipeline = Some(downsample_pipeline);
        self.blur_pipeline = Some(blur_pipeline);
        self.final_pipeline = Some(final_pipeline);
        self.final_format = Some(ctx.surface_format);
    }

    fn render(&mut self, frame: &GpuFrame) {
        if self.final_format != Some(frame.format) {
            // Surface format changed (unexpected) — force re-setup on next frame.
            self.flame_pipeline = None;
            self.downsample_pipeline = None;
            self.blur_pipeline = None;
            self.final_pipeline = None;
            self.final_format = None;
            return;
        }

        // Lazily create/recreate intermediate targets and bind groups.
        self.ensure_targets(frame);

        let Some(globals_buffer) = &self.globals_buffer else {
            return;
        };
        let Some(globals_bind_group) = &self.globals_bind_group else {
            return;
        };
        let Some(sample_bind_group) = &self.sample_bind_group else {
            return;
        };
        let Some(final_bind_group) = &self.final_bind_group else {
            return;
        };
        let Some(blur_x_bind_group) = &self.blur_x_bind_group else {
            return;
        };
        let Some(blur_y_bind_group) = &self.blur_y_bind_group else {
            return;
        };

        let Some(flame_pipeline) = &self.flame_pipeline else {
            return;
        };
        let Some(downsample_pipeline) = &self.downsample_pipeline else {
            return;
        };
        let Some(blur_pipeline) = &self.blur_pipeline else {
            return;
        };
        let Some(final_pipeline) = &self.final_pipeline else {
            return;
        };

        let Some(film_view) = &self.film_view else {
            return;
        };
        let Some(bloom_down_view) = &self.bloom_down_view else {
            return;
        };
        let Some(bloom_temp_view) = &self.bloom_temp_view else {
            return;
        };
        let Some(bloom_blur_view) = &self.bloom_blur_view else {
            return;
        };

        // Keep animation stable even if a frame stalls.
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_tick)
            .as_secs_f32()
            .min(1.0 / 30.0);
        self.last_tick = now;
        self.sim_time += dt;

        // Update globals.
        let elapsed = self.sim_time;
        let is_hdr = frame.is_hdr();
        #[allow(clippy::cast_precision_loss)]
        let (w, h) = (frame.width as f32, frame.height as f32);

        // HDR tuning: bigger highlight range + tighter bloom (to keep detail).
        let edr_gain = if is_hdr { 6.0 } else { 1.0 };
        let bloom_intensity = if is_hdr { 2.2 } else { 1.0 };
        let bloom_threshold = if is_hdr { 2.2 } else { 1.0 };
        let bloom_radius = if is_hdr { 2.2 } else { 1.6 };
        let flame_strength = if is_hdr { 2.20 } else { 1.0 };
        let wind = 0.12;

        let globals: [f32; 12] = [
            elapsed,
            1.15, // exposure
            bloom_threshold,
            bloom_intensity,
            edr_gain,
            bloom_radius,
            wind,
            flame_strength,
            w.max(1.0),
            h.max(1.0),
            1.0 / w.max(1.0),
            1.0 / h.max(1.0),
        ];
        frame
            .queue
            .write_buffer(globals_buffer, 0, bytemuck::bytes_of(&globals));

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Flame Film Encoder"),
            });

        // Pass 1: flame -> HDR film buffer.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Flame Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: film_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(flame_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        // Pass 2: threshold + downsample -> bloom_down.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Downsample Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: bloom_down_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(downsample_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.set_bind_group(1, sample_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        // Pass 3: blur X -> bloom_temp.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Blur X Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: bloom_temp_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(blur_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.set_bind_group(1, sample_bind_group, &[]);
            pass.set_bind_group(2, blur_x_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        // Pass 4: blur Y -> bloom_blur.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Blur Y Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: bloom_blur_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(blur_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.set_bind_group(1, sample_bind_group, &[]);
            pass.set_bind_group(2, blur_y_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        // Pass 5: composite + tonemap -> surface.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Final Composite Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(final_pipeline);
            pass.set_bind_group(0, globals_bind_group, &[]);
            pass.set_bind_group(1, final_bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
    }
}

waterui_ffi::export!();
