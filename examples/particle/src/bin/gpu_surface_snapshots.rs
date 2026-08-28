use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use particle_example::{explosion_system, flame_system, rain_system};
use waterui::prelude::Environment;
use waterui_graphics::{GpuRuntime, OffscreenRenderConfig, OffscreenRenderOutput, OffscreenSize};
use waterui_particle::ParticleSystem;

struct SnapshotSpec {
    name: &'static str,
    width: u32,
    height: u32,
    background: [u8; 3],
    system: fn() -> ParticleSystem,
}

fn composite_over_opaque_background(pixels: &[u8], background: [u8; 3]) -> Vec<u8> {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| {
            let alpha = u16::from(pixel[3]);
            let inv_alpha = 255_u16 - alpha;
            let red = u16::from(pixel[0]) + (u16::from(background[0]) * inv_alpha + 127) / 255;
            let green = u16::from(pixel[1]) + (u16::from(background[1]) * inv_alpha + 127) / 255;
            let blue = u16::from(pixel[2]) + (u16::from(background[2]) * inv_alpha + 127) / 255;
            [
                red.min(255) as u8,
                green.min(255) as u8,
                blue.min(255) as u8,
                255,
            ]
        })
        .collect()
}

async fn write_snapshot(runtime: &GpuRuntime, output_dir: &Path, spec: SnapshotSpec) {
    let size = OffscreenSize::try_from_pixels(spec.width, spec.height)
        .expect("snapshot frame size must be valid");
    let render_config = OffscreenRenderConfig::new(size);
    let mut env = Environment::new();
    let output = (spec.system)()
        .render_offscreen(runtime, render_config, &mut env)
        .await
        .expect("particle snapshot render should succeed");

    let raw_png = output
        .to_png()
        .expect("raw particle png encoding should succeed");
    fs::write(output_dir.join(format!("{}.raw.png", spec.name)), raw_png)
        .expect("raw particle png write should succeed");

    let composited = OffscreenRenderOutput {
        width: output.width,
        height: output.height,
        rgba8: composite_over_opaque_background(&output.rgba8, spec.background),
    };
    let composited_png = composited
        .into_png()
        .expect("composited particle png encoding should succeed");
    fs::write(
        output_dir.join(format!("{}.png", spec.name)),
        composited_png,
    )
    .expect("composited particle png write should succeed");
}

async fn run() {
    let output_dir = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: cargo run -p particle-example --bin gpu_surface_snapshots -- <output-dir>");
    fs::create_dir_all(&output_dir).expect("snapshot output directory must be creatable");
    let runtime = GpuRuntime::new()
        .await
        .expect("particle snapshot export requires a working GPU runtime");

    let snapshots = [
        SnapshotSpec {
            name: "rain",
            width: 540,
            height: 960,
            background: [0x0F, 0x17, 0x2A],
            system: rain_system,
        },
        SnapshotSpec {
            name: "flame",
            width: 600,
            height: 600,
            background: [0x00, 0x00, 0x00],
            system: flame_system,
        },
        SnapshotSpec {
            name: "explosion",
            width: 600,
            height: 600,
            background: [0x00, 0x00, 0x00],
            system: explosion_system,
        },
    ];

    for spec in snapshots {
        write_snapshot(&runtime, &output_dir, spec).await;
    }
}

fn main() {
    pollster::block_on(run());
}
