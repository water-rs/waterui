//! Real browser WebGPU execution, including non-Send producers and device reuse.
#![cfg(target_arch = "wasm32")]

use cherenkov::kurbo::Rect;
use cherenkov::{
    Draw, Engine, FontSource, FrameTime, Glyph, GlyphRun, GlyphStyle, ImageData, Next, Offscreen,
    OffscreenFormat, Rgba8, Sampling, ShaderPaint, ShaderSource, WorkingColor,
};
use cherenkov_gpu::{
    Gpu, GpuConfig,
    interop::{
        ChromaSiting, ExternalFrame, FrameColor, GpuContent, GpuContentBox, InvalidFrame,
        Primaries, RgbAlpha, SharedDevice, Transfer, YuvMatrix, YuvRange,
        web::{InvalidWebTexture, WebTexture, WebTextureLease},
        wgpu,
    },
};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

struct LocalProducer {
    setups: Rc<Cell<u32>>,
    frames: Rc<Cell<u32>>,
    drops: Rc<Cell<u32>>,
    during_setup: Option<Box<dyn FnOnce()>>,
    device: wgpu::Device,
}

impl Drop for LocalProducer {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

impl GpuContent for LocalProducer {
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn setup(&mut self, context: &wgpu::Context<'_>) {
        assert_eq!(context.device, &self.device, "supplied device identity");
        assert_eq!(context.device.features(), self.device.features());
        gloo_timers::future::TimeoutFuture::new(0).await;
        self.during_setup.take().expect("one setup")();
        self.setups.set(self.setups.get() + 1);
    }
    fn render(&mut self, frame: &mut wgpu::Frame<'_>) {
        self.frames.set(self.frames.get() + 1);
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        frame.queue.submit([encoder.finish()]);
    }
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn local_producers_share_device_and_preserve_wakes_during_await() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .expect("WebGPU adapter required");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("WebGPU device");
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(SharedDevice {
            instance,
            adapter,
            device: device.clone(),
            queue,
        }),
        ..Default::default()
    })
    .await
    .expect("engine");
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    let color = nami::binding(WorkingColor::WHITE);
    surface.update(|tx| {
        tx[surface.root()]
            .content(surface.record(|r| r.fill(Rect::new(0., 0., 8., 16.), color.clone())));
    });
    engine.render(FrameTime::now()).await.expect("first frame");
    let wakes = Rc::new(Cell::new(0));
    let counter = wakes.clone();
    engine.set_waker(move || counter.set(counter.get() + 1));
    let setups = Rc::new(Cell::new(0));
    let frames = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let producer = GpuContentBox::new(
        LocalProducer {
            setups: setups.clone(),
            frames: frames.clone(),
            drops: drops.clone(),
            device,
            during_setup: Some(Box::new(move || {
                color.set(WorkingColor::new([0., 1., 0., 1.]));
            })),
        },
        || {},
    );
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer]
            .transform(cherenkov::kurbo::Affine::translate((8., 0.)))
            .content(engine.gpu_producer(producer).at((8, 16)));
    });
    let before = wakes.get();
    engine
        .render(FrameTime::now())
        .await
        .expect("producer setup");
    assert_eq!(
        wakes.get(),
        before + 1,
        "signal during setup requests next frame"
    );
    assert_eq!(
        engine.render(FrameTime::now()).await.expect("live update"),
        Next::Idle
    );
    assert_eq!(engine.stats().commands_lowered, 1);
    let pixels = surface.readback().await.expect("browser mapping").pixels;
    assert!((pixels[4 * 16 + 4][1] - 1.).abs() < 0.001);
    assert!((pixels[4 * 16 + 12][0] - 1.).abs() < 0.001);
    wasm_bindgen_test::console_log!("BROWSER_PIXELS {:?}", pixels);
    assert_eq!(setups.get(), 1);
    assert_eq!(frames.get(), 1);
    engine.render(FrameTime::now()).await.expect("idle");
    assert_eq!(engine.stats().commands_lowered, 0);
    assert_eq!(frames.get(), 1);
    drop(engine);
    // Readback queues after shutdown and observes disconnection, including
    // when surface/layer handles outlive the engine.
    assert!(surface.readback().await.is_err());
    assert_eq!(drops.get(), 1);
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn shader_validation_returns_errors_before_queueing() {
    let engine = Engine::<Gpu>::new(GpuConfig::default())
        .await
        .expect("engine");
    assert!(matches!(
        engine.shader(ShaderSource::wgsl("invalid shader")),
        Err(cherenkov::ResourceError::Shader(_))
    ));
    let shader = engine.shader(ShaderSource::wgsl(BLUE)).expect("shader");
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|r| {
            r.fill(
                Rect::new(0., 0., 8., 8.),
                ShaderPaint {
                    shader: shader.id(),
                    uniforms: vec![],
                },
            );
        }));
    });
    assert_eq!(
        engine
            .render(FrameTime::now())
            .await
            .expect("shader render"),
        Next::Idle
    );
    let pixels = surface.readback().await.expect("pixels").pixels;
    assert!((pixels[4 * 8 + 4][2] - 1.).abs() < 0.001);
}

