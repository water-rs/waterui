use std::os::fd::{AsRawFd, OwnedFd};
#[cfg(feature = "webview")]
use std::os::fd::{FromRawFd, IntoRawFd};
#[cfg(feature = "webview")]
use std::sync::Arc;

#[cfg(feature = "webview")]
use crate::abi::{MAX_PLANES, WaterWpeFrame};
#[cfg(feature = "webview")]
use crate::runtime::RuntimeApi;

#[cfg(feature = "webview")]
const DRM_FORMAT_MOD_LINEAR: u64 = 0;
#[cfg(feature = "webview")]
const DRM_FORMAT_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
#[cfg(feature = "webview")]
const DRM_FORMAT_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
#[cfg(feature = "webview")]
const DRM_FORMAT_ABGR8888: u32 = u32::from_le_bytes(*b"AB24");
#[cfg(feature = "webview")]
const DRM_FORMAT_XBGR8888: u32 = u32::from_le_bytes(*b"XB24");

/// Pixel format of a WPE DMA-BUF frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaBufFormat {
    /// Little-endian BGRA with meaningful alpha.
    Bgra8,
    /// Little-endian BGRX; consumers must force alpha to one.
    Bgrx8,
    /// Little-endian RGBA with meaningful alpha.
    Rgba8,
    /// Little-endian RGBX; consumers must force alpha to one.
    Rgbx8,
}

impl DmaBufFormat {
    #[cfg(feature = "webview")]
    fn from_fourcc(value: u32) -> Self {
        match value {
            DRM_FORMAT_ARGB8888 => Self::Bgra8,
            DRM_FORMAT_XRGB8888 => Self::Bgrx8,
            DRM_FORMAT_ABGR8888 => Self::Rgba8,
            DRM_FORMAT_XBGR8888 => Self::Rgbx8,
            _ => panic!("WPE returned unsupported DRM fourcc 0x{value:08x}"),
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) const fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Bgra8 | Self::Bgrx8 => wgpu::TextureFormat::Bgra8Unorm,
            Self::Rgba8 | Self::Rgbx8 => wgpu::TextureFormat::Rgba8Unorm,
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) const fn force_opaque(self) -> bool {
        matches!(self, Self::Bgrx8 | Self::Rgbx8)
    }
}

/// One owned plane in a DMA-BUF frame.
#[derive(Debug)]
pub struct DmaBufPlane {
    /// Owned DMA-BUF file descriptor.
    pub fd: OwnedFd,
    /// Byte offset of this plane.
    pub offset: u32,
    /// Bytes between adjacent rows.
    pub stride: u32,
}

/// A WPE frame and its explicit lifetime lease.
#[derive(Debug)]
pub struct DmaBufFrame {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Pixel format.
    pub format: DmaBufFormat,
    /// DRM format modifier describing the plane layout.
    pub modifier: u64,
    /// Owned planes.
    pub planes: Vec<DmaBufPlane>,
    /// Rendering completion fence supplied by WPE.
    pub rendering_fence: Option<OwnedFd>,
    #[cfg_attr(
        not(target_os = "linux"),
        expect(
            dead_code,
            reason = "the lease is consumed by Linux GPU composition while non-Linux checks retain drop semantics"
        )
    )]
    pub(crate) lease: DmaBufFrameLease,
}

impl DmaBufFrame {
    /// Creates an owned DMA-BUF frame whose descriptors are not leased from WPE.
    ///
    /// # Panics
    ///
    /// Panics when the dimensions are zero or the frame is not a single packed
    /// 32-bit plane.
    #[must_use]
    pub fn owned(
        width: u32,
        height: u32,
        format: DmaBufFormat,
        modifier: u64,
        planes: Vec<DmaBufPlane>,
        rendering_fence: Option<OwnedFd>,
    ) -> Self {
        assert!(width > 0 && height > 0, "DMA-BUF frame must be non-zero");
        assert_eq!(
            planes.len(),
            1,
            "WaterUI browser DMA-BUF frames require one packed 32-bit plane"
        );
        Self {
            width,
            height,
            format,
            modifier,
            planes,
            rendering_fence,
            lease: DmaBufFrameLease::Owned,
        }
    }

