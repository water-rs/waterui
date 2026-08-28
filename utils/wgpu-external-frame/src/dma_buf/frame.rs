use std::os::fd::{AsRawFd as _, OwnedFd};

const DRM_FORMAT_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
const DRM_FORMAT_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
const DRM_FORMAT_ABGR8888: u32 = u32::from_le_bytes(*b"AB24");
const DRM_FORMAT_XBGR8888: u32 = u32::from_le_bytes(*b"XB24");

/// The DRM format modifier that means "no modifier was negotiated".
///
/// Vulkan import needs an explicit modifier and rejects this value; EGL import
/// treats it as "omit the modifier attributes entirely".
pub const DRM_FORMAT_MOD_INVALID: u64 = u64::MAX;

/// Pixel format of a DMA-BUF frame.
///
/// Only the single-plane packed 32-bit formats are covered, which is what
/// browser and compositor output uses.
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
    /// Maps a DRM `fourcc` code onto the format it names.
    ///
    /// # Panics
    ///
    /// Panics for any code outside the four packed 32-bit formats above.
    #[must_use]
    pub fn from_fourcc(value: u32) -> Self {
        match value {
            DRM_FORMAT_ARGB8888 => Self::Bgra8,
            DRM_FORMAT_XRGB8888 => Self::Bgrx8,
            DRM_FORMAT_ABGR8888 => Self::Rgba8,
            DRM_FORMAT_XBGR8888 => Self::Rgbx8,
            _ => panic!("unsupported DMA-BUF DRM fourcc 0x{value:08x}"),
        }
    }

    /// The DRM `fourcc` code for this format.
    #[must_use]
    pub const fn fourcc(self) -> u32 {
        match self {
            Self::Bgra8 => DRM_FORMAT_ARGB8888,
            Self::Bgrx8 => DRM_FORMAT_XRGB8888,
            Self::Rgba8 => DRM_FORMAT_ABGR8888,
            Self::Rgbx8 => DRM_FORMAT_XBGR8888,
        }
    }

    /// The `wgpu` format an imported texture of this format carries.
    ///
    /// The `X` variants have no alpha of their own but still occupy the
    /// channel, so they import as their `A` counterpart; see
    /// [`Self::force_opaque`].
    #[must_use]
    pub const fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Bgra8 | Self::Bgrx8 => wgpu::TextureFormat::Bgra8Unorm,
            Self::Rgba8 | Self::Rgbx8 => wgpu::TextureFormat::Rgba8Unorm,
        }
    }

    /// Whether a consumer must ignore the imported alpha channel and treat the
    /// frame as opaque.
    #[must_use]
    pub const fn force_opaque(self) -> bool {
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

/// The producer's ownership of the buffer behind a [`DmaBufFrame`].
///
/// A producer that hands out a buffer from a pool needs to know when the
/// importer has read it, so it can put the buffer back. That is a two-step
/// protocol — the import is recorded, then the GPU work referencing it
/// completes — and this trait is both steps.
///
/// A lease is dropped rather than released whenever an import path is
/// abandoned, so implementations must treat `Drop` as an implicit release.
pub trait DmaBufLease: core::fmt::Debug + Send {
    /// Tells the producer the frame has been imported or copied.
    ///
    /// The GPU work reading it may still be in flight; only [`Self::release`]
    /// says it has finished.
    fn presented(&mut self);

    /// Returns the buffer to the producer once the importing GPU work has
    /// completed.
    ///
    /// `release_fence` is a fence the producer must wait on before reusing the
    /// buffer, for importers that can supply one instead of waiting themselves.
    fn release(self: Box<Self>, release_fence: Option<OwnedFd>);
}

/// A DMA-BUF frame, its planes, and the producer's lease on the buffer.
#[derive(Debug)]
pub struct DmaBufFrame {
    /// Pixel width of the allocation.
    pub width: u32,
    /// Pixel height of the allocation.
    pub height: u32,
    /// Pixel format.
    pub format: DmaBufFormat,
    /// DRM format modifier describing the plane layout.
    pub modifier: u64,
    /// Owned planes.
    pub planes: Vec<DmaBufPlane>,
    /// Rendering completion fence supplied by the producer.
    pub rendering_fence: Option<OwnedFd>,
    /// The part of the buffer that actually holds the image, when it is smaller
    /// than the allocation.
    visible: Option<(u32, u32)>,
    lease: Option<Box<dyn DmaBufLease>>,
}

impl DmaBufFrame {
    /// Creates a frame whose descriptors this side owns outright.
    ///
    /// Attach a producer's buffer lease with [`Self::with_lease`].
    ///
    /// # Panics
    ///
    /// Panics when the dimensions are zero or the frame is not a single packed
    /// 32-bit plane.
    #[must_use]
    pub fn new(
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
            "DMA-BUF frames require one packed 32-bit plane"
        );
        Self {
            width,
            height,
            format,
            modifier,
            planes,
            rendering_fence,
            visible: None,
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
            "a DMA-BUF frame carries at most one buffer lease"
        );
        self.lease = Some(lease);
        self
    }

    /// Narrows the frame to the region that actually holds the image.
    ///
    /// Presenting a padded buffer edge to edge stretches the picture and draws
    /// the padding gutter, which is what happens whenever a producer's coded
    /// size exceeds its visible rect.
    ///
    /// # Panics
    ///
    /// Panics when the visible size is zero or exceeds the buffer, since such
    /// a region cannot describe any part of the image this frame holds.
    #[must_use]
    pub fn with_visible_size(mut self, width: u32, height: u32) -> Self {
        assert!(
            width > 0 && height > 0 && width <= self.width && height <= self.height,
            "a DMA-BUF frame's visible size must be non-zero and within the buffer"
        );
        self.visible = Some((width, height));
        self
    }

    /// The extent that should be presented: the visible region when the buffer
    /// is padded, the whole buffer otherwise.
    #[must_use]
    pub fn visible_size(&self) -> (u32, u32) {
        self.visible.unwrap_or((self.width, self.height))
    }

    /// Returns whether the explicit rendering fence has signalled.
    ///
    /// A frame with no fence is ready as soon as it arrives.
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
        // SAFETY: `poll` reads `nfds` entries of the array it is given and
        // writes their `revents`. `descriptor` is one fully initialized
        // `pollfd` local, and `1` is exactly its length. Its `fd` is borrowed
        // from the `OwnedFd` this frame holds, so it is open for the call, and
        // a zero timeout makes the call return without blocking.
        let result = unsafe { libc::poll(&raw mut descriptor, 1, 0) };
        assert!(result >= 0, "failed to poll the DMA-BUF rendering fence");
        result == 1
    }

    /// Tells the producer the frame has been imported or copied.
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
                "an unleased DMA-BUF frame does not accept a release fence"
            ),
        }
    }
}