/// A shader paint filling its shape with opaque blue.
const BLUE: &str =
    "@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 0.0, 1.0, 1.0); }";

/// A font, an image and a shader registered and drawn in the same frame,
/// with no await between registration and recording: the synchronous pass
/// a scene builder runs.
#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn resources_registered_while_recording_draw_in_the_same_frame() {
    let engine = Engine::<Gpu>::new(GpuConfig::default())
        .await
        .expect("engine");
    let surface = engine
        .surface(Offscreen::new((48, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    let font = engine
        .font(FontSource::bytes(
            include_bytes!("../../scenes/fonts/NotoSans.ttf").as_slice(),
        ))
        .expect("font");
    let image = engine
        .image(ImageData::<Rgba8>::new(1, 1, vec![255; 4]).expect("image data"))
        .expect("image");
    let shader = engine.shader(ShaderSource::wgsl(BLUE)).expect("shader");
    surface.update(|tx| {
        tx[surface.root()].record(|c| {
            c.glyphs(
                GlyphRun {
                    font: font.id(),
                    size: 14.0,
                    coords: Vec::new().into(),
                    glyphs: vec![Glyph {
                        id: 36,
                        x: 2.0,
                        y: 13.0,
                        transform: None,
                    }]
                    .into(),
                    style: GlyphStyle::Fill,
                },
                WorkingColor::WHITE,
            );
            c.image(image.id(), Rect::new(16., 0., 32., 16.), Sampling::Nearest);
            c.fill(
                Rect::new(32., 0., 48., 16.),
                ShaderPaint {
                    shader: shader.id(),
                    uniforms: vec![],
                },
            );
        });
    });
    assert_eq!(
        engine.render(FrameTime::now()).await.expect("frame"),
        Next::Idle
    );
    let pixels = surface.readback().await.expect("pixels").pixels;
    let at = |x: usize, y: usize| pixels[y * 48 + x];
    assert!(
        (0..16).any(|y| (0..16).any(|x| at(x, y)[3] > 0.5)),
        "the glyph covers its cell"
    );
    for channel in at(24, 8) {
        assert!((channel - 1.).abs() < 0.001, "white image {:?}", at(24, 8));
    }
    assert!(
        (at(40, 8)[2] - 1.).abs() < 0.001,
        "blue shader {:?}",
        at(40, 8)
    );
    assert!(
        (at(40, 8)[3] - 1.).abs() < 0.001,
        "opaque shader {:?}",
        at(40, 8)
    );
}

struct YieldingEffect {
    callback: Option<filtrate::EffectRedrawCallback>,
    frames: Rc<Cell<u32>>,
}

impl filtrate::Effect for YieldingEffect {
    fn set_redraw_callback(&mut self, callback: filtrate::EffectRedrawCallback) {
        self.callback = Some(callback);
    }
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    async fn setup(&mut self, _: &filtrate::EffectContext<'_>) -> filtrate::EffectSetupResult {
        gloo_timers::future::TimeoutFuture::new(0).await;
        self.callback.as_ref().expect("redraw installed")();
        Ok(())
    }
    fn encode_render(
        &mut self,
        input: &filtrate::EffectInput<'_>,
        output: &filtrate::EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> filtrate::EffectRenderResult {
        self.frames.set(self.frames.get() + 1);
        encoder.copy_texture_to_texture(
            input.texture.as_image_copy(),
            output.texture.as_image_copy(),
            wgpu::Extent3d {
                width: input.width,
                height: input.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(false)
    }
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn filters_keep_redraw_requests_made_during_async_setup() {
    let engine = Engine::<Gpu>::new(GpuConfig::default())
        .await
        .expect("engine");
    let frames = Rc::new(Cell::new(0));
    let filter = engine.effect(cherenkov_gpu::interop::EffectBox::from(YieldingEffect {
        callback: None,
        frames: frames.clone(),
    }));
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()]
            .filter(&filter)
            .content(surface.record(|r| r.fill(Rect::new(0., 0., 8., 8.), WorkingColor::WHITE)));
    });
    assert!(matches!(
        engine.render(FrameTime::now()).await.expect("setup frame"),
        Next::At { .. }
    ));
    assert_eq!(
        engine
            .render(FrameTime::now())
            .await
            .expect("requested frame"),
        Next::Idle
    );
    assert_eq!(frames.get(), 2);
    engine.render(FrameTime::now()).await.expect("idle");
    assert_eq!(frames.get(), 2);
    let pixels = surface.readback().await.expect("filtered pixels").pixels;
    assert!((pixels[4 * 8 + 4][0] - 1.).abs() < 0.001);
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn browser_incremental_matches_full_lowering() {
    use cherenkov::Backend;
    let (mut renderer, _) = Gpu::init(GpuConfig::default())
        .await
        .expect("WebGPU adapter required");
    cherenkov::testing::incremental::equivalence(&mut renderer).await;
}

// ---- Foreign GPUTexture external frames (#167) --------------------------
//
// A producer holds a `GPUTexture` on the engine's device; `interop::web`
// wraps it in place and the shared RGB external-frame path samples it
// without a copy. The "producer" side of these tests is a `wgpu::Texture`
// on the same device: its `as_webgpu` handle is the same DOM `GPUTexture`
// a video element or web view would hand over.

/// The engine on a shared WebGPU device; the test keeps `device`/`queue`
/// clones for the producer side.
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn shared_engine() -> (wgpu::Device, wgpu::Queue, Engine<Gpu>) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .expect("WebGPU adapter required");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("WebGPU device");
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(SharedDevice {
            instance,
            adapter,
            device: device.clone(),
            queue: queue.clone(),
        }),
        ..Default::default()
    })
    .await
    .expect("engine");
    (device, queue, engine)
}

const fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

const fn plane_desc(
    format: wgpu::TextureFormat,
    size: wgpu::Extent3d,
    mips: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("foreign plane"),
        size,
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    }
}

