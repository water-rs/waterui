//! Filter composition, capture sizes and engine-provided effect timing.

use cherenkov::Instant;
use cherenkov::kurbo::Rect;
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    BlendMode, BlendSpace, Draw, Engine, FrameTime, Group, Next, Offscreen, OffscreenFormat,
    WorkingColor,
};
use cherenkov_gpu::{Gpu, GpuConfig, interop::EffectBox};
use filtrate::{
    Effect, EffectContext, EffectFrameTiming, EffectInput, EffectOutput, EffectRedrawCallback,
    EffectRenderResult, EffectSetupResult, RetainedMemory,
};
use std::error::Error;
use std::future::{Future, ready};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
    mpsc,
};
use std::time::Duration;
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, Color, ColorTargetState, ColorWrites,
    CommandEncoder, FragmentState, LoadOp, MultisampleState, Operations,
    PipelineCompilationOptions, PipelineLayoutDescriptor, PrimitiveState,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TextureSampleType,
    TextureViewDescriptor, TextureViewDimension, VertexState,
};

struct CopyEffect(mpsc::Sender<EffectFrameTiming>);

impl Effect for CopyEffect {
    fn setup(
        &mut self,
        _: &EffectContext<'_>,
    ) -> impl std::future::Future<Output = EffectSetupResult> {
        std::future::ready(Ok(()))
    }

    fn encode_render(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        self.0.send(input.timing()).expect("timing receiver alive");
        let (width, height) = input.size();
        encoder.copy_texture_to_texture(
            input.texture().as_image_copy(),
            output.texture().as_image_copy(),
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(true)
    }

    fn retained_memory(&self) -> filtrate::RetainedMemory {
        filtrate::RetainedMemory::default()
    }

    fn trim(&mut self) {}
}

split_test! {
fn filtered_group_isolates_blended_descendant_inside_pass_through_group()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let identity = engine.filter(filtrate::filters::ColorMatrix([
        1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]));
    let surface = wait!(engine.surface(Offscreen::new((8, 40), OffscreenFormat::LinearF32), || {}))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 8.0, 40.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            r.group(Group::new().filter(identity.id()), |r| {
                r.fill(
                    Rect::new(0.0, 0.0, 8.0, 40.0),
                    WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
                );
                r.group(Group::new(), |r| {
                    r.fill(
                        Rect::new(0.0, 0.0, 8.0, 40.0),
                        WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
                    );
                    r.group(Group::new().blend(BlendMode::DestOut), |r| {
                        r.fill(Rect::new(0.0, 12.0, 8.0, 28.0), WorkingColor::WHITE);
                    });
                });
            });
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    for y in 0..40 {
        let expected = if (12..28).contains(&y) {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            [0.0, 1.0, 0.0, 1.0]
        };
        for x in 0..8 {
            let pixel = pixels[y * 8 + x];
            for (actual, expected) in pixel.into_iter().zip(expected) {
                assert!(
                    (actual - expected).abs() <= 1.0e-3,
                    "pixel ({x}, {y}): {pixel:?}, expected {expected}"
                );
            }
        }
    }
    Ok(())
}
}

split_test! {
fn filtered_group_contains_direct_blended_descendant() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let identity = engine.filter(filtrate::filters::ColorMatrix([
        1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]));
    let surface = wait!(engine.surface(Offscreen::new((8, 40), OffscreenFormat::LinearF32), || {}))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 8.0, 40.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
            r.group(Group::new().filter(identity.id()), |r| {
                r.fill(
                    Rect::new(0.0, 0.0, 8.0, 40.0),
                    WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
                );
                r.group(Group::new().blend(BlendMode::DestOut), |r| {
                    r.fill(Rect::new(0.0, 12.0, 8.0, 28.0), WorkingColor::WHITE);
                });
            });
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    for y in 0..40 {
        let expected = if (12..28).contains(&y) {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [0.0, 0.0, 1.0, 1.0]
        };
        for x in 0..8 {
            let pixel = pixels[y * 8 + x];
            for (actual, expected) in pixel.into_iter().zip(expected) {
                assert!(
                    (actual - expected).abs() <= 1.0e-3,
                    "pixel ({x}, {y}): {pixel:?}, expected {expected}"
                );
            }
        }
    }
    Ok(())
}
}

