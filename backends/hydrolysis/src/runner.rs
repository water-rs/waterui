use std::cell::Cell;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;
#[cfg(feature = "winit")]
use std::{process::Command, str};

#[cfg(feature = "accessibility")]
use accesskit::{
    ActionRequest as AccessibilityActionRequest, TreeUpdate as AccessibilityTreeUpdate,
};
use executor_core::{
    LocalExecutor,
    async_task::{AsyncTask, Runnable},
    try_init_local_executor,
};
use nami::Signal as _;
use waterui::app::App;
use waterui::component::table::TableConfig;
use waterui::graphics::Color;
use waterui::window::WindowManager;
use waterui::window::{Window, WindowBackground};
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::Native;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::view::Hook;

use crate::env::{parse_bool_env, parse_positive_u64_env};
use crate::platform::OffscreenWindow;
use crate::platform::{InputEvent, KeyState, PlatformWindow};
use crate::renderer::{HydrolysisRenderer, HydrolysisTextContextMenuMode};
use crate::time::Instant;

fn init_main_thread_executors() {
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = waterui::inspector::maybe_init_from_env();
}

#[cfg(not(target_arch = "wasm32"))]
fn load_native_resource_fonts(renderer: &mut HydrolysisRenderer) {
    use parley::fontique::Blob;
    use std::sync::Arc;

    let mut roots = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        roots.push(current_dir.join("resources").join("fonts"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        roots.push(exe_dir.join("resources").join("fonts"));
    }

    let state = renderer.state_mut();
    for root in roots {
        if !root.exists() {
            continue;
        }
        let entries = std::fs::read_dir(&root).unwrap_or_else(|error| {
            panic!(
                "hydrolysis native font loader failed to read `{}`: {error}",
                root.display()
            )
        });
        for entry in entries {
            let entry = entry.unwrap_or_else(|error| {
                panic!(
                    "hydrolysis native font loader failed to read an entry in `{}`: {error}",
                    root.display()
                )
            });
            let path = entry.path();
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };
            if !extension.eq_ignore_ascii_case("ttf") && !extension.eq_ignore_ascii_case("otf") {
                continue;
            }
            let font_data = std::fs::read(&path).unwrap_or_else(|error| {
                panic!(
                    "hydrolysis native font loader failed to read `{}`: {error}",
                    path.display()
                )
            });
            let families = state
                .font_cx
                .collection
                .register_fonts(Blob::new(Arc::new(font_data)), None);
            tracing::debug!(
                target: "waterui::hydrolysis::fonts",
                path = %path.display(),
                families = families.len(),
                "registered native Hydrolysis font"
            );
        }
    }
}

fn install_native_component_hooks(env: &mut Environment) {
    env.insert(Hook::new(|_env: &Environment, config: TableConfig| {
        Native::new(config)
    }));
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "winit")))]
fn install_window_manager(env: &mut Environment, pending_windows: Rc<RefCell<Vec<Window>>>) {
    env.insert(WindowManager::new(move |window| {
        pending_windows.borrow_mut().push(window);
    }));
}

const DEFAULT_RENDER_DIAG_INTERVAL_MS: u64 = 1_000;
const DEFAULT_RENDER_DIAG_SLOW_FRAME_MS: u64 = 16;

#[derive(Clone, Copy)]
struct RenderDiagnosticsConfig {
    enabled: bool,
    interval: Duration,
    slow_frame_threshold: Duration,
}

impl RenderDiagnosticsConfig {
    fn from_env() -> Self {
        let enabled = parse_bool_env("hydrolysis runner", "WATERUI_HYDROLYSIS_RENDER_DIAG", false);
        let interval_ms = parse_positive_u64_env(
            "hydrolysis runner",
            "WATERUI_HYDROLYSIS_RENDER_DIAG_INTERVAL_MS",
            DEFAULT_RENDER_DIAG_INTERVAL_MS,
        );
        let slow_frame_ms = parse_positive_u64_env(
            "hydrolysis runner",
            "WATERUI_HYDROLYSIS_RENDER_DIAG_SLOW_FRAME_MS",
            DEFAULT_RENDER_DIAG_SLOW_FRAME_MS,
        );

        Self {
            enabled,
            interval: Duration::from_millis(interval_ms),
            slow_frame_threshold: Duration::from_millis(slow_frame_ms),
        }
    }
}

struct RenderPhaseSample {
    rebuild: Duration,
    acquire: Duration,
    render: Duration,
    present: Duration,
    total: Duration,
    rebuild_iterations: u32,
    rebuilt: bool,
}

#[derive(Default)]
struct RenderPhaseTotals {
    frames: u64,
    rebuild_frames: u64,
    rebuild_iterations: u64,
    slow_frames: u64,
    rebuild: Duration,
    acquire: Duration,
    render: Duration,
    present: Duration,
    total: Duration,
}

struct RenderDiagnostics {
    config: RenderDiagnosticsConfig,
    report_started_at: Instant,
    totals: RenderPhaseTotals,
}

impl RenderDiagnostics {
    fn new(config: RenderDiagnosticsConfig) -> Self {
        Self {
            config,
            report_started_at: Instant::now(),
            totals: RenderPhaseTotals::default(),
        }
    }

    fn enabled(&self) -> bool {
        self.config.enabled
    }

    fn record_frame(&mut self, window_title: &str, sample: RenderPhaseSample) {
        if !self.config.enabled {
            return;
        }

        self.totals.frames = self
            .totals
            .frames
            .checked_add(1)
            .expect("hydrolysis runner: render diagnostics frame counter overflow");
        if sample.rebuilt {
            self.totals.rebuild_frames = self
                .totals
                .rebuild_frames
                .checked_add(1)
                .expect("hydrolysis runner: render diagnostics rebuild frame counter overflow");
        }
        self.totals.rebuild_iterations = self
            .totals
            .rebuild_iterations
            .checked_add(u64::from(sample.rebuild_iterations))
            .expect("hydrolysis runner: render diagnostics rebuild iteration counter overflow");
        self.totals.rebuild += sample.rebuild;
        self.totals.acquire += sample.acquire;
        self.totals.render += sample.render;
        self.totals.present += sample.present;
        self.totals.total += sample.total;

        if sample.total >= self.config.slow_frame_threshold {
            self.totals.slow_frames = self
                .totals
                .slow_frames
                .checked_add(1)
                .expect("hydrolysis runner: render diagnostics slow frame counter overflow");
            tracing::warn!(
                target: "waterui::hydrolysis::render",
                window_title = %window_title,
                total_ms = duration_ms(sample.total),
                rebuild_ms = duration_ms(sample.rebuild),
                acquire_ms = duration_ms(sample.acquire),
                render_ms = duration_ms(sample.render),
                present_ms = duration_ms(sample.present),
                rebuild_iterations = sample.rebuild_iterations,
                "Hydrolysis slow frame detected"
            );
        }

        self.maybe_report(window_title);
    }

    fn maybe_report(&mut self, window_title: &str) {
        if self.totals.frames == 0 {
            return;
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.report_started_at);
        if elapsed < self.config.interval {
            return;
        }

        let frame_count = self.totals.frames as f64;
        let avg_total_ms = duration_ms(self.totals.total) / frame_count;
        let avg_rebuild_ms = duration_ms(self.totals.rebuild) / frame_count;
        let avg_acquire_ms = duration_ms(self.totals.acquire) / frame_count;
        let avg_render_ms = duration_ms(self.totals.render) / frame_count;
        let avg_present_ms = duration_ms(self.totals.present) / frame_count;
        let rebuild_ratio = self.totals.rebuild_frames as f64 / frame_count;
        let avg_rebuild_iterations = self.totals.rebuild_iterations as f64 / frame_count;
        let fps = self.totals.frames as f64 / elapsed.as_secs_f64();

        tracing::info!(
            target: "waterui::hydrolysis::render",
            window_title = %window_title,
            frames = self.totals.frames,
            interval_ms = duration_ms(elapsed),
            fps,
            rebuild_frames = self.totals.rebuild_frames,
            rebuild_ratio,
            avg_rebuild_iterations,
            avg_total_ms,
            avg_rebuild_ms,
            avg_acquire_ms,
            avg_render_ms,
            avg_present_ms,
            slow_frames = self.totals.slow_frames,
            slow_frame_threshold_ms = duration_ms(self.config.slow_frame_threshold),
            "Hydrolysis render diagnostics"
        );

        self.report_started_at = now;
        self.totals = RenderPhaseTotals::default();
    }
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn elapsed_or_zero(started_at: Option<Instant>) -> Duration {
    started_at.map_or(Duration::ZERO, |value| value.elapsed())
}

fn readback_texture_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    const BYTES_PER_PIXEL: u32 = 4;
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let unpadded_bytes_per_row = width * BYTES_PER_PIXEL;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hydrolysis-headless-readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hydrolysis-headless-readback-encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender
            .send(result)
            .expect("hydrolysis headless readback channel receiver dropped");
    });

    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    receiver
        .recv()
        .expect("hydrolysis headless readback callback dropped")
        .expect("hydrolysis headless failed to map readback buffer");

    let mapped = slice.get_mapped_range();
    let mut pixels = vec![0_u8; (width * height * BYTES_PER_PIXEL) as usize];
    for row in 0..height as usize {
        let source_start = row * padded_bytes_per_row as usize;
        let source_end = source_start + unpadded_bytes_per_row as usize;
        let destination_start = row * unpadded_bytes_per_row as usize;
        let destination_end = destination_start + unpadded_bytes_per_row as usize;
        pixels[destination_start..destination_end]
            .copy_from_slice(&mapped[source_start..source_end]);
    }
    drop(mapped);
    readback.unmap();
    pixels
}

