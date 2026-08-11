//! # Safety
//!
//! Resolving a symbol asserts that the bridge library exports it at the declared
//! signature — the contract between this crate and the bridge it is built
//! against. The `Library` outlives every pointer taken out of it.

use std::ffi::{c_char, c_double, c_int, c_uint, c_void};

pub const ABI_VERSION: u32 = 1;
pub const MAX_PLANES: usize = 4;

#[repr(C)]
pub struct WaterWpeRuntime {
    _private: [u8; 0],
}

#[repr(C)]
pub struct WaterWpePage {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WaterWpeBytes {
    pub data: *const u8,
    pub len: usize,
    pub user_data: *mut c_void,
    pub destroy: Option<unsafe extern "C" fn(*mut c_void)>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WaterWpeFrame {
    pub token: *mut c_void,
    pub width: c_uint,
    pub height: c_uint,
    pub format: c_uint,
    pub modifier: u64,
    pub n_planes: c_uint,
    pub fds: [c_int; MAX_PLANES],
    pub offsets: [c_uint; MAX_PLANES],
    pub strides: [c_uint; MAX_PLANES],
    pub rendering_fence_fd: c_int,
}

pub type DestroyNotify = unsafe extern "C" fn(*mut c_void);
pub type EventCallback =
    unsafe extern "C" fn(*mut c_void, c_uint, *const c_char, *const c_char, c_double);
pub type FrameCallback = unsafe extern "C" fn(*mut c_void, *const WaterWpeFrame);
/// Receives one bridge envelope verbatim and returns the reply script.
pub type MessageCallback = unsafe extern "C" fn(*mut c_void, *const c_char) -> WaterWpeBytes;
pub type ResultCallback = unsafe extern "C" fn(*mut c_void, bool, *const c_char, usize);

pub struct WpeApi {
    pub abi_version: unsafe extern "C" fn() -> c_uint,
    pub runtime_new: unsafe extern "C" fn(*mut *mut c_char) -> *mut WaterWpeRuntime,
    pub runtime_free: unsafe extern "C" fn(*mut WaterWpeRuntime),
    pub runtime_iteration: unsafe extern "C" fn(*mut WaterWpeRuntime) -> bool,
    pub string_free: unsafe extern "C" fn(*mut c_char),
    pub page_new: unsafe extern "C" fn(
        *mut WaterWpeRuntime,
        EventCallback,
        FrameCallback,
        MessageCallback,
        *mut c_void,
        DestroyNotify,
        *mut *mut c_char,
    ) -> *mut WaterWpePage,
    pub page_free: unsafe extern "C" fn(*mut WaterWpePage),
    pub page_load_uri: unsafe extern "C" fn(*mut WaterWpePage, *const c_char),
    pub page_go_back: unsafe extern "C" fn(*mut WaterWpePage),
    pub page_go_forward: unsafe extern "C" fn(*mut WaterWpePage),
    pub page_stop: unsafe extern "C" fn(*mut WaterWpePage),
    pub page_reload: unsafe extern "C" fn(*mut WaterWpePage),
    pub page_can_go_back: unsafe extern "C" fn(*mut WaterWpePage) -> bool,
    pub page_can_go_forward: unsafe extern "C" fn(*mut WaterWpePage) -> bool,
    pub page_set_redirects_enabled: unsafe extern "C" fn(*mut WaterWpePage, bool),
    pub page_set_user_agent: unsafe extern "C" fn(*mut WaterWpePage, *const c_char),
    pub page_resize: unsafe extern "C" fn(*mut WaterWpePage, c_uint, c_uint, c_double),
    pub page_set_focus: unsafe extern "C" fn(*mut WaterWpePage, bool),
    pub page_pointer_button:
        unsafe extern "C" fn(*mut WaterWpePage, bool, c_uint, c_double, c_double, c_uint, c_uint),
    pub page_pointer_move: unsafe extern "C" fn(
        *mut WaterWpePage,
        c_double,
        c_double,
        c_double,
        c_double,
        c_uint,
        c_uint,
    ),
    pub page_scroll: unsafe extern "C" fn(
        *mut WaterWpePage,
        c_double,
        c_double,
        c_double,
        c_double,
        bool,
        bool,
        c_uint,
        c_uint,
    ),
    pub page_key: unsafe extern "C" fn(*mut WaterWpePage, bool, c_uint, c_uint, c_uint, c_uint),
    pub page_add_script: unsafe extern "C" fn(*mut WaterWpePage, *const c_char, c_uint),
    pub page_set_cookie: unsafe extern "C" fn(*mut WaterWpePage, *const c_char),
    pub page_get_cookies: unsafe extern "C" fn(*mut WaterWpePage, ResultCallback, *mut c_void),
    pub page_run_javascript:
        unsafe extern "C" fn(*mut WaterWpePage, *const c_char, ResultCallback, *mut c_void),
    pub frame_presented: unsafe extern "C" fn(*mut c_void),
    pub frame_release: unsafe extern "C" fn(*mut c_void, c_int),
}

impl WpeApi {
    pub unsafe fn load(library: &libloading::Library) -> Self {
        unsafe fn symbol<T: Copy>(library: &libloading::Library, name: &[u8]) -> T {
            // SAFETY: symbol resolved from the bridge library, at the signature
            // declared here; see the module safety note.
            *unsafe {
                library
                    .get::<T>(name)
                    .unwrap_or_else(|error| panic!("bundled WPE ABI symbol is missing: {error}"))
            }
        }

        // SAFETY: symbol resolved from the bridge library, at the signature declared
        // here; see the module safety note.
        unsafe {
            Self {
                abi_version: symbol(library, b"water_wpe_abi_version\0"),
                runtime_new: symbol(library, b"water_wpe_runtime_new\0"),
                runtime_free: symbol(library, b"water_wpe_runtime_free\0"),
                runtime_iteration: symbol(library, b"water_wpe_runtime_iteration\0"),
                string_free: symbol(library, b"water_wpe_string_free\0"),
                page_new: symbol(library, b"water_wpe_page_new\0"),
                page_free: symbol(library, b"water_wpe_page_free\0"),
                page_load_uri: symbol(library, b"water_wpe_page_load_uri\0"),
                page_go_back: symbol(library, b"water_wpe_page_go_back\0"),
                page_go_forward: symbol(library, b"water_wpe_page_go_forward\0"),
                page_stop: symbol(library, b"water_wpe_page_stop\0"),
                page_reload: symbol(library, b"water_wpe_page_reload\0"),
                page_can_go_back: symbol(library, b"water_wpe_page_can_go_back\0"),
                page_can_go_forward: symbol(library, b"water_wpe_page_can_go_forward\0"),
                page_set_redirects_enabled: symbol(
                    library,
                    b"water_wpe_page_set_redirects_enabled\0",
                ),
                page_set_user_agent: symbol(library, b"water_wpe_page_set_user_agent\0"),
                page_resize: symbol(library, b"water_wpe_page_resize\0"),
                page_set_focus: symbol(library, b"water_wpe_page_set_focus\0"),
                page_pointer_button: symbol(library, b"water_wpe_page_pointer_button\0"),
                page_pointer_move: symbol(library, b"water_wpe_page_pointer_move\0"),
                page_scroll: symbol(library, b"water_wpe_page_scroll\0"),
                page_key: symbol(library, b"water_wpe_page_key\0"),
                page_add_script: symbol(library, b"water_wpe_page_add_script\0"),
                page_set_cookie: symbol(library, b"water_wpe_page_set_cookie\0"),
                page_get_cookies: symbol(library, b"water_wpe_page_get_cookies\0"),
                page_run_javascript: symbol(library, b"water_wpe_page_run_javascript\0"),
                frame_presented: symbol(library, b"water_wpe_frame_presented\0"),
                frame_release: symbol(library, b"water_wpe_frame_release\0"),
            }
        }
    }
}
