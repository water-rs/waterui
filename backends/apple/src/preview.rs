//! The in-process preview entry `water preview` drives — the `AppKit`
//! realization on macOS, the `UIKit` realization through Mac Catalyst.
//!
//! The generated preview binary calls [`run`] from `main` and returns its
//! `Result`. The process brings the backend up the way `crate::entry`
//! does — startup, the GPU runtime, native services — under the main run
//! loop rather than the platform's application run loop, mounts the view
//! on a root `HostView` inside a capture window under a fresh
//! [`ContentLedger`](waterui_capture::ContentLedger), waits for the content
//! its views owe — bounded by the run's content deadline — and for GPU
//! surfaces to present, captures the whole subtree and writes the PNG. The
//! process answers only through `main`'s return — no Dock icon, no focus,
//! no on-screen window, ever.
//!
//! - **`AppKit`:** the activation policy is `Prohibited` before `AppKit`
//!   finishes launching, and the capture window is never ordered in.
//! - **Mac Catalyst:** the binary is exec'd directly — no bundle, no
//!   `UIApplicationMain`, no scene. The `AppKit` application `UIKit`'s Mac
//!   support requires is brought up at the `Prohibited` policy
//!   ([`cocoa_ui::uikit::catalyst::BackgroundApplication`]), and the view
//!   mounts in a scene-less `UIWindow`, which `UIKit` never backs with an
//!   `NSWindow`. The window is what gives native controls the trait
//!   environment and layout pass they build their parts in — a detached
//!   hierarchy draws switches without knobs, sliders without thumbs and no
//!   steppers at all. The binary embeds [`preview_info_plist!`](crate::preview_info_plist)
//!   so `UIKit` runs under the iPad idiom (`UIDeviceFamily` 2): previews
//!   show the iOS look.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{ActivationPolicy, Application};
pub use cocoa_ui::bitmap::HdrRaster;
use cocoa_ui::bitmap::set_window_color_scheme;
#[cfg(feature = "gpu_surface")]
use cocoa_ui::capture::collect_capturables;
use cocoa_ui::core_animation::flush_transaction;
use cocoa_ui::dynamic_range::{DynamicRange, apply_to_view};
#[cfg(target_os = "macos")]
use cocoa_ui::objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSView};
use cocoa_ui::objc2_core_foundation::{CFRunLoop, CGPoint};
#[cfg(catalyst)]
use cocoa_ui::objc2_ui_kit::UIControl;
#[cfg(catalyst)]
use cocoa_ui::uikit::catalyst::{BackgroundApplication, runs_under_pad_idiom};
#[cfg(catalyst)]
use cocoa_ui::uikit::current_scheme;
#[cfg(catalyst)]
use cocoa_ui::view::retain_base;
use cocoa_ui::view::{display_immediately, layout_immediately};
use cocoa_ui::{ColorScheme, MainThreadMarker, Retained, Size};
use waterui::AnyView;
use waterui::task::{sleep, yield_now};
use waterui_backend_core::Environment;
use waterui_capture::ContentLedger;
use waterui_capture::view_renderer::RenderResult;
#[cfg(target_os = "macos")]
use waterui_preview_protocol::run::ScenarioPointerButton;
use waterui_preview_protocol::run::{
    Alpha, PreviewColorScheme, PreviewRunConfig, PreviewRunMode, RawFrameError,
    RawFrameStreamWriter, ScenarioEvent, ScenarioEventKind, VideoCaptureConfig,
    plan_scenario_timeline, write_hdr_png, write_png,
};
use waterui_resources::ResourceContext;

use crate::content::await_content;
use crate::{capture, theme};

/// The capture failure a preview run reports — `ViewCapture`'s with
/// `gpu_surface`, the bitmap readback's without it.
#[cfg(feature = "gpu_surface")]
type CaptureFailure = crate::capture_image::CaptureFailure;
/// See the `gpu_surface` alias.
#[cfg(not(feature = "gpu_surface"))]
type CaptureFailure = crate::capture::CaptureError;

/// The Mac Catalyst preview binary's `Info.plist`: a bundle identity,
/// `UIDeviceFamily` 2 — the iPad idiom under which `UIKit` draws the iOS
/// look — and `LSUIElement`. A UI-element process arrives at the
/// `Prohibited` policy, and `UIKit`'s Mac backgrounding controller leaves
/// its windows' scenes alone: in a process without it, the controller
/// raises on the scene-less capture window's unassociated scene, which
/// has no identity to track.
#[cfg(catalyst)]
#[doc(hidden)]
pub const INFO_PLIST: [u8; include_bytes!("preview_info.plist").len()] =
    *include_bytes!("preview_info.plist");

/// Embeds the preview binary's `Info.plist` in its `__TEXT,__info_plist`
/// section on Mac Catalyst, and expands to nothing elsewhere.
///
/// A Catalyst process exec'd without a bundle reads its main bundle's
/// `Info.plist` from that section, and `UIKit` picks the interface idiom
/// from its `UIDeviceFamily` — so the generated preview binary invokes
/// this once at its root. [`run`] fails fast when the process does not
/// run under the iPad idiom.
#[macro_export]
macro_rules! preview_info_plist {
    () => {
        #[cfg(all(target_os = "ios", target_abi = "macabi"))]
        #[used]
        #[unsafe(link_section = "__TEXT,__info_plist")]
        static WATERUI_PREVIEW_INFO_PLIST: [u8; $crate::preview::INFO_PLIST.len()] =
            $crate::preview::INFO_PLIST;
    };
}

