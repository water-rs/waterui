//! Stress Example - High pressure real-app workload for profiling
//!
//! This app is intended for professional profiling through native tools and `WaterUI` preview perf.
//! It drives three independent pressure paths:
//! - System animation path (metadata -> native animation acceleration)
//! - Custom animation path (framework/self-rendered GPU animation)
//! - Filter path (framework interpolation + effect rendering)
//!
//! Tune load via env vars:
//! - `WATERUI_STRESS_SYSTEM_COUNT` (default: 120)
//! - `WATERUI_STRESS_CUSTOM_COUNT` (default: 96)
//! - `WATERUI_STRESS_FILTER_COUNT` (default: 144)
//! - `WATERUI_STRESS_TOGGLE_MS` (default: 680)
//! - `WATERUI_STRESS_FILTER_TOGGLE_MS` (default: 240)

use core::time::Duration;

use waterui::animation::Animation;
use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::Binding;
use waterui::shape::{Circle, RoundedRectangle, ShapeExt};
use waterui::task::{sleep, spawn_local};

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn stress_card_background(index: usize) -> Color {
    match index % 6 {
        0 => Color::srgb_hex("#2563EB"),
        1 => Color::srgb_hex("#7C3AED"),
        2 => Color::srgb_hex("#0EA5E9"),
        3 => Color::srgb_hex("#059669"),
        4 => Color::srgb_hex("#D97706"),
        _ => Color::srgb_hex("#DC2626"),
    }
}

fn system_tile(toggle: &Binding<bool>, index: usize) -> impl View {
    let fast = Duration::from_millis(520);
    let easing = Animation::bezier(fast, 0.22, 1.0, 0.36, 1.0);

    let amp = ((index % 5) as f32).mul_add(2.0, 8.0);
    let hi_scale = ((index % 4) as f32).mul_add(0.09, 0.9);
    let lo_scale = ((index % 3) as f32).mul_add(0.08, 0.45);
    let hi_rot = (index % 8) as f32 * 22.5;

    let scale = toggle.select(hi_scale, lo_scale).with(easing.clone());
    let rotation = toggle.select(hi_rot, -hi_rot).with(easing);
    let offset_x = toggle
        .select(amp, -amp)
        .with(Animation::spring(260.0, 20.0));

    RoundedRectangle::new(0.22)
        .fill(stress_card_background(index))
        .size(24.0, 24.0)
        .scale(scale.clone(), scale)
        .rotation(rotation)
        .offset(offset_x, 0.0)
}

fn custom_tile(index: usize) -> impl View {
    let duration = 520 + (index % 7) as u64 * 110;
    Circle
        .morph_to(RoundedRectangle::new(0.26), stress_card_background(index))
        .duration(Duration::from_millis(duration))
        .size(42.0, 42.0)
}

fn filter_tile(
    blur_target: &Binding<f64>,
    saturation_target: &Binding<f64>,
    hue_target: &Binding<f64>,
    index: usize,
) -> impl View {
    let idx = index as f64;

    let blur = blur_target
        .clone()
        .map(move |v| (idx * 0.17).sin().mul_add(1.6, v).clamp(0.0, 12.0) as f32)
        .with(Animation::spring(220.0, 14.0));

    let saturation = saturation_target
        .clone()
        .map(move |v| (idx * 0.11).cos().mul_add(0.35, v).clamp(0.0, 2.0) as f32)
        .with(Animation::ease_in_out(Duration::from_millis(380)));

    let hue = hue_target
        .clone()
        .map(move |v| idx.mul_add(11.0, v).rem_euclid(360.0) as f32)
        .with(Animation::ease_in_out(Duration::from_millis(420)));

    zstack((
        stress_card_background(index),
        text("FX").caption().foreground(Color::srgb(255, 255, 255)),
    ))
    .size(62.0, 44.0)
    .blur(blur)
    .saturation(saturation)
    .hue_rotation(hue)
}

fn system_section(count: usize, toggle: &Binding<bool>) -> impl View {
    let columns = 18usize;
    let rows = count.div_ceil(columns);

    let grid: VStack<_> = (0..rows)
        .map(|row| {
            let line: HStack<_> = (0..columns)
                .map(|col| {
                    let i = row * columns + col;
                    system_tile(toggle, i)
                })
                .collect();
            line.spacing(4.0)
        })
        .collect();

    vstack((
        text("System Animation Pressure").headline(),
        text("Metadata animations offloaded to native animation engine").body(),
        grid.spacing(4.0),
    ))
    .padding()
}

