//! Headless `aarch64-apple-ios-sim` preview runner driven by `UIApplicationMain`
//! against an in-process `CAWindowServer` (`SimFramebuffer`) and `FBSWorkspace`
//! scene host.

use std::ffi::{CStr, c_char, c_void};
use std::path::Path;
use std::process;
use std::ptr;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cocoa_ui::bitmap::{ExtendedColorSpace, HdrRaster};
use cocoa_ui::core_animation::flush_transaction;
use cocoa_ui::dynamic_range::{DynamicRange, apply_to_view};
use cocoa_ui::objc2::Message as _;
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2_ui_kit::UIWindow;
use cocoa_ui::uikit::{HostView, ViewController, current_scheme, window_root};
use cocoa_ui::view::{display_immediately, layout_immediately};
use cocoa_ui::{ColorScheme, MainThreadMarker};
use executor_core::spawn_local;
use half::f16;
use waterui::AnyView;
#[cfg(feature = "inspector")]
use waterui::inspector::{InspectorRuntime, install as install_inspector};
use waterui::task::{sleep, yield_now};
use waterui::text::install_system_font_collection;
use waterui_backend_core::Environment;
use waterui_capture::ContentLedger;
use waterui_capture::view_renderer::RenderResult;
use waterui_preview_protocol::run::{
    Alpha, PreviewColorScheme, PreviewRunConfig, PreviewRunMode, RawFrameStreamWriter,
    ScenarioEvent, ScenarioEventKind, VideoCaptureConfig, plan_scenario_timeline, write_hdr_png,
    write_png,
};
use waterui_resources::ResourceContext;

use crate::content::await_content;
use crate::contract::KeepAlive;
use crate::dispatch::install as install_dispatch;
use crate::embedding::mount_content;
use crate::first_paint::{FirstPaint, mark as mark_first_paint};
use crate::fonts::FontRegistrations;
#[cfg(feature = "gpu_surface")]
use crate::gpu_runtime::prepare as prepare_gpu;
use crate::locale::install as install_locale;
use crate::preview::{PreviewError, RunTask, write_scenario_hdr_sample, write_scenario_sdr_sample};
use crate::reduce_motion::install as install_reduce_motion;
use crate::resources::install_application;
use crate::startup::initialize_for_preview;
use crate::theme;

const TOUCH_BEGAN: i32 = 0;
const TOUCH_MOVED: i32 = 1;
const TOUCH_ENDED: i32 = 2;
const TOUCH_CANCELLED: i32 = 3;

const FOURCC_RGHA: u32 = u32::from_be_bytes(*b"RGhA");
const FOURCC_BGRA: u32 = u32::from_be_bytes(*b"BGRA");

type SceneReadyCallback = extern "C" fn(window_ptr: *mut c_void, user_data: *mut c_void);

unsafe extern "C" {
    fn waterui_ios_sim_preview_bootstrap(
        logical_width: f64,
        logical_height: f64,
        scale: f64,
        color_scheme_override: i32,
    ) -> i32;
    fn waterui_ios_sim_preview_enter_app(
        on_scene_ready: SceneReadyCallback,
        user_data: *mut c_void,
    ) -> i32;
    fn waterui_ios_sim_dispatch_touch(phase: i32, x: f32, y: f32);
    fn waterui_ios_sim_dispatch_scroll(x: f32, y: f32, dx: f32, dy: f32);
    fn waterui_ios_sim_frame_count() -> u64;
    fn waterui_ios_sim_has_hosted_layers() -> bool;
    fn waterui_ios_sim_request_present();
    fn waterui_ios_sim_copy_latest_frame(
        out_buf: *mut u8,
        out_capacity: usize,
        out_width: *mut u32,
        out_height: *mut u32,
        out_pixel_format: *mut u32,
        out_bytes_per_element: *mut u32,
        out_seq: *mut u64,
    ) -> bool;
    fn waterui_ios_sim_copy_colorspace_name(out_buf: *mut c_char, out_capacity: usize) -> usize;
}