/// Why a preview run failed — the generated `main` propagates it for the
/// process to report and exit non-zero.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PreviewError {
    /// The run mode is not implemented by the Apple preview.
    #[error("apple preview does not support {0} runs")]
    UnsupportedMode(&'static str),
    /// Mounting, presenting or rasterizing the view failed.
    #[error(transparent)]
    Capture(#[from] CaptureFailure),
    /// A view still owed content when the run's content deadline passed, or
    /// its content failed to load.
    #[error(transparent)]
    Content(#[from] waterui_capture::content_ledger::ContentError),
    /// Encoding or writing the capture PNG failed.
    #[error(transparent)]
    Write(#[from] waterui_preview_protocol::run::PngError),
    /// Recording raw video frames or planning the scenario timeline failed.
    #[error(transparent)]
    RawFrame(#[from] RawFrameError),
    /// The run loop returned without the capture handing its result back.
    #[error("the main run loop returned before the capture settled")]
    RunLoopExited,
    /// The preview was started off the main thread, where no platform view
    /// can exist.
    #[error("the preview must run on the main thread")]
    NotMainThread,
    /// `AppKit` refused the `Prohibited` activation policy the preview must
    /// hold before it finishes launching, so a capture could show a window,
    /// take focus or put an icon in the Dock.
    #[cfg(target_os = "macos")]
    #[error(
        "the preview process must hold the Prohibited activation policy before AppKit finishes launching"
    )]
    ActivationPolicyRefused,
    /// The Catalyst preview binary does not run under the iPad idiom.
    #[cfg(catalyst)]
    #[error(
        "the Catalyst preview binary must embed `waterui_apple::preview_info_plist!()`: UIKit draws the iOS look only under the iPad idiom (UIDeviceFamily 2)"
    )]
    MissingPadIdiom,
}

/// Runs one preview capture and answers its result.
///
/// `compose` is the composition root the generated code builds from
/// `<crate>::app(waterui::configure_environment!(env))`: the backend's
/// environment in, the environment content mounts under out. `view` is the
/// `#[preview]` constructor. `resources` carries the explicit asset and
/// font directories — a process that never registered a bundle has no
/// `application()` fallback. `config` is the run configuration
/// `WATERUI_PREVIEW_RUN_CONFIG` named.
///
/// The process's stderr contract: once bring-up runs, every failure —
/// the `Err` the run answers and every panic inside it — reaches stderr
/// through `tracing`: the entry installs a stderr layer at error level
/// for the preview process regardless of `WATERUI_LOG`, and the returned
/// `Err` is logged at error level. An error returned before bring-up —
/// an unsupported mode — reaches stderr through `main`'s `Result`
/// propagation instead.
///
/// # Errors
///
/// [`PreviewError::UnsupportedMode`] for `Semantic` runs — the Apple preview
/// captures single images and scenario timelines. [`PreviewError::Content`]
/// when a view still owes content at the run's content deadline or its load
/// fails. [`PreviewError::Capture`] when the view cannot be mounted or
/// rasterized. [`PreviewError::Write`] when a PNG cannot be encoded or
/// written. [`PreviewError::RawFrame`] when a scenario timeline or raw video
/// stream cannot be written. [`PreviewError::RunLoopExited`] when the main run
/// loop returns without the capture's result — a return is never success
/// on its own. [`PreviewError::NotMainThread`] off the main thread.
/// [`PreviewError::ActivationPolicyRefused`] when `AppKit` refuses the
/// `Prohibited` activation policy before launch — the constraint the entry
/// exists to keep, so a refusal ends the run rather than falling back. On
/// Mac Catalyst, [`PreviewError::MissingPadIdiom`] when the process does
/// not run under the iPad idiom — the binary did not embed
/// [`preview_info_plist!`](crate::preview_info_plist).
///
/// Once per process: process bring-up happens inside the call and a
/// second `run` in one process is a bug — the bring-up fails loudly, the
/// way a repeated `initialize` does.
pub fn run(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> waterui::AnyView + 'static,
    resources: ResourceContext,
    config: PreviewRunConfig,
) -> Result<(), PreviewError> {
    let result = drive(compose, view, resources, config);
    if let Err(error) = &result {
        tracing::error!("preview run failed: {error}");
    }
    result
}

/// Runs one headless HDR preview capture and returns its unclamped extended
/// linear Display P3 `f32` RGBA raster (`HdrRaster`).
///
/// Follows the same once-per-process windowless bring-up contract as [`run`],
/// mounting the view with [`DynamicRange::High`] and reading back through the
/// `RGBA16F` (`kCVPixelFormatType_64RGBAHalf` / `MTLPixelFormat::RGBA16Float`)
/// `ExtendedLinearDisplayP3` pipeline so HDR (`> 1.0`) highlights from Liquid
/// Glass and extended-range layers are preserved without clipping.
///
/// # Errors
///
/// Same failure modes as [`run`].
pub fn capture_hdr(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> AnyView + 'static,
    resources: ResourceContext,
    width: u32,
    height: u32,
    color_scheme: Option<PreviewColorScheme>,
    content_deadline: Duration,
) -> Result<HdrRaster, PreviewError> {
    let result = drive_hdr(
        compose,
        view,
        resources,
        width,
        height,
        color_scheme,
        content_deadline,
    );
    if let Err(error) = &result {
        tracing::error!("preview HDR capture failed: {error}");
    }
    result
}

fn drive_hdr(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> AnyView + 'static,
    resources: ResourceContext,
    width: u32,
    height: u32,
    color_scheme: Option<PreviewColorScheme>,
    content_deadline: Duration,
) -> Result<HdrRaster, PreviewError> {
    use crate::contract::KeepAlive;
    #[cfg(feature = "gpu_surface")]
    use crate::gpu_runtime::prepare as prepare_gpu;
    use crate::startup::initialize_for_preview;

    let mtm = MainThreadMarker::new().ok_or(PreviewError::NotMainThread)?;
    #[cfg(feature = "inspector")]
    let inspector = initialize_for_preview();
    #[cfg(not(feature = "inspector"))]
    initialize_for_preview();

    let color_scheme = color_scheme.map(|scheme| match scheme {
        PreviewColorScheme::Light => ColorScheme::Light,
        PreviewColorScheme::Dark => ColorScheme::Dark,
    });
    let size = Size::new(f64::from(width), f64::from(height));

    let mut keepers = KeepAlive::default();
    let (mut env, host) = prepare(
        mtm,
        resources,
        color_scheme,
        &mut keepers,
        #[cfg(feature = "inspector")]
        inspector,
    )?;

    let env_ptr = &raw mut env;
    let outcome = Rc::new(RefCell::new(None::<Result<HdrRaster, PreviewError>>));
    let launch = {
        let outcome = Rc::clone(&outcome);
        move || {
            // SAFETY: `env_ptr` stays live until the capture completes and
            // stops `CFRunLoop::run` below.
            let env = unsafe { &mut *env_ptr };
            let env = compose(env.clone());
            executor_core::spawn_local(async move {
                *outcome.borrow_mut() = Some(
                    render_hdr(&env, mtm, &host, view, size, color_scheme, content_deadline).await,
                );
                CFRunLoop::current()
                    .expect("the main executor drives this task from the main thread's run loop")
                    .stop();
            })
            .detach();
        }
    };
    #[cfg(feature = "gpu_surface")]
    // SAFETY: `env` stays live until the completion stops `CFRunLoop::run`.
    unsafe {
        prepare_gpu(env_ptr, launch);
    }
    #[cfg(not(feature = "gpu_surface"))]
    launch();
    CFRunLoop::run();
    let result = outcome
        .borrow_mut()
        .take()
        .ok_or(PreviewError::RunLoopExited)?;
    drop(env);
    drop(keepers);
    result
}

