use waterui::app::App;
use waterui_controls::menu::resolve_menu_bar_items;

use crate::{IntoFFI, WuiEnv, array::WuiArray, views::WuiAnyViews, window::WuiWindow};
#[cfg(any(feature = "android-jni", test))]
use crate::{WuiAnyView, window::OwnedFfiHandle};
#[cfg(feature = "android-jni")]
use core::ffi::c_void;

/// FFI-compatible representation of an application.
///
/// This struct is returned by value from `waterui_app()`.
/// Native code can read fields directly.
#[repr(C)]
#[derive(Debug)]
pub struct WuiApp {
    /// Array of windows. The first window is the main window.
    pub windows: WuiArray<WuiWindow>,
    /// The application menu bar as resolved menu items.
    pub menu_bar: *mut WuiAnyViews,
    /// The application environment containing injected services.
    /// Returned to native for use during rendering.
    pub env: *mut WuiEnv,
}

/// Raw handles transferred from the exported app entry point to Android JNI.
#[cfg(feature = "android-jni")]
#[doc(hidden)]
#[repr(C)]
pub struct WuiAndroidAppHandles {
    pub content: *mut c_void,
    pub env: *mut c_void,
}

/// The two handles transferred to Android's single root activity.
#[cfg(any(feature = "android-jni", test))]
pub(crate) struct WuiAndroidApp {
    content: OwnedFfiHandle<WuiAnyView>,
    env: OwnedFfiHandle<WuiEnv>,
}

#[cfg(any(feature = "android-jni", test))]
impl WuiAndroidApp {
    pub(crate) fn into_raw_parts(self) -> (*mut WuiAnyView, *mut WuiEnv) {
        (self.content.into_raw(), self.env.into_raw())
    }
}

#[cfg(any(feature = "android-jni", test))]
impl WuiApp {
    /// Projects a `WaterUI` app onto Android's single-activity model.
    ///
    /// Android owns exactly one root content view and its environment. Window
    /// chrome, menu-bar, sizing, and state handles have no Android owner and are
    /// released before the two supported handles cross JNI.
    pub(crate) fn into_android_projection(self) -> WuiAndroidApp {
        let Self {
            mut windows,
            menu_bar,
            env,
        } = self;
        let menu_bar = OwnedFfiHandle::required(menu_bar, "WuiApp.menu_bar");
        let env = OwnedFfiHandle::required(env, "WuiApp.env");
        let window_count = windows.len();

        if window_count != 1 {
            for window in windows.as_mut_slice() {
                // SAFETY: `window` points at an initialized element of the array the
                // caller handed over, and each element is read once.
                let window = unsafe { core::ptr::read(window) };
                window.dispose_android();
            }
            windows.consume();
            panic!("Android backend requires exactly one window, got {window_count}");
        }

        // SAFETY: the branch above proves the array is non-empty, so the first element
        // is initialized; `consume` below frees the buffer without dropping elements,
        // so this moves out exactly once.
        let window = unsafe { windows.as_mut_slice().as_mut_ptr().read() };
        windows.consume();
        drop(menu_bar);

        WuiAndroidApp {
            content: window.into_android_content(),
            env,
        }
    }

    /// Projects and transfers the two Android-owned handles across the C ABI.
    #[cfg(feature = "android-jni")]
    #[doc(hidden)]
    pub fn into_android_handles(self) -> WuiAndroidAppHandles {
        let (content, env) = self.into_android_projection().into_raw_parts();
        WuiAndroidAppHandles {
            content: content.cast(),
            env: env.cast(),
        }
    }
}

impl IntoFFI for App {
    type FFI = WuiApp;