split_test! {
fn engine_executes_composed_filters_and_effects_after_resize()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16), || {}))?;
    let layer = surface.layer();
    let (send, receive) = mpsc::channel();
    let effect = engine.effect(EffectBox::from(CopyEffect(send)));
    let invert = engine.filter(filtrate::filters::Invert);
    surface.update(|tx| {
        tx[surface.root()].push(&layer).filter(effect.id());
        tx[&layer].filter(invert.id()).content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                WorkingColor::new([0.25, 0.5, 0.75, 1.0]),
            );
        }));
    });
    let start = Instant::now();
    assert!(matches!(
        wait!(engine.render(FrameTime::unpresented_at(start)))?,
        Next::At { .. }
    ));
    assert_eq!(receive.try_recv()?.presentation_time(), Duration::ZERO);
    let readback = wait!(surface.readback())?;
    for (actual, expected) in readback.pixels[16 * 32 + 16]
        .into_iter()
        .zip([0.75, 0.5, 0.25, 1.0])
    {
        assert!(
            (actual - expected).abs() < 0.002,
            "filter output {actual}, expected {expected}"
        );
    }
    surface.resize((8, 8))?;
    wait!(engine.render(FrameTime::unpresented_at(start + Duration::from_millis(250))))?;
    let timing = receive.try_recv()?;
    assert_eq!(timing.presentation_time(), Duration::from_millis(250));
    assert_eq!(timing.delta(), Duration::from_millis(250));
    assert_eq!(timing.sequence(), 1);
    surface.update(|tx| {
        tx[surface.root()].clear_filter();
    });
    assert_eq!(
        wait!(engine.render(FrameTime::unpresented_at(start + Duration::from_millis(500))))?,
        Next::Idle,
        "a registered effect detached from every layer cannot keep the host awake"
    );
    Ok(())
}
}

struct ObservedEffect {
    callbacks: mpsc::Sender<filtrate::EffectRedrawCallback>,
    timings: mpsc::Sender<EffectFrameTiming>,
    callback: Option<filtrate::EffectRedrawCallback>,
    fail_setup: bool,
}

impl Effect for ObservedEffect {
    fn set_redraw_callback(&mut self, callback: filtrate::EffectRedrawCallback) {
        self.callback = Some(callback);
    }

    fn setup(
        &mut self,
        _: &EffectContext<'_>,
    ) -> impl std::future::Future<Output = EffectSetupResult> {
        let callback = self
            .callback
            .as_ref()
            .expect("callback installed before setup");
        self.callbacks
            .send(callback.clone())
            .expect("callback receiver");
        callback();
        std::future::ready(if self.fail_setup {
            Err(filtrate::EffectSetupError::EmptyGraph)
        } else {
            Ok(())
        })
    }

