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
/// # Errors
///
/// [`PreviewError::UnsupportedMode`] for `Scenario` and `Semantic` runs —
/// the Apple preview captures images only. [`PreviewError::Capture`] when
/// `AppKit` cannot mount, present or rasterize the view.
/// [`PreviewError::Write`] when the PNG cannot be encoded or written.
/// [`PreviewError::RunLoopExited`] when the main run loop returns without
/// the capture's result — a return is never success on its own.
///
/// # Panics
///
/// Panics off the main thread, or when `AppKit` refuses the `Prohibited`
/// activation policy before launch — the constraint the entry exists to
/// keep, so a refusal is fatal rather than a fallback.
pub fn run(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> waterui::AnyView + 'static,
    resources: ResourceContext,
    config: PreviewRunConfig,
) -> Result<(), PreviewError> {
    let output = match config.mode {
        PreviewRunMode::Image { output } => output,
        PreviewRunMode::Scenario { .. } => {
            return Err(PreviewError::UnsupportedMode("scenario"));
        }
        PreviewRunMode::Semantic => return Err(PreviewError::UnsupportedMode("semantic")),
    };
    let size = cocoa_ui::Size::new(f64::from(config.width), f64::from(config.height));

    let mtm = MainThreadMarker::new().expect("preview runs on the main thread");
    // The keepers live on `run`'s frame, which outlives the capture.
    let mut keepers = crate::contract::KeepAlive::default();
    let mut env = prepare(mtm, resources, &mut keepers);
    let _keepers = keepers;

    let env_ptr = &raw mut env;
    let outcome = Rc::new(RefCell::new(None::<Result<(), PreviewError>>));
    // SAFETY: `env` is lent for the process — `prepare` hands it to the
    // completion, which renders, deposits the run's result in `outcome`
    // and stops the run loop this thread drives; nothing else reads it.
    unsafe {
        crate::gpu_runtime::prepare(env_ptr, {
            let outcome = Rc::clone(&outcome);
            move || {
                // `run` lent `env` for the process; this closure is the
                // only consumer and runs once, on the main thread.
                let env = &mut *env_ptr;
                crate::embedding::install_services(env);
                let env = compose(env.clone());
                executor_core::spawn_local(async move {
                    let result = render(&env, mtm, view, size)
                        .await
                        .map_err(PreviewError::from)
                        .and_then(|result| {
                            // Apple's bitmap context reads premultiplied
                            // RGBA8 — the convention `write_png` declares
                            // for the Apple capture.
                            waterui_preview_protocol::run::write_png(
                                &output,
                                result.width,
                                result.height,
                                result.rgba_data,
                                Alpha::Premultiplied,
                            )
                            .map_err(PreviewError::from)
                        });
                    *outcome.borrow_mut() = Some(result);
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

/// The backend bring-up `entry::run` performs, minus menus, window
/// realization and the termination machine — shared by [`run`] and the
/// native-test harness so the capture path is exercised exactly once.
///
/// Sets the `Prohibited` activation policy before `AppKit` finishes
/// launching: a capture never shows a window, takes focus or puts an
/// icon in the Dock, and a refusal only happens when the process arrived
/// bundled some other way, so it is fatal rather than a fallback. The
/// environment's keepers — theme, appearance observation, locale, font
/// registrations — land on `keepers`, which must outlive `env`.
///
/// # Panics
///
/// Panics when `AppKit` refuses the `Prohibited` activation policy.
pub(crate) fn prepare(
    mtm: MainThreadMarker,
    resources: ResourceContext,
    keepers: &mut crate::contract::KeepAlive,
) -> Environment {
    let inspector = crate::startup::initialize();
    environment(mtm, resources, keepers, inspector)
}

/// The environment half of [`prepare`], minus `startup::initialize`.
///
/// Process startup is not idempotent — tracing, the executors and the
/// panic hook install once — so a caller that already ran it (the
/// native-test harness's `initialize_process`) passes the inspector it
/// answered here instead.
///
/// # Panics
///
/// Panics when `AppKit` refuses the `Prohibited` activation policy.
pub(crate) fn environment(
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
    // `AppKit` must be told before launch completes. The set is
    // best-effort — a bundle that already declares `UIElement` or an
    // unbundled process arrives `Prohibited` and refuses a redundant
    // change — so the invariant asserted is the policy itself.
    let _ = application.set_activation_policy(ActivationPolicy::Prohibited);
    assert_eq!(
        cocoa_ui::objc2_app_kit::NSApplication::sharedApplication(mtm).activationPolicy(),
        cocoa_ui::objc2_app_kit::NSApplicationActivationPolicy::Prohibited,
        "the preview process must hold the Prohibited activation policy before AppKit finishes launching"
    );
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

/// Mounts `view` under `env` and captures the presented subtree.
///
/// The view mounts on a root `HostView` inside a never-ordered window
/// sized `size` — `run`'s render half, factored so the native-test
/// harness drives the identical path.
///
/// # Errors
///
/// Returns [`crate::capture::CaptureError`] when `AppKit` cannot produce
/// the bitmap.
#[expect(
    clippy::future_not_send,
    reason = "the capture runs on the main thread; the Retained AppKit objects it holds across the wait are not Send"
)]
pub(crate) async fn render(
    env: &Environment,
    mtm: MainThreadMarker,
    view: impl FnOnce() -> waterui::AnyView,
    size: cocoa_ui::Size,
) -> Result<waterui_core::view_renderer::RenderResult, crate::capture::CaptureError> {
    let window = cocoa_ui::appkit::Window::new(
        mtm,
        cocoa_ui::Rect::new(0.0, 0.0, size.width, size.height),
        cocoa_ui::appkit::WindowStyle::empty(),
    );
    let root = cocoa_ui::appkit::HostView::new(
        mtm,
        cocoa_ui::Rect::new(0.0, 0.0, size.width, size.height),
    );
    window.set_content_view(&root);
    let mut keepalive = crate::contract::KeepAlive::default();
    let _content = crate::embedding::mount_content(&root, view(), env, &mut keepalive);
    crate::first_paint::mark(&root, env);
    let result = crate::capture::capture_presented(&root).await?;
    drop((keepalive, window));
    Ok(result)
}