    fn into_ffi(self) -> Self::FFI {
        let (windows, menu_bar, env) = self.into_parts();
        let menu_bar = crate::menu_items_views(resolve_menu_bar_items(&menu_bar, &env));
        WuiApp {
            windows: windows.into_ffi(),
            menu_bar,
            env: env.into_ffi(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        any::Any as StdAny,
        cell::Cell,
        panic::{AssertUnwindSafe, catch_unwind},
        rc::Rc,
    };

    use nami::{Computed, Signal, binding, watcher::Context};
    use waterui::{AnyView, Environment, View, window::WindowState};
    use waterui_controls::menu::ResolvedMenuItem;

    use super::*;
    use crate::{IntoRust, window::WuiWindow};

    struct DropProbe(Rc<Cell<usize>>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    struct TrackedView(Rc<Cell<usize>>);

    impl Drop for TrackedView {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    impl View for TrackedView {
        fn body(self, _env: &Environment) -> impl View {}
    }

    #[derive(Clone)]
    struct TrackedMenuSignal {
        _probe: Rc<DropProbe>,
    }

    impl Signal for TrackedMenuSignal {
        type Output = Vec<ResolvedMenuItem>;
        type Guard = ();

        fn get(&self) -> Self::Output {
            Vec::new()
        }

        fn watch(&self, _watcher: impl Fn(Context<Self::Output>) + 'static) {}
    }

    struct CountingWindowStorage {
        windows: Vec<WuiWindow>,
        drops: Rc<Cell<usize>>,
    }

    impl AsRef<[WuiWindow]> for CountingWindowStorage {
        fn as_ref(&self) -> &[WuiWindow] {
            &self.windows
        }
    }

    impl Drop for CountingWindowStorage {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    fn tracked_window(toolbar_drops: &Rc<Cell<usize>>) -> WuiWindow {
        let mut window =
            waterui::window::Window::new("Android projection", binding(WindowState::Normal), || ())
                .into_ffi();
        window.toolbar = AnyView::new(TrackedView(toolbar_drops.clone())).into_ffi();
        window
    }

    fn tracked_app(
        windows: Vec<WuiWindow>,
        storage_drops: &Rc<Cell<usize>>,
        menu_drops: &Rc<Cell<usize>>,
        env_drops: &Rc<Cell<usize>>,
    ) -> WuiApp {
        let menu_bar = crate::menu_items_views(Computed::new(TrackedMenuSignal {
            _probe: Rc::new(DropProbe(menu_drops.clone())),
        }));
        let mut env = Environment::new();
        env.insert(DropProbe(env_drops.clone()));

        WuiApp {
            windows: WuiArray::new(CountingWindowStorage {
                windows,
                drops: storage_drops.clone(),
            }),
            menu_bar,
            env: env.into_ffi(),
        }
    }

    fn panic_message(payload: &(dyn StdAny + Send)) -> &str {
        if let Some(message) = payload.downcast_ref::<String>() {
            message
        } else if let Some(message) = payload.downcast_ref::<&'static str>() {
            message
        } else {
            panic!("Android window-count panic did not contain a string message");
        }
    }

    #[test]
    fn android_projection_moves_content_and_consumes_owned_storage() {
        let storage_drops = Rc::new(Cell::new(0));
        let toolbar_drops = Rc::new(Cell::new(0));
        let menu_drops = Rc::new(Cell::new(0));
        let env_drops = Rc::new(Cell::new(0));
        let app = tracked_app(
            vec![tracked_window(&toolbar_drops)],
            &storage_drops,
            &menu_drops,
            &env_drops,
        );

        let projection = app.into_android_projection();

        assert_eq!(storage_drops.get(), 1);
        assert_eq!(toolbar_drops.get(), 1);
        assert_eq!(menu_drops.get(), 1);
        assert_eq!(env_drops.get(), 0);
        assert!(!projection.content.as_ptr().is_null());
        assert!(!projection.env.as_ptr().is_null());

        let (content, env) = projection.into_raw_parts();
        // SAFETY: the caller contract makes `content` an owning handle consumed here.
        let content: AnyView = unsafe { IntoRust::into_rust(content) };
        // SAFETY: likewise for `env`.
        let env: Environment = unsafe { IntoRust::into_rust(env) };
        drop(content);
        drop(env);

        assert_eq!(env_drops.get(), 1);
    }

    #[test]
    fn android_projection_rejects_every_non_single_window_count_and_releases_ownership() {
        for window_count in [0_usize, 2] {
            let storage_drops = Rc::new(Cell::new(0));
            let toolbar_drops = Rc::new(Cell::new(0));
            let menu_drops = Rc::new(Cell::new(0));
            let env_drops = Rc::new(Cell::new(0));
            let windows = (0..window_count)
                .map(|_| tracked_window(&toolbar_drops))
                .collect();
            let app = tracked_app(windows, &storage_drops, &menu_drops, &env_drops);

            let result = catch_unwind(AssertUnwindSafe(|| app.into_android_projection()));
            let Err(payload) = result else {
                panic!("non-single-window Android app must panic")
            };

            assert_eq!(
                panic_message(payload.as_ref()),
                format!("Android backend requires exactly one window, got {window_count}")
            );
            assert_eq!(storage_drops.get(), 1);
            assert_eq!(toolbar_drops.get(), window_count);
            assert_eq!(menu_drops.get(), 1);
            assert_eq!(env_drops.get(), 1);
        }
    }
}
