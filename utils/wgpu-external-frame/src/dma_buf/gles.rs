//! DMA-BUF import through `EGL_LINUX_DMA_BUF_EXT` and
//! `glEGLImageTargetTexture2DOES`.

use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::os::fd::AsRawFd as _;

use glow::HasContext as _;

use super::frame::{DRM_FORMAT_MOD_INVALID, DmaBufFrame};

type EglDisplay = *mut c_void;
type EglImage = *mut c_void;
type EglGetCurrentDisplay = unsafe extern "C" fn() -> EglDisplay;
type EglCreateImage =
    unsafe extern "C" fn(EglDisplay, *mut c_void, c_uint, *mut c_void, *const c_int) -> EglImage;
type EglDestroyImage = unsafe extern "C" fn(EglDisplay, EglImage) -> c_uint;
type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;
type GlEglImageTargetTexture2d = unsafe extern "C" fn(c_uint, EglImage);

const EGL_NONE: c_int = 0x3038;
const EGL_WIDTH: c_int = 0x3057;
const EGL_HEIGHT: c_int = 0x3056;
const EGL_LINUX_DMA_BUF_EXT: c_uint = 0x3270;
const EGL_LINUX_DRM_FOURCC_EXT: c_int = 0x3271;
const EGL_DMA_BUF_PLANE0_FD_EXT: c_int = 0x3272;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: c_int = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: c_int = 0x3274;
const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: c_int = 0x3443;
const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: c_int = 0x3444;

/// The EGL and GLES entry points a DMA-BUF import needs, resolved once.
pub(super) struct GlesInterop {
    gl: glow::Context,
    egl_get_current_display: EglGetCurrentDisplay,
    egl_create_image: EglCreateImage,
    egl_destroy_image: EglDestroyImage,
    image_target_texture: GlEglImageTargetTexture2d,
    _egl_library: libloading::Library,
    _gles_library: libloading::Library,
}

impl core::fmt::Debug for GlesInterop {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GlesInterop")
            .finish_non_exhaustive()
    }
}