/// A texture on `device` standing in for a foreign producer's `GPUTexture`;
/// the producer keeps it — and so the DOM handle — alive across the lease.
fn foreign_plane(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    desc: &wgpu::TextureDescriptor<'_>,
    bytes_per_row: u32,
    data: &[u8],
) -> wgpu::Texture {
    let texture = device.create_texture(desc);
    if !data.is_empty() {
        queue.write_texture(
            texture.as_image_copy(),
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(desc.size.height),
            },
            desc.size,
        );
    }
    texture
}

/// `texel` tiled over a `w`×`h` plane.
fn fill(texel: &[u8], w: u32, h: u32) -> Vec<u8> {
    texel
        .iter()
        .copied()
        .cycle()
        .take(w as usize * h as usize * texel.len())
        .collect()
}

fn f16_plane(texel: [f32; 4], w: u32, h: u32) -> Vec<u8> {
    let bytes: Vec<u8> = texel
        .into_iter()
        .flat_map(|v| half::f16::from_f32(v).to_le_bytes())
        .collect();
    fill(&bytes, w, h)
}

/// The `GPUTexture`/`GPUDevice` handles plus a lease counting its releases.
fn offer(device: &wgpu::Device, plane: &wgpu::Texture, releases: &Rc<Cell<u32>>) -> WebTexture {
    let releases = releases.clone();
    WebTexture {
        texture: plane.as_webgpu().expect("WebGPU texture").clone(),
        device: device.as_webgpu().expect("WebGPU device").clone(),
        release: WebTextureLease::new(move || releases.set(releases.get() + 1)),
    }
}