#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained native objects it holds across awaits are not Send"
)]
async fn render_hdr(
    env: &Environment,
    mtm: MainThreadMarker,
    host: &Host,
    view: impl FnOnce() -> AnyView,
    size: Size,
    color_scheme: Option<ColorScheme>,
    content_deadline: Duration,
) -> Result<HdrRaster, PreviewError> {
    let ledger = ContentLedger::new();
    let content_env = env.extending(ledger.clone());
    let stage = Stage::mount(
        &content_env,
        mtm,
        host,
        view,
        size,
        color_scheme,
        DynamicRange::High,
    );
    await_content(&ledger, &stage.root, content_deadline).await?;
    let result = stage.capture_hdr().await?;
    drop(stage);
    Ok(result)
}

/// What a preview run executes once bring-up succeeds.
enum RunTask {
    Image {
        output: PathBuf,
    },
    Scenario {
        output_dir: Option<PathBuf>,
        captures_ms: Vec<u64>,
        events: Vec<ScenarioEvent>,
        video: Option<VideoCaptureConfig>,
    },
}

/// `run`'s body — the single `Result` boundary [`run`] reports through.
fn drive(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> AnyView + 'static,
    resources: ResourceContext,
    config: PreviewRunConfig,
) -> Result<(), PreviewError> {
    let load_started = Instant::now();
    let mtm = MainThreadMarker::new().ok_or(PreviewError::NotMainThread)?;
    // Startup before anything else can panic or error — the panic hook and
    // the unconditional stderr layer are what the contract guarantees
    // everything else through.
    #[cfg(feature = "inspector")]
    let inspector = crate::startup::initialize_for_preview();
    #[cfg(not(feature = "inspector"))]
    crate::startup::initialize_for_preview();

    let content_deadline = config.content_deadline();
    let hdr = config.hdr();
    let color_scheme = config.color_scheme().map(|scheme| match scheme {
        PreviewColorScheme::Light => ColorScheme::Light,
        PreviewColorScheme::Dark => ColorScheme::Dark,
    });
    let size = Size::new(f64::from(config.width()), f64::from(config.height()));
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

    // The keepers live on `drive`'s frame and are declared before `env`, so
    // `env` drops first and `keepers` — the theme, appearance observer,
    // locale and font registrations `prepare` requires to outlive `env` —
    // drops last.
    let mut keepers = crate::contract::KeepAlive::default();
    let (mut env, host) = prepare(
        mtm,
        resources,
        color_scheme,
        &mut keepers,
        #[cfg(feature = "inspector")]
        inspector,
    )?;

    let env_ptr = &raw mut env;
    let outcome = Rc::new(RefCell::new(None::<Result<(), PreviewError>>));
    let launch = {
        let outcome = Rc::clone(&outcome);
        move || {
            // `drive` lent `env` for its frame; this closure is the
            // only consumer and runs once, on the main thread.
            // SAFETY: `env_ptr` stays live until the capture completes.
            let env = unsafe { &mut *env_ptr };
            let env = compose(env.clone());
            tracing::info!(
                phase = "load",
                elapsed_ms = load_started.elapsed().as_millis(),
                "Apple preview phase"
            );
            executor_core::spawn_local(async move {
                *outcome.borrow_mut() = Some(
                    execute_run_task(
                        &env,
                        mtm,
                        &host,
                        view,
                        size,
                        color_scheme,
                        content_deadline,
                        hdr,
                        task,
                    )
                    .await,
                );
                CFRunLoop::current()
                    .expect("the main executor drives this task from the main thread's run loop")
                    .stop();
            })
            .detach();
        }
    };
    #[cfg(feature = "gpu_surface")]
    // SAFETY: `env` is lent to `drive`'s frame — the completion below
    // renders, deposits the run's result in `outcome` and stops the run
    // loop `drive` blocks in, so the borrow ends before the frame returns;
    // nothing else drains the main queue after a `RunLoopExited` return.
    unsafe {
        crate::gpu_runtime::prepare(env_ptr, launch);
    }
    #[cfg(not(feature = "gpu_surface"))]
    launch();
    // `prepare`'s completion and `render` both land on the main executor,
    // which this run loop drives; the task's `stop` is the only way out,
    // so a bare return can only come from the loop itself.
    CFRunLoop::run();
    outcome
        .borrow_mut()
        .take()
        .ok_or(PreviewError::RunLoopExited)?
}

/// What a capture needs from bring-up beyond the environment.
pub(crate) struct Host {
    /// The `AppKit` application a scene-less `UIWindow` requires.
    #[cfg(catalyst)]
    application: cocoa_ui::uikit::catalyst::BackgroundApplication,
    /// The theme the capture window's own appearance refreshes: `UIKit`
    /// answers the scheme per view controller, so the theme installed at
    /// bring-up follows the window the view mounts in.
    #[cfg(catalyst)]
    theme: Rc<crate::theme::ThemeSignals>,
}

