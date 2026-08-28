//! Offscreen GPU-surface snapshot example for the particle effect.

use core::{f32::consts::PI, num::NonZeroU32};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use waterui_core::Environment;
use waterui_graphics::{
    GpuRuntime, OffscreenRenderConfig, OffscreenRenderOutput, OffscreenSize, color::Color,
};
use waterui_particle::{ParticleShape, ParticleSystem};

struct SnapshotSpec {
    name: &'static str,
    width: u32,
    height: u32,
    frames: NonZeroU32,
    background: [u8; 3],
    system: ParticleSystem,
}

fn rain_scene() -> ParticleSystem {
    ParticleSystem::new(8_000)
        .emit_from_rect(1.4, 0.08)
        .at(0.5, -0.04)
        .rate(480_000.0)
        .life(0.6, 0.8)
        .speed(2.4, 4.2)
        .angle(PI * 0.49, PI * 0.51)
        .size(0.0008, 0.0015)
        .color(
            Color::srgb_hex("#D5E8FF").with_opacity(0.45),
            Color::srgb_hex("#E8F5FF").with_opacity(0.0),
        )
        .gravity(0.0, 5.0)
        .wind(0.05, 0.0)
        .stretch_with_velocity()
        .softness(0.35)
}

fn flame_scene() -> ParticleSystem {
    ParticleSystem::new(3_000)
        .emit_from_rect(0.05, 0.0)
        .at(0.5, 0.82)
        .rate(180_000.0)
        .life(0.4, 0.8)
        .speed(0.5, 1.2)
        .angle(PI * 1.4, PI * 1.6)
        .size(0.03, 0.06)
        .color(
            Color::srgb_hex("#FFB433").with_opacity(0.6),
            Color::srgb_hex("#FF2A0D").with_opacity(0.0),
        )
        .gravity(0.0, -1.0)
        .additive()
        .softness(0.6)
}

fn fog_scene() -> ParticleSystem {
    ParticleSystem::new(2_000)
        .emit_from_rect(1.5, 0.2)
        .at(0.5, 1.1)
        .rate(40_000.0)
        .life(8.0, 12.0)
        .speed(0.02, 0.08)
        .angle(PI * 1.4, PI * 1.6)
        .size(0.1, 0.25)
        .color(
            Color::srgb_hex("#C8D7CC").with_opacity(0.12),
            Color::srgb_hex("#C8D7CC").with_opacity(0.0),
        )
        .gravity(0.0, -0.01)
        .wind(0.02, 0.0)
        .softness(1.0)
}

fn explosion_scene() -> ParticleSystem {
    ParticleSystem::new(20_000)
        .emit_from_circle(0.05)
        .at(0.5, 0.5)
        .rate(1_200_000.0)
        .life(0.8, 1.5)
        .speed(0.5, 3.0)
        .angle(0.0, PI * 2.0)
        .size(0.003, 0.008)
        .color(
            Color::srgb_hex("#FF7A00").with_opacity(1.0),
            Color::srgb_hex("#333333").with_opacity(1.0),
        )
        .gravity(0.0, 3.0)
        .shape(ParticleShape::Rect)
        .softness(0.0)
}

fn bounce_box_scene() -> ParticleSystem {
    ParticleSystem::new(6_000)
        .emit_from_circle(0.02)
        .at(0.5, 0.18)
        .rate(90_000.0)
        .life(4.0, 6.0)
        .speed(0.5, 1.4)
        .angle(0.0, PI * 2.0)
        .size(0.006, 0.014)
        .color(
            Color::srgb_hex("#85DBFF").with_opacity(0.95),
            Color::srgb_hex("#2EA4FF").with_opacity(0.2),
        )
        .gravity(0.0, 1.4)
        .turbulence(0.2)
        .collide_with_particles(0.01, 16.0)
        .collide_with_rect(0.08, 0.08, 0.84, 0.84)
        .collide_with_circle_obstacle(0.38, 0.34, 0.06)
        .collide_with_circle_obstacle(0.5, 0.36, 0.08)
        .collide_with_circle_obstacle(0.62, 0.34, 0.06)
        .bounce(0.82)
        .surface_friction(0.9)
        .softness(0.25)
}

