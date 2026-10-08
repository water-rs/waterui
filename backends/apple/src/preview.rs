//! The `AppKit` preview entry — the in-process render `water preview`
//! drives on macOS.
//!
//! The generated preview binary calls [`run`] from `main` and returns its
//! `Result`. The process brings the backend up the way `crate::entry`
//! does — startup, the GPU runtime, native services — under the main run
//! loop rather than `NSApp.run`, mounts the view on a root `HostView`
//! inside a window that is never ordered in, waits for GPU surfaces to
//! present, captures the whole subtree through `crate::capture` and
//! writes the PNG. The window never orders, the activation policy is
//! `Prohibited` before `AppKit` finishes launching, and the process
//! answers only through `main`'s return — no Dock icon, no focus, no
//! on-screen window, ever.

use std::cell::RefCell;
use std::rc::Rc;

use cocoa_ui::MainThreadMarker;
use cocoa_ui::appkit::{ActivationPolicy, Application};
use cocoa_ui::objc2_core_foundation::CFRunLoop;
use waterui_backend_core::Environment;
use waterui_core::ResourceContext;
use waterui_preview_protocol::run::{Alpha, PreviewRunConfig, PreviewRunMode};

/// Why a preview run failed — the generated `main` propagates it for the
/// process to report and exit non-zero.
#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    /// The run mode is not implemented by the Apple preview.
    #[error("apple preview supports image captures only; {0} runs are not implemented")]
    UnsupportedMode(&'static str),
    /// Mounting, presenting or rasterizing the view failed.
    #[cfg(feature = "gpu_surface")]
    #[error(transparent)]
    Capture(#[from] crate::capture_image::CaptureFailure),
    /// Mounting, presenting or rasterizing the view failed.
    #[cfg(not(feature = "gpu_surface"))]
    #[error(transparent)]
    Capture(#[from] crate::capture::CaptureError),
    /// Encoding or writing the capture PNG failed.
    #[error(transparent)]
    Write(#[from] waterui_preview_protocol::run::PngError),
    /// The run loop returned without the capture handing its result back.
    #[error("the main run loop returned before the capture settled")]
    RunLoopExited,
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
/// [`PreviewError::UnsupportedMode`] for `Scenario` and `Semantic` runs —
/// the Apple preview captures images only. [`PreviewError::Capture`] when
/// the view cannot be mounted or rasterized. [`PreviewError::Write`]
/// when the PNG cannot be encoded or written. [`PreviewError::RunLoopExited`] when the main run
/// loop returns without the capture's result — a return is never success
/// on its own.
///
/// # Panics
///
/// Panics off the main thread, or when `AppKit` refuses the `Prohibited`
/// activation policy before launch — the constraint the entry exists to
/// keep, so a refusal is fatal rather than a fallback.
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

/// `run`'s body — the single `Result` boundary [`run`] reports through.
fn drive(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> waterui::AnyView + 'static,
    resources: ResourceContext,
    config: PreviewRunConfig,
) -> Result<(), PreviewError> {
    let mtm = MainThreadMarker::new().expect("preview runs on the main thread");
    // Startup before anything else can panic or error — the panic hook and
    // the unconditional stderr layer are what the contract guarantees
    // everything else through.
    let inspector = crate::startup::initialize_for_preview();

    let output = match config.mode {
        PreviewRunMode::Image { output } => output,
        PreviewRunMode::Scenario { .. } => {
            return Err(PreviewError::UnsupportedMode("scenario"));
        }
        PreviewRunMode::Semantic => return Err(PreviewError::UnsupportedMode("semantic")),
    };
    let size = cocoa_ui::Size::new(f64::from(config.width), f64::from(config.height));

    // The keepers live on `drive`'s frame and are declared before `env`, so
    // `env` drops first and `keepers` — the theme, appearance observer,
    // locale and font registrations `prepare` requires to outlive `env` —
    // drops last.
    let mut keepers = crate::contract::KeepAlive::default();
    let mut env = prepare(mtm, resources, &mut keepers, inspector);

    let env_ptr = &raw mut env;
    let outcome = Rc::new(RefCell::new(None::<Result<(), PreviewError>>));
    // SAFETY: `env` is lent to `drive`'s frame — the completion below
    // renders, deposits the run's result in `outcome` and stops the run
    // loop `drive` blocks in, so the borrow ends before the frame returns;
    // nothing else drains the main queue after a `RunLoopExited` return.
    unsafe {
        crate::gpu_runtime::prepare(env_ptr, {
            let outcome = Rc::clone(&outcome);
            move || {
                // `drive` lent `env` for its frame; this closure is the
                // only consumer and runs once, on the main thread.
                let env = &mut *env_ptr;
                crate::embedding::install_services(env);
                let env = compose(env.clone());
                #[cfg(feature = "webview")]
                let env = {
                    let mut env = env;
                    crate::components::webview::install_service(&mut env);
                    env
                };
                executor_core::spawn_local(async move {
                    *outcome.borrow_mut() =
                        Some(render_and_write(&env, mtm, view, size, &output).await);
                    CFRunLoop::current()
                        .expect("the render task runs on a thread that has a run loop")
                        .stop();
                })
                .detach();
            }
        });
    }
    // `prepare`'s completion and `render` both land on the main executor,
    // which this run loop drives; the task's `stop` is the only way out,
    // so a bare return can only come from the loop itself.
    CFRunLoop::run();
    outcome
        .borrow_mut()
        .take()
        .ok_or(PreviewError::RunLoopExited)?
}

/// The backend bring-up `entry::run` performs, minus startup, menus, window
/// realization and the termination machine.
///
/// Process startup is not idempotent — tracing, the executors and the
/// panic hook install once — so the caller passes the inspector
/// [`crate::startup::initialize_for_preview`] answered. Sets the
/// `Prohibited` activation policy before `AppKit` finishes launching: a
/// capture never shows a window, takes focus or puts an icon in the Dock.
/// The environment's keepers — theme, appearance observation, locale, font
/// registrations — land on `keepers`, which must outlive `env`.
///
/// # Panics
///
/// Panics when `AppKit` refuses the `Prohibited` activation policy.
pub(crate) fn prepare(
    mtm: MainThreadMarker,
    resources: ResourceContext,
    keepers: &mut crate::contract::KeepAlive,
    inspector: Option<waterui::inspector::InspectorRuntime>,
) -> Environment {
    let mut env = Environment::new();
    waterui::inspector::install(&mut env, inspector);
    waterui::text::install_system_font_collection(&mut env);
    let mut fonts = crate::fonts::FontRegistrations::default();
    env.insert(resources);
    crate::resources::install_application(&mut env, &mut fonts);
    crate::dispatch::install(&mut env);
    env.insert(crate::first_paint::FirstPaint::default());

    let application = Application::shared(mtm);
    // `AppKit` must be told before launch completes; a bundle that already
    // declares `UIElement`, or an unbundled process, arrives `Prohibited`
    // and may refuse a redundant change — so the set runs only when the
    // policy is not already there, and a refusal fails at the call that
    // refused it.
    if application.activation_policy() != ActivationPolicy::Prohibited {
        assert!(
            application.set_activation_policy(ActivationPolicy::Prohibited),
            "the preview process must hold the Prohibited activation policy before AppKit finishes launching"
        );
    }
    let theme = Rc::new(crate::theme::install(&mut env, application.color_scheme()));
    let appearance = application.observe_color_scheme({
        let theme = Rc::clone(&theme);
        move |scheme| crate::theme::refresh(&theme, scheme)
    });
    let locale = crate::locale::install(&mut env, mtm);
    keepers.keep(theme);
    keepers.keep(appearance);
    keepers.keep(locale);
    keepers.keep(fonts);
    env
}

/// Renders `view` under `env` inside a never-ordered window, captures the
/// presented subtree and writes its PNG — the one tail [`run`] runs and
/// every capture the entry performs ends in.
///
/// Apple's bitmap context reads premultiplied RGBA8 — the convention the
/// writer declares for the capture.
///
/// # Errors
///
/// Returns [`crate::capture::CaptureError`] when `AppKit` cannot mount,
/// present or rasterize the view, and [`waterui_preview_protocol::run::PngError`]
/// when the PNG cannot be encoded or written.
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained AppKit objects it holds across the wait are not Send"
)]
pub(crate) async fn render_and_write(
    env: &Environment,
    mtm: MainThreadMarker,
    view: impl FnOnce() -> waterui::AnyView,
    size: cocoa_ui::Size,
    output: &std::path::Path,
) -> Result<(), PreviewError> {
    let result = render(env, mtm, view, size).await?;
    waterui_preview_protocol::run::write_png(
        output,
        result.width,
        result.height,
        result.rgba_data,
        Alpha::Premultiplied,
    )?;
    Ok(())
}

/// Mounts `view` under `env` and captures the presented subtree.
///
/// The view mounts on a root `HostView` inside the capture window —
/// `cocoa_ui::bitmap::capture_window` builds the one never-ordered
/// construction both capture paths share — sized `size`.
///
/// # Errors
///
/// Returns [`crate::capture_image::CaptureFailure`] or, without
/// `gpu_surface`, [`crate::capture::CaptureError`] when the subtree
/// cannot be rasterized.
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained AppKit objects it holds across the wait are not Send"
)]
#[cfg(feature = "gpu_surface")]
async fn render(
    env: &Environment,
    mtm: MainThreadMarker,
    view: impl FnOnce() -> waterui::AnyView,
    size: cocoa_ui::Size,
) -> Result<waterui_core::view_renderer::RenderResult, crate::capture_image::CaptureFailure> {
    let window = cocoa_ui::bitmap::capture_window(mtm, size);
    let root = cocoa_ui::appkit::HostView::new(
        mtm,
        cocoa_ui::Rect::new(0.0, 0.0, size.width, size.height),
    );
    window.setContentView(Some(&*root));
    let mut keepalive = crate::contract::KeepAlive::default();
    let _content = crate::embedding::mount_content(&root, view(), env, &mut keepalive);
    crate::first_paint::mark(&root, env);
    // The preview mounts and captures inside one run-loop turn, so the
    // window's display pass never runs: run it explicitly — it draws the
    // backing stores and attaches every subview's backing layer into the
    // root layer tree `ViewCapture`'s `renderInContext` reads.
    window.displayIfNeeded();
    let scale = cocoa_ui::view::backing_scale_factor(&root);
    let result = crate::capture_image::capture_rgba(&root, env, scale, mtm).await?;
    let result = waterui_core::view_renderer::RenderResult {
        rgba_data: result.pixels,
        width: result.width,
        height: result.height,
    };
    drop((keepalive, window));
    Ok(result)
}

/// The `gpu_surface`-free preview: every leaf rasterizes through
/// `AppKit`'s own display path, so the plain bitmap readback covers the
/// whole subtree.
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained AppKit objects it holds across the wait are not Send"
)]
#[cfg(not(feature = "gpu_surface"))]
async fn render(
    env: &Environment,
    mtm: MainThreadMarker,
    view: impl FnOnce() -> waterui::AnyView,
    size: cocoa_ui::Size,
) -> Result<waterui_core::view_renderer::RenderResult, crate::capture::CaptureError> {
    let window = cocoa_ui::bitmap::capture_window(mtm, size);
    let root = cocoa_ui::appkit::HostView::new(
        mtm,
        cocoa_ui::Rect::new(0.0, 0.0, size.width, size.height),
    );
    window.setContentView(Some(&*root));
    let mut keepalive = crate::contract::KeepAlive::default();
    let _content = crate::embedding::mount_content(&root, view(), env, &mut keepalive);
    crate::first_paint::mark(&root, env);
    let result = crate::capture::capture_presented(&root).await?;
    drop((keepalive, window));
    Ok(result)
}
