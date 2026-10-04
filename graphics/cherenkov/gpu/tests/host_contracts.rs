//! The C1 host contract at one engine revision: a shared device, retained
//! texture output, retained producer content, external video planes, and
//! transparent presentation all through the APIs WaterUI/Hydrolysis use.

use cherenkov::kurbo::Rect;
use cherenkov::{
    __engine_block as block, __engine_fn as split_fn, __engine_test as split_test,
    __engine_wait as wait,
};
use cherenkov::{Engine, FrameTime, Next};
use cherenkov_gpu::{
    Gpu, GpuConfig,
    interop::{
        ExternalFrame, FrameColor, GpuContent, GpuContentBox, OutputAlpha, OutputColor, Presenter,
        SharedDevice, TextureOutput, TextureTarget, wgpu,
    },
};
use filtrate::{
    Effect, EffectContext, EffectFrameTiming, EffectInput, EffectOutput, EffectRenderResult,
    EffectSetupResult,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

split_fn! {
/// An adapter/device pair usable as a `SharedDevice`: passthrough backends
/// need `PASSTHROUGH_SHADERS`, which the engine requires for its precompiled
/// fixed shaders (issue #57).
fn shared_device()
-> Result<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue), Box<dyn std::error::Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        block!(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let required_features = match adapter.get_info().backend {
        wgpu::Backend::Vulkan | wgpu::Backend::Metal => wgpu::Features::PASSTHROUGH_SHADERS,
        _ => wgpu::Features::empty(),
    };
    let (device, queue) = block!(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        ..wgpu::DeviceDescriptor::default()
    }))?;
    Ok((instance, adapter, device, queue))
}
}

struct Producer {
    colors: mpsc::Receiver<wgpu::Color>,
    setups: Arc<AtomicUsize>,
    frames: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl Drop for Producer {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

impl GpuContent for Producer {
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "the wasm32 harness runs on the single-threaded page event loop"
        )
    )]
    async fn setup(&mut self, _: &wgpu::Context<'_>) {
        self.setups.fetch_add(1, Ordering::Relaxed);
    }

    fn render(&mut self, frame: &mut wgpu::Frame<'_>) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        let color = self.colors.try_recv().expect("producer frame available");
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        drop(pass);
        frame.queue.submit([encoder.finish()]);
    }
}

/// A passthrough effect: it proves the group ran without changing pixels.
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
        self.0.send(input.timing).expect("timing receiver alive");
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

fn producer() -> (
    GpuContentBox,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    mpsc::Sender<wgpu::Color>,
) {
    let setups = Arc::new(AtomicUsize::new(0));
    let frames = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let (send, colors) = mpsc::channel();
    let content = GpuContentBox::new(
        Producer {
            colors,
            setups: setups.clone(),
            frames: frames.clone(),
            drops: drops.clone(),
        },
        || {},
    );
    (content, setups, frames, drops, send)
}

fn nv12_planes(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::Texture) {
    let y = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("external luma"),
        size: wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // BT.709 video-range mid grey: Y' = 126, Cb/Cr = 128.
    queue.write_texture(
        y.as_image_copy(),
        &[126u8; 64],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(8),
        },
        wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
    );
    let uv = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("external chroma"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        uv.as_image_copy(),
        &[128u8; 32],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(4),
        },
        wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    (y, uv)
}

split_fn! {
fn presented_pixels(
    engine: &Engine<Gpu>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    backend: wgpu::Backend,
    source: &wgpu::Texture,
) -> Result<Vec<[f32; 4]>, Box<dyn std::error::Error>> {
    let (target, destinations) = TextureTarget::new((24, 24));
    let destination = wait!(engine.surface(target))?;
    let destination_texture = destinations.try_recv()?;
    let delivery = cherenkov_gpu::interop::shader_delivery(backend, device)?;
    let mut presenter = Presenter::new(device, delivery);
    presenter.texture(
        device,
        queue,
        &source.create_view(&wgpu::TextureViewDescriptor::default()),
        TextureOutput {
            texture: &destination_texture,
            color: OutputColor::LinearDisplayP3,
            alpha: OutputAlpha::Premultiplied,
            headroom: 1.0,
        },
    );
    Ok(wait!(destination.readback())?.pixels)
}
}