struct LaunchPayload {
    compose: Box<dyn FnOnce(Environment) -> Environment>,
    view: Box<dyn FnOnce() -> AnyView>,
    resources: ResourceContext,
    color_scheme: Option<ColorScheme>,
    content_deadline: Duration,
    hdr: bool,
    task: RunTask,
    load_started: Instant,
    #[cfg(feature = "inspector")]
    inspector: Option<InspectorRuntime>,
}

/// Runs the iOS Simulator preview inside `UIApplicationMain`.
pub(super) fn drive(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> AnyView + 'static,
    resources: ResourceContext,
    config: PreviewRunConfig,
) -> Result<(), PreviewError> {
    let load_started = Instant::now();
    let _mtm = MainThreadMarker::new().ok_or(PreviewError::NotMainThread)?;

    let logical_width = f64::from(config.width());
    let logical_height = f64::from(config.height());
    let scale = f64::from(config.scale().round().max(1.0));
    let content_deadline = config.content_deadline();
    let hdr = config.hdr();
    let color_scheme = config.color_scheme().map(|scheme| match scheme {
        PreviewColorScheme::Light => ColorScheme::Light,
        PreviewColorScheme::Dark => ColorScheme::Dark,
    });
    let scheme_code = match color_scheme {
        None => 0,
        Some(ColorScheme::Light) => 1,
        Some(ColorScheme::Dark) => 2,
    };

    let task = match config.into_mode() {
        PreviewRunMode::Image { output } => RunTask::Image { output },
        PreviewRunMode::Scenario {
            output_dir,
            captures_ms,
            events,
            video,
        } => RunTask::Scenario {
            output_dir,
            captures_ms,
            events,
            video,
        },
        PreviewRunMode::Semantic { .. } => return Err(PreviewError::UnsupportedMode("semantic")),
    };

    // Bootstrap CAWindowServer + BKS/FBS interposers BEFORE `initialize_for_preview()`
    // so `UIScreen.mainScreen` binds to our SimFramebuffer CADisplay on first access.
    // SAFETY: called once on the main thread before UIKit initializes UIScreen.
    let boot_status = unsafe {
        waterui_ios_sim_preview_bootstrap(logical_width, logical_height, scale, scheme_code)
    };
    if boot_status != 0 {
        return Err(PreviewError::SimulatorBootstrap(format!(
            "iOS Simulator preview bootstrap failed with status {boot_status}"
        )));
    }

    #[cfg(feature = "inspector")]
    let inspector = initialize_for_preview();
    #[cfg(not(feature = "inspector"))]
    initialize_for_preview();

    let payload = Box::new(LaunchPayload {
        compose: Box::new(compose),
        view: Box::new(view),
        resources,
        color_scheme,
        content_deadline,
        hdr,
        task,
        load_started,
        #[cfg(feature = "inspector")]
        inspector,
    });
    let user_data = Box::into_raw(payload).cast::<c_void>();

    // SAFETY: called on the main thread with a valid callback and owned heap payload.
    let status = unsafe { waterui_ios_sim_preview_enter_app(on_scene_ready, user_data) };
    if status != 0 {
        return Err(PreviewError::SimulatorBootstrap(format!(
            "iOS Simulator preview UIApplicationMain failed with status {status}"
        )));
    }
    Err(PreviewError::RunLoopExited)
}

