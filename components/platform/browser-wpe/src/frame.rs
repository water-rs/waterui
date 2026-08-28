//! WPE's half of the DMA-BUF frame contract: the buffer lease.
//!
//! The frame itself — planes, format, fence, import — belongs to
//! [`wgpu_external_frame::dma_buf`], which knows nothing about WPE. What is
//! WPE's is the *lease*: the engine hands out a buffer from its own pool and
//! wants it back once the compositor has read it, which is the two-step
//! present/release protocol [`DmaBufLease`] describes.
//!
//! # Safety
//!
//! As in `page`, the `unsafe` here is calls through the WPE bridge ABI. The
//! function pointers come from a `RuntimeApi` the runtime keeps mapped, and the
//! frame pointer is the one this lease owns. The bridge marshals the WPE object
//! operations back onto the runtime's `GMainContext`.

use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::sync::Arc;

use wgpu_external_frame::dma_buf::{DmaBufFormat, DmaBufFrame, DmaBufLease, DmaBufPlane};

use crate::abi::{MAX_PLANES, WaterWpeFrame};
use crate::runtime::RuntimeApi;

const DRM_FORMAT_MOD_LINEAR: u64 = 0;

/// Builds an owned frame from a WPE bridge frame, taking over its descriptors.
///
/// # Safety
///
/// `frame` must be a live bridge frame whose descriptors and token this call
/// takes ownership of; see the module safety note.
pub unsafe fn frame_from_abi(api: Arc<RuntimeApi>, frame: &WaterWpeFrame) -> DmaBufFrame {
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
                // SAFETY: bridge ABI call on the frame this lease owns; see the
                // module safety note.
                fd: unsafe { OwnedFd::from_raw_fd(frame.fds[index]) },
                offset: frame.offsets[index],
                stride: frame.strides[index],
            }
        })
        .collect();
    let rendering_fence = (frame.rendering_fence_fd >= 0)
        // SAFETY: bridge ABI call on the frame this lease owns; see the module
        // safety note.
        .then(|| unsafe { OwnedFd::from_raw_fd(frame.rendering_fence_fd) });
    // WPE hands over exactly the picture, so the frame needs no visible-size
    // narrowing; only a browser's padded shared image does.
    DmaBufFrame::new(
        frame.width,
        frame.height,
        DmaBufFormat::from_fourcc(frame.format),
        frame.modifier,
        planes,
        rendering_fence,
    )
    .with_lease(Box::new(WpeFrameLease {
        api,
        token: frame.token,
        presented: false,
        released: false,
    }))
}

/// Exact WPE buffer ownership token.
pub struct WpeFrameLease {
    api: Arc<RuntimeApi>,
    token: *mut std::ffi::c_void,
    presented: bool,
    released: bool,
}

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
unsafe impl Send for WpeFrameLease {}

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

impl Drop for WpeFrameLease {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        if !self.presented {
            // SAFETY: bridge ABI call on the frame this lease owns; see the module
            // safety note.
            unsafe { (self.api.api.frame_presented)(self.token) };
        }
        // SAFETY: bridge ABI call on the frame this lease owns; see the module
        // safety note.
        unsafe { (self.api.api.frame_release)(self.token, -1) };
    }
}
