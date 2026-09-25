//! The frames `WPEPlatform` produces, and the lease that owns each buffer.
//!
//! `render_buffer` hands the bridge a `WPEBuffer` from the engine's pool: a
//! `WPEBufferDMABuf` to import onto the GPU when the display has a render
//! node, a `WPEBufferSHM` — shared-memory pixels to upload — when it does
//! not. [`WpeFrame`] carries either kind; the DMA-BUF half delegates to
//! [`wgpu_external_frame::dma_buf`], which knows nothing about WPE.
//!
//! What is WPE's is the *lease*: the engine hands out a buffer from its own
//! pool and wants it back once the compositor has read it, which is the
//! two-step present/release protocol `WpeFrameLease` describes — shared by
//! both buffer kinds.
//!
//! # Safety
//!
//! As in `page`, the `unsafe` here is calls through the WPE bridge ABI. The
//! function pointers come from a `RuntimeApi` the runtime keeps mapped, and the
//! frame pointer is the one this lease owns. The bridge marshals the WPE object
//! operations back onto the runtime's `GMainContext`.

use std::os::fd::{AsRawFd, OwnedFd};

use wgpu_external_frame::dma_buf::{DmaBufFormat, DmaBufFrame, DmaBufLease};

#[cfg(feature = "webview")]
use std::os::fd::{FromRawFd, IntoRawFd};
#[cfg(feature = "webview")]
use std::sync::Arc;

#[cfg(feature = "webview")]
use wgpu_external_frame::dma_buf::DmaBufPlane;

#[cfg(feature = "webview")]
use crate::abi::{MAX_PLANES, WATER_WPE_BUFFER_DMA_BUF, WATER_WPE_BUFFER_SHM, WaterWpeFrame};
#[cfg(feature = "webview")]
use crate::runtime::RuntimeApi;

#[cfg(feature = "webview")]
const DRM_FORMAT_MOD_LINEAR: u64 = 0;

/// One frame the bundled engine produced, in whichever form the platform
/// delivered it.
#[derive(Debug)]
pub enum WpeFrame {
    /// A GPU-resident buffer to import.
    DmaBuf(DmaBufFrame),
    /// CPU-accessible pixels to upload.
    Shm(ShmFrame),
}

impl WpeFrame {
    /// Pixel width of the frame.
    #[must_use]
    pub const fn width(&self) -> u32 {
        match self {
            Self::DmaBuf(frame) => frame.width,
            Self::Shm(frame) => frame.width,
        }
    }

    /// Pixel height of the frame.
    #[must_use]
    pub const fn height(&self) -> u32 {
        match self {
            Self::DmaBuf(frame) => frame.height,
            Self::Shm(frame) => frame.height,
        }
    }

    /// Pixel format of the frame.
    #[must_use]
    pub const fn format(&self) -> DmaBufFormat {
        match self {
            Self::DmaBuf(frame) => frame.format,
            Self::Shm(frame) => frame.format,
        }
    }

    /// Returns whether the producer's rendering fence has signalled.
    ///
    /// A frame with no fence is ready as soon as it arrives.
    #[must_use]
    pub fn is_render_ready(&self) -> bool {
        match self {
            Self::DmaBuf(frame) => frame.is_render_ready(),
            Self::Shm(frame) => frame.is_render_ready(),
        }
    }

    /// Tells WPE the frame has been imported or copied by the backend.
    ///
    /// # Panics
    ///
    /// Panics when the frame was already presented.
    pub fn presented(&mut self) {
        match self {
            Self::DmaBuf(frame) => frame.presented(),
            Self::Shm(frame) => frame.presented(),
        }
    }

    /// Returns the buffer to WPE after backend GPU work has completed.
    ///
    /// # Panics
    ///
    /// Panics when the frame was not presented first.
    pub fn release(self, release_fence: Option<OwnedFd>) {
        match self {
            Self::DmaBuf(frame) => frame.release(release_fence),
            Self::Shm(frame) => frame.release(release_fence),
        }
    }
}

/// A frame carried as shared-memory pixels: what `WPEPlatform` hands out on a
/// display with no render node — CI runners, VMs, containers.
///
/// `pixels` borrows the producer's `GBytes`; the buffer lease the frame
/// carries is what keeps that memory mapped, so the pixels are valid until
/// the frame is released or dropped.
#[derive(Debug)]
pub struct ShmFrame {
    /// Pixel width of the frame.
    pub width: u32,
    /// Pixel height of the frame.
    pub height: u32,
    /// Pixel format.
    pub format: DmaBufFormat,
    /// Bytes between adjacent rows.
    pub stride: u32,
    pixels: *const u8,
    pixels_len: usize,
    /// Rendering completion fence supplied by the producer.
    pub rendering_fence: Option<OwnedFd>,
    lease: Option<Box<dyn DmaBufLease>>,
}

