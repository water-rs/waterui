//! Map entry points for builds that left the map out.
//!
//! The Apple backend is one prebuilt Swift package. It is compiled once, with
//! no knowledge of which Cargo features the application it links against chose,
//! so every symbol it names has to exist — and it names the map's reactive
//! entry points unconditionally, from the generic watcher plumbing. Without
//! these definitions an application that does not use maps fails to link at
//! all, which is exactly what happened: `_waterui_new_watcher_region`,
//! undefined, in an application with no map in it.
//!
//! Compiling the real map instead would defeat the point of the feature —
//! nothing of `waterui-map` would ever be stripped. So the names exist and the
//! bodies do not: no map view can be built without the feature, so nothing can
//! reach one of these. Calling one anyway means the C ABI and the feature set
//! disagree, which is a build-configuration bug and says so.

use core::ffi::c_void;

/// Defines the four entry points [`crate::ffi_computed!`] generates for a type,
/// as symbols that exist for the linker and refuse to run.
macro_rules! absent_computed {
    ($($ident:ident),+ $(,)?) => {
        pastey::paste! {
            $(
                #[doc = concat!("Absent `", stringify!($ident), "` computed reader.")]
                ///
                /// # Safety
                ///
                /// Never call this: it exists only so a map-free build links.
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn [< waterui_read_computed_ $ident >](
                    _computed: *const c_void,
                ) -> *mut c_void {
                    absent(stringify!($ident))
                }

                #[doc = concat!("Absent `", stringify!($ident), "` computed watcher.")]
                ///
                /// # Safety
                ///
                /// Never call this: it exists only so a map-free build links.
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn [< waterui_watch_computed_ $ident >](
                    _computed: *const c_void,
                    _watcher: *mut c_void,
                ) -> *mut c_void {
                    absent(stringify!($ident))
                }

                #[doc = concat!("Absent `", stringify!($ident), "` computed drop.")]
                ///
                /// # Safety
                ///
                /// Never call this: it exists only so a map-free build links.
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn [< waterui_drop_computed_ $ident >](
                    _computed: *mut c_void,
                ) {
                    absent(stringify!($ident));
                }

                #[doc = concat!("Absent `", stringify!($ident), "` watcher constructor.")]
                ///
                /// # Safety
                ///
                /// Never call this: it exists only so a map-free build links.
                #[unsafe(no_mangle)]
                pub unsafe extern "C" fn [< waterui_new_watcher_ $ident >](
                    _data: *mut c_void,
                    _call: *mut c_void,
                    _drop: *mut c_void,
                ) -> *mut c_void {
                    absent(stringify!($ident))
                }
            )+
        }
    };
}

/// Reports that the C ABI and the feature set disagree.
///
/// Reaching this means a backend called a map entry point in a build compiled
/// without maps, which no view tree can produce — so it is a wiring bug, not a
/// user error, and it fails loudly rather than returning something invented.
#[cold]
#[track_caller]
fn absent(entry: &str) -> ! {
    panic!(
        "waterui: `{entry}` was called in a build compiled without the `map` feature. \
         Enable `map` on the `waterui` dependency, or stop calling it."
    )
}

absent_computed!(region, annotations, user_location);