extern "C" fn on_scene_ready(window_ptr: *mut c_void, user_data: *mut c_void) {
    // SAFETY: `user_data` was produced by `Box::into_raw` in `drive` and is invoked once.
    let payload = unsafe { Box::from_raw(user_data.cast::<LaunchPayload>()) };
    // SAFETY: `window_ptr` is a live `UIWindow` passed from `WaterUIPreviewSceneDelegate`.
    let window = unsafe {
        Retained::retain(window_ptr.cast::<UIWindow>())
            .expect("scene delegate must pass a valid UIWindow")
    };
    let mtm = MainThreadMarker::new().expect("scene connection runs on the main thread");

    let mut keepers = KeepAlive::default();
    let mut env = Environment::new();
    #[cfg(feature = "inspector")]
    install_inspector(&mut env, payload.inspector);
    install_system_font_collection(&mut env);
    let mut fonts = FontRegistrations::default();
    env.insert(payload.resources);
    install_application(&mut env, &mut fonts);
    install_dispatch(&mut env);
    env.insert(FirstPaint::default());

    let initial_scheme = payload.color_scheme.unwrap_or_else(current_scheme);
    let installed_theme = Rc::new(theme::install(&mut env, initial_scheme));
    let locale = install_locale(&mut env, mtm);
    keepers.keep(locale);
    keepers.keep(install_reduce_motion(&mut env, mtm));
    keepers.keep(fonts);

    let env_box = Box::new(env);
    let env_ptr = Box::into_raw(env_box);

    let compose = payload.compose;
    let view = payload.view;
    let color_scheme = payload.color_scheme;
    let content_deadline = payload.content_deadline;
    let hdr = payload.hdr;
    let task = payload.task;
    let load_started = payload.load_started;

    let launch = move || {
        // SAFETY: `env_ptr` stays allocated until the spawned task finishes and exits the process.
        let base_env = unsafe { Box::from_raw(env_ptr) };
        let env = compose(*base_env);
        tracing::info!(
            phase = "load",
            elapsed_ms = load_started.elapsed().as_millis(),
            "Apple iOS Simulator preview phase"
        );
        spawn_local(async move {
            let result = execute_sim_task(
                &env,
                mtm,
                window,
                installed_theme,
                view,
                color_scheme,
                content_deadline,
                hdr,
                task,
            )
            .await;
            drop(keepers);
            match result {
                Ok(()) => process::exit(0),
                Err(error) => {
                    tracing::error!("preview run failed: {error}");
                    process::exit(1);
                }
            }
        })
        .detach();
    };

    #[cfg(feature = "gpu_surface")]
    // SAFETY: `env_ptr` remains valid until `launch` reclaims it.
    unsafe {
        prepare_gpu(env_ptr, launch);
    }
    #[cfg(not(feature = "gpu_surface"))]
    launch();
}

#[expect(
    clippy::future_not_send,
    reason = "UIKit views and windows are main-thread bound"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "internal preview coordinator bundling scene launch state"
)]
async fn execute_sim_task(
    env: &Environment,
    mtm: MainThreadMarker,
    window: Retained<UIWindow>,
    installed_theme: Rc<theme::ThemeSignals>,
    view: Box<dyn FnOnce() -> AnyView>,
    color_scheme: Option<ColorScheme>,
    content_deadline: Duration,
    hdr: bool,
    task: RunTask,
) -> Result<(), PreviewError> {
    let ledger = ContentLedger::new();
    let content_env = env.extending(ledger.clone());
    let dynamic_range = if hdr {
        DynamicRange::High
    } else {
        DynamicRange::Standard
    };

    let controller = ViewController::new(mtm, window_root(mtm));
    window.setRootViewController(Some(&**controller));
    let root = controller.host_view().retain();
    apply_to_view(dynamic_range, &root);
    window.makeKeyAndVisible();

    let resolved_scheme = color_scheme.unwrap_or_else(|| controller.color_scheme());
    theme::refresh(&installed_theme, resolved_scheme);
    let _appearance = color_scheme.is_none().then(|| {
        controller.observe_color_scheme({
            let installed_theme = Rc::clone(&installed_theme);
            move |scheme| theme::refresh(&installed_theme, scheme)
        })
    });

    let mut mount_keepalive = KeepAlive::default();
    let _mounted = mount_content(&root, view(), &content_env, &mut mount_keepalive, mtm);
    mark_first_paint(&root, &content_env);
    window.layoutIfNeeded();
    flush_view(&root);

    await_content(&ledger, &root, content_deadline).await?;
    window.layoutIfNeeded();
    flush_view(&root);

    // Wait for the scene's CALayerHost contexts to attach and Core Animation to
    // present a fresh frame containing the settled view tree.
    wait_for_hosted_layers(content_deadline).await?;
    let seq_before = frame_count();
    // SAFETY: called on the main thread after CAWindowServer initialized.
    unsafe { waterui_ios_sim_request_present() };
    wait_for_frames(seq_before.saturating_add(1), content_deadline).await?;

    match task {
        RunTask::Image { output } => {
            let render_started = Instant::now();
            let frame = read_latest_frame()?;
            tracing::info!(
                phase = "render",
                elapsed_ms = render_started.elapsed().as_millis(),
                width = frame.width,
                height = frame.height,
                colorspace = frame.colorspace.as_str(),
                "Apple iOS Simulator preview phase"
            );
            let encode_started = Instant::now();
            write_single_frame(&output, frame, hdr)?;
            tracing::info!(
                phase = "encode",
                elapsed_ms = encode_started.elapsed().as_millis(),
                "Apple iOS Simulator preview phase"
            );
        }
        RunTask::Scenario {
            output_dir,
            captures_ms,
            events,
            video,
        } => {
            run_scenario_loop(
                &ledger,
                &root,
                content_deadline,
                hdr,
                output_dir.as_deref(),
                &captures_ms,
                &events,
                video.as_ref(),
            )
            .await?;
        }
    }

    Ok(())
}