impl GlesInterop {
    pub(super) fn new() -> Self {
        // SAFETY: `Library::new` is unsafe because `dlopen` runs the library's
        // initializers, which can execute arbitrary code. These are the two
        // system EGL/GLES runtime libraries named by their versioned SONAMEs,
        // not a caller-supplied path, and the process is already running on
        // them: wgpu's GLES backend opened the same libraries to create the
        // device this interop serves, so `dlopen` returns the existing handle
        // and no new initializer runs. Both handles are stored in `Self`, so
        // every symbol resolved below stays valid for as long as it is callable.
        let egl_library = unsafe { libloading::Library::new("libEGL.so.1") }
            .unwrap_or_else(|error| panic!("failed to load libEGL.so.1: {error}"));
        // SAFETY: as above, for the GLES runtime.
        let gles_library = unsafe { libloading::Library::new("libGLESv2.so.2") }
            .unwrap_or_else(|error| panic!("failed to load libGLESv2.so.2: {error}"));
        // SAFETY: `EglGetProcAddress` is the signature of `eglGetProcAddress`,
        // which is the name being resolved, satisfying `load_library_symbol`'s
        // contract. A platform that does not export it panics inside the loader
        // rather than returning a null pointer to call through.
        let get_proc = unsafe {
            load_library_symbol::<EglGetProcAddress>(
                &[&egl_library],
                b"eglGetProcAddress\0",
                "eglGetProcAddress",
            )
        };
        // SAFETY: `EglGetCurrentDisplay` is the signature of
        // `eglGetCurrentDisplay`, the name resolved here, which is what
        // `load_egl_symbol` requires; an absent name panics in the loader.
        let egl_get_current_display = unsafe {
            load_egl_symbol::<EglGetCurrentDisplay>(
                &egl_library,
                &gles_library,
                get_proc,
                "eglGetCurrentDisplay",
            )
        };
        // SAFETY: `EglCreateImage` is the signature of `eglCreateImageKHR`, the
        // name resolved here; an absent name panics in the loader.
        let egl_create_image = unsafe {
            load_egl_symbol::<EglCreateImage>(
                &egl_library,
                &gles_library,
                get_proc,
                "eglCreateImageKHR",
            )
        };
        // SAFETY: `EglDestroyImage` is the signature of `eglDestroyImageKHR`,
        // the name resolved here; an absent name panics in the loader.
        let egl_destroy_image = unsafe {
            load_egl_symbol::<EglDestroyImage>(
                &egl_library,
                &gles_library,
                get_proc,
                "eglDestroyImageKHR",
            )
        };
        // SAFETY: `GlEglImageTargetTexture2d` is the signature of
        // `glEGLImageTargetTexture2DOES`, the name resolved here. This one is an
        // extension entry point, so it usually arrives through
        // `eglGetProcAddress` rather than `dlsym`; either way the loader panics
        // if the driver lacks it.
        let image_target_texture = unsafe {
            load_egl_symbol::<GlEglImageTargetTexture2d>(
                &egl_library,
                &gles_library,
                get_proc,
                "glEGLImageTargetTexture2DOES",
            )
        };
        // SAFETY: `from_loader_function` requires the loader to return either a
        // null pointer or a pointer to a function with the signature glow
        // expects for that name. `load_egl_address` resolves names only through
        // `dlsym` on the two GL libraries above and `eglGetProcAddress`, so any
        // non-null result is the platform's own implementation of exactly that
        // GL entry point; unknown names come back null, which glow records as
        // unavailable rather than calling. glow copies the pointers out during
        // this call, and the libraries backing them are kept alive by the
        // handles moved into `Self` below.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                load_egl_address(&egl_library, &gles_library, get_proc, name)
            })
        };
        Self {
            gl,
            egl_get_current_display,
            egl_create_image,
            egl_destroy_image,
            image_target_texture,
            _egl_library: egl_library,
            _gles_library: gles_library,
        }
    }

    /// Blocks until every GL command recorded so far has completed.
    pub(super) fn finish(&self) {
        // SAFETY: every glow entry point is unsafe because it requires the GL
        // context its function pointers were loaded from to be current on this
        // thread. `GlesInterop` is neither `Send` nor `Sync` (it holds the
        // `libloading` handles and raw EGL pointers) and is reached here only
        // through `&self` on the thread where wgpu's GLES device keeps its
        // context current. `glFinish` takes no arguments, so currency is the
        // only precondition; it is what makes a preceding copy complete before
        // the producer is told the buffer is free.
        unsafe {
            self.gl.finish();
        }
    }

    pub(super) fn copy_dma_buf(&self, frame: &DmaBufFrame, destination: &wgpu::Texture) {
        // SAFETY: `egl_get_current_display` holds the address `dlsym`/
        // `eglGetProcAddress` returned for `eglGetCurrentDisplay`, whose EGL
        // signature is exactly the `EglGetCurrentDisplay` alias. It takes no
        // arguments and only reads the calling thread's EGL binding, so the
        // sole precondition is being on the thread wgpu made current — which
        // `&self` on this non-`Send` type guarantees. A thread with no current
        // context yields `EGL_NO_DISPLAY`, caught just below.
        let display = unsafe { (self.egl_get_current_display)() };
        assert!(
            !display.is_null(),
            "DMA-BUF EGL import requires a current EGL display"
        );
        let attributes = egl_attributes(frame);
        // SAFETY: `egl_create_image` is the resolved `eglCreateImageKHR`, whose
        // signature matches `EglCreateImage`. `display` was just asserted
        // non-null and comes from this thread's current binding; the context
        // and buffer arguments are `EGL_NO_CONTEXT`/`NULL`, which is what the
        // `EGL_LINUX_DMA_BUF_EXT` target requires. `attributes` is built by
        // `egl_attributes`, which terminates the list with `EGL_NONE` and
        // passes the plane's file descriptor while `frame` still owns it, so
        // the descriptor is open for the whole call. EGL does not take
        // ownership of that descriptor, so `frame` may still close it later.
        // `attributes` outlives the call, as EGL only reads it here.
        let image = unsafe {
            (self.egl_create_image)(
                display,
                std::ptr::null_mut(),
                EGL_LINUX_DMA_BUF_EXT,
                std::ptr::null_mut(),
                attributes.as_ptr(),
            )
        };
        assert!(!image.is_null(), "failed to import the DMA-BUF as EGLImage");
        self.blit_egl_image(frame, destination, image);
        // SAFETY: the resolved `eglDestroyImageKHR`, matching `EglDestroyImage`.
        // `image` was created non-null from `display` just above and has not
        // been destroyed since; `blit_egl_image` only binds it to a texture and
        // does not consume it. Destroying it here is the single matching
        // release for that single creation, and the GL work referencing it has
        // already been recorded against the texture.
        let destroyed = unsafe { (self.egl_destroy_image)(display, image) };
        assert_eq!(destroyed, 1, "failed to destroy the imported EGLImage");
    }

    fn blit_egl_image(&self, frame: &DmaBufFrame, destination: &wgpu::Texture, image: EglImage) {
        // The visible extent, which is the buffer's own size unless a producer
        // padded its allocation; the destination texture holds the picture.
        let (visible_width, visible_height) = frame.visible_size();
        let width = i32::try_from(visible_width).expect("DMA-BUF frame width exceeds EGLint");
        let height = i32::try_from(visible_height).expect("DMA-BUF frame height exceeds EGLint");
        // SAFETY: `Texture::as_hal` is unsafe because it exposes the backend
        // object behind wgpu's tracking, so the caller must both name the
        // backend the texture really belongs to and not invalidate wgpu's view
        // of it. The backend is checked: this method is only reached from
        // `copy_dma_buf`, which the importer selects solely for
        // `wgpu::Backend::Gl`, and a mismatch surfaces as `None` and panics
        // here rather than being transmuted. The guard's only use is to read
        // `inner` for the raw texture name; the texture is not destroyed,
        // reallocated, or relabelled, and the blit below writes through the
        // ordinary GL pipeline, which is a state wgpu re-establishes for its
        // own next command.
        let destination = unsafe {
            destination
                .as_hal::<wgpu::hal::api::Gles>()
                .expect("DMA-BUF destination texture is not GLES")
        };
        let wgpu::hal::gles::TextureInner::Texture {
            raw: destination_texture,
            target: destination_target,
        } = destination.inner
        else {
            panic!("the DMA-BUF destination must be a GLES texture");
        };
        assert_eq!(
            destination_target,
            glow::TEXTURE_2D,
            "the DMA-BUF destination must be a two-dimensional GLES texture"
        );

        // SAFETY: every call in this block is a glow GL entry point, unsafe for
        // the one shared reason that GL requires its context to be current on
        // the calling thread; `&self` on this non-`Send` type reaches here only
        // on the thread where wgpu keeps the GLES context current. Beyond
        // currency the arguments are checked rather than assumed:
        // `source_texture`, `read` and `draw` are names GL just handed back,
        // each used only between its creation and its deletion at the end of
        // the block; `destination_texture`/`destination_target` come from the
        // live hal guard above and `destination_target` was asserted to be
        // `TEXTURE_2D`; `image` is the non-null `EGLImage` its caller keeps
        // alive across this call; and both framebuffers are asserted complete
        // before `blit_framebuffer` reads or writes through them, with `width`
        // and `height` converted from the frame's own dimensions. The block
        // unbinds both framebuffers and the texture before deleting them, so it
        // leaves no name bound for wgpu's next command to trip over.
        unsafe {
            let source_texture = self.gl.create_texture().unwrap_or_else(|error| {
                panic!("failed to create the GLES source texture: {error}")
            });
            self.gl.bind_texture(glow::TEXTURE_2D, Some(source_texture));
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                i32::try_from(glow::NEAREST).expect("GL_NEAREST must fit GLint"),
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                i32::try_from(glow::NEAREST).expect("GL_NEAREST must fit GLint"),
            );
            (self.image_target_texture)(glow::TEXTURE_2D, image);

            let read = self
                .gl
                .create_framebuffer()
                .unwrap_or_else(|error| panic!("failed to create the GLES read FBO: {error}"));
            let draw = self
                .gl
                .create_framebuffer()
                .unwrap_or_else(|error| panic!("failed to create the GLES draw FBO: {error}"));
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(read));
            self.gl.framebuffer_texture_2d(
                glow::READ_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(source_texture),
                0,
            );
            assert_eq!(
                self.gl.check_framebuffer_status(glow::READ_FRAMEBUFFER),
                glow::FRAMEBUFFER_COMPLETE,
                "the EGLImage read framebuffer is incomplete"
            );
            self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(draw));
            self.gl.framebuffer_texture_2d(
                glow::DRAW_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                destination_target,
                Some(destination_texture),
                0,
            );
            assert_eq!(
                self.gl.check_framebuffer_status(glow::DRAW_FRAMEBUFFER),
                glow::FRAMEBUFFER_COMPLETE,
                "the DMA-BUF destination framebuffer is incomplete"
            );
            self.gl.blit_framebuffer(
                0,
                0,
                width,
                height,
                0,
                0,
                width,
                height,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
            self.gl.delete_framebuffer(read);
            self.gl.delete_framebuffer(draw);
            self.gl.delete_texture(source_texture);
        }
    }
}

