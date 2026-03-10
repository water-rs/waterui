use core::{f32::consts::PI, num::NonZeroU32};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use waterui_core::Environment;
use waterui_graphics::{OffscreenRenderConfig, OffscreenRenderOutput, OffscreenSize, color::Color};
use waterui_particle::{ParticleShape, ParticleSystem};

const FLAME_SEQUENCE_FRAMES: [u32; 8] = [1, 2, 4, 6, 8, 12, 18, 24];

struct SnapshotSpec {
    name: &'static str,
    width: u32,
    height: u32,
    frame_count: NonZeroU32,
    background: [u8; 3],
    system: ParticleSystem,
}

fn rain_scene() -> ParticleSystem {
    ParticleSystem::new(8_000)
        .emit_from_rect(1.4, 0.08)
        .at(0.5, -0.04)
        .rate(480_000.0)
        .life(0.6..0.8)
        .speed(2.4..4.2)
        .angle(PI * 0.49..PI * 0.51)
        .size(0.0008..0.0015)
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
    ParticleSystem::new(4_000)
        .emit_from_rect(0.035, 0.018)
        .at(0.5, 0.86)
        .rate(110_000.0)
        .life(0.24..0.48)
        .speed(0.8..1.75)
        .angle(PI * 1.44..PI * 1.56)
        .size(0.012..0.028)
        .color(
            Color::srgb_hex("#FFF2A6").with_opacity(0.18),
            Color::srgb_hex("#FF5A1F").with_opacity(0.0),
        )
        .gravity(0.0, -1.6)
        .wind(0.03, 0.0)
        .turbulence(0.28)
        .drag(0.94)
        .stretch_with_velocity()
        .additive()
        .softness(0.45)
}

fn explosion_scene() -> ParticleSystem {
    ParticleSystem::new(20_000)
        .emit_from_circle(0.05)
        .at(0.5, 0.5)
        .rate(1_200_000.0)
        .life(0.8..1.5)
        .speed(0.5..3.0)
        .angle(0.0..PI * 2.0)
        .size(0.003..0.008)
        .color(
            Color::srgb_hex("#FF7A00").with_opacity(1.0),
            Color::srgb_hex("#333333").with_opacity(1.0),
        )
        .gravity(0.0, 3.0)
        .shape(ParticleShape::Rect)
        .softness(0.0)
}

fn composite_over_opaque_background(
    output: &OffscreenRenderOutput,
    background: [u8; 3],
) -> OffscreenRenderOutput {
    let rgba8 = output
        .rgba8
        .chunks_exact(4)
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

fn render_snapshot(
    system: ParticleSystem,
    width: u32,
    height: u32,
    frame_count: NonZeroU32,
) -> OffscreenRenderOutput {
    let size =
        OffscreenSize::try_from_pixels(width, height).expect("snapshot frame size must be valid");
    let render_config = OffscreenRenderConfig::new(size);
    let mut env = Environment::new();
    system
        .render_offscreen_frames(render_config, &mut env, frame_count)
        .expect("particle snapshot render should succeed")
}

fn write_render_output(
    output_dir: &Path,
    name: &str,
    background: [u8; 3],
    output: &OffscreenRenderOutput,
) {
    fs::write(
        output_dir.join(format!("{name}.raw.png")),
        output
            .to_png()
            .expect("raw particle png encoding should succeed"),
    )
    .expect("raw particle png write should succeed");

    let composited = composite_over_opaque_background(output, background);
    fs::write(
        output_dir.join(format!("{name}.png")),
        composited
            .to_png()
            .expect("composited particle png encoding should succeed"),
    )
    .expect("composited particle png write should succeed");
}

fn write_snapshot(output_dir: &Path, spec: SnapshotSpec) {
    let output = render_snapshot(spec.system, spec.width, spec.height, spec.frame_count);
    write_render_output(output_dir, spec.name, spec.background, &output);
}

fn write_flame_sequence(output_dir: &Path) {
    let sequence_dir = output_dir.join("flame_sequence");
    fs::create_dir_all(&sequence_dir).expect("flame sequence directory must be creatable");

    for frame_count in FLAME_SEQUENCE_FRAMES {
        let frame_count = NonZeroU32::new(frame_count).expect("non-zero literal");
        let output = render_snapshot(flame_scene(), 600, 600, frame_count);
        write_render_output(
            &sequence_dir,
            &format!("flame-{:02}", frame_count.get()),
            [0x00, 0x00, 0x00],
            &output,
        );
    }
}

fn main() {
    let output_dir = env::args_os().nth(1).map(PathBuf::from).expect(
        "usage: cargo run -p waterui-particle --example gpu_surface_snapshots -- <output-dir>",
    );
    fs::create_dir_all(&output_dir).expect("snapshot output directory must be creatable");

    let snapshots = [
        SnapshotSpec {
            name: "rain",
            width: 540,
            height: 960,
            frame_count: NonZeroU32::new(8).expect("non-zero literal"),
            background: [0x0F, 0x17, 0x2A],
            system: rain_scene(),
        },
        SnapshotSpec {
            name: "flame",
            width: 600,
            height: 600,
            frame_count: NonZeroU32::new(18).expect("non-zero literal"),
            background: [0x00, 0x00, 0x00],
            system: flame_scene(),
        },
        SnapshotSpec {
            name: "explosion",
            width: 600,
            height: 600,
            frame_count: NonZeroU32::new(8).expect("non-zero literal"),
            background: [0x00, 0x00, 0x00],
            system: explosion_scene(),
        },
    ];

    for spec in snapshots {
        write_snapshot(&output_dir, spec);
    }

    write_flame_sequence(&output_dir);
}