/// Installs one RGB external frame of `plane` on a fresh layer of `surface`.
fn show_frame(
    surface: &cherenkov::Surface<Gpu>,
    engine: &Engine<Gpu>,
    plane: &wgpu::Texture,
    alpha: RgbAlpha,
    color: FrameColor,
    at: (f64, f64),
) -> cherenkov::Layer {
    let (video, sink) = engine.frame_producer();
    sink.submit(
        ExternalFrame::rgb(plane.clone(), alpha, color).expect("plane meets the frame contract"),
    );
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer]
            .transform(cherenkov::kurbo::Affine::translate(at))
            .content(video.at((plane.width(), plane.height())));
    });
    layer
}

/// Swaps the content of an attached layer for a new external frame.
fn replace_frame(
    surface: &cherenkov::Surface<Gpu>,
    engine: &Engine<Gpu>,
    layer: &cherenkov::Layer,
    plane: &wgpu::Texture,
    alpha: RgbAlpha,
    color: FrameColor,
) {
    let (video, sink) = engine.frame_producer();
    sink.submit(
        ExternalFrame::rgb(plane.clone(), alpha, color).expect("plane meets the frame contract"),
    );
    surface.update(|tx| {
        tx[layer].content(video.at((plane.width(), plane.height())));
    });
}