fn egl_attributes(frame: &DmaBufFrame) -> Vec<c_int> {
    assert_eq!(
        frame.planes.len(),
        1,
        "DMA-BUF EGL import requires one packed plane"
    );
    let plane = &frame.planes[0];
    let fourcc = frame.format.fourcc();
    let mut attributes = vec![
        EGL_WIDTH,
        i32::try_from(frame.width).expect("DMA-BUF frame width exceeds EGLint"),
        EGL_HEIGHT,
        i32::try_from(frame.height).expect("DMA-BUF frame height exceeds EGLint"),
        EGL_LINUX_DRM_FOURCC_EXT,
        i32::from_ne_bytes(fourcc.to_ne_bytes()),
        EGL_DMA_BUF_PLANE0_FD_EXT,
        plane.fd.as_raw_fd(),
        EGL_DMA_BUF_PLANE0_OFFSET_EXT,
        i32::try_from(plane.offset).expect("DMA-BUF plane offset exceeds EGLint"),
        EGL_DMA_BUF_PLANE0_PITCH_EXT,
        i32::try_from(plane.stride).expect("DMA-BUF plane stride exceeds EGLint"),
    ];
    if frame.modifier != DRM_FORMAT_MOD_INVALID {
        let modifier_low = u32::try_from(frame.modifier & u64::from(u32::MAX))
            .expect("DMA-BUF modifier low bits must fit u32");
        let modifier_high =
            u32::try_from(frame.modifier >> 32).expect("DMA-BUF modifier high bits must fit u32");
        attributes.extend([
            EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
            i32::from_ne_bytes(modifier_low.to_ne_bytes()),
            EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
            i32::from_ne_bytes(modifier_high.to_ne_bytes()),
        ]);
    }
    attributes.push(EGL_NONE);
    attributes
}