fn custom_section(count: usize) -> impl View {
    let columns = 14usize;
    let rows = count.div_ceil(columns);

    let grid: VStack<_> = (0..rows)
        .map(|row| {
            let line: HStack<_> = (0..columns)
                .map(|col| {
                    let i = row * columns + col;
                    custom_tile(i)
                })
                .collect();
            line.spacing(6.0)
        })
        .collect();

    vstack((
        text("Custom Animation Pressure").headline(),
        text("Renderer-side geometry morph animation (framework-driven)").body(),
        grid.spacing(6.0),
    ))
    .padding()
}

fn filter_section(
    count: usize,
    blur_target: &Binding<f64>,
    saturation_target: &Binding<f64>,
    hue_target: &Binding<f64>,
) -> impl View {
    let columns = 12usize;
    let rows = count.div_ceil(columns);

    let grid: VStack<_> = (0..rows)
        .map(|row| {
            let line: HStack<_> = (0..columns)
                .map(|col| {
                    let i = row * columns + col;
                    filter_tile(blur_target, saturation_target, hue_target, i)
                })
                .collect();
            line.spacing(6.0)
        })
        .collect();

    vstack((
        text("Filter Pressure").headline(),
        text("Filter interpolation + effect rendering with retargeting").body(),
        grid.spacing(6.0),
    ))
    .padding()
}

#[preview]
fn system_pressure_preview() -> impl View {
    let toggle = Binding::bool(false);
    scroll(system_section(120, &toggle))
}

#[preview]
fn custom_pressure_preview() -> impl View {
    scroll(custom_section(96))
}

#[preview]
fn filter_pressure_preview() -> impl View {
    let blur_target = Binding::f64(1.0);
    let saturation_target = Binding::f64(1.0);
    let hue_target = Binding::f64(0.0);
    scroll(filter_section(
        144,
        &blur_target,
        &saturation_target,
        &hue_target,
    ))
}

/// High-pressure profiling workload. Counts are tunable via the
/// `WATERUI_STRESS_*` environment variables.
#[preview]
pub fn demo() -> impl View {
    let system_count = env_usize("WATERUI_STRESS_SYSTEM_COUNT", 120).clamp(18, 1_440);
    let custom_count = env_usize("WATERUI_STRESS_CUSTOM_COUNT", 96).clamp(14, 1_120);
    let filter_count = env_usize("WATERUI_STRESS_FILTER_COUNT", 144).clamp(12, 1_200);

    let toggle_ms = env_u64("WATERUI_STRESS_TOGGLE_MS", 680).max(120);
    let filter_toggle_ms = env_u64("WATERUI_STRESS_FILTER_TOGGLE_MS", 240).max(80);

    let system_toggle = Binding::bool(false);

    let blur_target = Binding::f64(1.0);
    let saturation_target = Binding::f64(1.0);
    let hue_target = Binding::f64(0.0);

    {
        let system_toggle = system_toggle.clone();
        spawn_local(async move {
            loop {
                system_toggle.set(!system_toggle.get());
                sleep(Duration::from_millis(toggle_ms)).await;
            }
        })
        .detach();
    }

    {
        let blur_target = blur_target.clone();
        let saturation_target = saturation_target.clone();
        let hue_target = hue_target.clone();

        spawn_local(async move {
            let mut phase = false;
            loop {
                phase = !phase;
                if phase {
                    blur_target.set(10.5);
                    saturation_target.set(1.85);
                    hue_target.set(260.0);
                } else {
                    blur_target.set(0.8);
                    saturation_target.set(0.45);
                    hue_target.set(30.0);
                }
                sleep(Duration::from_millis(filter_toggle_ms)).await;
            }
        })
        .detach();
    }

    scroll(
        vstack((
            text("WaterUI Stress App").title(),
            text(format!(
                "SYSTEM={system_count}, CUSTOM={custom_count}, FILTER={filter_count}, TOGGLE={toggle_ms}ms/{filter_toggle_ms}ms"
            ))
            .caption(),
            Divider,
            system_section(system_count, &system_toggle),
            Divider,
            custom_section(custom_count),
            Divider,
            filter_section(filter_count, &blur_target, &saturation_target, &hue_target),
        ))
        .padding(),
    )
}

/// Standalone entry point for the stress example.
pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
