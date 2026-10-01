//! # Safety
//!
//! Resolving a symbol asserts that the bridge library exports it at the declared
//! signature — the contract between this crate and the bridge it is built
//! against. The `Library` outlives every pointer taken out of it.

use std::ffi::{c_char, c_double, c_int, c_uint, c_void};

pub const ABI_VERSION: u32 = 4;
pub const MAX_PLANES: usize = 4;

/// `WaterWpeFrame::kind` for a `WPEBufferDMABuf` frame.
pub const WATER_WPE_BUFFER_DMA_BUF: u32 = 1;
/// `WaterWpeFrame::kind` for a `WPEBufferSHM` frame.
pub const WATER_WPE_BUFFER_SHM: u32 = 2;

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
    /// One of the `WATER_WPE_BUFFER_*` constants: which `WPEBuffer` subclass
    /// produced the frame.
    pub kind: c_uint,
    /// `WATER_WPE_BUFFER_SHM` frames only: the buffer's pixels, borrowed for
    /// the token's lifetime. `format` is a DRM fourcc here too — the bridge
    /// translates WPE's `WPEPixelFormat`.
    pub shm_data: *const u8,
    pub shm_len: usize,
    pub shm_stride: c_uint,
}

pub type DestroyNotify = unsafe extern "C" fn(*mut c_void);
pub type EventCallback =
    unsafe extern "C" fn(*mut c_void, c_uint, *const c_char, *const c_char, c_double);
pub type FrameCallback = unsafe extern "C" fn(*mut c_void, *const WaterWpeFrame);
/// Receives the calling document's origin and one bridge envelope verbatim, and
/// returns the reply script.
pub type MessageCallback =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> WaterWpeBytes;
pub type ResultCallback = unsafe extern "C" fn(*mut c_void, bool, *const c_char, usize);

/// One answer to a `waterui://localhost` request. `headers` is `"Name: value"`
/// lines joined by `\n`; each [`WaterWpeBytes`] releases its own storage
/// through `destroy`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WaterWpeAssetResponse {
    pub status: c_uint,
    pub headers: WaterWpeBytes,
    pub body: WaterWpeBytes,
}