/// Resolves `name` from the first library that exports it.
///
/// # Safety
///
/// `T` must be the type of the symbol named by `name` in whichever of
/// `libraries` exports it. Callers in this module pass the `unsafe extern "C"`
/// aliases declared above, each matching the EGL/GL signature for the name it
/// is paired with.
unsafe fn load_library_symbol<T: Copy>(
    libraries: &[&libloading::Library],
    name: &[u8],
    label: &str,
) -> T {
    for library in libraries {
        // SAFETY: `Library::get` is unsafe because it reinterprets the address
        // `dlsym` returns as `T` without being able to check it. The caller
        // guarantees, per this function's own contract, that `T` is the type of
        // `name` in these libraries. `name` is a NUL-terminated byte string, as
        // `dlsym` requires. The returned `Symbol` borrows the library, and the
        // `*symbol` copy is a bare function pointer whose validity is tied to
        // the library staying loaded — which `GlesInterop` ensures by owning
        // both handles for as long as it can call them.
        if let Ok(symbol) = unsafe { library.get::<T>(name) } {
            return *symbol;
        }
    }
    panic!("the required DMA-BUF import symbol `{label}` is unavailable")
}

/// Resolves an EGL/GL entry point, falling back to `eglGetProcAddress` for the
/// extension entry points the libraries do not export directly.
///
/// # Safety
///
/// `T` must be the type of the symbol named `name`. Every caller in this module
/// pairs one of the `unsafe extern "C"` aliases declared above with the EGL name
/// it was written for.
unsafe fn load_egl_symbol<T: Copy>(
    egl: &libloading::Library,
    gles: &libloading::Library,
    get_proc: EglGetProcAddress,
    name: &str,
) -> T {
    let name_with_nul =
        CString::new(name).unwrap_or_else(|_| panic!("EGL symbol contains a NUL byte"));
    for library in [egl, gles] {
        // SAFETY: `T` is the symbol's type by this function's contract, and
        // `as_bytes_with_nul` supplies the NUL-terminated name `dlsym` wants.
        // The copied-out function pointer stays valid because `GlesInterop`
        // keeps both libraries loaded for as long as it holds the pointer.
        if let Ok(symbol) = unsafe { library.get::<T>(name_with_nul.as_bytes_with_nul()) } {
            return *symbol;
        }
    }
    // SAFETY: `get_proc` is the address resolved for `eglGetProcAddress`, whose
    // signature is `EglGetProcAddress`. It takes a NUL-terminated name, which
    // `as_ptr` provides from a `CString` that outlives the call, and returns
    // either null or the entry point for that name.
    let address = unsafe { get_proc(name_with_nul.as_ptr()) };
    assert!(
        !address.is_null(),
        "the required DMA-BUF import symbol `{name}` is unavailable"
    );
    assert_eq!(
        size_of::<T>(),
        size_of::<*const c_void>(),
        "the resolved symbol pointer has an unexpected size"
    );
    // SAFETY: `transmute_copy` reinterprets the resolved address as `T`. The
    // sizes are asserted equal immediately above, so no memory outside
    // `address` is read; `T` is one of the `unsafe extern "C" fn` aliases, whose
    // validity invariant is being a non-null pointer to a function of that
    // signature — non-null is asserted, and the signature holds by this
    // function's contract. The address belongs to a library `GlesInterop` keeps
    // loaded, so the resulting pointer stays callable.
    unsafe { std::mem::transmute_copy(&address) }
}