    #[cfg(feature = "webview")]
    pub(crate) unsafe fn from_abi(api: Arc<RuntimeApi>, frame: &WaterWpeFrame) -> Self {
        assert!(
            frame.width > 0 && frame.height > 0,
            "WPE returned a zero-sized frame"
        );
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
                    fd: unsafe { OwnedFd::from_raw_fd(frame.fds[index]) },
                    offset: frame.offsets[index],
                    stride: frame.strides[index],
                }
            })
            .collect();
        let rendering_fence = (frame.rendering_fence_fd >= 0)
            .then(|| unsafe { OwnedFd::from_raw_fd(frame.rendering_fence_fd) });
        Self {
            width: frame.width,
            height: frame.height,
            format: DmaBufFormat::from_fourcc(frame.format),
            modifier: frame.modifier,
            planes,
            rendering_fence,
            lease: DmaBufFrameLease::Wpe(WpeFrameLease {
                api,
                token: frame.token,
                presented: false,
                released: false,
            }),
        }
    }

    /// Returns whether the explicit rendering fence has signalled.
    ///
    /// # Panics
    ///
    /// Panics when polling the rendering fence fails.
    #[must_use]
    pub fn is_render_ready(&self) -> bool {
        let Some(fence) = &self.rendering_fence else {
            return true;
        };
        let mut descriptor = libc::pollfd {
            fd: fence.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&raw mut descriptor, 1, 0) };
        assert!(result >= 0, "failed to poll WPE rendering fence");
        result == 1
    }
}

#[derive(Debug)]
#[cfg_attr(
    not(target_os = "linux"),
    expect(
        dead_code,
        reason = "lease variants are consumed by Linux GPU composition while non-Linux checks retain drop semantics"
    )
)]
pub enum DmaBufFrameLease {
    #[cfg(feature = "webview")]
    Wpe(WpeFrameLease),
    Owned,
}

#[cfg(all(target_os = "linux", feature = "webview"))]
impl DmaBufFrameLease {
    pub(crate) fn presented(&mut self) {
        match self {
            Self::Wpe(lease) => lease.presented(),
            Self::Owned => {}
        }
    }

    pub(crate) fn release(self, release_fence_fd: Option<OwnedFd>) {
        match self {
            Self::Wpe(lease) => lease.release(release_fence_fd),
            Self::Owned => {
                assert!(
                    release_fence_fd.is_none(),
                    "owned browser DMA-BUF frames do not accept a release fence"
                );
            }
        }
    }
}

#[cfg(all(target_os = "linux", not(feature = "webview")))]
impl DmaBufFrameLease {
    pub(crate) const fn presented(&self) {
        assert!(matches!(self, Self::Owned));
    }

    pub(crate) fn release(self, release_fence_fd: Option<OwnedFd>) {
        assert!(matches!(self, Self::Owned));
        assert!(
            release_fence_fd.is_none(),
            "owned browser DMA-BUF frames do not accept a release fence"
        );
        drop(release_fence_fd);
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

// Completion is intentionally thread-safe in the bridge: it marshals the WPE
// object operations back onto the runtime's GMainContext.
#[cfg(feature = "webview")]
unsafe impl Send for WpeFrameLease {}

#[cfg(feature = "webview")]
impl WpeFrameLease {
    /// Tells WPE the frame has been imported or copied by the backend.
    ///
    /// # Panics
    ///
    /// Panics when the frame was already presented.
    pub fn presented(&mut self) {
        assert!(!self.presented, "WPE frame was presented more than once");
        unsafe { (self.api.api.frame_presented)(self.token) };
        self.presented = true;
    }

    /// Returns the buffer to WPE after backend GPU work has completed.
    ///
    /// # Panics
    ///
    /// Panics when the frame was not presented first.
    pub fn release(mut self, release_fence_fd: Option<OwnedFd>) {
        assert!(self.presented, "WPE frame must be presented before release");
        let fd = release_fence_fd.map_or(-1, IntoRawFd::into_raw_fd);
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
            unsafe { (self.api.api.frame_presented)(self.token) };
        }
        unsafe { (self.api.api.frame_release)(self.token, -1) };
    }
}