#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn pixels(surface: &cherenkov::Surface<Gpu>) -> Vec<[f32; 4]> {
    surface.readback().await.expect("readback").pixels
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_rgb_formats_sample_in_place() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");

    // The same colour (r=1.0, g=0, b≈0.5) in each accepted RGB format. A
    // dominant red with the blue channel near 0.2 linear: close enough to
    // catch a byte-order or format mix-up in either direction.
    let cases: [(wgpu::TextureFormat, Vec<u8>, u32); 3] = [
        (
            wgpu::TextureFormat::Rgba8Unorm,
            fill(&[255, 0, 128, 255], 4, 4),
            16,
        ),
        (
            wgpu::TextureFormat::Bgra8Unorm,
            fill(&[128, 0, 255, 255], 4, 4),
            16,
        ),
        (
            wgpu::TextureFormat::Rgba16Float,
            f16_plane([1.0, 0.0, 0.502, 1.0], 4, 4),
            32,
        ),
    ];
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].transform(cherenkov::kurbo::Affine::translate((2.0, 2.0)));
    });
    for (format, data, row) in cases {
        let plane = foreign_plane(
            &device,
            &queue,
            &plane_desc(
                format,
                extent(4, 4),
                1,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            ),
            row,
            &data,
        );
        let imported = cherenkov_gpu::interop::web::import_texture(
            &device,
            &queue,
            offer(&device, &plane, &releases),
        )
        .await
        .expect("same-device import");
        replace_frame(
            &surface,
            &engine,
            &layer,
            &imported,
            RgbAlpha::Opaque,
            FrameColor::SRGB,
        );
        engine.render(FrameTime::now()).await.expect("render");
        let px = pixels(&surface).await[3 * 16 + 3];
        assert!(
            px[0] > 0.8 && px[1] < 0.1 && (0.1..0.35).contains(&px[2]) && px[3] > 0.99,
            "{format:?} sampled in place: {px:?}"
        );
        drop(imported);
    }
    drop(layer);
    engine.render(FrameTime::now()).await.expect("teardown");
    assert_eq!(releases.get(), 3, "each replaced lease released once");
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_rgb_alpha_modes() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    let plane = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba16Float,
            extent(4, 4),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ),
        32,
        &f16_plane([0.25, 0.25, 0.25, 0.5], 4, 4),
    );
    let imported = cherenkov_gpu::interop::web::import_texture(
        &device,
        &queue,
        offer(&device, &plane, &releases),
    )
    .await
    .expect("import");
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].transform(cherenkov::kurbo::Affine::translate((2.0, 2.0)));
    });
    // texel (0.25, 0.25, 0.25, 0.5) linear: straight multiplies rgb by alpha,
    // premultiplied passes through, opaque forces alpha to 1.
    for (alpha, expected) in [
        (RgbAlpha::Straight, [0.125, 0.125, 0.125, 0.5]),
        (RgbAlpha::Premultiplied, [0.25, 0.25, 0.25, 0.5]),
        (RgbAlpha::Opaque, [0.25, 0.25, 0.25, 1.0]),
    ] {
        replace_frame(
            &surface,
            &engine,
            &layer,
            &imported,
            alpha,
            FrameColor::LINEAR_P3,
        );
        engine.render(FrameTime::now()).await.expect("render");
        let px = pixels(&surface).await[3 * 16 + 3];
        for (got, want) in px.iter().zip(expected) {
            assert!(
                (got - want).abs() < 0.02,
                "{alpha:?}: {px:?} vs {expected:?}"
            );
        }
    }
    drop(imported);
    drop(layer);
    engine.render(FrameTime::now()).await.expect("teardown");
    assert_eq!(releases.get(), 1, "one lease, released once at teardown");
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_rgb_colour_metadata() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].transform(cherenkov::kurbo::Affine::translate((2.0, 2.0)));
    });
    // The same 0.75 grey texel through four colour contracts: only metadata
    // changes, so distinct decoded levels prove the contract reaches the
    // fragment stage. srgb < linear < hlg < pq in the working space.
    let grey = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba16Float,
            extent(4, 4),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ),
        32,
        &f16_plane([0.75, 0.75, 0.75, 1.0], 4, 4),
    );
    let imported = cherenkov_gpu::interop::web::import_texture(
        &device,
        &queue,
        offer(&device, &grey, &releases),
    )
    .await
    .expect("import");
    let mut levels = Vec::new();
    for color in [
        FrameColor::SRGB,
        FrameColor::LINEAR_P3,
        FrameColor::bt2020_hlg(1000.0),
        FrameColor::BT2020_PQ,
    ] {
        replace_frame(
            &surface,
            &engine,
            &layer,
            &imported,
            RgbAlpha::Opaque,
            color,
        );
        engine.render(FrameTime::now()).await.expect("render");
        let px = pixels(&surface).await[3 * 16 + 3];
        levels.push(px[0]);
    }
    let [srgb, linear, hlg, pq] = levels[..] else {
        panic!("four colour contracts")
    };
    assert!(
        (linear - 0.75).abs() < 0.02,
        "linear P3 passes through: {linear}"
    );
    assert!(srgb < linear, "srgb decodes darker than linear: {srgb}");
    assert!(
        hlg > linear && hlg < 1.4,
        "hlg lands near reference white: {hlg}"
    );
    assert!(pq > 3.0, "pq decodes above SDR white: {pq}");
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_rgb_primaries_convert_to_working_space() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].transform(cherenkov::kurbo::Affine::translate((2.0, 2.0)));
    });
    // A pure BT.2020 red sits outside P3, so the conversion must leave the
    // P3 green channel negative — a linear-P3 source would keep it 0.
    let red = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba16Float,
            extent(4, 4),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ),
        32,
        &f16_plane([1.0, 0.0, 0.0, 1.0], 4, 4),
    );
    let imported = cherenkov_gpu::interop::web::import_texture(
        &device,
        &queue,
        offer(&device, &red, &releases),
    )
    .await
    .expect("import");
    let bt2020_linear = FrameColor {
        matrix: YuvMatrix::Bt2020,
        range: YuvRange::Full,
        chroma_siting: ChromaSiting::CENTERED,
        primaries: Primaries::Bt2020,
        transfer: Transfer::Linear,
        reference_white: 203.0,
        hlg_peak: 0.0,
    };
    replace_frame(
        &surface,
        &engine,
        &layer,
        &imported,
        RgbAlpha::Opaque,
        bt2020_linear,
    );
    engine.render(FrameTime::now()).await.expect("render");
    let px = pixels(&surface).await[3 * 16 + 3];
    assert!(
        px[0] > 1.1 && px[1] < -0.05,
        "BT.2020 red converts to out-of-gamut P3: {px:?}"
    );
    replace_frame(
        &surface,
        &engine,
        &layer,
        &imported,
        RgbAlpha::Opaque,
        FrameColor::LINEAR_P3,
    );
    engine.render(FrameTime::now()).await.expect("render");
    let px = pixels(&surface).await[3 * 16 + 3];
    assert!(
        (px[0] - 1.0).abs() < 0.02 && px[1].abs() < 0.02,
        "P3 red passes through unchanged: {px:?}"
    );
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_frame_shared_across_layers_and_retires_once() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    let plane = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba8Unorm,
            extent(4, 4),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ),
        16,
        &fill(&[255, 0, 0, 255], 4, 4),
    );
    let imported = cherenkov_gpu::interop::web::import_texture(
        &device,
        &queue,
        offer(&device, &plane, &releases),
    )
    .await
    .expect("import");
    // Two layer attachments share the one wrapped texture — and its lease.
    let layer_a = show_frame(
        &surface,
        &engine,
        &imported,
        RgbAlpha::Opaque,
        FrameColor::SRGB,
        (1.0, 1.0),
    );
    let layer_b = show_frame(
        &surface,
        &engine,
        &imported,
        RgbAlpha::Opaque,
        FrameColor::SRGB,
        (8.0, 8.0),
    );
    drop(imported);
    engine.render(FrameTime::now()).await.expect("render");
    let pixels = pixels(&surface).await;
    assert!(pixels[2 * 16 + 2][0] > 0.7, "first attachment draws");
    assert!(pixels[9 * 16 + 9][0] > 0.7, "second attachment draws");
    drop(layer_a);
    engine.render(FrameTime::now()).await.expect("render");
    assert_eq!(releases.get(), 0, "second attachment retains the lease");
    drop(layer_b);
    engine.render(FrameTime::now()).await.expect("render");
    assert_eq!(releases.get(), 1, "lease returned after the last use");
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_frame_replacement_mid_flight_and_teardown() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    let plane = |texel: [u8; 4]| {
        foreign_plane(
            &device,
            &queue,
            &plane_desc(
                wgpu::TextureFormat::Rgba8Unorm,
                extent(4, 4),
                1,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            ),
            16,
            &fill(&texel, 4, 4),
        )
    };
    let red = plane([255, 0, 0, 255]);
    let green = plane([0, 255, 0, 255]);
    let imported_red = cherenkov_gpu::interop::web::import_texture(
        &device,
        &queue,
        offer(&device, &red, &releases),
    )
    .await
    .expect("import");
    let imported_green = cherenkov_gpu::interop::web::import_texture(
        &device,
        &queue,
        offer(&device, &green, &releases),
    )
    .await
    .expect("import");
    let layer = show_frame(
        &surface,
        &engine,
        &imported_red,
        RgbAlpha::Opaque,
        FrameColor::SRGB,
        (2.0, 2.0),
    );
    // The first submission may still be in flight on the GPU when the frame
    // is replaced; the retired lease still fires exactly once, once the
    // caller's own clone of the wrapper is dropped too.
    engine.render(FrameTime::now()).await.expect("first frame");
    replace_frame(
        &surface,
        &engine,
        &layer,
        &imported_green,
        RgbAlpha::Opaque,
        FrameColor::SRGB,
    );
    engine.render(FrameTime::now()).await.expect("replacement");
    drop(imported_red);
    assert_eq!(releases.get(), 1, "replaced lease released once");
    let px = pixels(&surface).await[3 * 16 + 3];
    assert!(
        px[1] > 0.7 && px[0] < 0.2,
        "replacement frame draws: {px:?}"
    );

    // Surface teardown retires the live slot.
    drop(imported_green);
    drop(layer);
    drop(surface);
    engine
        .render(FrameTime::now())
        .await
        .expect("teardown pump");
    assert_eq!(releases.get(), 2, "surface teardown released the lease");

    // Engine teardown does the same for a surviving surface.
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    let imported_red = cherenkov_gpu::interop::web::import_texture(
        &device,
        &queue,
        offer(&device, &red, &releases),
    )
    .await
    .expect("import");
    let layer_2 = show_frame(
        &surface,
        &engine,
        &imported_red,
        RgbAlpha::Opaque,
        FrameColor::SRGB,
        (2.0, 2.0),
    );
    drop(imported_red);
    drop(engine);
    assert!(
        surface.readback().await.is_err(),
        "readback observes engine shutdown"
    );
    // The frame op queued on the surface side still holds a clone until the
    // surface — and its queue — is dropped.
    drop(layer_2);
    drop(surface);
    assert_eq!(releases.get(), 3, "engine teardown released the lease");
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_import_rejects_contract_violations() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let import = |plane: &wgpu::Texture, token: &wgpu::Device| {
        cherenkov_gpu::interop::web::import_texture(&device, &queue, offer(token, plane, &releases))
    };
    // Every rejection also releases the lease: the producer's release hook
    // is the single disposal point.
    let no_binding = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba8Unorm,
            extent(4, 4),
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
        ),
        0,
        &[],
    );
    assert_eq!(
        import(&no_binding, &device).await.unwrap_err(),
        InvalidWebTexture::Contract(InvalidFrame::PlaneUsage),
        "missing TEXTURE_BINDING"
    );
    let wrong_format = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::R8Unorm,
            extent(4, 4),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ),
        0,
        &[],
    );
    assert_eq!(
        import(&wrong_format, &device).await.unwrap_err(),
        InvalidWebTexture::Contract(InvalidFrame::PlaneFormat),
        "unsupported format"
    );
    let mipmapped = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba8Unorm,
            extent(4, 4),
            4,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        ),
        0,
        &[],
    );
    assert_eq!(
        import(&mipmapped, &device).await.unwrap_err(),
        InvalidWebTexture::Contract(InvalidFrame::PlaneGeometry),
        "more than one mip level"
    );
    let destroyed = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba8Unorm,
            extent(4, 4),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ),
        0,
        &[],
    );
    destroyed.destroy();
    assert_eq!(
        import(&destroyed, &device).await.unwrap_err(),
        InvalidWebTexture::Unusable,
        "destroyed before import"
    );
    assert_eq!(releases.get(), 4, "every rejection released its lease");
    drop(engine);
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_import_rejects_foreign_and_transient_sources() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let import = |plane: &wgpu::Texture, token: &wgpu::Device| {
        cherenkov_gpu::interop::web::import_texture(&device, &queue, offer(token, plane, &releases))
    };
    // A second adapter/device: wrong token, and a texture that lives on the
    // other device despite an honest-looking token.
    let other_instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let other_adapter = other_instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await
        .expect("second adapter");
    let (other_device, other_queue) = other_adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("second device");
    let foreign_device_plane = foreign_plane(
        &other_device,
        &other_queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba8Unorm,
            extent(4, 4),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ),
        0,
        &[],
    );
    assert_eq!(
        import(&foreign_device_plane, &other_device)
            .await
            .unwrap_err(),
        InvalidWebTexture::DeviceMismatch,
        "other device's token"
    );
    assert_eq!(
        import(&foreign_device_plane, &device).await.unwrap_err(),
        InvalidWebTexture::Unusable,
        "other device's texture behind the engine's token"
    );

    // A GPUExternalTexture supplied where a GPUTexture is required.
    let js_device = device.as_webgpu().expect("WebGPU device");
    let canvas: wasm_bindgen::JsValue =
        js_sys::eval("document.createElement('canvas')").expect("canvas");
    let init = js_sys::Object::new();
    js_sys::Reflect::set(&init, &"timestamp".into(), &0.into()).expect("timestamp");
    let args = js_sys::Array::of2(&canvas, &init);
    let video_frame = js_sys::Reflect::construct(
        &js_sys::Reflect::get(&js_sys::global(), &"VideoFrame".into())
            .expect("VideoFrame")
            .unchecked_into::<js_sys::Function>(),
        &args,
    )
    .expect("VideoFrame");
    let descriptor = js_sys::Object::new();
    js_sys::Reflect::set(&descriptor, &"source".into(), &video_frame).expect("source");
    let external = js_sys::Reflect::get(js_device.as_ref(), &"importExternalTexture".into())
        .expect("importExternalTexture")
        .unchecked_into::<js_sys::Function>()
        .call1(js_device.as_ref(), &descriptor)
        .expect("GPUExternalTexture");
    let not_a_texture = WebTexture {
        texture: external.unchecked_into::<wgpu::webgpu::GpuTexture>(),
        device: js_device.clone(),
        release: {
            let releases = releases.clone();
            WebTextureLease::new(move || releases.set(releases.get() + 1))
        },
    };
    assert_eq!(
        cherenkov_gpu::interop::web::import_texture(&device, &queue, not_a_texture)
            .await
            .unwrap_err(),
        InvalidWebTexture::NotATexture,
        "GPUExternalTexture rejected"
    );
    assert_eq!(releases.get(), 3, "every rejection released its lease");
    drop(engine);
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_import_copies_no_pixels() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    // A quarter-megabyte plane: any import-side copy or conversion target
    // would show up in engine-reported GPU memory.
    let plane = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba8Unorm,
            extent(256, 256),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ),
        1024,
        &fill(&[200, 40, 40, 255], 256, 256),
    );
    let before = engine.memory().await.gpu.0;
    let imported = cherenkov_gpu::interop::web::import_texture(
        &device,
        &queue,
        offer(&device, &plane, &releases),
    )
    .await
    .expect("import");
    let _layer = show_frame(
        &surface,
        &engine,
        &imported,
        RgbAlpha::Opaque,
        FrameColor::SRGB,
        (2.0, 2.0),
    );
    engine.render(FrameTime::now()).await.expect("render");
    let after = engine.memory().await.gpu.0;
    assert!(
        after - before < 8 * 1024,
        "import allocated {} bytes against a 256 KiB plane",
        after - before
    );
}