    fn encode_render(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> EffectRenderResult {
        self.timings.send(input.timing()).expect("timing receiver");
        let (width, height) = input.size();
        encoder.copy_texture_to_texture(
            input.texture().as_image_copy(),
            output.texture().as_image_copy(),
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Ok(false)
    }

    fn retained_memory(&self) -> filtrate::RetainedMemory {
        filtrate::RetainedMemory::default()
    }

    fn trim(&mut self) {}
}

split_test! {
fn filter_wakes_coalesce_and_stop_after_detach() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake = Arc::clone(&wakes);
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(
        Offscreen::new((8, 8), OffscreenFormat::LinearF16),
        move || {
            wake.fetch_add(1, Ordering::Relaxed);
        }
    ))?;
    let (callbacks, receive) = mpsc::channel();
    let (timings, frames) = mpsc::channel();
    let effect = engine.effect(EffectBox::from(ObservedEffect {
        callbacks,
        timings,
        callback: None,
        fail_setup: false,
    }));
    surface.update(|tx| {
        tx[surface.root()].filter(effect.id()).content(
            surface.record(|r| r.fill(Rect::new(0.0, 0.0, 8.0, 8.0), WorkingColor::WHITE)),
        );
    });
    assert!(
        matches!(wait!(engine.render(FrameTime::now()))?, Next::At { .. }),
        "setup-time request survives consumption"
    );
    let callback = receive.try_recv()?;
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(
        engine.stats().commands_lowered,
        0,
        "filter redraw reuses prepared commands"
    );
    assert_eq!(frames.try_iter().count(), 2);
    let before = wakes.load(Ordering::Relaxed);
    callback();
    callback();
    assert_eq!(wakes.load(Ordering::Relaxed), before + 1);
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    surface.update(|tx| {
        tx[surface.root()].clear_filter();
    });
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    let before = wakes.load(Ordering::Relaxed);
    callback();
    assert_eq!(wakes.load(Ordering::Relaxed), before);
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    surface.update(|tx| {
        tx[surface.root()].filter(effect.id());
    });
    wait!(engine.render(FrameTime::now()))?;
    drop(surface);
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    let before = wakes.load(Ordering::Relaxed);
    callback();
    assert_eq!(
        wakes.load(Ordering::Relaxed),
        before,
        "destroyed surface disables its filter wake"
    );
    drop(effect);
    wait!(engine.render(FrameTime::now()))?;
    callback();
    assert_eq!(wakes.load(Ordering::Relaxed), before);
    Ok(())
}
}

split_test! {
fn filter_setup_failure_keeps_render_thread_alive() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        timestamps: true,
        ..GpuConfig::default()
    }))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {}))?;
    let (callbacks, _receive) = mpsc::channel();
    let (timings, _frames) = mpsc::channel();
    let effect = engine.effect(EffectBox::from(ObservedEffect {
        callbacks,
        timings,
        callback: None,
        fail_setup: true,
    }));
    surface.update(|tx| {
        tx[surface.root()].filter(effect.id()).content(
            surface.record(|r| r.fill(Rect::new(0.0, 0.0, 8.0, 8.0), WorkingColor::WHITE)),
        );
    });
    assert!(
        matches!(wait!(engine.render(FrameTime::now())), Err(cherenkov::RenderError::Render(message)) if message.contains("setup"))
    );
    surface.update(|tx| {
        tx[surface.root()].clear_filter();
    });
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(
        wait!(surface.readback())?.pixels[4 * 8 + 4].map(f32::to_bits),
        [1.0_f32.to_bits(); 4]
    );
    wait!(engine.finish_timings())?;
    Ok(())
}
}

split_test! {
fn shared_effect_consumes_one_delta_per_presentation() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {}))?;
    let (callbacks, _receive) = mpsc::channel();
    let (timings, frames) = mpsc::channel();
    let effect = engine.effect(EffectBox::from(ObservedEffect {
        callbacks,
        timings,
        callback: None,
        fail_setup: false,
    }));
    let first = surface.layer();
    let second = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&first).push(&second);
        for layer in [&first, &second] {
            tx[layer].filter(effect.id()).content(
                surface.record(|r| r.fill(Rect::new(0.0, 0.0, 8.0, 8.0), WorkingColor::WHITE)),
            );
        }
    });
    let start = Instant::now();
    wait!(engine.render(FrameTime::unpresented_at(start)))?;
    assert_eq!(frames.try_iter().count(), 2);
    wait!(engine.render(FrameTime::unpresented_at(start + Duration::from_millis(250))))?;
    let a = frames.try_recv()?;
    let b = frames.try_recv()?;
    assert_eq!(a.presentation_time(), b.presentation_time());
    assert_eq!(a.sequence(), b.sequence());
    assert_eq!(a.delta(), Duration::from_millis(250));
    assert_eq!(b.delta(), Duration::ZERO);
    Ok(())
}
}