#[cfg(feature = "winit")]
fn probe_accessibility_runtime() -> bool {
    let output = Command::new("busctl")
        .args([
            "--user",
            "get-property",
            "org.a11y.Bus",
            "/org/a11y/bus",
            "org.a11y.Status",
            "ScreenReaderEnabled",
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            let stderr = str::from_utf8(&output.stderr)
                .map(str::trim)
                .unwrap_or("<non-utf8 stderr>");
            tracing::warn!(
                target: "waterui::hydrolysis::a11y",
                status = %output.status,
                stderr,
                "disabling accesskit adapter: org.a11y.Bus probe failed"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                target: "waterui::hydrolysis::a11y",
                error = %error,
                "disabling accesskit adapter: failed to execute busctl probe"
            );
            false
        }
    }
}

struct RuntimeWindow<P: PlatformWindow> {
    window: Window,
    platform: P,
    renderer: HydrolysisRenderer,
    needs_rebuild: bool,
    pointer_position: Option<(f32, f32)>,
    render_diagnostics: RenderDiagnostics,
}

impl<P: PlatformWindow> RuntimeWindow<P> {
    fn new(
        window: Window,
        platform: P,
        renderer: HydrolysisRenderer,
        render_diagnostics_config: RenderDiagnosticsConfig,
    ) -> Self {
        Self {
            window,
            platform,
            renderer,
            needs_rebuild: true,
            pointer_position: None,
            render_diagnostics: RenderDiagnostics::new(render_diagnostics_config),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadlessSnapshot {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[derive(Debug)]
struct RenderWindowResult {
    rebuilt: bool,
    snapshot: Option<HeadlessSnapshot>,
    profile: FrameProfile,
}

/// Phase timing for one Hydrolysis frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FramePhases {
    /// Time spent draining local executor work before input dispatch.
    pub executor_before: Duration,
    /// Time spent dispatching pending input.
    pub input: Duration,
    /// Time spent advancing animations and invalidation clocks.
    pub animation: Duration,
    /// Time spent rebuilding scene/layout state.
    pub rebuild: Duration,
    /// Time spent acquiring the target frame.
    pub acquire: Duration,
    /// Time spent submitting rendering work.
    pub render: Duration,
    /// Time spent presenting the frame.
    pub present: Duration,
    /// Time spent draining local executor work after rendering.
    pub executor_after: Duration,
}

/// Counter snapshot for one Hydrolysis frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameCounters {
    /// Number of rebuild loop iterations in this frame.
    pub rebuild_iterations: u32,
    /// Measurement cache hits in this frame.
    pub measurement_cache_hits: u32,
    /// Measurement cache misses in this frame.
    pub measurement_cache_misses: u32,
    /// Whether this frame rendered to the target.
    pub rendered: bool,
    /// Whether this frame captured a CPU snapshot.
    pub captured_snapshot: bool,
}

/// Detailed profile for one Hydrolysis frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameProfile {
    /// Total wall-clock duration for the frame pump.
    pub total: Duration,
    /// Phase timing breakdown.
    pub phases: FramePhases,
    /// Counter snapshot.
    pub counters: FrameCounters,
}

impl FrameProfile {
    fn with_total(mut self, total: Duration) -> Self {
        self.total = total;
        self
    }
}

fn schedule_redraw_or_rebuild<P: PlatformWindow>(runtime: &mut RuntimeWindow<P>, changed: bool) {
    if !changed {
        return;
    }
    if runtime.renderer.take_rebuild_request() {
        runtime.needs_rebuild = true;
    } else {
        runtime.renderer.request_redraw();
    }
    runtime.platform.request_redraw();
}

fn schedule_scene_rebuild<P: PlatformWindow>(runtime: &mut RuntimeWindow<P>, changed: bool) {
    if !changed {
        return;
    }
    runtime.needs_rebuild = true;
    runtime.platform.request_redraw();
}

fn create_bounds(width: u32, height: u32, scale_factor: f64) -> vello::kurbo::Rect {
    assert!(
        scale_factor.is_finite() && scale_factor > 0.0,
        "hydrolysis runner: invalid scale factor {scale_factor}"
    );
    vello::kurbo::Rect::new(
        0.0,
        0.0,
        f64::from(width) / scale_factor,
        f64::from(height) / scale_factor,
    )
}

fn window_clear_color(window: &Window, env: &Environment) -> vello::peniko::Color {
    match &window.background {
        WindowBackground::Opaque => vello::peniko::Color::WHITE,
        WindowBackground::Color(color) => {
            let resolved = color.resolve(env).get();
            let srgb = resolved.to_srgb_with_headroom();
            vello::peniko::Color::new([srgb.red, srgb.green, srgb.blue, resolved.opacity])
        }
    }
}

fn render_window<P: PlatformWindow>(runtime: &mut RuntimeWindow<P>, env: &Environment) {
    let _ = render_window_with_capture(runtime, env, false);
}

fn rebuild_window_scene<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) -> (bool, u32, Duration) {
    let diagnostics_enabled = runtime.render_diagnostics.enabled();
    let scale_factor = runtime.platform.scale_factor();
    let surface = runtime.platform.surface();
    let (width, height) = surface.size();
    let bounds = create_bounds(width, height, scale_factor);
    let root_transform = vello::kurbo::Affine::scale(scale_factor);
    runtime
        .renderer
        .set_frame_resources(surface.device(), surface.queue());

    let rebuild_started_at = diagnostics_enabled.then(Instant::now);
    if runtime.renderer.advance_animations() {
        runtime.needs_rebuild = true;
    }

    let mut rebuilt = false;
    let mut rebuild_iterations = 0u32;
    loop {
        let should_rebuild = runtime.needs_rebuild || runtime.renderer.take_rebuild_request();
        if !should_rebuild {
            break;
        }
        rebuild_iterations = rebuild_iterations
            .checked_add(1)
            .expect("hydrolysis runner: rebuild iteration counter overflow");
        assert!(
            rebuild_iterations <= 64,
            "hydrolysis runner: rebuild loop exceeded 64 iterations in a single pump"
        );
        runtime.renderer.reset_scene();
        runtime.renderer.begin_rebuild_frame();
        runtime.renderer.set_window_bounds(bounds);
        let content = runtime
            .renderer
            .with_local_state_env(env, |_local_env| runtime.window.build_content());
        runtime.renderer.dispatch_with_transform(
            content,
            env,
            bounds,
            root_transform,
            vello::kurbo::Affine::IDENTITY,
        );
        runtime
            .renderer
            .render_active_text_context_menu_overlay(env, root_transform);
        runtime.renderer.finish_rebuild_frame();
        runtime
            .renderer
            .sync_active_interactions_after_layout(runtime.pointer_position);
        runtime.needs_rebuild = false;
        rebuilt = true;
        if let Some((x, y)) = runtime.pointer_position
            && runtime.renderer.sync_pointer_hover_state(x, y, env)
        {
            if runtime.renderer.take_rebuild_request() {
                runtime.needs_rebuild = true;
            } else {
                runtime.renderer.request_redraw();
            }
        }
    }
    (
        rebuilt,
        rebuild_iterations,
        elapsed_or_zero(rebuild_started_at),
    )
}