#[wasm_bindgen_test(async)]
#[expect(
    clippy::future_not_send,
    reason = "the browser engine is single-threaded and its futures run on the page's event loop"
)]
async fn web_frame_idle_and_coalesced_wake() {
    let (device, queue, engine) = shared_engine().await;
    let releases = Rc::new(Cell::new(0));
    let surface = engine
        .surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))
        .await
        .expect("surface");
    let plane = foreign_plane(
        &device,
        &queue,
        &plane_desc(
            wgpu::TextureFormat::Rgba8Unorm,
            extent(4, 4),
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        ),
        16,
        &fill(&[255, 0, 0, 255], 4, 4),
    );
    let imported = cherenkov_gpu::interop::web::import_texture(
        &device,
        &queue,
        offer(&device, &plane, &releases),
    )
    .await
    .expect("import");
    let wakes = Rc::new(Cell::new(0));
    let counter = wakes.clone();
    engine.set_waker(move || counter.set(counter.get() + 1));
    let layer = show_frame(
        &surface,
        &engine,
        &imported,
        RgbAlpha::Opaque,
        FrameColor::SRGB,
        (2.0, 2.0),
    );
    // Both installs land before the next render: the armed-once waker fires
    // a single host wake for the pair.
    replace_frame(
        &surface,
        &engine,
        &layer,
        &imported,
        RgbAlpha::Opaque,
        FrameColor::SRGB,
    );
    assert_eq!(wakes.get(), 1, "queued installs coalesce to one wake");
    assert_eq!(
        engine.render(FrameTime::now()).await.expect("render"),
        Next::Idle
    );
    // An unchanged retained frame neither wakes the host nor lowers again.
    assert_eq!(
        engine.render(FrameTime::now()).await.expect("steady"),
        Next::Idle
    );
    assert_eq!(wakes.get(), 1, "no wake without a new frame");
    assert_eq!(engine.stats().commands_lowered, 0, "nothing re-lowered");
    replace_frame(
        &surface,
        &engine,
        &layer,
        &imported,
        RgbAlpha::Opaque,
        FrameColor::SRGB,
    );
    assert_eq!(wakes.get(), 2, "a new frame wakes once more");
    assert_eq!(
        engine.render(FrameTime::now()).await.expect("render"),
        Next::Idle
    );
}