split_test! {
/// `filter_images` counts the textures a chain uploads itself (#2766): a
/// CPU auxiliary image's bytes join `gpu` while the filter is registered
/// and leave with it.
fn layer_filter_reports_its_uploaded_image() -> Result<(), Box<dyn std::error::Error>> {
    // A 16×16 Rgba8 auxiliary image — 1024 bytes the executor uploads at
    // setup and retains for the chain's life.
    const IMAGE_BYTES: u64 = 16 * 16 * 4;
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {}))?;
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(
            surface.record(|r| r.fill(Rect::new(0.0, 0.0, 8.0, 8.0), WorkingColor::WHITE)),
        );
    });
    wait!(engine.render(FrameTime::now()))?;
    // Both ends are measured trimmed, so the renderer's grow-only caches
    // (a filtered layer's isolated target among them) cannot tell them apart.
    engine.trim(cherenkov::Pressure::Critical);
    let before = wait!(engine.memory())?;
    assert_eq!(before.filter_images, cherenkov::Bytes(0));
    // A filter that uploads nothing gives what a filtered layer costs
    // apart from its chain's own targets and images: the layer's isolated
    // target is part of `gpu` whatever the filter.
    let invert = engine.filter(filtrate::filters::Invert);
    surface.update(|tx| {
        tx[&layer].filter(invert.id());
    });
    wait!(engine.render(FrameTime::now()))?;
    let plain = wait!(engine.memory())?;
    assert_eq!(plain.filter_images, cherenkov::Bytes(0));
    let blend = engine.filter(filtrate::filters::BlendWithImage {
        image: filtrate::FilterImage::from_rgba8(16, 16, vec![128; 16 * 16 * 4]),
        amount: 0.5,
        mode: filtrate::filters::BlendMode::Normal,
    });
    surface.update(|tx| {
        tx[&layer].filter(blend.id());
    });
    drop(invert);
    wait!(engine.render(FrameTime::now()))?;
    let memory = wait!(engine.memory())?;
    assert_eq!(memory.filter_images, cherenkov::Bytes(IMAGE_BYTES));
    // The upload joins `gpu` exactly once: with each side's chain targets
    // and images taken out, the two filtered layers cost the same.
    let rest = |m: &cherenkov::MemoryUsage| m.gpu.0 - m.filter_targets.0 - m.filter_images.0;
    assert_eq!(
        rest(&memory),
        rest(&plain),
        "the uploaded image's bytes join `gpu` once, with the chain's targets"
    );
    // The upload leaves with the chain: detaching and dropping the
    // filter returns `gpu` to its baseline.
    surface.update(|tx| {
        tx[&layer].clear_filter();
    });
    drop(blend);
    wait!(engine.render(FrameTime::now()))?;
    engine.trim(cherenkov::Pressure::Critical);
    let after = wait!(engine.memory())?;
    assert_eq!(after.filter_images, cherenkov::Bytes(0));
    assert_eq!(after.filter_targets, cherenkov::Bytes(0));
    assert_eq!(after.gpu, before.gpu);
    Ok(())
}
}

split_test! {
fn recorded_filter_groups_survive_opacity_speculation_and_live_patches()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {}))?;
    let invert = engine.filter(filtrate::filters::Invert);
    let color = nami::binding(WorkingColor::new([0.25, 0.5, 0.75, 1.0]));
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.group(cherenkov::Group::new().opacity(0.5), |r| {
                r.group(cherenkov::Group::new().filter(invert.id()), |r| {
                    r.fill(Rect::new(0.0, 0.0, 8.0, 8.0), color.clone());
                });
            });
        }));
    });
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    let pixel = wait!(surface.readback())?.pixels[4 * 8 + 4];
    for (actual, expected) in pixel.into_iter().zip([0.375, 0.25, 0.125, 0.5]) {
        assert!(
            (actual - expected).abs() < 0.002,
            "group filter/opacity: {pixel:?}"
        );
    }
    color.set(WorkingColor::new([0.75, 0.5, 0.25, 1.0]));
    wait!(engine.render(FrameTime::now()))?;
    assert_eq!(engine.stats().commands_lowered, 1);
    let pixel = wait!(surface.readback())?.pixels[4 * 8 + 4];
    for (actual, expected) in pixel.into_iter().zip([0.125, 0.25, 0.375, 0.5]) {
        assert!(
            (actual - expected).abs() < 0.002,
            "live group filter/opacity: {pixel:?}"
        );
    }
    Ok(())
}
}