/// The backend bring-up `entry::run` performs, minus startup, menus, window
/// realization and the termination machine.
///
/// Process startup is not idempotent — tracing, the executors and the
/// panic hook install once — so the caller passes the inspector
/// [`crate::startup::initialize_for_preview`] answered. Holds the process
/// at the `Prohibited` activation policy before the platform finishes
/// launching: a capture never shows a window, takes focus or puts an icon
/// in the Dock. The environment's keepers — theme, appearance observation,
/// locale, font registrations — land on `keepers`, which must outlive
/// `env`.
///
/// # Errors
///
/// [`PreviewError::ActivationPolicyRefused`] when `AppKit` refuses the
/// `Prohibited` activation policy, and on Mac Catalyst
/// [`PreviewError::MissingPadIdiom`] when the process does not run under
/// the iPad idiom.
pub(crate) fn prepare(
    mtm: MainThreadMarker,
    resources: ResourceContext,
    color_scheme: Option<ColorScheme>,
    keepers: &mut crate::contract::KeepAlive,
    #[cfg(feature = "inspector")] inspector: Option<waterui::inspector::InspectorRuntime>,
) -> Result<(Environment, Host), PreviewError> {
    let mut env = Environment::new();
    #[cfg(feature = "inspector")]
    waterui::inspector::install(&mut env, inspector);
    waterui::text::install_system_font_collection(&mut env);
    let mut fonts = crate::fonts::FontRegistrations::default();
    env.insert(resources);
    crate::resources::install_application(&mut env, &mut fonts);
    crate::dispatch::install(&mut env);
    env.insert(crate::first_paint::FirstPaint::default());

    #[cfg(target_os = "macos")]
    let host = {
        let application = Application::shared(mtm);
        // `AppKit` must be told before launch completes; a bundle that
        // already declares `UIElement`, or an unbundled process, arrives
        // `Prohibited` and may refuse a redundant change — so the set runs
        // only when the policy is not already there, and a refusal ends
        // the run at the call that refused it.
        if application.activation_policy() != ActivationPolicy::Prohibited
            && !application.set_activation_policy(ActivationPolicy::Prohibited)
        {
            return Err(PreviewError::ActivationPolicyRefused);
        }
        let initial = color_scheme.unwrap_or_else(|| application.color_scheme());
        let installed_theme = Rc::new(theme::install(&mut env, initial));
        if color_scheme.is_none() {
            let appearance = application.observe_color_scheme({
                let installed_theme = Rc::clone(&installed_theme);
                move |scheme| theme::refresh(&installed_theme, scheme)
            });
            keepers.keep(appearance);
        }
        keepers.keep(installed_theme);
        Host {}
    };
    #[cfg(catalyst)]
    let host = {
        let application = BackgroundApplication::bring_up(mtm);
        if !runs_under_pad_idiom(mtm) {
            return Err(PreviewError::MissingPadIdiom);
        }
        // No controller exists yet when `color_scheme` is `None`; the capture
        // window refreshes the theme from its own traits once the view mounts.
        let initial = color_scheme.unwrap_or_else(current_scheme);
        let installed_theme = Rc::new(theme::install(&mut env, initial));
        Host {
            application,
            theme: installed_theme,
        }
    };
    let locale = crate::locale::install(&mut env, mtm);
    keepers.keep(locale);
    keepers.keep(fonts);
    Ok((env, host))
}

/// What one capture produces: the viewport, how long it waits for owed
/// content, whether to capture HDR, and where the PNG goes.
struct Capture<'a> {
    /// The capture window's size.
    size: Size,
    /// Optional color scheme override for the capture window.
    color_scheme: Option<ColorScheme>,
    /// How long the capture waits for content its views owe.
    content_deadline: Duration,
    /// Whether to capture unclamped linear Display P3 HDR and encode 16-bit PQ PNG.
    hdr: bool,
    /// The PNG destination.
    output: &'a Path,
}

/// Renders `view` under `env` inside a capture window, awaits the content
/// its views owe, captures the presented subtree and writes its PNG — the
/// one tail [`run`] runs and every capture the entry performs ends in.
///
/// The view mounts under a fresh [`ContentLedger`](waterui_capture::ContentLedger):
/// the capture rasterizes only once a full drain leaves nothing owed.
/// Apple's bitmap context reads premultiplied RGBA8 (SDR) or premultiplied
/// extended linear Display P3 `f32` RGBA (HDR) — the convention the writer
/// declares for the capture.
///
/// # Errors
///
/// Returns [`PreviewError::Content`] when content is still owed at the
/// deadline or fails to load, [`PreviewError::Capture`] when the platform
/// cannot mount, present or rasterize the view, and
/// [`PreviewError::Write`] when the PNG cannot be encoded or written.
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
)]
async fn render_and_write(
    env: &Environment,
    mtm: MainThreadMarker,
    host: &Host,
    view: impl FnOnce() -> AnyView,
    capture: Capture<'_>,
) -> Result<(), PreviewError> {
    let render_started = Instant::now();
    let ledger = ContentLedger::new();
    let content_env = env.extending(ledger.clone());
    let dynamic_range = if capture.hdr {
        DynamicRange::High
    } else {
        DynamicRange::Standard
    };
    let stage = Stage::mount(
        &content_env,
        mtm,
        host,
        view,
        capture.size,
        capture.color_scheme,
        dynamic_range,
    );
    await_content(&ledger, &stage.root, capture.content_deadline).await?;
    if capture.hdr {
        let raster = stage.capture_hdr().await?;
        drop(stage);
        tracing::info!(
            phase = "render",
            elapsed_ms = render_started.elapsed().as_millis(),
            "Apple preview phase"
        );
        let encode_started = Instant::now();
        write_hdr_png(
            capture.output,
            raster.width,
            raster.height,
            raster.pixels,
            Alpha::Premultiplied,
        )?;
        tracing::info!(
            phase = "encode",
            elapsed_ms = encode_started.elapsed().as_millis(),
            "Apple preview phase"
        );
    } else {
        let result = stage.capture().await?;
        drop(stage);
        tracing::info!(
            phase = "render",
            elapsed_ms = render_started.elapsed().as_millis(),
            "Apple preview phase"
        );
        let encode_started = Instant::now();
        let (rgba_data, width, height) = result.into_parts();
        write_png(
            capture.output,
            width,
            height,
            rgba_data,
            Alpha::Premultiplied,
        )?;
        tracing::info!(
            phase = "encode",
            elapsed_ms = encode_started.elapsed().as_millis(),
            "Apple preview phase"
        );
    }
    Ok(())
}

/// Inputs for a scenario timeline capture on Apple (`AppKit` or Mac Catalyst).
struct ScenarioRun<'a> {
    size: Size,
    color_scheme: Option<ColorScheme>,
    content_deadline: Duration,
    hdr: bool,
    output_dir: Option<&'a Path>,
    captures_ms: &'a [u64],
    events: &'a [ScenarioEvent],
    video: Option<&'a VideoCaptureConfig>,
}

