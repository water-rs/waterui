//! Application mount: the work `waterui_apple_main` does for Apple, minus
//! the never-returning run loop — Android drives `nativeCreate` from the
//! activity's `onCreate` and hands back an opaque handle.

use alloc::boxed::Box;
use alloc::rc::Rc;

use jni::Env;
use jni::objects::JObject;
use jni::sys::jlong;
use waterui::app::App;
use waterui_backend_core::Environment;
use waterui_locale::Locale;

use crate::contract::{KeepAlive, Mounted, PlatformView};
use crate::jvm::Platform;
use crate::theme::ThemeSignals;

/// What a mounted application keeps alive: the environment signals resolve
/// through, the mounted content leaf, and the platform-feed handles the
/// lifecycle calls push into. `platform` is the whole of what the earlier
/// design kept in statics — the runtime owns it and shares it by `Rc`.
struct Runtime {
    _env: Environment,
    _root: PlatformView,
    _content: Mounted,
    theme: ThemeSignals,
    locale: waterui::reactive::Binding<Locale>,
    platform: Rc<Platform>,
    /// Everything the window kept alive — the background watcher among it.
    _keepalive: KeepAlive,
}

/// The `export_app!` expansion's call: establish the JNI globals, run
/// startup, then declare and mount the app into `root`.
///
/// Called once on the main thread by `WaterActivity.onCreate`, from
/// `dev.waterui.android.WaterRuntime.nativeCreate`.
///
/// # Errors
///
/// Any JNI failure surfaces as a pending exception the host's
/// `EnvUnowned::with_env` resolves as a thrown `RuntimeException` — mount is
/// not retryable, and a partially mounted runtime is not a state the app can
/// keep.
///
/// # Panics
///
/// When `app` declares no window — a `WaterUI` `App` always declares one.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the app factory runs exactly once — FnOnce::call_once consumes it"
)]
pub fn mount(
    env: &mut Env<'_>,
    activity: JObject<'_>,
    root: JObject<'_>,
    app: impl FnOnce(Environment) -> App,
) -> jni::errors::Result<jlong> {
    // `EnvUnowned::with_env` has already initialized the `JavaVM` singleton —
    // every later attach is a TLS check.
    let platform = Rc::new(Platform::new(env, &activity)?);
    platform.refresh_metrics(env)?;
    let inspector = crate::startup::initialize(env, &platform);
    let root = env.new_global_ref(root)?;

    let mut env_rust = Environment::new();
    waterui::inspector::install(&mut env_rust, inspector);
    waterui::text::install_system_font_collection(&mut env_rust);
    // The platform travels beside the dispatcher: `dispatch::render`
    // resolves both out of the environment for the render context.
    env_rust.insert(Rc::clone(&platform));
    crate::dispatch::install(&mut env_rust);
    let theme = crate::theme::install(&mut env_rust, env, &platform)?;
    let locale = crate::locale::install(&mut env_rust, &platform);

    let parts = app(env_rust).into_parts();
    let app_env = parts.env;
    // The skeleton embeds the first window; multi-window applications are a
    // later port's surface.
    let Some(window) = parts.windows.into_iter().next() else {
        panic!("a WaterUI app declares at least one window");
    };

    // Content renders under the environment `app` returned: the app's own
    // installs landed as overlays on the clone it was handed — `insert`
    // never propagates between clones.
    let mut keepalive = KeepAlive::default();
    let content = crate::embedding::mount_content(window.build_content(), &root, &app_env);
    // The resolved window background is painted onto the host root behind
    // the content leaf, and kept tracking its signal — the watcher needs its
    // own reference to the root and a `Rc` clone of the platform, which the
    // runtime also holds.
    let watched_root = env.new_global_ref(root.as_ref())?;
    let watched_platform = platform.clone();
    keepalive.bind(&window.resolved_background(&app_env), move |color| {
        crate::embedding::set_root_background(&watched_root, &watched_platform, color);
    });

    let runtime = Box::new(Runtime {
        _env: app_env,
        _root: root,
        _content: content,
        theme,
        locale,
        platform,
        _keepalive: keepalive,
    });
    Ok(crate::handle::pointer_to_jlong(Box::into_raw(runtime)))
}

/// The `Runtime` behind `handle` — the JNI border the lifecycle calls cross.
///
/// # Safety
///
/// `handle` is the `jlong` `mount` returned and has not been destroyed.
unsafe fn runtime<'a>(handle: jlong) -> &'a mut Runtime {
    // SAFETY: the caller's contract — a handle `mount` minted and
    // `nativeDestroy` has not consumed.
    unsafe { &mut *crate::handle::jlong_to_pointer::<Runtime>(handle) }
}

/// `WaterRuntime.nativeDestroy` — tears the runtime down and frees its
/// handle. Called once, on the main thread, from `WaterActivity.onDestroy`.
///
/// # Safety
///
/// The handle is consumed: callers must not touch it again.
///
/// # Panics
///
/// When `handle` is not a pointer `mount` minted — the host's contract
/// forbids that.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_WaterRuntime_nativeDestroy<'caller>(
    mut unowned_env: jni::EnvUnowned<'caller>,
    _this: jni::objects::JObject<'caller>,
    handle: jlong,
) {
    let outcome = unowned_env.with_env(|_env| -> jni::errors::Result<()> {
        // SAFETY: per the host's contract, `handle` is a live runtime and is
        // never used after this call.
        drop(unsafe { Box::from_raw(crate::handle::jlong_to_pointer::<Runtime>(handle)) });
        Ok(())
    });
    outcome.resolve::<crate::policy::ThrowRuntimeExAndDefault>();
}

/// `WaterRuntime.nativeOnConfigurationChanged` — pushes the platform's new
/// appearance into the theme and locale bindings.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_WaterRuntime_nativeOnConfigurationChanged<
    'caller,
>(
    mut unowned_env: jni::EnvUnowned<'caller>,
    _this: jni::objects::JObject<'caller>,
    handle: jlong,
) {
    let outcome = unowned_env.with_env(|env| -> jni::errors::Result<()> {
        // SAFETY: the handle stays live until nativeDestroy.
        let runtime = unsafe { runtime(handle) };
        crate::theme::refresh(env, &runtime.theme)?;
        crate::locale::refresh(&runtime.locale, &runtime.platform);
        Ok(())
    });
    outcome.resolve::<crate::policy::ThrowRuntimeExAndDefault>();
}

/// `WaterRuntime.nativeOnTrimMemory` — memory pressure forwarded for the
/// backend's caches.
///
/// The skeleton has no caches to shed, so the call is a no-op that keeps the
/// JNI surface pinned for the port that does.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_WaterRuntime_nativeOnTrimMemory<'caller>(
    mut unowned_env: jni::EnvUnowned<'caller>,
    _this: jni::objects::JObject<'caller>,
    _handle: jlong,
    level: jni::sys::jint,
) {
    let outcome = unowned_env.with_env(|_env| -> jni::errors::Result<()> {
        tracing::debug!(target: "waterui::runtime", level, "onTrimMemory");
        Ok(())
    });
    outcome.resolve::<crate::policy::ThrowRuntimeExAndDefault>();
}