fn assert_rgba_near(actual: [f32; 4], expected: [f32; 4], tol: f32, context: &str) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!(
            (a - e).abs() <= tol,
            "{context}: pixel {actual:?}, expected {expected:?} (tol {tol})"
        );
    }
}

const RESIZE_SHADER_WGSL: &str = r"
@group(0) @binding(0) var src_tex: texture_2d<f32>;

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOut {
    let x = f32((vid << 1u) & 2u) * 2.0 - 1.0;
    let y = 1.0 - f32(vid & 2u) * 2.0;
    return VertexOut(vec4<f32>(x, y, 0.0, 1.0));
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let in_dims = vec2<i32>(textureDimensions(src_tex));
    let coord = vec2<i32>(floor(pos.xy));
    if coord.x < in_dims.x && coord.y < in_dims.y {
        return textureLoad(src_tex, coord, 0);
    }
    let center = textureLoad(src_tex, in_dims / 2, 0);
    return vec4<f32>(center.g, center.b, center.r, center.a * 0.5);
}

@fragment
fn fs_rotate_pad(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let in_dims = vec2<i32>(textureDimensions(src_tex));
    let clamped = clamp(vec2<i32>(floor(pos.xy)) - vec2<i32>(5, 5), vec2<i32>(0), in_dims - 1);
    let c = textureLoad(src_tex, clamped, 0);
    return vec4<f32>(c.g, c.b, c.r, c.a);
}
";

struct DynamicOutputShaderEffect {
    width: Arc<AtomicU32>,
    height: Arc<AtomicU32>,
    relative_grow: (u32, u32),
    entry_point: &'static str,
    observed: mpsc::Sender<((u32, u32), (u32, u32))>,
    callback: Option<EffectRedrawCallback>,
    callbacks: Option<mpsc::Sender<EffectRedrawCallback>>,
    layout: Option<BindGroupLayout>,
    pipeline: Option<RenderPipeline>,
}

impl DynamicOutputShaderEffect {
    fn new(
        width: Arc<AtomicU32>,
        height: Arc<AtomicU32>,
        observed: mpsc::Sender<((u32, u32), (u32, u32))>,
        callbacks: Option<mpsc::Sender<EffectRedrawCallback>>,
    ) -> Self {
        Self {
            width,
            height,
            relative_grow: (0, 0),
            entry_point: "fs_main",
            observed,
            callback: None,
            callbacks,
            layout: None,
            pipeline: None,
        }
    }

    fn relative_rotate(
        grow_w: u32,
        grow_h: u32,
        observed: mpsc::Sender<((u32, u32), (u32, u32))>,
    ) -> Self {
        Self {
            width: Arc::new(AtomicU32::new(0)),
            height: Arc::new(AtomicU32::new(0)),
            relative_grow: (grow_w, grow_h),
            entry_point: "fs_rotate_pad",
            observed,
            callback: None,
            callbacks: None,
            layout: None,
            pipeline: None,
        }
    }
}

impl Effect for DynamicOutputShaderEffect {
    fn output_size(&self, input_width: u32, input_height: u32) -> (u32, u32) {
        if self.relative_grow != (0, 0) {
            return (
                input_width + self.relative_grow.0,
                input_height + self.relative_grow.1,
            );
        }
        let w = self.width.load(Ordering::Acquire);
        let h = self.height.load(Ordering::Acquire);
        (
            if w == 0 { input_width } else { w },
            if h == 0 { input_height } else { h },
        )
    }