/// Dispatches a parsed preview run task to single-frame or scenario rendering.
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained native objects it holds across awaits are not Send"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "internal helper bundling drive's locals before dispatch"
)]
async fn execute_run_task(
    env: &Environment,
    mtm: MainThreadMarker,
    host: &Host,
    view: impl FnOnce() -> AnyView,
    size: Size,
    color_scheme: Option<ColorScheme>,
    content_deadline: Duration,
    hdr: bool,
    task: RunTask,
) -> Result<(), PreviewError> {
    match task {
        RunTask::Image { output } => {
            render_and_write(
                env,
                mtm,
                host,
                view,
                Capture {
                    size,
                    color_scheme,
                    content_deadline,
                    hdr,
                    output: &output,
                },
            )
            .await
        }
        RunTask::Scenario {
            output_dir,
            captures_ms,
            events,
            video,
        } => {
            render_scenario(
                env,
                mtm,
                host,
                view,
                ScenarioRun {
                    size,
                    color_scheme,
                    content_deadline,
                    hdr,
                    output_dir: output_dir.as_deref(),
                    captures_ms: &captures_ms,
                    events: &events,
                    video: video.as_ref(),
                },
            )
            .await
        }
    }
}

/// Runs a timeline capture: replays `scenario.events` at their timestamps and
/// captures PNGs and/or fixed-FPS raw frames (SDR RGBA8 or HDR RGBA16F linear
/// Display P3) through the same windowless `Stage` capture path as single-image
/// previews.
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained native objects it holds across awaits are not Send"
)]
async fn render_scenario(
    env: &Environment,
    mtm: MainThreadMarker,
    host: &Host,
    view: impl FnOnce() -> AnyView,
    scenario: ScenarioRun<'_>,
) -> Result<(), PreviewError> {
    let samples = plan_scenario_timeline(scenario.captures_ms, scenario.video, scenario.events)?;
    let mut video_writer = match scenario.video {
        Some(video) if scenario.hdr => Some(RawFrameStreamWriter::create_hdr(
            video.output(),
            video.fps(),
        )?),
        Some(video) => Some(RawFrameStreamWriter::create(video.output(), video.fps())?),
        None => None,
    };

    let ledger = ContentLedger::new();
    let content_env = env.extending(ledger.clone());
    let dynamic_range = if scenario.hdr {
        DynamicRange::High
    } else {
        DynamicRange::Standard
    };
    let stage = Stage::mount(
        &content_env,
        mtm,
        host,
        view,
        scenario.size,
        scenario.color_scheme,
        dynamic_range,
    );
    await_content(&ledger, &stage.root, scenario.content_deadline).await?;

    let mut input_state = ScenarioInputState::default();
    let mut event_index = 0usize;
    let started_at = Instant::now();

    for sample in samples {
        while event_index < scenario.events.len() {
            let event = scenario.events[event_index];
            let event_offset = Duration::from_millis(event.at_ms());
            if event_offset > sample.offset() {
                break;
            }
            if let Some(wait) = event_offset.checked_sub(started_at.elapsed())
                && !wait.is_zero()
            {
                sleep(wait).await;
            }
            dispatch_scenario_event(&stage, &mut input_state, event);
            flush_stage(&stage);
            yield_now().await;
            event_index += 1;
        }

        if let Some(wait) = sample.offset().checked_sub(started_at.elapsed())
            && !wait.is_zero()
        {
            sleep(wait).await;
        }
        await_content(&ledger, &stage.root, scenario.content_deadline).await?;
        if scenario.hdr {
            let raster = stage.capture_hdr().await?;
            write_scenario_hdr_sample(
                raster,
                sample.png_capture_ms(),
                sample.record_video(),
                sample.offset(),
                scenario.output_dir,
                video_writer.as_mut(),
            )?;
        } else {
            let result = stage.capture().await?;
            write_scenario_sdr_sample(
                result,
                sample.png_capture_ms(),
                sample.record_video(),
                sample.offset(),
                scenario.output_dir,
                video_writer.as_mut(),
            )?;
        }
    }

    drop(stage);
    if let Some(writer) = video_writer {
        let _ = writer.finish()?;
    }
    Ok(())
}

fn write_scenario_sdr_sample(
    result: RenderResult,
    png_capture_ms: Option<u64>,
    record_video: bool,
    offset: Duration,
    output_dir: Option<&Path>,
    video_writer: Option<&mut RawFrameStreamWriter>,
) -> Result<(), PreviewError> {
    let (rgba_data, width, height) = result.into_parts();
    match (png_capture_ms, record_video) {
        (Some(capture_ms), true) => {
            if let Some(output_dir) = output_dir {
                write_png(
                    &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
                    width,
                    height,
                    rgba_data.clone(),
                    Alpha::Premultiplied,
                )?;
            }
            if let Some(writer) = video_writer {
                writer.write_frame(width, height, rgba_data, Alpha::Premultiplied, offset)?;
            }
        }
        (Some(capture_ms), false) => {
            if let Some(output_dir) = output_dir {
                write_png(
                    &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
                    width,
                    height,
                    rgba_data,
                    Alpha::Premultiplied,
                )?;
            }
        }
        (None, true) => {
            if let Some(writer) = video_writer {
                writer.write_frame(width, height, rgba_data, Alpha::Premultiplied, offset)?;
            }
        }
        (None, false) => {}
    }
    Ok(())
}

fn write_scenario_hdr_sample(
    raster: HdrRaster,
    png_capture_ms: Option<u64>,
    record_video: bool,
    offset: Duration,
    output_dir: Option<&Path>,
    video_writer: Option<&mut RawFrameStreamWriter>,
) -> Result<(), PreviewError> {
    // Write video first from a slice borrow so the PNG writer can take
    // ownership of `raster.pixels` without cloning the frame buffer.
    if record_video && let Some(writer) = video_writer {
        writer.write_hdr_frame_f32(
            raster.width,
            raster.height,
            &raster.pixels,
            Alpha::Premultiplied,
            offset,
        )?;
    }
    if let (Some(capture_ms), Some(output_dir)) = (png_capture_ms, output_dir) {
        write_hdr_png(
            &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
            raster.width,
            raster.height,
            raster.pixels,
            Alpha::Premultiplied,
        )?;
    }
    Ok(())
}