split_test! {
fn host_contracts_at_one_revision() -> Result<(), Box<dyn std::error::Error>> {
    let (instance, adapter, device, queue) = wait!(shared_device())?;
    let backend = adapter.get_info().backend;
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake = wakes.clone();
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        device: Some(SharedDevice {
            instance,
            adapter,
            device: device.clone(),
            queue: queue.clone(),
        }),
        redraw: Some(cherenkov_gpu::interop::RedrawCallback::new(move || {
            wake.fetch_add(1, Ordering::Relaxed);
        })),
        ..GpuConfig::default()
    }))?;

    // Retained texture output: the host gets a texture now and a notification
    // only when the allocation changes again.
    let (target, textures) = TextureTarget::new((16, 16));
    let surface = wait!(engine.surface(target))?;
    let first_output = textures.try_recv()?;
    assert_eq!((first_output.width(), first_output.height()), (16, 16));

    // Custom GPU content inside clip, opacity and effect groups.
    let (content, setups, frames, drops, send) = producer();
    let redraw = content.redraw_handle();
    send.send(wgpu::Color::RED)?;
    let (effect_frames, effect_timings) = mpsc::channel();
    let effect = engine.effect(cherenkov_gpu::interop::EffectBox::from(CopyEffect(
        effect_frames,
    )));
    let producer_layer = surface.layer();

    // External video planes on the same device, sampled in place.
    let (y, uv) = nv12_planes(&device, &queue);
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::yuv(y, uv, FrameColor::BT709_VIDEO)?);
    let video_layer = surface.layer();

    surface.update(|tx| {
        tx[surface.root()].push(&producer_layer).push(&video_layer);
        tx[&producer_layer]
            .transform(cherenkov::kurbo::Affine::translate((1.0, 1.0)))
            .clip(Rect::new(0.0, 0.0, 4.0, 4.0))
            .opacity(0.5_f32)
            .filter(&effect)
            .content(engine.gpu_producer(content).at((8, 8)));
        tx[&video_layer]
            .transform(cherenkov::kurbo::Affine::translate((8.0, 8.0)))
            .content(video.at((8, 8)));
    });

    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(setups.load(Ordering::Relaxed), 1, "producer setup ran once");
    assert_eq!(frames.load(Ordering::Relaxed), 1);
    assert_eq!(effect_timings.try_iter().count(), 1, "effect group ran");

    // Resize publishes exactly one replacement texture and does not recreate
    // unrelated producers.
    surface.resize((24, 24))?;
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    let resized_output = textures.try_recv()?;
    assert_eq!((resized_output.width(), resized_output.height()), (24, 24));
    assert_eq!(
        setups.load(Ordering::Relaxed),
        1,
        "resize did not recreate unrelated producers"
    );
    assert!(
        textures.try_recv().is_err(),
        "no extra texture notification"
    );

    // Detached producer content must not keep waking the host.
    surface.update(|tx| {
        tx[surface.root()].remove(&producer_layer);
    });
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    let before = wakes.load(Ordering::Relaxed);
    send.send(wgpu::Color::BLUE)?;
    redraw.request_redraw();
    assert_eq!(
        wakes.load(Ordering::Relaxed),
        before,
        "detached content does not wake host"
    );
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(frames.load(Ordering::Relaxed), 1, "detached producer idle");

    // The external frame decoded in place: video grey lands inside its quad.
    drop(producer_layer);
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
    assert_eq!(drops.load(Ordering::Relaxed), 1, "producer teardown ran");

    // Transparent presentation: the retained working texture presents with
    // premultiplied alpha so empty pixels stay transparent for the host.
    let pixels = wait!(presented_pixels(&engine, &device, &queue, backend, &resized_output))?;
    let empty = pixels[0];
    assert!(
        empty[3].abs() < 0.001,
        "empty pixel stays transparent: {empty:?}"
    );
    let video_pixel = pixels[10 * 24 + 10];
    assert!(
        video_pixel[0] > 0.05
            && video_pixel[0].abs() - video_pixel[1].abs() < 0.05
            && video_pixel[1].abs() - video_pixel[2].abs() < 0.05,
        "external video frame decoded in place: {video_pixel:?}"
    );
    Ok(())
}
}