    fn set_redraw_callback(&mut self, callback: EffectRedrawCallback) {
        if let Some(tx) = &self.callbacks {
            tx.send(callback.clone()).expect("callback receiver alive");
        }
        self.callback = Some(callback);
    }

    fn setup(&mut self, ctx: &EffectContext<'_>) -> impl Future<Output = EffectSetupResult> {
        let module = ctx.device.create_shader_module(ShaderModuleDescriptor {
            label: Some("dynamic output test shader"),
            source: ShaderSource::Wgsl(RESIZE_SHADER_WGSL.into()),
        });
        let layout = ctx
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("dynamic output test layout"),
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: false },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("dynamic output test pipeline layout"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = ctx
            .device
            .create_render_pipeline(&RenderPipelineDescriptor {
                label: Some("dynamic output test pipeline"),
                layout: Some(&pipeline_layout),
                vertex: VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: PipelineCompilationOptions::default(),
                },
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                fragment: Some(FragmentState {
                    module: &module,
                    entry_point: Some(self.entry_point),
                    targets: &[Some(ColorTargetState {
                        format: ctx.output_format,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                    compilation_options: PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache: None,
            });
        self.layout = Some(layout);
        self.pipeline = Some(pipeline);
        ready(Ok(()))
    }

    fn encode_render(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut CommandEncoder,
    ) -> EffectRenderResult {
        self.observed
            .send((input.size(), output.size()))
            .expect("size observer alive");
        let input_view = input
            .texture()
            .create_view(&TextureViewDescriptor::default());
        let output_view = output
            .texture()
            .create_view(&TextureViewDescriptor::default());
        let layout = self.layout.as_ref().expect("setup ran");
        let pipeline = self.pipeline.as_ref().expect("setup ran");
        let bind_group = input.device().create_bind_group(&BindGroupDescriptor {
            label: Some("dynamic output bind group"),
            layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&input_view),
            }],
        });
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("dynamic output pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &output_view,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::TRANSPARENT),
                    store: StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
        Ok(false)
    }

    fn retained_memory(&self) -> RetainedMemory {
        RetainedMemory::default()
    }

    fn trim(&mut self) {}
}

split_test! {
/// A small captured subtree whose effect declares window-sized output must
/// keep unscaled tight input dimensions and render outside source bounds
/// across the whole surface without clipping to the input scratch texture.
fn small_subtree_effect_expands_to_window_size_without_scaling_input()
-> Result<(), Box<dyn Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(
        Offscreen::new((64, 64), OffscreenFormat::LinearF16),
        || {}
    ))?;
    let width = Arc::new(AtomicU32::new(64));
    let height = Arc::new(AtomicU32::new(64));
    let (obs_tx, obs_rx) = mpsc::channel();
    let effect = engine.effect(EffectBox::from(DynamicOutputShaderEffect::new(
        Arc::clone(&width),
        Arc::clone(&height),
        obs_tx,
        None,
    )));
    // Subtree is a 12x12 rect at (16, 20)..(28, 32): AA margin (2px) + tight pad (1px) gives [13, 17, 18, 18].
    // Premultiplied input color: [0.4, 0.2, 0.1, 0.5]; outer tint: [0.2, 0.1, 0.4, 0.25].
    let src_color = WorkingColor::new([0.4, 0.2, 0.1, 0.5]);
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.group(Group::new().filter(effect.id()), |r| {
                r.fill(Rect::new(16.0, 20.0, 28.0, 32.0), src_color);
            });
        }));
    });
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(obs_rx.try_recv()?, ((18, 18), (64, 64)));
    let pixels = wait!(surface.readback())?.pixels;
    // Unscaled captured source inside (18, 18) -> interior pixel (6, 6) preserves premultiplied alpha.
    assert_rgba_near(
        pixels[6 * 64 + 6],
        [0.2, 0.1, 0.05, 0.5],
        0.005,
        "window-expanded unscaled source region",
    );
    // Pixels outside original source bounds [13..31, 17..35] still receive premultiplied effect output.
    assert_rgba_near(
        pixels[55 * 64 + 55],
        [0.1, 0.05, 0.2, 0.25],
        0.005,
        "outside source bounds at bottom-right",
    );
    assert_rgba_near(
        pixels[2 * 64 + 45],
        [0.1, 0.05, 0.2, 0.25],
        0.005,
        "outside source bounds at top-right",
    );
    Ok(())
}
}

