//! Multi-frame checks of the preserved-attachment presentation contract.

use cherenkov::{Draw, Engine, FrameTime, Readback, WorkingColor, damage::Region, kurbo::Rect};

use super::{OutputAlpha, OutputColor, Presentation, Presenter, RetainedRegion, TextureOutput};
use crate::{
    Gpu, GpuConfig,
    interop::{SharedDevice, TextureTarget, shader_delivery},
};

fn check(actual: &Readback, before: &Readback, complete: &Readback, repair: &Region) {
    for y in 0..actual.height {
        for x in 0..actual.width {
            let offset = (y * actual.width + x) as usize;
            let updated = repair.rectangles().iter().any(|rect| {
                (rect.x0()..rect.x1()).contains(&x) && (rect.y0()..rect.y1()).contains(&y)
            });
            let expected = if updated { complete } else { before }.pixels[offset];
            for (got, want) in actual.pixels[offset].iter().zip(expected) {
                assert!(
                    (got - want).abs() < 0.001,
                    "pixel ({x}, {y}): {got} != {want}"
                );
            }
        }
    }
}

fn save(name: &str, readback: &Readback) -> std::io::Result<()> {
    let directory = std::env::temp_dir().join("cherenkov-retained-present");
    std::fs::create_dir_all(&directory)?;
    cherenkov_oracle::F32Image {
        width: readback.width,
        height: readback.height,
        pixels: readback.pixels.clone(),
    }
    .write_png(&directory.join(name))
}

#[test]
fn retained_present_preserves_pixels_and_full_texture_coordinates()
-> Result<(), Box<dyn std::error::Error>> {
    let shared = SharedDevice::create(&GpuConfig::default())?;
    let device = shared.device.clone();
    let queue = shared.queue.clone();
    let delivery = shader_delivery(shared.adapter.get_info().backend, &device)?;
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    })?;
    let size = (64, 48);
    let (target, sources) = TextureTarget::new(size);
    let source = engine.surface(target, || {})?;
    source.update(|tx| {
        tx[source.root()].content(source.record(|r| {
            for (bounds, color) in [
                (Rect::new(0.0, 0.0, 32.0, 24.0), [1.0, 0.0, 0.0, 1.0]),
                (Rect::new(32.0, 0.0, 64.0, 24.0), [0.0, 1.0, 0.0, 1.0]),
                (Rect::new(0.0, 24.0, 32.0, 48.0), [0.0, 0.0, 1.0, 1.0]),
                (Rect::new(32.0, 24.0, 64.0, 48.0), [4.0, 0.5, 0.0, 1.0]),
            ] {
                r.fill(bounds, WorkingColor::new(color));
            }
        }));
    });
    let (target, destinations) = TextureTarget::new(size);
    let destination = engine.surface(target, || {})?;
    destination.clear_color(WorkingColor::new([0.125, 0.125, 0.125, 1.0]));
    let (target, references) = TextureTarget::new(size);
    let reference = engine.surface(target, || {})?;
    engine.render(FrameTime::now())?;
    let source_texture = sources.try_recv()?;
    let output = destinations.try_recv()?;
    let reference_texture = references.try_recv()?;
    let view = source_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut presenter = Presenter::new(&device, delivery);
    let texture = TextureOutput {
        texture: &output,
        color: OutputColor::LinearDisplayP3,
        alpha: OutputAlpha::Premultiplied,
        headroom: 4.0,
    };
    presenter.texture(
        &device,
        &queue,
        &view,
        TextureOutput {
            texture: &reference_texture,
            ..texture
        },
    );
    let complete = reference.readback()?;
    let before = destination.readback()?;
    let uniform = Presenter::uniform(&device);
    queue.write_buffer(&uniform, 0, &Presenter::uniform_bytes(&texture));
    let mut repair = Region::new(size);
    for step in 0..3 {
        match step {
            0 => {}
            1 => {
                repair.add(8, 6, 20, 16);
                repair.add(36, 28, 20, 16);
            }
            2 => repair.mark_full(),
            _ => unreachable!(),
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        presenter.encode(
            &device,
            &mut encoder,
            &view,
            &Presentation {
                texture,
                coverage: RetainedRegion(&repair),
            },
            &uniform,
            None,
        );
        queue.submit([encoder.finish()]);
        let actual = destination.readback()?;
        check(&actual, &before, &complete, &repair);
        if step == 1 {
            save("partial.png", &actual)?;
        }
    }
    save("before.png", &before)?;
    save("complete.png", &complete)?;
    Ok(())
}