fn pump_window_semantics<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) -> bool {
    runtime.platform.apply_properties(&runtime.window);
    #[cfg(feature = "winit")]
    runtime
        .renderer
        .set_accessibility_root_label(runtime.window.title.get().as_str());

    let should_rebuild = runtime.needs_rebuild || runtime.renderer.take_rebuild_request();
    if !should_rebuild {
        return false;
    }

    let (rebuilt, _, _) = rebuild_window_scene(runtime, env);
    runtime.renderer.clear_frame_resources();
    runtime
        .platform
        .sync_text_input_state(runtime.renderer.focused_text_input_state());
    if let Some((x, y)) = runtime.pointer_position {
        runtime
            .platform
            .set_cursor_style(runtime.renderer.cursor_style_at(x, y));
    }
    if runtime.renderer.take_redraw_request() {
        runtime.platform.request_redraw();
    }
    rebuilt
}

fn render_window_with_capture<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    capture_snapshot: bool,
) -> RenderWindowResult {
    runtime.platform.apply_properties(&runtime.window);
    #[cfg(feature = "winit")]
    runtime
        .renderer
        .set_accessibility_root_label(runtime.window.title.get().as_str());
    let mut snapshot = None;
    let mut rebuilt = false;
    let profile;
    {
        let diagnostics_enabled = runtime.render_diagnostics.enabled();
        let frame_started_at = diagnostics_enabled.then(Instant::now);
        let (scene_rebuilt, rebuild_iterations, rebuild_duration) =
            rebuild_window_scene(runtime, env);
        rebuilt |= scene_rebuilt;

        let surface = runtime.platform.surface();
        let (width, height) = surface.size();
        let format = surface.format();
        let clear_color = window_clear_color(&runtime.window, env);
        let acquire_started_at = Instant::now();
        let frame = match surface.acquire() {
            Ok(frame) => frame,
            Err(crate::platform::SurfaceError::Surface(
                wgpu::SurfaceError::Lost
                | wgpu::SurfaceError::Outdated
                | wgpu::SurfaceError::Timeout
                | wgpu::SurfaceError::Other,
            )) => {
                runtime.needs_rebuild = true;
                runtime.platform.request_redraw();
                let (measurement_cache_hits, measurement_cache_misses) =
                    runtime.renderer.measurement_cache_stats();
                return RenderWindowResult {
                    rebuilt,
                    snapshot,
                    profile: FrameProfile {
                        phases: FramePhases {
                            rebuild: rebuild_duration,
                            acquire: acquire_started_at.elapsed(),
                            ..FramePhases::default()
                        },
                        counters: FrameCounters {
                            rebuild_iterations,
                            measurement_cache_hits,
                            measurement_cache_misses,
                            rendered: false,
                            captured_snapshot: false,
                        },
                        ..FrameProfile::default()
                    },
                };
            }
            Err(error) => panic!("hydrolysis runner: failed to acquire frame: {error}"),
        };
        let acquire_duration = acquire_started_at.elapsed();
        let render_started_at = Instant::now();
        runtime
            .renderer
            .render_scene_to_surface(crate::renderer::HydrolysisRenderTarget {
                device: surface.device(),
                queue: surface.queue(),
                view: frame.view(),
                format,
                width,
                height,
                base_color: clear_color,
            });
        if capture_snapshot {
            snapshot = Some(HeadlessSnapshot {
                width,
                height,
                rgba8: readback_texture_rgba8(
                    surface.device(),
                    surface.queue(),
                    frame.texture(),
                    width,
                    height,
                ),
            });
        }
        runtime.renderer.clear_frame_resources();
        let render_duration = render_started_at.elapsed();
        let present_started_at = Instant::now();
        surface.present(frame);
        let present_duration = present_started_at.elapsed();
        let (measurement_cache_hits, measurement_cache_misses) =
            runtime.renderer.measurement_cache_stats();
        profile = FrameProfile {
            phases: FramePhases {
                rebuild: rebuild_duration,
                acquire: acquire_duration,
                render: render_duration,
                present: present_duration,
                ..FramePhases::default()
            },
            counters: FrameCounters {
                rebuild_iterations,
                measurement_cache_hits,
                measurement_cache_misses,
                rendered: true,
                captured_snapshot: capture_snapshot,
            },
            ..FrameProfile::default()
        };

        if diagnostics_enabled {
            let window_title = runtime.window.title.get();
            runtime.render_diagnostics.record_frame(
                window_title.as_str(),
                RenderPhaseSample {
                    rebuild: rebuild_duration,
                    acquire: acquire_duration,
                    render: render_duration,
                    present: present_duration,
                    total: elapsed_or_zero(frame_started_at),
                    rebuild_iterations,
                    rebuilt: rebuild_iterations > 0,
                },
            );
        }
    }

    runtime
        .platform
        .sync_text_input_state(runtime.renderer.focused_text_input_state());
    if let Some((x, y)) = runtime.pointer_position {
        runtime
            .platform
            .set_cursor_style(runtime.renderer.cursor_style_at(x, y));
    }
    if runtime.renderer.take_redraw_request() {
        runtime.platform.request_redraw();
    }

    RenderWindowResult {
        rebuilt,
        snapshot,
        profile,
    }
}

fn physical_to_logical_dimension(value: u32, scale_factor: f64) -> f32 {
    assert!(
        scale_factor.is_finite() && scale_factor > 0.0,
        "hydrolysis runner: invalid scale factor {scale_factor}"
    );
    (f64::from(value) / scale_factor) as f32
}

fn handle_input_events<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
) -> bool {
    handle_input_events_with(runtime, env, |_runtime, env| env.clone())
}

