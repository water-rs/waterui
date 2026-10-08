//! The Android host runner (`src/runner/android`).
//!
//! Hydrolysis on Android is a Kotlin host (`android/host`, `android/gpu`)
//! owning a native session built here: one `AndroidHostWindow` of host
//! services behind a `RuntimeWindow`, one `AndroidSurface` GPU attachment
//! the Cherenkov engine presents through, one eventfd-woken executor on the
//! main `ALooper`, and one JNI surface (`jni.rs`) every entry point crosses.
//!
//! The app's own `cdylib` registers its app factory from `JNI_OnLoad`:
//!
//! ```ignore
//! hydrolysis::android::register_app(|| {
//!     (build_app(), Rc::new(Material3::defaults()) as Rc<dyn Style>)
//! });
//! ```
//!
//! Logging installs at `nativeInit`, which also receives the launch intent's
//! `waterui.log.level` extra — the level the CLI's `--logs` asked for.
//!
//! Session create/destroy, metrics, surface attach/resize/destroy, frame
//! transactions and input then arrive through `NativeBridge`'s JNI calls —
//! the Kotlin side never touches this module's internals directly.

mod accessibility;
mod gpu;
mod host;
mod ime;
pub(crate) mod jni;
mod platform_views;

use std::rc::Rc;
use std::sync::OnceLock;

use tracing::level_filters::LevelFilter;
use tracing_log::AsLog;
use waterui::app::App;

/// The app factory the Kotlin host instantiates per session create.
///
/// A factory is a plain `Fn()` — the app cdylib registers it once at
/// `JNI_OnLoad`, and every `nativeCreateSession` calls it again so a
/// recreated activity gets a fresh `App`, not a reused tree.
type AppFactory = Box<dyn Fn() -> (App, Rc<dyn crate::Style>) + Send + Sync>;

static APP_FACTORY: OnceLock<AppFactory> = OnceLock::new();

/// Registers the app the Kotlin host mounts. Called once, from the app
/// cdylib's `JNI_OnLoad` — before that, `nativeCreateSession` fails with an
/// explicit error rather than mounting an empty window.
///
/// # Panics
/// Panics when called a second time: the factory is a singleton.
pub fn register_app(factory: impl Fn() -> (App, Rc<dyn crate::Style>) + Send + Sync + 'static) {
    let factory: AppFactory = Box::new(factory);
    // `register_app` may run on a non-UI JNI thread; the factory moves to
    // the UI thread on the first create.
    assert!(
        APP_FACTORY.set(factory).is_ok(),
        "hydrolysis android: register_app was called twice"
    );
}

/// Installs Android logging once: `log` records (wgpu, ndk) and `tracing`
/// records both land in logcat under the app's tag.
///
/// `level` is the launch intent's `waterui.log.level` extra the Kotlin host
/// passes through `nativeInit` — the CLI's `--logs` choice. `None` keeps the
/// INFO ceiling a launch without `--logs` has always had.
///
/// A panic hook forwards panic payloads there too — Android doesn't capture
/// stderr, so without this a native panic surfaces as a bare JNI exception
/// with no cause.
pub(crate) fn init_logging(level: Option<LevelFilter>) {
    let level = level.unwrap_or(LevelFilter::INFO);
    android_logger::init_once(android_logger::Config::default().with_max_level(level.as_log()));
    std::panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic");
        let location = info.location().map_or_else(
            || "unknown location".to_owned(),
            |location| format!("{location}"),
        );
        // Write through __android_log_write rather than `log` — a panic must
        // never depend on logger state to reach logcat.
        android_log(format!("panic at {location}: {payload}").as_bytes());
    }));
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(AndroidLogWriter)
        .with_ansi(false)
        .try_init();
    // Route `tracing` records emitted on `log`-subscribed spans through the
    // same logcat writer.
    tracing_log::LogTracer::init().ok();
}

/// A `tracing` writer that forwards each record to `__android_log_write`
/// under the `hydrolysis` tag.
struct AndroidLogWriter;

impl std::io::Write for AndroidLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        android_log(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for AndroidLogWriter {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        Self
    }
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: i32, tag: *const u8, text: *const u8) -> i32;
}

fn android_log(message: &[u8]) {
    // SAFETY: __android_log_write takes (priority, tag, text); tag/text are
    // NUL-terminated — a copy with a trailing NUL is sound and temporary.
    let tag = c"hydrolysis";
    let mut text = Vec::with_capacity(message.len() + 1);
    text.extend_from_slice(message);
    for byte in &mut text {
        if *byte == 0 {
            *byte = b' ';
        }
    }
    text.push(0);
    // SAFETY: `tag` is a C string literal and `text` was NUL-terminated
    // above; both pointers stay valid for the call.
    unsafe {
        __android_log_write(4, tag.as_ptr().cast(), text.as_ptr().cast());
    }
}

/// The registered factory's next `(App, Style)` pair — the entry point the
/// session-create path calls on the UI thread.
pub(crate) fn instantiate_app() -> (App, Rc<dyn crate::Style>) {
    let factory = APP_FACTORY.get().unwrap_or_else(|| {
        panic!(
            "hydrolysis android: no app registered — the app cdylib must call \
             hydrolysis::android::register_app from JNI_OnLoad"
        )
    });
    factory()
}
