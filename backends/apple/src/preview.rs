//! The in-process preview entry `water preview` drives — the `AppKit`
//! realization on macOS, the `UIKit` realization through Mac Catalyst.
//!
//! The generated preview binary calls [`run`] from `main` and returns its
//! `Result`. The process brings the backend up the way `crate::entry`
//! does — startup, the GPU runtime, native services — under the main run
//! loop rather than the platform's application run loop, mounts the view
//! on a root `HostView` inside a capture window under a fresh
//! [`ContentLedger`](waterui_core::ContentLedger), waits for the content
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
use std::rc::Rc;

use cocoa_ui::MainThreadMarker;
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{ActivationPolicy, Application};
use cocoa_ui::objc2_core_foundation::CFRunLoop;
use waterui_backend_core::Environment;
use waterui_core::ResourceContext;
use waterui_preview_protocol::run::{Alpha, PreviewRunConfig, PreviewRunMode};

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
    #[error("apple preview supports image captures only; {0} runs are not implemented")]
    UnsupportedMode(&'static str),
    /// Mounting, presenting or rasterizing the view failed.
    #[error(transparent)]
    Capture(#[from] CaptureFailure),
    /// A view still owed content when the run's content deadline passed, or
    /// its content failed to load.
    #[error(transparent)]
    Content(#[from] waterui_core::content_ledger::ContentError),
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
/// the Apple preview captures images only. [`PreviewError::Content`] when a
/// view still owes content at the run's content deadline or its load
/// fails. [`PreviewError::Capture`] when the view cannot be mounted or
/// rasterized. [`PreviewError::Write`] when the PNG cannot be encoded or
/// written. [`PreviewError::RunLoopExited`] when the main run
/// loop returns without the capture's result — a return is never success
/// on its own.
///
/// # Panics
///
/// Panics off the main thread, or when `AppKit` refuses the `Prohibited`
/// activation policy before launch — the constraint the entry exists to
/// keep, so a refusal is fatal rather than a fallback. On Mac Catalyst,
/// also panics when the process does not run under the iPad idiom — the
/// binary did not embed [`preview_info_plist!`](crate::preview_info_plist).
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

    let content_deadline = config.content_deadline();
    let output = match config.mode() {
        PreviewRunMode::Image { output } => output.clone(),
        PreviewRunMode::Scenario { .. } => {
            return Err(PreviewError::UnsupportedMode("scenario"));
        }
        PreviewRunMode::Semantic => return Err(PreviewError::UnsupportedMode("semantic")),
    };
    let size = cocoa_ui::Size::new(f64::from(config.width()), f64::from(config.height()));

    // The keepers live on `drive`'s frame and are declared before `env`, so
    // `env` drops first and `keepers` — the theme, appearance observer,
    // locale and font registrations `prepare` requires to outlive `env` —
    // drops last.
    let mut keepers = crate::contract::KeepAlive::default();
    let (mut env, host) = prepare(mtm, resources, &mut keepers, inspector);

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
                    *outcome.borrow_mut() = Some(
                        render_and_write(
                            &env,
                            mtm,
                            &host,
                            view,
                            Capture {
                                size,
                                content_deadline,
                                output: &output,
                            },
                        )
                        .await,
                    );
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
/// # Panics
///
/// Panics when `AppKit` refuses the `Prohibited` activation policy, and on
/// Mac Catalyst when the process does not run under the iPad idiom.
pub(crate) fn prepare(
    mtm: MainThreadMarker,
    resources: ResourceContext,
    keepers: &mut crate::contract::KeepAlive,
    inspector: Option<waterui::inspector::InspectorRuntime>,
) -> (Environment, Host) {
    let mut env = Environment::new();
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
        // only when the policy is not already there, and a refusal fails at
        // the call that refused it.
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
        keepers.keep(theme);
        keepers.keep(appearance);
        Host {}
    };
    #[cfg(catalyst)]
    let host = {
        let application = cocoa_ui::uikit::catalyst::BackgroundApplication::bring_up(mtm);
        assert!(
            cocoa_ui::uikit::catalyst::runs_under_pad_idiom(mtm),
            "the Catalyst preview binary must embed `waterui_apple::preview_info_plist!()`: UIKit draws the iOS look only under the iPad idiom (UIDeviceFamily 2)"
        );
        // No controller exists yet to answer the scheme; the capture window
        // refreshes the theme from its own traits once the view mounts.
        let theme = Rc::new(crate::theme::install(
            &mut env,
            cocoa_ui::uikit::current_scheme(),
        ));
        Host { application, theme }
    };
    let locale = crate::locale::install(&mut env, mtm);
    keepers.keep(locale);
    keepers.keep(fonts);
    (env, host)
}

/// What one capture produces: the viewport, how long it waits for owed
/// content, and where the PNG goes.
struct Capture<'a> {
    /// The capture window's size.
    size: cocoa_ui::Size,
    /// How long the capture waits for content its views owe.
    content_deadline: core::time::Duration,
    /// The PNG destination.
    output: &'a std::path::Path,
}

/// Renders `view` under `env` inside a capture window, awaits the content
/// its views owe, captures the presented subtree and writes its PNG — the
/// one tail [`run`] runs and every capture the entry performs ends in.
///
/// The view mounts under a fresh [`ContentLedger`](waterui_core::ContentLedger):
/// the capture rasterizes only once a full drain leaves nothing owed.
/// Apple's bitmap context reads premultiplied RGBA8 — the convention the
/// writer declares for the capture.
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
    view: impl FnOnce() -> waterui::AnyView,
    capture: Capture<'_>,
) -> Result<(), PreviewError> {
    let ledger = waterui_core::ContentLedger::new();
    let content_env = env.extending(ledger.clone());
    let stage = Stage::mount(&content_env, mtm, host, view, capture.size);
    crate::content::await_content(&ledger, &stage.root, capture.content_deadline).await?;
    let result = stage.capture().await?;
    drop(stage);
    waterui_preview_protocol::run::write_png(
        capture.output,
        result.width(),
        result.height(),
        result.rgba_data().to_vec(),
        Alpha::Premultiplied,
    )?;
    Ok(())
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
    _appearance: cocoa_ui::uikit::ColorSchemeObservation,
    /// The root the content mounts on and the capture reads.
    root: cocoa_ui::Retained<HostView>,
    /// The window's root controller, owning `root`.
    #[cfg(catalyst)]
    _controller: cocoa_ui::Retained<cocoa_ui::uikit::ViewController>,
    /// The capture window — never ordered in on `AppKit`, scene-less on
    /// Mac Catalyst. Only the `AppKit` `ViewCapture` path reads it, to run
    /// the display pass; everywhere else its role is to stay alive.
    #[cfg_attr(
        not(all(target_os = "macos", feature = "gpu_surface")),
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
    ) -> Self {
        // `cocoa_ui::bitmap::capture_window` builds the one never-ordered
        // construction every `AppKit` capture shares.
        let window = cocoa_ui::bitmap::capture_window(mtm, size);
        let root = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, size.width, size.height));
        window.setContentView(Some(&*root));
        let mut keepalive = crate::contract::KeepAlive::default();
        let content = crate::embedding::mount_content(&root, view(), env, &mut keepalive);
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
    ) -> Self {
        use cocoa_ui::objc2::Message;

        let window = cocoa_ui::bitmap::scene_less_capture_window(mtm, &host.application, size);
        let controller =
            cocoa_ui::uikit::ViewController::new(mtm, cocoa_ui::uikit::window_root(mtm));
        window.setRootViewController(Some(&**controller));
        let root = controller.host_view().retain();
        // The render target is an RGBA8 bitmap, so the root declares
        // `DynamicRange::Standard` before the content attaches: a leaf
        // resolving its inherited mode answers from the tag rather than
        // a display the window never reaches.
        cocoa_ui::dynamic_range::apply_to_view(
            cocoa_ui::dynamic_range::DynamicRange::Standard,
            &root,
        );
        window.setHidden(false);
        // The window carries the traits the bring-up had no controller to
        // read: refresh the theme from them before the content resolves it.
        crate::theme::refresh(&host.theme, controller.color_scheme());
        let appearance = controller.observe_color_scheme({
            let theme = Rc::clone(&host.theme);
            move |scheme| crate::theme::refresh(&theme, scheme)
        });
        let mut keepalive = crate::contract::KeepAlive::default();
        let content = crate::embedding::mount_content(&root, view(), env, &mut keepalive);
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

    /// Captures the presented subtree: `ViewCapture` composites the native
    /// raster with every GPU surface inside it. The owed content has
    /// settled by now, so a zero-size subtree is genuinely empty, not still
    /// loading.
    #[cfg(feature = "gpu_surface")]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture(&self) -> Result<waterui_core::view_renderer::RenderResult, CaptureFailure> {
        // The window is never ordered in, so its display pass never runs on
        // its own: run it explicitly — it draws the backing stores and
        // attaches every subview's backing layer into the root layer tree
        // `ViewCapture`'s `renderInContext` reads.
        #[cfg(target_os = "macos")]
        self.window.displayIfNeeded();
        let result = self.drive.capture_rgba().await?;
        Ok(waterui_core::view_renderer::RenderResult::new(
            result.pixels,
            result.width,
            result.height,
        ))
    }

    /// The `gpu_surface`-free capture: every leaf rasterizes through the
    /// platform's own display path, so the plain bitmap readback covers the
    /// whole subtree.
    #[cfg(not(feature = "gpu_surface"))]
    #[expect(
        clippy::future_not_send,
        reason = "the capture runs on the main thread; the Retained native objects it holds across the wait are not Send"
    )]
    async fn capture(&self) -> Result<waterui_core::view_renderer::RenderResult, CaptureFailure> {
        crate::capture::capture_presented(&self.root).await
    }
}