fn composite_over_opaque_background(
    output: &OffscreenRenderOutput,
    background: [u8; 3],
) -> OffscreenRenderOutput {
    let rgba8 = output
        .rgba8
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
        .collect();

    OffscreenRenderOutput {
        width: output.width,
        height: output.height,
        rgba8,
    }
}

#[expect(
    clippy::future_not_send,
    reason = "offscreen GpuView state is intentionally main-thread local"
)]
async fn write_snapshot(runtime: &GpuRuntime, output_dir: &Path, spec: SnapshotSpec) {
    let size = OffscreenSize::try_from_pixels(spec.width, spec.height)
        .expect("snapshot frame size must be valid");
    let render_config = OffscreenRenderConfig::new(size);
    let mut env = Environment::new();
    let output = spec
        .system
        .render_offscreen_frames(runtime, render_config, &mut env, spec.frames)
        .await
        .expect("particle snapshot render should succeed");

    fs::write(
        output_dir.join(format!("{}.raw.png", spec.name)),
        output
            .to_png()
            .expect("raw particle png encoding should succeed"),
    )
    .expect("raw particle png write should succeed");

    let composited = composite_over_opaque_background(&output, spec.background);
    fs::write(
        output_dir.join(format!("{}.png", spec.name)),
        composited
            .to_png()
            .expect("composited particle png encoding should succeed"),
    )
    .expect("composited particle png write should succeed");
}

#[expect(
    clippy::future_not_send,
    reason = "offscreen GpuView state is intentionally main-thread local"
)]
async fn run() {
    let output_dir = env::args_os().nth(1).map(PathBuf::from).expect(
        "usage: cargo run -p waterui-particle --example gpu_surface_snapshots -- <output-dir>",
    );
    fs::create_dir_all(&output_dir).expect("snapshot output directory must be creatable");
    let runtime = GpuRuntime::new()
        .await
        .expect("particle snapshot export requires a working GPU runtime");

    let snapshots = [
        SnapshotSpec {
            name: "rain",
            width: 540,
            height: 960,
            frames: NonZeroU32::new(8).expect("non-zero literal"),
            background: [0x0F, 0x17, 0x2A],
            system: rain_scene(),
        },
        SnapshotSpec {
            name: "flame_02",
            width: 600,
            height: 600,
            frames: NonZeroU32::new(2).expect("non-zero literal"),
            background: [0x00, 0x00, 0x00],
            system: flame_scene(),
        },
        SnapshotSpec {
            name: "flame_08",
            width: 600,
            height: 600,
            frames: NonZeroU32::new(8).expect("non-zero literal"),
            background: [0x00, 0x00, 0x00],
            system: flame_scene(),
        },
        SnapshotSpec {
            name: "flame_24",
            width: 600,
            height: 600,
            frames: NonZeroU32::new(24).expect("non-zero literal"),
            background: [0x00, 0x00, 0x00],
            system: flame_scene(),
        },
        SnapshotSpec {
            name: "fog_08",
            width: 600,
            height: 600,
            frames: NonZeroU32::new(8).expect("non-zero literal"),
            background: [0x08, 0x10, 0x12],
            system: fog_scene(),
        },
        SnapshotSpec {
            name: "fog_24",
            width: 600,
            height: 600,
            frames: NonZeroU32::new(24).expect("non-zero literal"),
            background: [0x08, 0x10, 0x12],
            system: fog_scene(),
        },
        SnapshotSpec {
            name: "explosion",
            width: 600,
            height: 600,
            frames: NonZeroU32::new(8).expect("non-zero literal"),
            background: [0x00, 0x00, 0x00],
            system: explosion_scene(),
        },
        SnapshotSpec {
            name: "bounce_box",
            width: 600,
            height: 600,
            frames: NonZeroU32::new(8).expect("non-zero literal"),
            background: [0x06, 0x14, 0x1F],
            system: bounce_box_scene(),
        },
    ];

    for spec in snapshots {
        write_snapshot(&runtime, &output_dir, spec).await;
    }
}

fn main() {
    pollster::block_on(run());
}
