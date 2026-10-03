//! Application mount: the work `waterui_apple_main` does for Apple, minus
//! the never-returning run loop — Android drives `nativeCreate` from the
//! activity's `onCreate` and hands back an opaque handle.

use alloc::boxed::Box;

use jni::objects::{Global, JObject};
use jni::sys::jlong;
use jni::Env;
use waterui::app::App;
use waterui_backend_core::Environment;
use waterui_locale::Locale;

use crate::contract::{KeepAlive, Mounted, PlatformView};
use crate::jvm;
use crate::theme::ThemeSignals;

/// What a mounted application keeps alive: the environment signals resolve
/// through, the mounted content leaf, and the platform-feed handles the
/// lifecycle calls push into.
struct Runtime {
    env: Environment,
    root: PlatformView,
    content: Mounted,
    theme: ThemeSignals,
    locale: waterui::reactive::Binding<Locale>,
    /// Everything the window kept alive — the background watcher among it.
    keepalive: KeepAlive,
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
pub(crate) fn mount(
    env: &mut Env<'_>,
    activity: JObject<'_>,
    root: JObject<'_>,
    app: impl FnOnce(Environment) -> App,
) -> jni::errors::Result<jlong> {
    // `EnvUnowned::with_env` has already initialized the `JavaVM` singleton —
    // every later attach is a TLS check.
    jvm::install(env, &activity)?;
    jvm::refresh_metrics(env)?;
    let inspector = crate::startup::initialize(env);
    let root = env.new_global_ref(root)?;

    let mut env_rust = Environment::new();
    waterui::inspector::install(&mut env_rust, inspector);
    waterui::text::install_system_font_collection(&mut env_rust);
    crate::dispatch::install(&mut env_rust);
    let theme = crate::theme::install(&mut env_rust);
    let locale = crate::locale::install(&mut env_rust);

    let parts = app(env_rust).into_parts();
    let mut app_env = parts.env;
    // The skeleton embeds the first window; multi-window applications are a
    // later port's surface.
    let Some(window) = parts.windows.into_iter().next() else {
        panic!("a WaterUI app declares at least one window");
    };

    // Content renders under the environment `app` returned: the app's own
    // installs landed as overlays on the clone it was handed — `insert`
    // never propagates between clones.
    let mut keepalive = KeepAlive::default();
    let content = crate::embedding::mount_content(window.build_content(), &root, &app_env)?;
    // The resolved window background is painted onto the host root behind
    // the content leaf, and kept tracking its signal — the watcher needs its
    // own reference to the root, which the runtime also holds.
    let watched_root = env.new_global_ref(root.as_ref())?;
    keepalive.bind(&window.resolved_background(&app_env), move |color| {
        crate::embedding::set_root_background(&watched_root, color);
    });

    let runtime = Box::new(Runtime {
        env: app_env,
        root,
        content,
        theme,
        locale,
        keepalive,
    });
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a jlong is 64 bits on every Android target"
    )]
    Ok(Box::into_raw(runtime) as jlong)
}

/// The `Runtime` behind `handle` — the JNI border the lifecycle calls cross.
///
/// # Safety
///
/// `handle` is the `jlong` `mount` returned and has not been destroyed.
unsafe fn runtime<'a>(handle: jlong) -> &'a mut Runtime {
    // SAFETY: the caller's contract — a handle `mount` minted and
    // `nativeDestroy` has not consumed.
    unsafe { &mut *(handle as usize as *mut Runtime) }
}

/// `WaterRuntime.nativeDestroy` — tears the runtime down and frees its
/// handle. Called once, on the main thread, from `WaterActivity.onDestroy`.
///
/// # Safety
///
/// The handle is consumed: callers must not touch it again.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_WaterRuntime_nativeDestroy<'caller>(
    unowned_env: jni::EnvUnowned<'caller>,
    _this: jni::objects::JObject<'caller>,
    handle: jlong,
) {
    let outcome = unowned_env.with_env(|_env| -> jni::errors::Result<()> {
        // SAFETY: per the host's contract, `handle` is a live runtime and is
        // never used after this call.
        drop(unsafe { Box::from_raw(handle as usize as *mut Runtime) });
        Ok(())
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// `WaterRuntime.nativeOnConfigurationChanged` — pushes the platform's new
/// appearance into the theme and locale bindings.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_WaterRuntime_nativeOnConfigurationChanged<
    'caller,
>(
    unowned_env: jni::EnvUnowned<'caller>,
    _this: jni::objects::JObject<'caller>,
    handle: jlong,
) {
    let outcome = unowned_env.with_env(|_env| -> jni::errors::Result<()> {
        // SAFETY: the handle stays live until nativeDestroy.
        let runtime = unsafe { runtime(handle) };
        crate::theme::refresh(&runtime.theme);
        crate::locale::refresh(&runtime.locale);
        Ok(())
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}

/// `WaterRuntime.nativeOnTrimMemory` — memory pressure forwarded for the
/// backend's caches; the skeleton has no caches to shed, so the call is a
/// no-op that keeps the JNI surface pinned for the port that does.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_WaterRuntime_nativeOnTrimMemory<'caller>(
    unowned_env: jni::EnvUnowned<'caller>,
    _this: jni::objects::JObject<'caller>,
    _handle: jlong,
    level: jni::sys::jint,
) {
    let outcome = unowned_env.with_env(|_env| -> jni::errors::Result<()> {
        tracing::debug!(target: "waterui::runtime", level, "onTrimMemory");
        Ok(())
    });
    outcome.resolve::<jni::errors::ThrowRuntimeExAndDefault>();
}