split_test! {
/// Reactive `Effect::output_size` changes (growth and shrink) re-plan
/// targets, composite bounds, and damage cleanly without re-lowering
/// (`commands_lowered == 0`), preserving source placement and premultiplied alpha.
fn reactive_output_size_grows_and_shrinks_without_relowering() -> Result<(), Box<dyn Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(
        Offscreen::new((64, 64), OffscreenFormat::LinearF16),
        || {}
    ))?;
    let out_w = Arc::new(AtomicU32::new(28));
    let out_h = Arc::new(AtomicU32::new(22));
    let (obs_tx, obs_rx) = mpsc::channel();
    let (cb_tx, cb_rx) = mpsc::channel();
    let effect = engine.effect(EffectBox::from(DynamicOutputShaderEffect::new(
        Arc::clone(&out_w),
        Arc::clone(&out_h),
        obs_tx,
        Some(cb_tx),
    )));
    // Subtree is 12x12 at (16, 20)..(28, 32) -> tight_region [13, 17, 18, 18].
    // Frame 1 output: 28x22 placed at (13, 17) -> covers [13..41, 17..39], blended over teal.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(Rect::new(0.0, 0.0, 64.0, 64.0), WorkingColor::new([0.0, 0.2, 0.4, 1.0]));
            r.group(Group::new().filter(effect.id()).blend(BlendMode::PlusLighter), |r| {
                r.fill(
                    Rect::new(16.0, 20.0, 28.0, 32.0),
                    WorkingColor::new([0.4, 0.2, 0.1, 0.5]),
                );
            });
        }));
    });
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(obs_rx.try_recv()?, ((18, 18), (28, 22)));
    let callback = cb_rx.try_recv()?;
    let pixels = wait!(surface.readback())?.pixels;
    assert_rgba_near(pixels[22 * 64 + 18], [0.2, 0.3, 0.45, 1.0], 0.005, "grow frame src PlusLighter");
    assert_rgba_near(pixels[25 * 64 + 38], [0.1, 0.25, 0.6, 1.0], 0.005, "grow frame expanded PlusLighter");
    assert_rgba_near(pixels[5 * 64 + 5], [0.0, 0.2, 0.4, 1.0], 0.005, "outside placed grow");

    // Frame 2: reactive asymmetric shrink to (7, 10) at (13, 17) -> [13..20, 17..27].
    out_w.store(7, Ordering::Release);
    out_h.store(10, Ordering::Release);
    callback();
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(engine.stats().commands_lowered, 0);
    assert_eq!(obs_rx.try_recv()?, ((18, 18), (7, 10)));
    let pixels = wait!(surface.readback())?.pixels;
    assert_rgba_near(pixels[22 * 64 + 18], [0.2, 0.3, 0.45, 1.0], 0.005, "shrink keeps origin src");
    assert_rgba_near(
        pixels[25 * 64 + 38],
        [0.0, 0.2, 0.4, 1.0],
        0.005,
        "previously expanded region cleared on reactive shrink",
    );
    assert_rgba_near(
        pixels[25 * 64 + 25],
        [0.0, 0.2, 0.4, 1.0],
        0.005,
        "clipped beyond shrunk width cleared to background",
    );

    // Frame 3: reactive expansion to window size (64, 64) with zero re-lowering.
    out_w.store(64, Ordering::Release);
    out_h.store(64, Ordering::Release);
    callback();
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(engine.stats().commands_lowered, 0);
    assert_eq!(obs_rx.try_recv()?, ((18, 18), (64, 64)));
    let pixels = wait!(surface.readback())?.pixels;
    assert_rgba_near(pixels[60 * 64 + 60], [0.1, 0.25, 0.6, 1.0], 0.005, "reactive window expansion");
    Ok(())
}
}