fn flush_stage(stage: &Stage) {
    layout_immediately(&stage.root);
    display_immediately(&stage.root);
    flush_transaction();
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct ScenarioInputState {
    pressed_button: Option<ScenarioPointerButton>,
    pressed_view: Option<Retained<NSView>>,
}

#[cfg(catalyst)]
#[derive(Default)]
struct ScenarioInputState {
    pressed_control: Option<Retained<UIControl>>,
}

#[cfg(target_os = "macos")]
fn make_mouse_event(
    event_type: NSEventType,
    window_point: CGPoint,
    timestamp: f64,
    window_number: isize,
    click_count: isize,
) -> Option<Retained<NSEvent>> {
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        event_type,
        window_point,
        NSEventModifierFlags::empty(),
        timestamp,
        window_number,
        None,
        0,
        click_count,
        if click_count > 0 { 1.0 } else { 0.0 },
    )
}

#[cfg(target_os = "macos")]
fn dispatch_mouse_button_event(
    target: &NSView,
    button: ScenarioPointerButton,
    phase: MouseButtonPhase,
    window_point: CGPoint,
    timestamp: f64,
    window_number: isize,
) {
    let event_type = match (phase, button) {
        (MouseButtonPhase::Down, ScenarioPointerButton::Secondary) => NSEventType::RightMouseDown,
        (MouseButtonPhase::Down, ScenarioPointerButton::Middle) => NSEventType::OtherMouseDown,
        (MouseButtonPhase::Down, _) => NSEventType::LeftMouseDown,
        (MouseButtonPhase::Dragged, ScenarioPointerButton::Secondary) => {
            NSEventType::RightMouseDragged
        }
        (MouseButtonPhase::Dragged, ScenarioPointerButton::Middle) => {
            NSEventType::OtherMouseDragged
        }
        (MouseButtonPhase::Dragged, _) => NSEventType::LeftMouseDragged,
        (MouseButtonPhase::Up, ScenarioPointerButton::Secondary) => NSEventType::RightMouseUp,
        (MouseButtonPhase::Up, ScenarioPointerButton::Middle) => NSEventType::OtherMouseUp,
        (MouseButtonPhase::Up, _) => NSEventType::LeftMouseUp,
    };
    let Some(ns_event) = make_mouse_event(event_type, window_point, timestamp, window_number, 1)
    else {
        return;
    };
    match (phase, button) {
        (MouseButtonPhase::Down, ScenarioPointerButton::Secondary) => {
            target.rightMouseDown(&ns_event);
        }
        (MouseButtonPhase::Down, ScenarioPointerButton::Middle) => target.otherMouseDown(&ns_event),
        (MouseButtonPhase::Down, _) => target.mouseDown(&ns_event),
        (MouseButtonPhase::Dragged, ScenarioPointerButton::Secondary) => {
            target.rightMouseDragged(&ns_event);
        }
        (MouseButtonPhase::Dragged, ScenarioPointerButton::Middle) => {
            target.otherMouseDragged(&ns_event);
        }
        (MouseButtonPhase::Dragged, _) => target.mouseDragged(&ns_event),
        (MouseButtonPhase::Up, ScenarioPointerButton::Secondary) => target.rightMouseUp(&ns_event),
        (MouseButtonPhase::Up, ScenarioPointerButton::Middle) => target.otherMouseUp(&ns_event),
        (MouseButtonPhase::Up, _) => target.mouseUp(&ns_event),
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum MouseButtonPhase {
    Down,
    Dragged,
    Up,
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "CGEvent scroll wheel deltas are integer pixels or lines"
)]
fn dispatch_scenario_event(stage: &Stage, input: &mut ScenarioInputState, event: ScenarioEvent) {
    use objc2_core_graphics::{CGEvent, CGScrollEventUnit};

    let root: &NSView = &stage.root;
    let local_point = CGPoint::new(f64::from(event.x()), f64::from(event.y()));
    let window_point = root.convertPoint_toView(local_point, None);
    // SAFETY: `root` is owned by the live capture window on the main thread;
    // reading its superview only retains the parent view for coordinate conversion.
    let superview = unsafe { root.superview() };
    let superview_point = root.convertPoint_toView(local_point, superview.as_deref());
    let window_number = stage.window.windowNumber();
    let timestamp = Duration::from_millis(event.at_ms()).as_secs_f64();

    match event.kind() {
        ScenarioEventKind::PointerDown => {
            let hit = root.hitTest(superview_point);
            let target = hit.as_deref().unwrap_or(root);
            dispatch_mouse_button_event(
                target,
                event.button(),
                MouseButtonPhase::Down,
                window_point,
                timestamp,
                window_number,
            );
            input.pressed_button = Some(event.button());
            input.pressed_view = hit;
        }
        ScenarioEventKind::PointerMove => {
            if let Some(button) = input.pressed_button {
                let target = input.pressed_view.as_deref().unwrap_or(root);
                dispatch_mouse_button_event(
                    target,
                    button,
                    MouseButtonPhase::Dragged,
                    window_point,
                    timestamp,
                    window_number,
                );
            } else {
                let hit = root.hitTest(superview_point);
                let target = hit.as_deref().unwrap_or(root);
                if let Some(ns_event) = make_mouse_event(
                    NSEventType::MouseMoved,
                    window_point,
                    timestamp,
                    window_number,
                    0,
                ) {
                    target.mouseMoved(&ns_event);
                }
            }
        }
        ScenarioEventKind::PointerUp => {
            let button = input
                .pressed_button
                .take()
                .unwrap_or_else(|| event.button());
            let target = input
                .pressed_view
                .take()
                .or_else(|| root.hitTest(superview_point));
            let target_ref = target.as_deref().unwrap_or(root);
            dispatch_mouse_button_event(
                target_ref,
                button,
                MouseButtonPhase::Up,
                window_point,
                timestamp,
                window_number,
            );
        }
        ScenarioEventKind::PointerCancel => {
            input.pressed_button = None;
            input.pressed_view = None;
        }
        ScenarioEventKind::Scroll => {
            let unit = if event.is_line_delta() {
                CGScrollEventUnit::Line
            } else {
                CGScrollEventUnit::Pixel
            };
            let wheel_y = event.dy().round() as i32;
            let wheel_x = event.dx().round() as i32;
            if let Some(cg_event) =
                CGEvent::new_scroll_wheel_event2(None, unit, 2, wheel_y, wheel_x, 0)
                && let Some(ns_event) = NSEvent::eventWithCGEvent(&cg_event)
            {
                let hit = root.hitTest(superview_point);
                let target = hit.as_deref().unwrap_or(root);
                target.scrollWheel(&ns_event);
            }
        }
        _ => {}
    }
}

#[cfg(catalyst)]
fn dispatch_scenario_event(stage: &Stage, input: &mut ScenarioInputState, event: ScenarioEvent) {
    use cocoa_ui::objc2_ui_kit::{UIControlEvents, UIScrollView, UIView};

    let root: &UIView = &stage.root;
    let point = CGPoint::new(f64::from(event.x()), f64::from(event.y()));

    let find_control = |start: &UIView| -> Option<Retained<UIControl>> {
        let mut cursor: Option<Retained<UIView>> = Some(retain_base(start));
        while let Some(view) = cursor {
            if let Ok(control) = view.clone().downcast::<UIControl>() {
                return Some(control);
            }
            cursor = view.superview();
        }
        None
    };

    let find_scroll_view = |start: &UIView| -> Option<Retained<UIScrollView>> {
        let mut cursor: Option<Retained<UIView>> = Some(retain_base(start));
        while let Some(view) = cursor {
            if let Ok(scroll_view) = view.clone().downcast::<UIScrollView>() {
                return Some(scroll_view);
            }
            cursor = view.superview();
        }
        None
    };

    match event.kind() {
        ScenarioEventKind::PointerDown => {
            if let Some(hit) = root.hitTest_withEvent(point, None)
                && let Some(control) = find_control(&hit)
            {
                control.setHighlighted(true);
                control.sendActionsForControlEvents(UIControlEvents::TouchDown);
                input.pressed_control = Some(control);
            }
        }
        ScenarioEventKind::PointerMove => {
            if let Some(control) = &input.pressed_control {
                let local = control.convertPoint_fromView(point, Some(root));
                let inside = control.pointInside_withEvent(local, None);
                control.setHighlighted(inside);
            }
        }
        ScenarioEventKind::PointerUp => {
            if let Some(control) = input.pressed_control.take() {
                let local = control.convertPoint_fromView(point, Some(root));
                let inside = control.pointInside_withEvent(local, None);
                control.setHighlighted(false);
                control.sendActionsForControlEvents(if inside {
                    UIControlEvents::TouchUpInside
                } else {
                    UIControlEvents::TouchUpOutside
                });
            }
        }
        ScenarioEventKind::PointerCancel => {
            if let Some(control) = input.pressed_control.take() {
                control.setHighlighted(false);
                control.sendActionsForControlEvents(UIControlEvents::TouchCancel);
            }
        }
        ScenarioEventKind::Scroll => {
            if let Some(hit) = root.hitTest_withEvent(point, None)
                && let Some(scroll_view) = find_scroll_view(&hit)
            {
                let scale = if event.is_line_delta() { 20.0 } else { 1.0 };
                let current = scroll_view.contentOffset();
                let content = scroll_view.contentSize();
                let bounds = scroll_view.bounds().size;
                let max_x = (content.width - bounds.width).max(0.0);
                let max_y = (content.height - bounds.height).max(0.0);
                let next_x =
                    f64::mul_add(f64::from(event.dx()), -scale, current.x).clamp(0.0, max_x);
                let next_y =
                    f64::mul_add(f64::from(event.dy()), -scale, current.y).clamp(0.0, max_y);
                scroll_view.setContentOffset_animated(CGPoint::new(next_x, next_y), false);
            }
        }
        _ => {}
    }
}

/// The mounted preview: the view on a root `HostView` inside its capture
/// window, alive for the capture.
///
/// Fields drop in declaration order: the mounted content detaches while
/// its root, controller and window are still retained.
struct Stage {
    /// The windowless frame drive — declared first so its `ViewCapture`
    /// shuts down, ending every external-rendering scope, before the
    /// mounted subtree detaches.
    #[cfg(feature = "gpu_surface")]
    drive: Rc<crate::capture_image::CaptureDrive>,
    /// The mounted subtree.
    _content: Rc<crate::contract::Mounted>,
    /// The mount's watchers and guards.
    _keepalive: crate::contract::KeepAlive,
    /// The capture window's appearance, feeding the theme.
    #[cfg(catalyst)]
    _appearance: Option<cocoa_ui::uikit::ColorSchemeObservation>,
    /// The root the content mounts on and the capture reads.
    root: cocoa_ui::Retained<HostView>,
    /// The window's root controller, owning `root`.
    #[cfg(catalyst)]
    _controller: cocoa_ui::Retained<cocoa_ui::uikit::ViewController>,
    /// The capture window — never ordered in on `AppKit`, scene-less on
    /// Mac Catalyst.
    #[cfg_attr(
        catalyst,
        expect(
            dead_code,
            reason = "the capture window's role is to stay alive for the capture"
        )
    )]
    window: CaptureWindow,
}