#[expect(
    clippy::future_not_send,
    reason = "UIKit views are main-thread bound across scenario sleeps"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "internal helper executing the planned scenario timeline"
)]
async fn run_scenario_loop(
    ledger: &ContentLedger,
    root: &HostView,
    content_deadline: Duration,
    hdr: bool,
    output_dir: Option<&Path>,
    captures_ms: &[u64],
    events: &[ScenarioEvent],
    video: Option<&VideoCaptureConfig>,
) -> Result<(), PreviewError> {
    let samples = plan_scenario_timeline(captures_ms, video, events)?;
    let mut video_writer = match video {
        Some(video) if hdr => Some(RawFrameStreamWriter::create_hdr(
            video.output(),
            video.fps(),
        )?),
        Some(video) => Some(RawFrameStreamWriter::create(video.output(), video.fps())?),
        None => None,
    };

    let mut event_index = 0usize;
    let started_at = Instant::now();

    for sample in samples {
        while event_index < events.len() {
            let event = events[event_index];
            let event_offset = Duration::from_millis(event.at_ms());
            if event_offset > sample.offset() {
                break;
            }
            if let Some(wait) = event_offset.checked_sub(started_at.elapsed())
                && !wait.is_zero()
            {
                sleep(wait).await;
            }
            dispatch_hid_event(event);
            flush_transaction();
            yield_now().await;
            event_index += 1;
        }

        if let Some(wait) = sample.offset().checked_sub(started_at.elapsed())
            && !wait.is_zero()
        {
            sleep(wait).await;
        }
        await_content(ledger, root, content_deadline).await?;
        let frame = read_latest_frame()?;
        if hdr {
            let raster = frame.into_hdr_raster()?;
            write_scenario_hdr_sample(
                raster,
                sample.png_capture_ms(),
                sample.record_video(),
                sample.offset(),
                output_dir,
                video_writer.as_mut(),
            )?;
        } else {
            let result = frame.into_sdr_render_result()?;
            write_scenario_sdr_sample(
                result,
                sample.png_capture_ms(),
                sample.record_video(),
                sample.offset(),
                output_dir,
                video_writer.as_mut(),
            )?;
        }
    }

    if let Some(writer) = video_writer {
        let _ = writer.finish()?;
    }
    Ok(())
}

fn dispatch_hid_event(event: ScenarioEvent) {
    match event.kind() {
        ScenarioEventKind::PointerDown => {
            // SAFETY: called on the main thread after UIApplicationMain started.
            unsafe { waterui_ios_sim_dispatch_touch(TOUCH_BEGAN, event.x(), event.y()) };
        }
        ScenarioEventKind::PointerMove => {
            // SAFETY: called on the main thread after UIApplicationMain started.
            unsafe { waterui_ios_sim_dispatch_touch(TOUCH_MOVED, event.x(), event.y()) };
        }
        ScenarioEventKind::PointerUp => {
            // SAFETY: called on the main thread after UIApplicationMain started.
            unsafe { waterui_ios_sim_dispatch_touch(TOUCH_ENDED, event.x(), event.y()) };
        }
        ScenarioEventKind::PointerCancel => {
            // SAFETY: called on the main thread after UIApplicationMain started.
            unsafe { waterui_ios_sim_dispatch_touch(TOUCH_CANCELLED, event.x(), event.y()) };
        }
        ScenarioEventKind::Scroll => {
            let scale = if event.is_line_delta() { 20.0 } else { 1.0 };
            let dx = event.dx() * scale;
            let dy = event.dy() * scale;
            // SAFETY: called on the main thread after UIApplicationMain started.
            unsafe { waterui_ios_sim_dispatch_scroll(event.x(), event.y(), dx, dy) };
        }
        _ => {}
    }
}

