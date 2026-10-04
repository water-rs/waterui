//! Filter composition, capture sizes and engine-provided effect timing.

use cherenkov::Instant;
use cherenkov::kurbo::Rect;
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    BlendMode, Draw, Engine, FrameTime, Group, Next, Offscreen, OffscreenFormat, WorkingColor,
};
use cherenkov_gpu::{Gpu, GpuConfig, interop::EffectBox};
use filtrate::{
    Effect, EffectContext, EffectFrameTiming, EffectInput, EffectOutput, EffectRenderResult,
    EffectSetupResult,
};
use std::sync::mpsc;
use std::time::Duration;

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
        assert_eq!(input.texture.width(), input.width);
        assert_eq!(input.texture.height(), input.height);
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
        Ok(true)
    }
}

split_test! {
fn filtered_group_isolates_blended_descendant_inside_pass_through_group()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let identity = engine.filter(filtrate::filters::ColorMatrix([
        1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]));
    let surface = wait!(engine.surface(Offscreen::new((8, 40), OffscreenFormat::LinearF32)))?;
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
    let surface = wait!(engine.surface(Offscreen::new((8, 40), OffscreenFormat::LinearF32)))?;
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
    let surface = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    let layer = surface.layer();
    let (send, receive) = mpsc::channel();
    let effect = engine.effect(EffectBox::from(CopyEffect(send)));
    let invert = engine.filter(filtrate::filters::Invert);
    surface.update(|tx| {
        tx[surface.root()].push(&layer).filter(&effect);
        tx[&layer].filter(&invert).content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                WorkingColor::new([0.25, 0.5, 0.75, 1.0]),
            );
        }));
    });
    let start = Instant::now();
    assert!(matches!(
        wait!(engine.render(FrameTime::at(start)))?,
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
    wait!(engine.render(FrameTime::at(start + Duration::from_millis(250))))?;
    let timing = receive.try_recv()?;
    assert_eq!(timing.presentation_time(), Duration::from_millis(250));
    assert_eq!(timing.delta(), Duration::from_millis(250));
    assert_eq!(timing.sequence(), 1);
    surface.update(|tx| {
        tx[surface.root()].clear_filter();
    });
    assert_eq!(
        wait!(engine.render(FrameTime::at(start + Duration::from_millis(500))))?,
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
        self.timings.send(input.timing).expect("timing receiver");
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

split_test! {
fn filter_wakes_coalesce_and_stop_after_detach() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake = Arc::clone(&wakes);
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        redraw: Some(cherenkov_gpu::interop::RedrawCallback::new(move || {
            wake.fetch_add(1, Ordering::Relaxed);
        })),
        ..GpuConfig::default()
    }))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    let (callbacks, receive) = mpsc::channel();
    let (timings, frames) = mpsc::channel();
    let effect = engine.effect(EffectBox::from(ObservedEffect {
        callbacks,
        timings,
        callback: None,
        fail_setup: false,
    }));
    surface.update(|tx| {
        tx[surface.root()].filter(&effect).content(
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
        tx[surface.root()].filter(&effect);
    });
    wait!(engine.render(FrameTime::now()))?;
    drop(surface);
    assert_eq!(wait!(engine.render(FrameTime::now()))?, Next::Idle);
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
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    let (callbacks, _receive) = mpsc::channel();
    let (timings, _frames) = mpsc::channel();
    let effect = engine.effect(EffectBox::from(ObservedEffect {
        callbacks,
        timings,
        callback: None,
        fail_setup: true,
    }));
    surface.update(|tx| {
        tx[surface.root()].filter(&effect).content(
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
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
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
            tx[layer].filter(&effect).content(
                surface.record(|r| r.fill(Rect::new(0.0, 0.0, 8.0, 8.0), WorkingColor::WHITE)),
            );
        }
    });
    let start = Instant::now();
    wait!(engine.render(FrameTime::at(start)))?;
    assert_eq!(frames.try_iter().count(), 2);
    wait!(engine.render(FrameTime::at(start + Duration::from_millis(250))))?;
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
fn recorded_filter_groups_survive_opacity_speculation_and_live_patches()
-> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
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