fn handle_input_events_with<P, F>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    input_env: F,
) -> bool
where
    P: PlatformWindow,
    F: Fn(&RuntimeWindow<P>, &Environment) -> Environment,
{
    let mut should_close = runtime.window.state.get() == waterui::window::WindowState::Closed;
    for event in runtime.platform.drain_events() {
        match event {
            InputEvent::CloseRequested => {
                runtime
                    .window
                    .state
                    .set(waterui::window::WindowState::Closed);
                should_close = true;
            }
            InputEvent::Moved { x, y } => {
                let frame = runtime.window.frame.get();
                runtime.window.frame.set(waterui_core::layout::Rect::new(
                    waterui_core::layout::Point::new(x, y),
                    *frame.size(),
                ));
            }
            InputEvent::Resize { width, height } => {
                let frame = runtime.window.frame.get();
                let logical_width =
                    physical_to_logical_dimension(width, runtime.platform.scale_factor());
                let logical_height =
                    physical_to_logical_dimension(height, runtime.platform.scale_factor());
                let frame = waterui_core::layout::Rect::new(
                    frame.origin(),
                    waterui_core::layout::Size::new(logical_width, logical_height),
                );
                runtime.window.frame.set(frame);
                runtime.needs_rebuild = true;
                runtime.platform.request_redraw();
            }
            InputEvent::PointerDown { x, y, button } => {
                runtime.pointer_position = Some((x, y));
                let changed =
                    runtime
                        .renderer
                        .handle_pointer_down(x, y, button, &input_env(runtime, env));
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_down",
                    x,
                    y,
                    button = ?button,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::PointerUp { x, y, button } => {
                runtime.pointer_position = Some((x, y));
                let changed =
                    runtime
                        .renderer
                        .handle_pointer_up(x, y, button, &input_env(runtime, env));
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_up",
                    x,
                    y,
                    button = ?button,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::PointerMove { x, y } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime
                    .renderer
                    .handle_pointer_move(x, y, &input_env(runtime, env));
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_move",
                    x,
                    y,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::PointerCancel => {
                let changed = runtime
                    .renderer
                    .handle_pointer_cancel(&input_env(runtime, env));
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "pointer_cancel",
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::Scroll {
                x,
                y,
                dx,
                dy,
                is_line_delta,
            } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime.renderer.handle_scroll(x, y, dx, dy, is_line_delta);
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "scroll",
                    x,
                    y,
                    dx,
                    dy,
                    is_line_delta,
                    changed,
                    "runner dispatched input event"
                );
                schedule_scene_rebuild(runtime, changed);
            }
            InputEvent::Magnification { x, y, delta, phase } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime
                    .renderer
                    .handle_magnification(x, y, delta, phase, env);
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::Rotation { x, y, delta, phase } => {
                runtime.pointer_position = Some((x, y));
                let changed = runtime.renderer.handle_rotation(x, y, delta, phase, env);
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::TextInput { text } => {
                let changed = runtime.renderer.handle_text_input(text.as_str());
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "text_input",
                    text = text.as_str(),
                    changed,
                    "runner dispatched input event"
                );
                if changed {
                    runtime.needs_rebuild = true;
                }
            }
            InputEvent::Key {
                key,
                state: KeyState::Pressed,
                modifiers,
            } => {
                let changed = runtime.renderer.handle_key(&key, modifiers);
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "key_pressed",
                    key = ?key,
                    modifiers = ?modifiers,
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::ImePreedit { text } => {
                let changed = runtime.renderer.handle_ime_preedit(text.as_str());
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "ime_preedit",
                    text = text.as_str(),
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::ImeCommit { text } => {
                let changed = runtime.renderer.handle_ime_commit(text.as_str());
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "ime_commit",
                    text = text.as_str(),
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::ImeDisabled => {
                let changed = runtime.renderer.handle_ime_disabled();
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = "ime_disabled",
                    changed,
                    "runner dispatched input event"
                );
                schedule_redraw_or_rebuild(runtime, changed);
            }
            InputEvent::Key {
                state: KeyState::Released,
                ..
            } => {}
        }
    }
    runtime
        .platform
        .sync_text_input_state(runtime.renderer.focused_text_input_state());
    if let Some((x, y)) = runtime.pointer_position {
        runtime
            .platform
            .set_cursor_style(runtime.renderer.cursor_style_at(x, y));
    }
    should_close
}

fn advance_runtime<P: PlatformWindow>(
    runtime: &mut RuntimeWindow<P>,
    env: &Environment,
    now: Instant,
) -> Option<Instant> {
    runtime.renderer.set_frame_instant(now);
    runtime
        .platform
        .sync_text_input_state(runtime.renderer.focused_text_input_state());
    if runtime.renderer.poll_gpu_surface_redraw_handles() {
        runtime.platform.request_redraw();
    }
    if runtime.renderer.handle_gesture_tick(now, env) {
        runtime.needs_rebuild = true;
    }
    if runtime.renderer.advance_animations() {
        runtime.needs_rebuild = true;
    }
    if runtime.renderer.take_rebuild_request() {
        runtime.needs_rebuild = true;
    }
    let next_deadline = runtime.renderer.next_gesture_deadline();
    if runtime.needs_rebuild {
        runtime.platform.request_redraw();
    }
    next_deadline
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct HeadlessPlatformWindow {
    inner: OffscreenWindow,
    pending_events: VecDeque<InputEvent>,
    redraw_requested: Cell<bool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl HeadlessPlatformWindow {
    fn new(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            inner: OffscreenWindow::new(width, height, format),
            pending_events: VecDeque::new(),
            redraw_requested: Cell::new(false),
        }
    }

    #[cfg(any(test, feature = "testing"))]
    fn new_for_tests(width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        Self {
            inner: OffscreenWindow::new_for_tests(width, height, format),
            pending_events: VecDeque::new(),
            redraw_requested: Cell::new(false),
        }
    }

    fn push_event(&mut self, event: InputEvent) {
        self.pending_events.push_back(event);
    }

    fn take_redraw_request(&self) -> bool {
        self.redraw_requested.replace(false)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PlatformWindow for HeadlessPlatformWindow {
    fn surface(&mut self) -> &mut dyn crate::platform::SurfaceProvider {
        self.inner.surface()
    }

    fn apply_properties(&mut self, window: &Window) {
        self.inner.apply_properties(window);
    }

    fn drain_events(&mut self) -> Vec<InputEvent> {
        self.pending_events.drain(..).collect()
    }

    fn request_redraw(&self) {
        self.redraw_requested.set(true);
    }

    fn scale_factor(&self) -> f64 {
        self.inner.scale_factor()
    }

    fn sync_text_input_state(&mut self, state: Option<crate::platform::TextInputState>) {
        self.inner.sync_text_input_state(state);
    }

    fn set_cursor_style(&mut self, style: waterui::cursor::CursorStyle) {
        self.inner.set_cursor_style(style);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
struct HeadlessMainThreadExecutor {
    runnable_tx: mpsc::Sender<Runnable>,
    runnable_rx: Rc<mpsc::Receiver<Runnable>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for HeadlessMainThreadExecutor {
    fn default() -> Self {
        let (runnable_tx, runnable_rx) = mpsc::channel();
        Self {
            runnable_tx,
            runnable_rx: Rc::new(runnable_rx),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl HeadlessMainThreadExecutor {
    fn drain(&self) -> bool {
        let mut ran = false;
        loop {
            let Ok(runnable) = self.runnable_rx.try_recv() else {
                return ran;
            };
            ran = true;
            runnable.run();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl LocalExecutor for HeadlessMainThreadExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: std::future::Future + 'static,
    {
        let runnable_tx = self.runnable_tx.clone();
        let (runnable, task) = executor_core::async_task::spawn_local(fut, move |runnable| {
            let _ = runnable_tx.send(runnable);
        });
        runnable.schedule();
        task
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct HeadlessPumpResult {
    pub rebuilt: bool,
    pub profile: FrameProfile,
    #[cfg(feature = "accessibility")]
    pub tree_update: Option<AccessibilityTreeUpdate>,
    pub snapshot: Option<HeadlessSnapshot>,
    #[cfg(feature = "accessibility")]
    pub ui_focus: Option<accesskit::NodeId>,
}

#[cfg(not(target_arch = "wasm32"))]
pub struct HeadlessRuntime {
    env: Environment,
    runtime: RuntimeWindow<HeadlessPlatformWindow>,
    pending_window_queue: Rc<RefCell<Vec<Window>>>,
    popup_windows: Vec<RuntimeWindow<HeadlessPlatformWindow>>,
    create_platform: fn(u32, u32, wgpu::TextureFormat) -> HeadlessPlatformWindow,
    local_executor: HeadlessMainThreadExecutor,
}

#[cfg(not(target_arch = "wasm32"))]
impl HeadlessRuntime {
    #[must_use]
    pub fn new(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
    ) -> Self {
        Self::new_with_platform_window(env, content, width, height, HeadlessPlatformWindow::new)
    }

    /// Creates a headless runtime for WaterUI test hosts.
    ///
    /// This constructor allows compute-capable software adapters for CI-only
    /// semantic testing while keeping [`Self::new`] on production adapter
    /// selection.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn new_for_tests(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
    ) -> Self {
        Self::new_with_platform_window(
            env,
            content,
            width,
            height,
            HeadlessPlatformWindow::new_for_tests,
        )
    }

    fn new_with_platform_window(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
        create_platform: fn(u32, u32, wgpu::TextureFormat) -> HeadlessPlatformWindow,
    ) -> Self {
        init_main_thread_executors();
        let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
        let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
        install_native_component_hooks(&mut env);
        env.insert(WindowManager::new({
            let pending_window_queue = Rc::clone(&pending_window_queue);
            move |window| {
                pending_window_queue.borrow_mut().push(window);
            }
        }));
        env.insert(HydrolysisTextContextMenuMode::Overlay);
        env.insert(waterui_core::ViewRenderer::new(
            crate::view_renderer::HydrolysisViewRenderer::default(),
        ));

        let local_executor = HeadlessMainThreadExecutor::default();
        let _ = try_init_local_executor(waterui::task::monitored_local_executor(
            local_executor.clone(),
        ));

        let content_builder = content.clone();
        let window = Window::new(
            "",
            waterui_core::binding(waterui::window::WindowState::Normal),
            move || content_builder.build(),
        )
        .background(Color::transparent());
        window.frame.set(waterui_core::layout::Rect::new(
            waterui_core::layout::Point::zero(),
            waterui_core::layout::Size::new(width.max(1) as f32, height.max(1) as f32),
        ));

        let mut platform =
            create_platform(width.max(1), height.max(1), wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        load_native_resource_fonts(&mut renderer);

        Self {
            env,
            runtime: RuntimeWindow::new(
                window,
                platform,
                renderer,
                RenderDiagnosticsConfig {
                    enabled: false,
                    interval: Duration::from_secs(1),
                    slow_frame_threshold: Duration::from_millis(16),
                },
            ),
            pending_window_queue,
            popup_windows: Vec::new(),
            create_platform,
            local_executor,
        }
    }

    fn create_popup_runtime(&self, window: Window) -> RuntimeWindow<HeadlessPlatformWindow> {
        let frame = window.frame.get();
        let width = frame.width().max(1.0) as u32;
        let height = frame.height().max(1.0) as u32;
        let mut platform = (self.create_platform)(width, height, wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        load_native_resource_fonts(&mut renderer);
        RuntimeWindow::new(
            window,
            platform,
            renderer,
            RenderDiagnosticsConfig {
                enabled: false,
                interval: Duration::from_secs(1),
                slow_frame_threshold: Duration::from_millis(16),
            },
        )
    }

    fn mount_pending_popup_windows(&mut self) {
        let pending = self
            .pending_window_queue
            .borrow_mut()
            .drain(..)
            .collect::<Vec<_>>();
        for window in pending {
            self.popup_windows.push(self.create_popup_runtime(window));
        }
    }

    pub fn push_input_event(&mut self, event: InputEvent) {
        self.runtime.platform.push_event(event);
    }

    #[cfg(feature = "accessibility")]
    pub fn perform_accessibility_action(&mut self, request: AccessibilityActionRequest) -> bool {
        let changed = self
            .runtime
            .renderer
            .handle_accessibility_action(request, &self.env);
        if changed {
            self.runtime.needs_rebuild = true;
            self.runtime.platform.request_redraw();
        }
        changed
    }

    #[cfg(feature = "accessibility")]
    pub fn clear_ui_focus(&mut self) -> bool {
        let changed = self.runtime.renderer.clear_ui_focus();
        if changed {
            self.runtime.needs_rebuild = true;
            self.runtime.platform.request_redraw();
        }
        changed
    }

    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn focused_ui_node(&self) -> Option<accesskit::NodeId> {
        self.runtime.renderer.focused_ui_node()
    }

    pub fn pump(&mut self, capture_snapshot: bool) -> HeadlessPumpResult {
        self.pump_at(capture_snapshot, Instant::now())
    }

    pub fn pump_semantic(&mut self) -> HeadlessPumpResult {
        self.pump_semantic_at(Instant::now())
    }

    pub fn pump_offscreen(&mut self) -> HeadlessPumpResult {
        self.pump_at(false, Instant::now())
    }

    pub fn pump_snapshot(&mut self) -> HeadlessPumpResult {
        self.pump_at(true, Instant::now())
    }

    pub fn pump_semantic_at(&mut self, at: Instant) -> HeadlessPumpResult {
        let frame_started_at = Instant::now();
        self.runtime.renderer.set_frame_instant(at);
        let executor_before_started_at = Instant::now();
        let drained_before = self.local_executor.drain();
        let executor_before = executor_before_started_at.elapsed();
        let input_started_at = Instant::now();
        let _ = handle_input_events(&mut self.runtime, &self.env);
        let input = input_started_at.elapsed();
        let animation_started_at = Instant::now();
        let _ = advance_runtime(&mut self.runtime, &self.env, at);
        let animation = animation_started_at.elapsed();
        self.mount_pending_popup_windows();
        for popup in &mut self.popup_windows {
            popup.renderer.set_frame_instant(at);
            let _ = handle_input_events(popup, &self.env);
            let _ = advance_runtime(popup, &self.env, at);
        }
        let rebuilt = pump_window_semantics(&mut self.runtime, &self.env);
        for popup in &mut self.popup_windows {
            let _ = pump_window_semantics(popup, &self.env);
        }
        let executor_after_started_at = Instant::now();
        let drained_after = self.local_executor.drain();
        let executor_after = executor_after_started_at.elapsed();

        HeadlessPumpResult {
            rebuilt: rebuilt || drained_before || drained_after,
            profile: FrameProfile {
                phases: FramePhases {
                    executor_before,
                    input,
                    animation,
                    executor_after,
                    ..FramePhases::default()
                },
                ..FrameProfile::default()
            }
            .with_total(frame_started_at.elapsed()),
            #[cfg(feature = "accessibility")]
            tree_update: self.runtime.renderer.take_accessibility_tree_update(),
            snapshot: None,
            #[cfg(feature = "accessibility")]
            ui_focus: self.runtime.renderer.focused_ui_node(),
        }
    }

    pub fn pump_at(&mut self, capture_snapshot: bool, at: Instant) -> HeadlessPumpResult {
        let frame_started_at = Instant::now();
        self.runtime.renderer.set_frame_instant(at);
        let executor_before_started_at = Instant::now();
        let drained_before = self.local_executor.drain();
        let executor_before = executor_before_started_at.elapsed();
        let input_started_at = Instant::now();
        let _ = handle_input_events(&mut self.runtime, &self.env);
        let input = input_started_at.elapsed();
        let animation_started_at = Instant::now();
        let _ = advance_runtime(&mut self.runtime, &self.env, at);
        let animation = animation_started_at.elapsed();
        self.mount_pending_popup_windows();
        for popup in &mut self.popup_windows {
            popup.renderer.set_frame_instant(at);
            let _ = handle_input_events(popup, &self.env);
            let _ = advance_runtime(popup, &self.env, at);
        }
        let should_render = capture_snapshot
            || self.runtime.needs_rebuild
            || self.runtime.platform.take_redraw_request();
        let mut render_result = should_render
            .then(|| render_window_with_capture(&mut self.runtime, &self.env, capture_snapshot));
        if capture_snapshot && !self.popup_windows.is_empty() {
            if let Some(snapshot) = render_result
                .as_mut()
                .and_then(|result| result.snapshot.as_mut())
            {
                for popup in &mut self.popup_windows {
                    let Some(popup_snapshot) =
                        render_window_with_capture(popup, &self.env, true).snapshot
                    else {
                        continue;
                    };
                    composite_popup_snapshot(snapshot, &popup_snapshot, popup.window.frame.get());
                }
            }
        }
        let executor_after_started_at = Instant::now();
        let drained_after = self.local_executor.drain();
        let executor_after = executor_after_started_at.elapsed();

        let mut profile = render_result
            .as_ref()
            .map_or_else(FrameProfile::default, |result| result.profile);
        profile.phases.executor_before = executor_before;
        profile.phases.input = input;
        profile.phases.animation = animation;
        profile.phases.executor_after = executor_after;

        HeadlessPumpResult {
            rebuilt: render_result.as_ref().is_some_and(|result| result.rebuilt)
                || drained_before
                || drained_after,
            profile: profile.with_total(frame_started_at.elapsed()),
            #[cfg(feature = "accessibility")]
            tree_update: self.runtime.renderer.take_accessibility_tree_update(),
            snapshot: render_result.and_then(|result| result.snapshot),
            #[cfg(feature = "accessibility")]
            ui_focus: self.runtime.renderer.focused_ui_node(),
        }
    }
}

fn composite_popup_snapshot(
    target: &mut HeadlessSnapshot,
    source: &HeadlessSnapshot,
    frame: waterui_core::layout::Rect,
) {
    let offset_x = frame.x().round() as i32;
    let offset_y = frame.y().round() as i32;
    for source_y in 0..source.height {
        let target_y = offset_y + i32::try_from(source_y).expect("source y should fit i32");
        if target_y < 0 || target_y >= i32::try_from(target.height).expect("height should fit i32")
        {
            continue;
        }
        for source_x in 0..source.width {
            let target_x = offset_x + i32::try_from(source_x).expect("source x should fit i32");
            if target_x < 0
                || target_x >= i32::try_from(target.width).expect("width should fit i32")
            {
                continue;
            }
            let source_index = ((source_y * source.width + source_x) * 4) as usize;
            let target_index = ((u32::try_from(target_y).expect("target y should be non-negative")
                * target.width
                + u32::try_from(target_x).expect("target x should be non-negative"))
                * 4) as usize;
            composite_pixel(
                &mut target.rgba8[target_index..target_index + 4],
                &source.rgba8[source_index..source_index + 4],
            );
        }
    }
}

fn composite_pixel(target: &mut [u8], source: &[u8]) {
    let source_alpha = f32::from(source[3]) / 255.0;
    if source_alpha <= 0.0 {
        return;
    }
    let target_alpha = f32::from(target[3]) / 255.0;
    let out_alpha = source_alpha + target_alpha * (1.0 - source_alpha);
    for channel in 0..3 {
        let source_channel = f32::from(source[channel]) / 255.0;
        let target_channel = f32::from(target[channel]) / 255.0;
        let out = (source_channel * source_alpha
            + target_channel * target_alpha * (1.0 - source_alpha))
            / out_alpha;
        target[channel] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    target[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{
        HeadlessPlatformWindow, RenderDiagnosticsConfig, RuntimeWindow, schedule_redraw_or_rebuild,
        schedule_scene_rebuild,
    };
    use crate::platform::PlatformWindow as _;
    use crate::renderer::HydrolysisRenderer;
    use core::time::Duration;
    use waterui::window::{Window, WindowState};
    use waterui_core::binding;

    #[test]
    fn changed_redraw_only_input_wakes_platform_window() {
        let mut runtime = test_runtime_window();
        runtime.needs_rebuild = false;

        schedule_redraw_or_rebuild(&mut runtime, true);

        assert!(!runtime.needs_rebuild);
        assert!(runtime.renderer.take_redraw_request());
        assert!(runtime.platform.take_redraw_request());
    }

    #[test]
    fn changed_rebuild_input_wakes_platform_window() {
        let mut runtime = test_runtime_window();
        runtime.needs_rebuild = false;
        runtime.renderer.request_rebuild();

        schedule_redraw_or_rebuild(&mut runtime, true);

        assert!(runtime.needs_rebuild);
        assert!(
            runtime.platform.take_redraw_request(),
            "rebuild input must wake the platform event loop for the next frame"
        );
    }

    #[test]
    fn changed_scroll_input_rebuilds_scene_and_wakes_platform_window() {
        let mut runtime = test_runtime_window();
        runtime.needs_rebuild = false;

        schedule_scene_rebuild(&mut runtime, true);

        assert!(
            runtime.needs_rebuild,
            "scroll changes alter baked scene transforms and must rebuild the scene"
        );
        assert!(
            runtime.platform.take_redraw_request(),
            "scroll rebuilds must wake the platform event loop for the next frame"
        );
    }

    fn test_runtime_window() -> RuntimeWindow<HeadlessPlatformWindow> {
        let window = Window::new("", binding(WindowState::Normal), || ());
        let mut platform =
            HeadlessPlatformWindow::new_for_tests(16, 16, wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        RuntimeWindow::new(
            window,
            platform,
            renderer,
            RenderDiagnosticsConfig {
                enabled: false,
                interval: Duration::from_secs(1),
                slow_frame_threshold: Duration::from_millis(16),
            },
        )
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "winit")))]
pub fn run(app: App) {
    init_main_thread_executors();
    let (windows, _menu_bar, env) = app.into_parts();
    let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
    let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
    let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
    install_native_component_hooks(&mut env);
    install_window_manager(&mut env, Rc::clone(&pending_window_queue));
    env.insert(HydrolysisTextContextMenuMode::Overlay);
    env.insert(waterui_core::ViewRenderer::new(
        crate::view_renderer::HydrolysisViewRenderer::default(),
    ));
    let mut pending_windows = VecDeque::from(windows);
    while let Some(window) = pending_windows.pop_front() {
        let frame = window.frame.get();
        let width = frame.width().max(1.0) as u32;
        let height = frame.height().max(1.0) as u32;
        let mut platform = OffscreenWindow::new(width, height, wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        load_native_resource_fonts(&mut renderer);
        let mut runtime = RuntimeWindow::new(window, platform, renderer, render_diagnostics_config);
        render_window(&mut runtime, &env);
        pending_windows.extend(pending_window_queue.borrow_mut().drain(..));
    }
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub fn run(app: App) {
    init_main_thread_executors();
    web_runner::run(app);
}

#[cfg(all(not(target_arch = "wasm32"), feature = "winit"))]
pub fn run(app: App) {
    initialize_tracing_from_env();
    init_main_thread_executors();
    winit_runner::run(app);
}

#[cfg(all(not(target_arch = "wasm32"), feature = "winit"))]
fn initialize_tracing_from_env() {
    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web_runner {
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        future::Future,
        rc::Rc,
        sync::Arc,
    };

    use async_task::spawn_unchecked as spawn_local_task;
    use executor_core::{
        LocalExecutor,
        async_task::{AsyncTask, Runnable},
        try_init_local_executor,
    };
    use fontique::{Blob, FontInfoOverride, GenericFamily};
    use js_sys::Uint8Array;
    use serde::Deserialize;
    use wasm_bindgen::{JsCast, closure::Closure};
    use wasm_bindgen_futures::JsFuture;
    use waterui::app::App;
    use waterui::window::WindowState;
    use waterui_core::Environment;
    use web_sys::Response;

    use crate::platform::{BrowserWindow, PlatformWindow};
    use crate::renderer::{HydrolysisRenderer, HydrolysisTextContextMenuMode};
    use crate::runner::{
        RenderDiagnosticsConfig, RuntimeWindow, handle_input_events, render_window,
    };

    const WEB_FONT_MANIFEST_PATH: &str = "fonts/waterui-fonts.json";

    #[derive(Debug, Deserialize)]
    struct WebFontManifest {
        default_family: String,
        fonts: Vec<WebFontManifestEntry>,
    }

    #[derive(Debug, Deserialize)]
    struct WebFontManifestEntry {
        name: String,
        file_name: String,
    }

    async fn fetch_response(path: &str) -> Response {
        let window = web_sys::window().expect("hydrolysis web font loader requires browser window");
        let response = JsFuture::from(window.fetch_with_str(path))
            .await
            .unwrap_or_else(|error| {
                panic!("hydrolysis web font fetch failed for `{path}`: {error:?}")
            });
        let response: Response = response.dyn_into().unwrap_or_else(|_| {
            panic!("hydrolysis web font fetch returned non-Response for `{path}`")
        });
        assert!(
            response.ok(),
            "hydrolysis web font fetch failed for `{path}` with HTTP status {}",
            response.status()
        );
        response
    }

    async fn fetch_bytes(path: &str) -> Vec<u8> {
        let response = fetch_response(path).await;
        let array_buffer = JsFuture::from(response.array_buffer().unwrap_or_else(|error| {
            panic!("hydrolysis web font response array_buffer failed for `{path}`: {error:?}")
        }))
        .await
        .unwrap_or_else(|error| {
            panic!("hydrolysis web font array_buffer await failed for `{path}`: {error:?}")
        });
        let bytes = Uint8Array::new(&array_buffer);
        let mut data = vec![0_u8; bytes.length() as usize];
        bytes.copy_to(&mut data);
        data
    }

    async fn fetch_text(path: &str) -> String {
        String::from_utf8(fetch_bytes(path).await).unwrap_or_else(|error| {
            panic!("hydrolysis web font manifest `{path}` is not valid UTF-8: {error}")
        })
    }

    async fn load_web_fonts(renderer: &mut HydrolysisRenderer) {
        let manifest_text = fetch_text(WEB_FONT_MANIFEST_PATH).await;
        let manifest: WebFontManifest = serde_json::from_str(&manifest_text).unwrap_or_else(
            |error| panic!("hydrolysis web font manifest parse failed for `{WEB_FONT_MANIFEST_PATH}`: {error}"),
        );

        let mut default_family_ids = Vec::new();
        let state = renderer.state_mut();
        for font in manifest.fonts {
            let font_path = format!("fonts/{}", font.file_name);
            let font_data = fetch_bytes(&font_path).await;
            let families = state.font_cx.collection.register_fonts(
                Blob::new(Arc::new(font_data)),
                Some(FontInfoOverride {
                    family_name: Some(font.name.as_str()),
                    ..Default::default()
                }),
            );
            if font.name == manifest.default_family {
                default_family_ids.extend(families.into_iter().map(|(family_id, _)| family_id));
            }
        }

        assert!(
            !default_family_ids.is_empty(),
            "hydrolysis web font manifest default family `{}` did not register any fonts",
            manifest.default_family
        );
        state
            .font_cx
            .collection
            .set_generic_families(GenericFamily::SansSerif, default_family_ids.iter().copied());
        state.font_cx.collection.set_generic_families(
            GenericFamily::UiSansSerif,
            default_family_ids.iter().copied(),
        );
    }

    #[derive(Clone)]
    struct BrowserMainThreadExecutor {
        runnable_queue: Rc<RefCell<VecDeque<Runnable>>>,
        schedule_frame: Rc<dyn Fn()>,
    }

    impl LocalExecutor for BrowserMainThreadExecutor {
        type Task<T: 'static> = AsyncTask<T>;

        fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
        where
            Fut: Future + 'static,
        {
            let runnable_queue = self.runnable_queue.clone();
            let schedule_frame = self.schedule_frame.clone();
            let (runnable, task) = unsafe {
                // SAFETY: the browser executor is single-threaded and every runnable is queued
                // and polled on the same main-thread event loop.
                spawn_local_task(fut, move |runnable: Runnable| {
                    runnable_queue.borrow_mut().push_back(runnable);
                    schedule_frame();
                })
            };
            runnable.schedule();
            AsyncTask::from(task)
        }
    }

    struct BrowserRunner {
        env: Environment,
        runtime: RuntimeWindow<BrowserWindow>,
        runnable_queue: Rc<RefCell<VecDeque<Runnable>>>,
    }

    impl BrowserRunner {
        fn drain_local_executor_queue(&self) {
            while let Some(runnable) = self.runnable_queue.borrow_mut().pop_front() {
                runnable.run();
            }
        }

        fn frame(&mut self) -> bool {
            self.drain_local_executor_queue();
            let should_close = handle_input_events(&mut self.runtime, &self.env);
            if should_close || self.runtime.window.state.get() == WindowState::Closed {
                return false;
            }
            render_window(&mut self.runtime, &self.env);
            true
        }

        fn needs_next_frame(&self) -> bool {
            self.runtime.platform.take_redraw_request() || !self.runnable_queue.borrow().is_empty()
        }
    }

    struct BrowserRunnerHandle {
        runner: RefCell<BrowserRunner>,
        raf_pending: Cell<bool>,
        raf_callback: RefCell<Option<Closure<dyn FnMut(f64)>>>,
    }

    impl BrowserRunnerHandle {
        fn schedule_frame(self: &Rc<Self>) {
            if self.raf_pending.replace(true) {
                return;
            }

            let browser_window = web_sys::window()
                .expect("hydrolysis web runner: browser window unavailable for animation frame");
            let callback = self.raf_callback.borrow();
            let callback = callback
                .as_ref()
                .expect("hydrolysis web runner: animation frame callback not initialized");
            browser_window
                .request_animation_frame(callback.as_ref().unchecked_ref())
                .expect("hydrolysis web runner: failed to schedule animation frame");
        }

        fn frame(self: &Rc<Self>) {
            self.raf_pending.set(false);
            let should_continue = self.runner.borrow_mut().frame();
            if !should_continue {
                return;
            }

            if self.runner.borrow().needs_next_frame() {
                self.schedule_frame();
            }
        }
    }

    pub fn run(app: App) {
        wasm_bindgen_futures::spawn_local(async move {
            let schedule_frame_ref: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
            let browser_schedule = {
                let schedule_frame_ref = schedule_frame_ref.clone();
                Rc::new(move || {
                    let schedule = schedule_frame_ref
                        .borrow()
                        .as_ref()
                        .cloned()
                        .expect("hydrolysis web runner: frame scheduler is not ready");
                    schedule();
                }) as Rc<dyn Fn()>
            };
            let runnable_queue = Rc::new(RefCell::new(VecDeque::new()));
            let local_executor = BrowserMainThreadExecutor {
                runnable_queue: runnable_queue.clone(),
                schedule_frame: browser_schedule.clone(),
            };
            let _ =
                try_init_local_executor(waterui::task::monitored_local_executor(local_executor));

            let (windows, _menu_bar, env) = app.into_parts();
            let mut windows = windows.into_iter();
            let window = windows
                .next()
                .expect("hydrolysis web runner requires exactly one window");
            assert!(
                windows.next().is_none(),
                "hydrolysis web runner supports exactly one window"
            );

            let mut env = env;
            let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
            super::install_native_component_hooks(&mut env);
            env.insert(HydrolysisTextContextMenuMode::Overlay);
            env.insert(waterui_core::ViewRenderer::new(
                crate::view_renderer::HydrolysisViewRenderer::default(),
            ));

            let mut platform = BrowserWindow::new(browser_schedule).await;
            platform.apply_properties(&window);
            let mut renderer = {
                let surface = platform.surface();
                HydrolysisRenderer::new(surface.device())
            };
            load_web_fonts(&mut renderer).await;
            let runtime = RuntimeWindow::new(window, platform, renderer, render_diagnostics_config);
            let runner = BrowserRunner {
                env,
                runtime,
                runnable_queue,
            };

            let handle = Rc::new(BrowserRunnerHandle {
                runner: RefCell::new(runner),
                raf_pending: Cell::new(false),
                raf_callback: RefCell::new(None),
            });
            let callback_handle = handle.clone();
            let callback = Closure::wrap(
                Box::new(move |_ts: f64| callback_handle.frame()) as Box<dyn FnMut(f64)>
            );
            *handle.raf_callback.borrow_mut() = Some(callback);
            *schedule_frame_ref.borrow_mut() = Some({
                let handle = handle.clone();
                Rc::new(move || handle.schedule_frame())
            });
            handle.schedule_frame();
        });
    }
}

#[cfg(feature = "winit")]
mod winit_runner {
    use async_task::spawn_local as spawn_local_task;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::future::Future;
    use std::mem;
    use std::rc::Rc;
    use std::sync::{Arc, mpsc};
    use std::time::Instant;

    use accesskit_winit::{
        Adapter as AccessKitAdapter, Event as AccessKitEvent, WindowEvent as AccessKitWindowEvent,
    };
    use executor_core::{
        LocalExecutor,
        async_task::{AsyncTask, Runnable},
        try_init_local_executor,
    };
    use nami::Signal;
    use waterui::app::App;
    use waterui::window::{Window, WindowState};
    use waterui_core::Environment;

    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    #[cfg(any(hydrolysis_wayland_platform, docsrs))]
    use winit::platform::wayland::EventLoopExtWayland;
    use winit::window::{Window as NativeWindow, WindowId};

    use crate::platform::{PlatformWindow, WinitWindow};
    use crate::renderer::{
        HydrolysisRenderer, HydrolysisTextContextMenuMode, HydrolysisWindowOrigin,
    };
    use crate::runner::{
        RenderDiagnosticsConfig, RuntimeWindow, advance_runtime, handle_input_events_with,
        render_window,
    };

    #[derive(Debug)]
    enum RunnerEvent {
        PollLocalTasks,
        MountPendingWindows,
        AccessKit(AccessKitEvent),
    }

    impl From<AccessKitEvent> for RunnerEvent {
        fn from(value: AccessKitEvent) -> Self {
            Self::AccessKit(value)
        }
    }

    #[derive(Clone)]
    struct WinitMainThreadExecutor {
        runnable_tx: mpsc::Sender<Runnable>,
        event_proxy: winit::event_loop::EventLoopProxy<RunnerEvent>,
    }

    impl LocalExecutor for WinitMainThreadExecutor {
        type Task<T: 'static> = AsyncTask<T>;

        fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
        where
            Fut: Future + 'static,
        {
            let runnable_tx = self.runnable_tx.clone();
            let event_proxy = self.event_proxy.clone();
            let (runnable, task) = spawn_local_task(fut, move |runnable: Runnable| {
                if runnable_tx.send(runnable).is_err() {
                    return;
                }
                let _ = event_proxy.send_event(RunnerEvent::PollLocalTasks);
            });
            runnable.schedule();
            AsyncTask::from(task)
        }
    }

    pub fn run(app: App) {
        let event_loop = EventLoop::<RunnerEvent>::with_user_event()
            .build()
            .expect("hydrolysis runner: failed to create event loop");
        let event_proxy = event_loop.create_proxy();
        let (local_runnable_tx, local_runnable_rx) = mpsc::channel::<Runnable>();
        let local_executor = WinitMainThreadExecutor {
            runnable_tx: local_runnable_tx,
            event_proxy: event_proxy.clone(),
        };
        let _ = try_init_local_executor(waterui::task::monitored_local_executor(local_executor));

        let (windows, _menu_bar, env) = app.into_parts();
        let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
        let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
        let render_diagnostics_config = RenderDiagnosticsConfig::from_env();
        super::install_native_component_hooks(&mut env);
        let text_context_menu_mode = {
            #[cfg(any(hydrolysis_wayland_platform, docsrs))]
            {
                if event_loop.is_wayland() {
                    HydrolysisTextContextMenuMode::Overlay
                } else {
                    HydrolysisTextContextMenuMode::NativeWindow
                }
            }
            #[cfg(not(any(hydrolysis_wayland_platform, docsrs)))]
            {
                HydrolysisTextContextMenuMode::NativeWindow
            }
        };
        env.insert(text_context_menu_mode);
        env.insert(waterui::window::WindowManager::new({
            let pending_window_queue = Rc::clone(&pending_window_queue);
            let event_proxy = event_proxy.clone();
            move |window| {
                pending_window_queue.borrow_mut().push(window);
                let _ = event_proxy.send_event(RunnerEvent::MountPendingWindows);
            }
        }));
        env.insert(waterui_core::ViewRenderer::new(
            crate::view_renderer::HydrolysisViewRenderer::default(),
        ));
        let mut runner = WinitRunner {
            env,
            pending_windows: windows,
            pending_window_queue,
            windows: HashMap::new(),
            accesskit_adapters: HashMap::new(),
            accessibility_enabled: super::probe_accessibility_runtime(),
            local_runnable_rx,
            event_proxy,
            render_diagnostics_config,
        };

        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop
            .run_app(&mut runner)
            .expect("hydrolysis runner: event loop failed");
    }

    struct WinitRunner {
        env: Environment,
        pending_windows: Vec<Window>,
        pending_window_queue: Rc<RefCell<Vec<Window>>>,
        windows: HashMap<WindowId, RuntimeWindow<WinitWindow>>,
        accesskit_adapters: HashMap<WindowId, AccessKitAdapter>,
        accessibility_enabled: bool,
        local_runnable_rx: mpsc::Receiver<Runnable>,
        event_proxy: winit::event_loop::EventLoopProxy<RunnerEvent>,
        render_diagnostics_config: RenderDiagnosticsConfig,
    }

    impl WinitRunner {
        fn drain_local_executor_queue(&self) {
            while let Ok(runnable) = self.local_runnable_rx.try_recv() {
                runnable.run();
            }
        }

        fn current_window_origin(runtime: &RuntimeWindow<WinitWindow>) -> HydrolysisWindowOrigin {
            let native_window = runtime.platform.native_window();
            if let Ok(position) = native_window.outer_position() {
                let logical = position.to_logical::<f64>(native_window.scale_factor());
                return HydrolysisWindowOrigin {
                    x: logical.x as f32,
                    y: logical.y as f32,
                };
            }
            HydrolysisWindowOrigin {
                x: runtime.window.frame.get().x(),
                y: runtime.window.frame.get().y(),
            }
        }
        fn create_runtime_window(
            &mut self,
            event_loop: &ActiveEventLoop,
            window: Window,
        ) -> (RuntimeWindow<WinitWindow>, Option<AccessKitAdapter>) {
            let frame = window.frame.get();
            let attributes = NativeWindow::default_attributes()
                .with_title(window.title.get().as_str())
                .with_resizable(window.resizable)
                .with_visible(false)
                .with_decorations(!matches!(
                    window.style,
                    waterui::window::WindowStyle::Borderless
                ))
                .with_inner_size(winit::dpi::LogicalSize::new(
                    frame.width() as f64,
                    frame.height() as f64,
                ));

            let native_window = Arc::new(
                event_loop
                    .create_window(attributes)
                    .expect("hydrolysis runner: failed to create winit window"),
            );
            let mut platform = pollster::block_on(WinitWindow::new(native_window));
            platform.apply_properties(&window);
            let mut renderer = {
                let surface = platform.surface();
                HydrolysisRenderer::new(surface.device())
            };
            super::load_native_resource_fonts(&mut renderer);
            let adapter = if self.accessibility_enabled {
                let adapter = AccessKitAdapter::with_event_loop_proxy(
                    event_loop,
                    platform.native_window(),
                    self.event_proxy.clone(),
                );
                tracing::trace!(
                    target: "waterui::hydrolysis::a11y",
                    window_id = ?platform.id(),
                    title = window.title.get().as_str(),
                    "created accesskit adapter for window"
                );
                Some(adapter)
            } else {
                tracing::warn!(
                    target: "waterui::hydrolysis::a11y",
                    window_id = ?platform.id(),
                    title = window.title.get().as_str(),
                    "accessibility adapter disabled: org.a11y.Bus is unavailable"
                );
                None
            };
            platform.native_window().set_visible(true);
            (
                RuntimeWindow::new(window, platform, renderer, self.render_diagnostics_config),
                adapter,
            )
        }

        fn mount_pending_windows(&mut self, event_loop: &ActiveEventLoop) {
            let mut pending = mem::take(&mut self.pending_windows);
            pending.extend(self.pending_window_queue.borrow_mut().drain(..));
            for window in pending {
                let (runtime, adapter) = self.create_runtime_window(event_loop, window);
                let id = runtime.platform.id();
                self.windows.insert(id, runtime);
                if let Some(adapter) = adapter {
                    self.accesskit_adapters.insert(id, adapter);
                }
            }
        }

        fn handle_input_events(
            runtime: &mut RuntimeWindow<WinitWindow>,
            env: &Environment,
        ) -> bool {
            handle_input_events_with(runtime, env, |runtime, env| {
                env.extending(Self::current_window_origin(runtime))
            })
        }

        fn remove_closed_windows(&mut self, event_loop: &ActiveEventLoop) {
            let mut close_ids = Vec::new();
            for (id, runtime) in &mut self.windows {
                runtime.platform.apply_properties(&runtime.window);
                if runtime.window.state.get() == WindowState::Closed {
                    close_ids.push(*id);
                }
            }

            for id in close_ids {
                self.windows.remove(&id);
                self.accesskit_adapters.remove(&id);
            }

            if self.windows.is_empty() && self.pending_windows.is_empty() {
                event_loop.exit();
            }
        }
    }

    impl ApplicationHandler<RunnerEvent> for WinitRunner {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.drain_local_executor_queue();
            self.mount_pending_windows(event_loop);
            for runtime in self.windows.values() {
                runtime.platform.request_redraw();
            }
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            window_id: WindowId,
            event: WindowEvent,
        ) {
            self.drain_local_executor_queue();
            let should_close = {
                let Some(runtime) = self.windows.get_mut(&window_id) else {
                    return;
                };
                if let Some(adapter) = self.accesskit_adapters.get_mut(&window_id) {
                    adapter.process_event(runtime.platform.native_window(), &event);
                }
                runtime.platform.handle_window_event(&event);
                Self::handle_input_events(runtime, &self.env)
            };

            if !self.pending_window_queue.borrow().is_empty() || !self.pending_windows.is_empty() {
                self.mount_pending_windows(event_loop);
            }

            if should_close {
                self.windows.remove(&window_id);
                self.accesskit_adapters.remove(&window_id);
                if self.windows.is_empty() && self.pending_windows.is_empty() {
                    event_loop.exit();
                }
                return;
            }

            if let WindowEvent::RedrawRequested = event {
                let Some(runtime) = self.windows.get_mut(&window_id) else {
                    return;
                };
                render_window(runtime, &self.env);
                if let Some(adapter) = self.accesskit_adapters.get_mut(&window_id) {
                    if let Some(update) = runtime.renderer.take_accessibility_tree_update() {
                        tracing::trace!(
                            target: "waterui::hydrolysis::a11y",
                            window_id = ?window_id,
                            "publishing accessibility tree update on redraw"
                        );
                        adapter.update_if_active(|| update);
                    } else {
                        tracing::trace!(
                            target: "waterui::hydrolysis::a11y",
                            window_id = ?window_id,
                            "no accessibility tree update available on redraw"
                        );
                    }
                }
            }
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            self.drain_local_executor_queue();
            self.mount_pending_windows(event_loop);
            let now = Instant::now();
            let mut next_gesture_deadline: Option<Instant> = None;
            for runtime in self.windows.values_mut() {
                if let Some(deadline) = advance_runtime(runtime, &self.env, now) {
                    next_gesture_deadline = Some(match next_gesture_deadline {
                        Some(existing) => existing.min(deadline),
                        None => deadline,
                    });
                }
            }
            if let Some(deadline) = next_gesture_deadline {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            self.remove_closed_windows(event_loop);
        }

        fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: RunnerEvent) {
            match event {
                RunnerEvent::PollLocalTasks => self.drain_local_executor_queue(),
                RunnerEvent::MountPendingWindows => {
                    self.mount_pending_windows(_event_loop);
                }
                RunnerEvent::AccessKit(event) => {
                    let Some(runtime) = self.windows.get_mut(&event.window_id) else {
                        return;
                    };
                    let Some(adapter) = self.accesskit_adapters.get_mut(&event.window_id) else {
                        return;
                    };
                    match event.window_event {
                        AccessKitWindowEvent::InitialTreeRequested => {
                            tracing::trace!(
                                target: "waterui::hydrolysis::a11y",
                                window_id = ?event.window_id,
                                "accesskit initial tree requested"
                            );
                            if let Some(update) = runtime.renderer.take_accessibility_tree_update()
                            {
                                tracing::trace!(
                                    target: "waterui::hydrolysis::a11y",
                                    window_id = ?event.window_id,
                                    "publishing accessibility tree update for initial request"
                                );
                                adapter.update_if_active(|| update);
                            } else {
                                tracing::trace!(
                                    target: "waterui::hydrolysis::a11y",
                                    window_id = ?event.window_id,
                                    "missing accessibility tree update for initial request, scheduling rebuild"
                                );
                                runtime.needs_rebuild = true;
                                runtime.platform.request_redraw();
                            }
                        }
                        AccessKitWindowEvent::ActionRequested(request) => {
                            tracing::trace!(
                                target: "waterui::hydrolysis::a11y",
                                window_id = ?event.window_id,
                                action = ?request.action,
                                target = ?request.target_node,
                                "accesskit action requested"
                            );
                            if runtime
                                .renderer
                                .handle_accessibility_action(request, &self.env)
                            {
                                runtime.needs_rebuild = true;
                                runtime.platform.request_redraw();
                            }
                        }
                        AccessKitWindowEvent::AccessibilityDeactivated => {}
                    }
                }
            }
        }
    }
}