fn flush_view(root: &HostView) {
    layout_immediately(root);
    display_immediately(root);
    flush_transaction();
}

fn frame_count() -> u64 {
    // SAFETY: thread-safe atomic/locked read in `SimFramebuffer`.
    unsafe { waterui_ios_sim_frame_count() }
}

fn has_hosted_layers() -> bool {
    // SAFETY: called on the main thread where `gHostedContextIDs` is mutated.
    unsafe { waterui_ios_sim_has_hosted_layers() }
}

async fn wait_for_hosted_layers(deadline: Duration) -> Result<(), PreviewError> {
    let start = Instant::now();
    let max_wait = deadline.max(Duration::from_secs(5));
    while !has_hosted_layers() {
        if start.elapsed() >= max_wait {
            return Err(PreviewError::SimulatorFramebuffer(format!(
                "scene layers did not attach to CAWindowServer after {}ms",
                start.elapsed().as_millis()
            )));
        }
        sleep(Duration::from_millis(8)).await;
    }
    Ok(())
}

async fn wait_for_frames(target_seq: u64, deadline: Duration) -> Result<(), PreviewError> {
    let start = Instant::now();
    let max_wait = deadline.max(Duration::from_secs(5));
    while frame_count() < target_seq {
        if start.elapsed() >= max_wait {
            return Err(PreviewError::SimulatorFramebuffer(format!(
                "SimFramebuffer produced {} frame(s) after {}ms (expected >= {target_seq})",
                frame_count(),
                start.elapsed().as_millis()
            )));
        }
        sleep(Duration::from_millis(8)).await;
    }
    Ok(())
}

struct CapturedSimFrame {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    pixel_format: u32,
    bytes_per_element: u32,
    colorspace: String,
}