/// The root host view of the platform realization.
#[cfg(target_os = "macos")]
type HostView = cocoa_ui::appkit::HostView;
/// See the macOS alias.
#[cfg(catalyst)]
type HostView = cocoa_ui::uikit::HostView;

/// The platform capture window.
#[cfg(target_os = "macos")]
type CaptureWindow = cocoa_ui::Retained<cocoa_ui::objc2_app_kit::NSWindow>;
/// See the macOS alias.
#[cfg(catalyst)]
type CaptureWindow = cocoa_ui::Retained<cocoa_ui::bitmap::CaptureWindow>;

impl Stage {
    /// Mounts `view` under `env` on a root `HostView` filling a capture
    /// window sized `size` — the root path a window's content takes
    /// (`embedding::mount_content`).
    #[cfg(target_os = "macos")]
    fn mount(
        env: &Environment,
        mtm: MainThreadMarker,
        _host: &Host,
        view: impl FnOnce() -> waterui::AnyView,
        size: cocoa_ui::Size,
        color_scheme: Option<ColorScheme>,
        dynamic_range: DynamicRange,
    ) -> Self {
        // `cocoa_ui::bitmap::capture_window` builds the one never-ordered
        // construction every `AppKit` capture shares.
        let window = cocoa_ui::bitmap::capture_window(mtm, size);
        if let Some(scheme) = color_scheme {
            set_window_color_scheme(&window, scheme);
        }
        let root = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, size.width, size.height));
        window.setContentView(Some(&*root));
        apply_to_view(dynamic_range, &root);
        let mut keepalive = crate::contract::KeepAlive::default();
        let content = crate::embedding::mount_content(&root, view(), env, &mut keepalive, mtm);
        crate::first_paint::mark(&root, env);
        // The window never orders in, so the surfaces inside cannot
        // present: the drive keeps their frames — and the content their
        // scenes settle — moving until the terminal capture.
        #[cfg(feature = "gpu_surface")]
        let drive = crate::capture_image::CaptureDrive::begin(&root, env, mtm);
        Self {
            #[cfg(feature = "gpu_surface")]
            drive,
            _content: content,
            _keepalive: keepalive,
            root,
            window,
        }
    }

    /// Mounts `view` under `env` on the root `HostView` of a view
    /// controller filling a scene-less capture window sized `size` — the
    /// root path a scene window's content takes (`windows::connect`,
    /// `embedding::mount_content`).
    #[cfg(catalyst)]
    fn mount(
        env: &Environment,
        mtm: MainThreadMarker,
        host: &Host,
        view: impl FnOnce() -> waterui::AnyView,
        size: cocoa_ui::Size,
        color_scheme: Option<ColorScheme>,
        dynamic_range: DynamicRange,
    ) -> Self {
        use cocoa_ui::objc2::Message;

        let window = cocoa_ui::bitmap::scene_less_capture_window(mtm, &host.application, size);
        if let Some(scheme) = color_scheme {
            set_window_color_scheme(&window, scheme);
        }
        let controller =
            cocoa_ui::uikit::ViewController::new(mtm, cocoa_ui::uikit::window_root(mtm));
        window.setRootViewController(Some(&**controller));
        let root = controller.host_view().retain();
        // Tag the root with the capture's dynamic range before content mounts
        // so leaves resolving inherited range answer from the tag rather than
        // a display the scene-less window never reaches.
        apply_to_view(dynamic_range, &root);
        window.setHidden(false);
        // The window carries the traits the bring-up had no controller to
        // read: refresh the theme from them (or the requested scheme) before
        // the content resolves it.
        let initial_scheme = color_scheme.unwrap_or_else(|| controller.color_scheme());
        theme::refresh(&host.theme, initial_scheme);
        let appearance = color_scheme.is_none().then(|| {
            controller.observe_color_scheme({
                let installed_theme = Rc::clone(&host.theme);
                move |scheme| theme::refresh(&installed_theme, scheme)
            })
        });
        let mut keepalive = crate::contract::KeepAlive::default();
        let content = crate::embedding::mount_content(&root, view(), env, &mut keepalive, mtm);
        crate::first_paint::mark(&root, env);
        window.layoutIfNeeded();
        // The scene-less window never reaches a presentable scene, so the
        // surfaces inside cannot present: the drive keeps their frames —
        // and the content their scenes settle — moving until the terminal
        // capture.
        #[cfg(feature = "gpu_surface")]
        let drive = crate::capture_image::CaptureDrive::begin(&root, env, mtm);
        Self {
            #[cfg(feature = "gpu_surface")]
            drive,
            _content: content,
            _keepalive: keepalive,
            _appearance: appearance,
            root,
            _controller: controller,
            window,
        }
    }

    /// Captures the presented subtree: pure-native trees render directly
    /// through `capture::capture_presented` (using the opt-in in-process
    /// `Core Animation` render server so Liquid Glass and backdrop materials
    /// composite without any visible window), while trees carrying GPU
    /// surfaces composite their render-server native raster with every GPU
    /// surface through `ViewCapture`.
    #[cfg(feature = "gpu_surface")]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture(&self) -> Result<RenderResult, CaptureFailure> {
        // The window is never ordered in, so its display pass never runs on
        // its own: run it explicitly so backing stores and subview layers are
        // current before either capture path reads them.
        #[cfg(target_os = "macos")]
        self.window.displayIfNeeded();
        let mut has_capturable = false;
        collect_capturables(&self.root, &mut |_| has_capturable = true);
        if !has_capturable {
            return capture::capture_presented(&self.root)
                .await
                .map_err(CaptureFailure::NativeCapture);
        }
        let result = self.drive.capture_rgba().await?;
        Ok(RenderResult::new(
            result.pixels,
            result.width,
            result.height,
        ))
    }

    /// Captures the presented subtree into an unclamped extended linear Display
    /// P3 `f32` RGBA raster (`HdrRaster`), preserving HDR (`> 1.0`) highlights
    /// from both native Liquid Glass / HDR layers and GPU surfaces.
    #[cfg(feature = "gpu_surface")]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture_hdr(&self) -> Result<HdrRaster, CaptureFailure> {
        #[cfg(target_os = "macos")]
        self.window.displayIfNeeded();
        let mut has_capturable = false;
        collect_capturables(&self.root, &mut |_| has_capturable = true);
        if !has_capturable {
            return capture::capture_presented_hdr(&self.root)
                .await
                .map_err(CaptureFailure::NativeCapture);
        }
        self.drive.capture_hdr().await
    }

    /// The `gpu_surface`-free capture: every leaf rasterizes through
    /// `capture::capture_presented` (using the in-process `Core Animation`
    /// render server so glass and backdrop materials composite cleanly).
    #[cfg(not(feature = "gpu_surface"))]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture(&self) -> Result<RenderResult, CaptureFailure> {
        capture::capture_presented(&self.root).await
    }

    /// The `gpu_surface`-free HDR capture: every leaf rasterizes through
    /// `capture::capture_presented_hdr` (`RGBA16F` + `ExtendedLinearDisplayP3`).
    #[cfg(not(feature = "gpu_surface"))]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture_hdr(&self) -> Result<HdrRaster, CaptureFailure> {
        capture::capture_presented_hdr(&self.root).await
    }
}