/// Answers one request on the page's asset origin from the engine's own
/// method and URI.
pub type AssetCallback =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> WaterWpeAssetResponse;

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
    pub page_set_asset_server:
        unsafe extern "C" fn(*mut WaterWpePage, AssetCallback, *mut c_void, DestroyNotify),
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
    pub page_evaluate: unsafe extern "C" fn(*mut WaterWpePage, *const c_char),
    pub page_add_script:
        unsafe extern "C" fn(*mut WaterWpePage, *const c_char, *const c_char, c_uint),
    pub page_set_cookie: unsafe extern "C" fn(*mut WaterWpePage, *const c_char),
    pub page_get_cookies: unsafe extern "C" fn(*mut WaterWpePage, ResultCallback, *mut c_void),
    pub page_run_javascript:
        unsafe extern "C" fn(*mut WaterWpePage, *const c_char, ResultCallback, *mut c_void),
    pub page_call_async_javascript:
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
                page_set_asset_server: symbol(library, b"water_wpe_page_set_asset_server\0"),
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
                page_evaluate: symbol(library, b"water_wpe_page_evaluate\0"),
                page_add_script: symbol(library, b"water_wpe_page_add_script\0"),
                page_set_cookie: symbol(library, b"water_wpe_page_set_cookie\0"),
                page_get_cookies: symbol(library, b"water_wpe_page_get_cookies\0"),
                page_run_javascript: symbol(library, b"water_wpe_page_run_javascript\0"),
                page_call_async_javascript: symbol(
                    library,
                    b"water_wpe_page_call_async_javascript\0",
                ),
                frame_presented: symbol(library, b"water_wpe_frame_presented\0"),
                frame_release: symbol(library, b"water_wpe_frame_release\0"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! `WaterWpeFrame` is one struct declared twice — once in
    //! `native/waterui_wpe.h` for the bridge, once above for Rust — and the two
    //! must agree field for field. Parsing the header keeps a drift on either
    //! side from landing silently: only a compiler would see the real C
    //! layout, and the bridge is only ever built inside CI.

    use std::mem::{align_of, offset_of, size_of};

    use super::{ABI_VERSION, MAX_PLANES, WaterWpeFrame};

    const HEADER: &str = include_str!("../native/waterui_wpe.h");

    fn header_define(name: &str) -> u64 {
        let prefix = format!("#define {name} ");
        let line = HEADER
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with(prefix.as_str()))
            .unwrap_or_else(|| panic!("{name} is not defined in waterui_wpe.h"));
        line[prefix.len()..]
            .trim()
            .parse()
            .unwrap_or_else(|error| panic!("{name} is not an integer: {error}"))
    }

    /// The members of `typedef struct { ... } Name;` as `(type, name, array)`
    /// triples, comments stripped and pointer stars folded into the type.
    fn header_struct_fields(name: &str) -> Vec<(String, String, Option<String>)> {
        let end = HEADER
            .find(&format!("}} {name};"))
            .unwrap_or_else(|| panic!("{name} is not declared in waterui_wpe.h"));
        let start = HEADER[..end].rfind("typedef struct {").map_or_else(
            || panic!("{name} is not a typedef struct"),
            |index| index + "typedef struct {".len(),
        );
        let mut body = String::new();
        let mut rest = &HEADER[start..end];
        while let Some(open) = rest.find("/*") {
            body.push_str(&rest[..open]);
            let close = rest[open..]
                .find("*/")
                .expect("waterui_wpe.h has an unterminated comment");
            rest = &rest[open + close + 2..];
        }
        body.push_str(rest);
        body.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                let line = line
                    .strip_suffix(';')
                    .unwrap_or_else(|| panic!("bad member declaration `{line}`"));
                let (declarator, array) = match line.split_once('[') {
                    Some((declarator, extent)) => (
                        declarator.trim_end(),
                        Some(
                            extent
                                .strip_suffix(']')
                                .expect("an array declarator must end with `]`")
                                .to_owned(),
                        ),
                    ),
                    None => (line, None),
                };
                let mut tokens: Vec<&str> = declarator.split_whitespace().collect();
                let mut name = tokens
                    .pop()
                    .expect("a member declaration needs a declarator");
                let mut pointers = 0;
                while let Some(stripped) = name.strip_prefix('*') {
                    pointers += 1;
                    name = stripped;
                }
                let mut ty = tokens.join(" ");
                for _ in 0..pointers {
                    ty.push_str(" *");
                }
                (ty, name.to_owned(), array)
            })
            .collect()
    }

    /// The `(size, align)` a C type carries on the platforms the bridge runs
    /// on, expressed through the equivalent Rust type.
    fn c_layout(ty: &str) -> (usize, usize) {
        match ty {
            "void *" | "const uint8_t *" => (size_of::<*const u8>(), align_of::<*const u8>()),
            "size_t" => (size_of::<usize>(), align_of::<usize>()),
            "uint64_t" => (size_of::<u64>(), align_of::<u64>()),
            "int" => (size_of::<i32>(), align_of::<i32>()),
            "uint32_t" => (size_of::<u32>(), align_of::<u32>()),
            other => panic!("the test needs a C layout entry for `{other}`"),
        }
    }

    fn rust_offset(name: &str) -> usize {
        match name {
            "token" => offset_of!(WaterWpeFrame, token),
            "width" => offset_of!(WaterWpeFrame, width),
            "height" => offset_of!(WaterWpeFrame, height),
            "format" => offset_of!(WaterWpeFrame, format),
            "modifier" => offset_of!(WaterWpeFrame, modifier),
            "n_planes" => offset_of!(WaterWpeFrame, n_planes),
            "fds" => offset_of!(WaterWpeFrame, fds),
            "offsets" => offset_of!(WaterWpeFrame, offsets),
            "strides" => offset_of!(WaterWpeFrame, strides),
            "rendering_fence_fd" => offset_of!(WaterWpeFrame, rendering_fence_fd),
            "kind" => offset_of!(WaterWpeFrame, kind),
            "shm_data" => offset_of!(WaterWpeFrame, shm_data),
            "shm_len" => offset_of!(WaterWpeFrame, shm_len),
            "shm_stride" => offset_of!(WaterWpeFrame, shm_stride),
            other => panic!("the test needs a Rust offset for field `{other}`"),
        }
    }

    #[test]
    fn water_wpe_frame_matches_the_bridge_header() {
        assert_eq!(
            header_define("WATER_WPE_ABI_VERSION"),
            u64::from(ABI_VERSION)
        );
        assert_eq!(header_define("WATER_WPE_MAX_PLANES"), MAX_PLANES as u64);

        let fields = header_struct_fields("WaterWpeFrame");
        let expected: &[(&str, &str)] = &[
            ("void *", "token"),
            ("uint32_t", "width"),
            ("uint32_t", "height"),
            ("uint32_t", "format"),
            ("uint64_t", "modifier"),
            ("uint32_t", "n_planes"),
            ("int", "fds"),
            ("uint32_t", "offsets"),
            ("uint32_t", "strides"),
            ("int", "rendering_fence_fd"),
            ("uint32_t", "kind"),
            ("const uint8_t *", "shm_data"),
            ("size_t", "shm_len"),
            ("uint32_t", "shm_stride"),
        ];
        assert_eq!(
            fields.len(),
            expected.len(),
            "WaterWpeFrame field count drifted"
        );

        // Walk the header's declarations the way a C compiler would: each
        // member sits at its alignment, then advances by size × extent.
        let mut offset = 0usize;
        let mut alignment = 1usize;
        for ((ty, name, array), &(expected_ty, expected_name)) in fields.iter().zip(expected.iter())
        {
            assert_eq!(
                (ty.as_str(), name.as_str()),
                (expected_ty, expected_name),
                "WaterWpeFrame declaration drifted"
            );
            let (size, align) = c_layout(ty);
            let count = array.as_deref().map_or(1, |extent| {
                usize::try_from(
                    extent
                        .parse::<u64>()
                        .unwrap_or_else(|_| header_define(extent)),
                )
                .expect("an array extent must fit usize")
            });
            offset = offset.next_multiple_of(align);
            assert_eq!(
                offset,
                rust_offset(name),
                "field `{name}` sits at a different offset in the header and the Rust struct"
            );
            offset += size * count;
            alignment = alignment.max(align);
        }
        assert_eq!(
            offset.next_multiple_of(alignment),
            size_of::<WaterWpeFrame>(),
            "WaterWpeFrame size drifted between the header and the Rust struct"
        );
    }
}