fn read_latest_frame() -> Result<CapturedSimFrame, PreviewError> {
    let mut width = 0u32;
    let mut height = 0u32;
    let mut pixel_format = 0u32;
    let mut bpe = 0u32;
    let mut seq = 0u64;

    // Query dimensions first.
    // SAFETY: passing null buffer queries the latest frame metadata without copying.
    let ok = unsafe {
        waterui_ios_sim_copy_latest_frame(
            ptr::null_mut(),
            0,
            &raw mut width,
            &raw mut height,
            &raw mut pixel_format,
            &raw mut bpe,
            &raw mut seq,
        )
    };
    if !ok || width == 0 || height == 0 || bpe == 0 {
        return Err(PreviewError::SimulatorFramebuffer(
            "SimFramebuffer has no presented frame available".to_string(),
        ));
    }

    let required = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(bpe as usize))
        .ok_or_else(|| {
            PreviewError::SimulatorFramebuffer("SimFramebuffer dimensions overflow".to_string())
        })?;
    let mut bytes = vec![0u8; required];
    // SAFETY: `bytes` has capacity `required` matching the queried frame size.
    let copied = unsafe {
        waterui_ios_sim_copy_latest_frame(
            bytes.as_mut_ptr(),
            bytes.len(),
            &raw mut width,
            &raw mut height,
            &raw mut pixel_format,
            &raw mut bpe,
            &raw mut seq,
        )
    };
    if !copied {
        return Err(PreviewError::SimulatorFramebuffer(
            "SimFramebuffer frame copy failed".to_string(),
        ));
    }

    let mut cs_buf = [0 as c_char; 128];
    // SAFETY: `cs_buf` is a writable NUL-terminated buffer of 128 bytes.
    let cs_len = unsafe { waterui_ios_sim_copy_colorspace_name(cs_buf.as_mut_ptr(), cs_buf.len()) };
    let colorspace = if cs_len > 0 {
        // SAFETY: `waterui_simfb_copy_colorspace_name` uses `strlcpy` and always NUL-terminates.
        unsafe { CStr::from_ptr(cs_buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    } else {
        String::new()
    };

    Ok(CapturedSimFrame {
        bytes,
        width,
        height,
        pixel_format,
        bytes_per_element: bpe,
        colorspace,
    })
}

fn write_single_frame(
    output: &Path,
    frame: CapturedSimFrame,
    hdr: bool,
) -> Result<(), PreviewError> {
    if hdr {
        let raster = frame.into_hdr_raster()?;
        write_hdr_png(
            output,
            raster.width,
            raster.height,
            raster.pixels,
            Alpha::Premultiplied,
        )?;
    } else {
        let result = frame.into_sdr_render_result()?;
        let (rgba, width, height) = result.into_parts();
        write_png(output, width, height, rgba, Alpha::Premultiplied)?;
    }
    Ok(())
}

impl CapturedSimFrame {
    fn into_hdr_raster(self) -> Result<HdrRaster, PreviewError> {
        if self.pixel_format == FOURCC_RGHA && self.bytes_per_element == 8 {
            let mut pixels = Vec::with_capacity((self.width as usize) * (self.height as usize));
            for chunk in self.bytes.as_chunks::<8>().0 {
                let r = f16::from_le_bytes([chunk[0], chunk[1]]).to_f32();
                let g = f16::from_le_bytes([chunk[2], chunk[3]]).to_f32();
                let b = f16::from_le_bytes([chunk[4], chunk[5]]).to_f32();
                let a = f16::from_le_bytes([chunk[6], chunk[7]]).to_f32();
                pixels.push([r, g, b, a]);
            }
            let space = iosurface_color_space(&self.colorspace);
            return Ok(HdrRaster::from_extended_rgba(
                pixels,
                self.width,
                self.height,
                space,
            ));
        }
        if self.pixel_format == FOURCC_BGRA && self.bytes_per_element == 4 {
            let mut pixels = Vec::with_capacity((self.width as usize) * (self.height as usize));
            for chunk in self.bytes.as_chunks::<4>().0 {
                pixels.push([
                    f32::from(chunk[2]) / 255.0,
                    f32::from(chunk[1]) / 255.0,
                    f32::from(chunk[0]) / 255.0,
                    f32::from(chunk[3]) / 255.0,
                ]);
            }
            return Ok(HdrRaster::from_extended_rgba(
                pixels,
                self.width,
                self.height,
                ExtendedColorSpace::ExtendedDisplayP3,
            ));
        }
        Err(PreviewError::SimulatorFramebuffer(format!(
            "unsupported SimFramebuffer pixel format 0x{:08x} (bpe={})",
            self.pixel_format, self.bytes_per_element
        )))
    }

    fn into_sdr_render_result(self) -> Result<RenderResult, PreviewError> {
        if self.pixel_format == FOURCC_BGRA && self.bytes_per_element == 4 {
            let mut rgba = Vec::with_capacity(self.bytes.len());
            for chunk in self.bytes.as_chunks::<4>().0 {
                rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
            }
            return Ok(RenderResult::new(rgba, self.width, self.height));
        }
        let raster = self.into_hdr_raster()?;
        let rgba = raster.to_display_p3_rgba8();
        Ok(RenderResult::new(rgba, raster.width, raster.height))
    }
}

fn iosurface_color_space(name: &str) -> ExtendedColorSpace {
    if name.contains("LinearDisplayP3") {
        ExtendedColorSpace::ExtendedLinearDisplayP3
    } else if name.contains("LinearSRGB") {
        ExtendedColorSpace::ExtendedLinearSrgb
    } else if name.contains("DisplayP3") {
        ExtendedColorSpace::ExtendedDisplayP3
    } else {
        // Core Animation's simulator `RGhA` swapchain defaults to extended sRGB.
        ExtendedColorSpace::ExtendedSrgb
    }
}