/// Resolves the address of GL entry point `name`, or null when it is absent.
///
/// This is glow's loader, so it deliberately returns a bare address rather than
/// a typed pointer: glow is what knows the signature for each name.
fn load_egl_address(
    egl: &libloading::Library,
    gles: &libloading::Library,
    get_proc: EglGetProcAddress,
    name: &str,
) -> *const c_void {
    let name_with_nul = CString::new(name).expect("GL symbol contains a NUL byte");
    for library in [egl, gles] {
        // SAFETY: `Library::get` needs the requested type to describe the
        // symbol. The type asked for here is `*const c_void`, the address
        // itself, which is what `dlsym` returns for any symbol whatsoever, so
        // no signature is being claimed and no call is made through it. The
        // name is NUL-terminated as `dlsym` requires.
        let Ok(symbol) =
            (unsafe { library.get::<*const c_void>(name_with_nul.as_bytes_with_nul()) })
        else {
            continue;
        };
        if !symbol.is_null() {
            return *symbol;
        }
    }
    // SAFETY: `get_proc` is the resolved `eglGetProcAddress`, matching
    // `EglGetProcAddress`. Its one argument must be a NUL-terminated name,
    // which `as_ptr` gives from a `CString` alive for the whole call; it
    // returns null for names the driver does not implement. Extension entry
    // points such as `glEGLImageTargetTexture2DOES` are reachable only this
    // way, which is why the `dlsym` attempts above are allowed to miss.
    unsafe { get_proc(name_with_nul.as_ptr()) }
}