split_test! {
/// Nested filters honor inner output expansion before outer capture and
/// preserve filter chain order and sRGB-encoded space conversion across
/// distinct input/output dimensions.
fn nested_filters_and_srgb_scope_preserve_order_and_sizes() -> Result<(), Box<dyn Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(
        Offscreen::new((64, 64), OffscreenFormat::LinearF16),
        || {}
    ))?;
    let inner_obs = mpsc::channel();
    let mid_obs = mpsc::channel();
    let outer_obs = mpsc::channel();
    // Inner expands 12x12 [19..31, 19..31] (tight [18, 18, 14, 14]) to (28, 28) at (18, 18)..(46, 46).
    let inner = engine.effect(EffectBox::from(DynamicOutputShaderEffect::new(
        Arc::new(AtomicU32::new(28)),
        Arc::new(AtomicU32::new(28)),
        inner_obs.0,
        None,
    )));
    // Mid observes inner's expanded composite (padded -> 30x30) and outputs (32, 32).
    let mid = engine.effect(EffectBox::from(DynamicOutputShaderEffect::new(
        Arc::new(AtomicU32::new(32)),
        Arc::new(AtomicU32::new(32)),
        mid_obs.0,
        None,
    )));
    // Outer rotates channels RGB -> GBR and grows by +10,+10 inside an SrgbEncoded scope.
    let outer_effect = engine.effect(EffectBox::from(DynamicOutputShaderEffect::relative_rotate(
        10,
        10,
        outer_obs.0,
    )));
    let inner_layer = surface.layer();
    let mid_layer = surface.layer();
    let outer_layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&outer_layer);
        tx[&outer_layer]
            .push(&mid_layer)
            .blend_space(BlendSpace::SrgbEncoded)
            .filter(outer_effect.id());
        tx[&mid_layer].push(&inner_layer).filter(mid.id());
        tx[&inner_layer].filter(inner.id()).content(surface.record(|r| {
            r.fill(Rect::new(19.0, 19.0, 31.0, 31.0), WorkingColor::new([0.8, 0.4, 0.2, 1.0]));
        }));
    });
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(inner_obs.1.try_recv()?, ((18, 18), (28, 28)));
    assert_eq!(mid_obs.1.try_recv()?, ((30, 30), (32, 32)));
    assert_eq!(outer_obs.1.try_recv()?, ((34, 34), (44, 44)));
    // Expanded output from outer_effect covers [14..58, 14..58]:
    // - (30, 30) reads inner's unscaled source ([0.8, 0.4, 0.2, 1.0]) -> rotated [0.4, 0.2, 0.8, 1.0]
    // - (50, 50) (outside inner [16..44] and mid [15..47]) reads mid's expanded region -> rotated.
    let pixels = wait!(surface.readback())?.pixels;
    let interior = pixels[30 * 64 + 30];
    assert!(
        interior[2] > interior[0] && interior[0] > interior[1] && interior[3] > 0.99,
        "outer rotated interior source channels cleanly in sRGB scope: {interior:?}"
    );
    let expanded_sample = pixels[50 * 64 + 50];
    assert!(
        expanded_sample[3] > 0.1,
        "outer expanded region receives mid's expanded output in sRGB scope: {expanded_sample:?}"
    );
    assert!(
        expanded_sample[1] > expanded_sample[2] && expanded_sample[2] > expanded_sample[0],
        "outer channel rotation preserved filter chain order: {expanded_sample:?}"
    );
    Ok(())
}
}