impl ShmFrame {
    /// Creates a shared-memory frame borrowing `pixels`.
    ///
    /// Attach the producer's buffer lease with [`Self::with_lease`].
    ///
    /// # Safety
    ///
    /// `pixels` must point to `pixels_len` readable bytes laid out as `height`
    /// rows of `stride` bytes, and must stay valid until the frame is released
    /// or dropped — the attached lease is what keeps it alive.
    ///
    /// # Panics
    ///
    /// Panics when the dimensions are zero or the stride cannot hold a row.
    #[must_use]
    pub unsafe fn new(
        width: u32,
        height: u32,
        format: DmaBufFormat,
        stride: u32,
        pixels: *const u8,
        pixels_len: usize,
        rendering_fence: Option<OwnedFd>,
    ) -> Self {
        assert!(width > 0 && height > 0, "SHM frame must be non-zero");
        let row_bytes = usize::try_from(width).expect("SHM width must fit usize") * 4;
        let stride_bytes = usize::try_from(stride).expect("SHM stride must fit usize");
        assert!(
            stride_bytes >= row_bytes,
            "SHM stride {stride} cannot hold a {row_bytes}-byte row"
        );
        let rows = usize::try_from(height).expect("SHM height must fit usize");
        let required = stride_bytes * (rows - 1) + row_bytes;
        assert!(
            pixels_len >= required,
            "SHM frame is {pixels_len} bytes but {width}x{height} at stride {stride} needs {required}"
        );
        Self {
            width,
            height,
            format,
            stride,
            pixels,
            pixels_len,
            rendering_fence,
            lease: None,
        }
    }

    /// Attaches the producer's lease on the buffer this frame borrows.
    ///
    /// # Panics
    ///
    /// Panics when the frame already carries a lease, since only one producer
    /// can own the buffer.
    #[must_use]
    pub fn with_lease(mut self, lease: Box<dyn DmaBufLease>) -> Self {
        assert!(
            self.lease.is_none(),
            "an SHM frame carries at most one buffer lease"
        );
        self.lease = Some(lease);
        self
    }

    /// The frame's pixels: `height` rows of `stride` bytes in `format`.
    #[must_use]
    pub const fn pixels(&self) -> &[u8] {
        // SAFETY: `new`'s contract keeps the pointer valid until the frame is
        // released or dropped, and the slice borrow cannot outlive either.
        unsafe { std::slice::from_raw_parts(self.pixels, self.pixels_len) }
    }

    /// Returns whether the producer's rendering fence has signalled.
    ///
    /// A frame with no fence is ready as soon as it arrives — the usual case,
    /// since CPU rendering completes before the buffer ships.
    ///
    /// # Panics
    ///
    /// Panics when polling the rendering fence fails.
    #[must_use]
    pub fn is_render_ready(&self) -> bool {
        self.rendering_fence.as_ref().is_none_or(fence_signalled)
    }

    /// Tells the producer the frame has been copied by the backend.
    ///
    /// Does nothing for a frame with no lease.
    ///
    /// # Panics
    ///
    /// Panics when the producer rejects the transition, typically because the
    /// frame was presented twice.
    pub fn presented(&mut self) {
        if let Some(lease) = self.lease.as_mut() {
            lease.presented();
        }
    }

    /// Returns the buffer to the producer and drops the frame's descriptors.
    ///
    /// # Panics
    ///
    /// Panics when a frame with no lease is given a release fence: there is no
    /// producer to hand it to, so accepting it would silently discard it.
    pub fn release(self, release_fence: Option<OwnedFd>) {
        match self.lease {
            Some(lease) => lease.release(release_fence),
            None => assert!(
                release_fence.is_none(),
                "an unleased SHM frame does not accept a release fence"
            ),
        }
    }
}

// SAFETY: the pixel memory is owned by the producer's buffer, which the lease
// keeps mapped; the backend reads it only while uploading, before `release`.
// The lease itself is `Send` — the bridge marshals the release back onto the
// runtime's `GMainContext`.
unsafe impl Send for ShmFrame {}

/// Polls a producer's rendering fence without blocking.
fn fence_signalled(fence: &OwnedFd) -> bool {
    let mut descriptor = libc::pollfd {
        fd: fence.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `poll` reads `nfds` entries of the array it is given and writes
    // their `revents`. `descriptor` is one fully initialized `pollfd` local,
    // and `1` is exactly its length. Its `fd` is borrowed from the `OwnedFd`
    // this frame holds, so it is open for the call, and a zero timeout makes
    // the call return without blocking.
    let result = unsafe { libc::poll(&raw mut descriptor, 1, 0) };
    assert!(result >= 0, "failed to poll the WPE rendering fence");
    result == 1
}

/// Builds an owned frame from a WPE bridge frame, taking over its descriptors.
///
/// # Safety
///
/// `frame` must be a live bridge frame whose descriptors and token this call
/// takes ownership of; see the module safety note.
#[cfg(feature = "webview")]
pub unsafe fn frame_from_abi(api: Arc<RuntimeApi>, frame: &WaterWpeFrame) -> WpeFrame {
    assert!(
        frame.width > 0 && frame.height > 0,
        "WPE returned a zero-sized frame"
    );
    let rendering_fence = (frame.rendering_fence_fd >= 0)
        // SAFETY: bridge ABI call on the frame this lease owns; see the module
        // safety note.
        .then(|| unsafe { OwnedFd::from_raw_fd(frame.rendering_fence_fd) });
    let lease = Box::new(WpeFrameLease {
        api,
        token: frame.token,
        presented: false,
        released: false,
    });
    match frame.kind {
        WATER_WPE_BUFFER_DMA_BUF => {
            assert_eq!(
                frame.modifier, DRM_FORMAT_MOD_LINEAR,
                "bundled WPE must negotiate DRM_FORMAT_MOD_LINEAR"
            );
            let n_planes = usize::try_from(frame.n_planes).expect("WPE plane count must fit usize");
            assert!(
                (1..=MAX_PLANES).contains(&n_planes),
                "WPE returned invalid DMA-BUF plane count {n_planes}"
            );
            assert_eq!(
                n_planes, 1,
                "WaterUI's WPE output contract requires one packed 32-bit plane"
            );
            let planes = (0..n_planes)
                .map(|index| {
                    assert!(frame.fds[index] >= 0, "WPE DMA-BUF plane fd is invalid");
                    DmaBufPlane {
                        // SAFETY: bridge ABI call on the frame this lease owns; see
                        // the module safety note.
                        fd: unsafe { OwnedFd::from_raw_fd(frame.fds[index]) },
                        offset: frame.offsets[index],
                        stride: frame.strides[index],
                    }
                })
                .collect();
            // WPE hands over exactly the picture, so the frame needs no
            // visible-size narrowing; only a browser's padded shared image
            // does.
            WpeFrame::DmaBuf(
                DmaBufFrame::new(
                    frame.width,
                    frame.height,
                    DmaBufFormat::from_fourcc(frame.format),
                    frame.modifier,
                    planes,
                    rendering_fence,
                )
                .with_lease(lease),
            )
        }
        WATER_WPE_BUFFER_SHM => WpeFrame::Shm(
            // SAFETY: the lease keeps the `WPEBuffer` — owner of the `GBytes`
            // `shm_data` points into — alive until the frame is released, so
            // the pixels outlive every read this side makes; see the module
            // safety note.
            unsafe {
                ShmFrame::new(
                    frame.width,
                    frame.height,
                    DmaBufFormat::from_fourcc(frame.format),
                    frame.shm_stride,
                    frame.shm_data,
                    frame.shm_len,
                    rendering_fence,
                )
            }
            .with_lease(lease),
        ),
        kind => panic!("bundled WPE returned an unknown WPEBuffer kind {kind}"),
    }
}

/// Exact WPE buffer ownership token.
#[cfg(feature = "webview")]
pub struct WpeFrameLease {
    api: Arc<RuntimeApi>,
    token: *mut std::ffi::c_void,
    presented: bool,
    released: bool,
}

#[cfg(feature = "webview")]
impl core::fmt::Debug for WpeFrameLease {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WpeFrameLease")
            .field("presented", &self.presented)
            .field("released", &self.released)
            .finish_non_exhaustive()
    }
}

// SAFETY: completion is thread-safe in the bridge — it marshals every WPE object
// operation back onto the runtime's GMainContext, so the lease itself carries no
// thread affinity.
#[cfg(feature = "webview")]
unsafe impl Send for WpeFrameLease {}

#[cfg(feature = "webview")]
impl DmaBufLease for WpeFrameLease {
    /// Tells WPE the frame has been imported or copied by the backend.
    ///
    /// # Panics
    ///
    /// Panics when the frame was already presented.
    fn presented(&mut self) {
        assert!(!self.presented, "WPE frame was presented more than once");
        // SAFETY: bridge ABI call on the frame this lease owns; see the module
        // safety note.
        unsafe { (self.api.api.frame_presented)(self.token) };
        self.presented = true;
    }

    /// Returns the buffer to WPE after backend GPU work has completed.
    ///
    /// # Panics
    ///
    /// Panics when the frame was not presented first.
    fn release(mut self: Box<Self>, release_fence: Option<OwnedFd>) {
        assert!(self.presented, "WPE frame must be presented before release");
        let fd = release_fence.map_or(-1, IntoRawFd::into_raw_fd);
        // SAFETY: bridge ABI call on the frame this lease owns; see the module
        // safety note.
        unsafe { (self.api.api.frame_release)(self.token, fd) };
        self.released = true;
    }
}

#[cfg(feature = "webview")]
impl Drop for WpeFrameLease {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        if !self.presented {
            // SAFETY: bridge ABI call on the frame this lease owns; see the
            // module safety note.
            unsafe { (self.api.api.frame_presented)(self.token) };
        }
        // SAFETY: bridge ABI call on the frame this lease owns; see the module
        // safety note.
        unsafe { (self.api.api.frame_release)(self.token, -1) };
    }
}
