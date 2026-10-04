//! A narrow binding over the NDK's `ASurfaceControl` and
//! `ASurfaceTransaction` (API 29), behind owning wrappers.
//!
//! `ndk-sys` 0.6 carries the types this uses (`ARect`, `ADataSpace`,
//! `AHdrMetadata_*`, `AHardwareBuffer`, `ANativeWindow`) but not
//! `surface_control.h`, and the `ndk` crate has no surface-control module,
//! so the functions are declared here. Every one is API 29, the level the
//! realization needs.

use std::ffi::{CStr, c_void};
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::ptr::NonNull;

use ndk_sys::{
    ADataSpace, AHardwareBuffer, AHdrMetadata_cta861_3, AHdrMetadata_smpte2086, ANativeWindow,
    ANativeWindowTransform, ARect,
};

use super::plan::{BufferTransform, Dataspace, IRect, Op, Range, Standard, TransferFn};
use crate::interop::HdrMetadata;

/// `ASurfaceControl`.
#[repr(C)]
pub struct ASurfaceControl {
    _opaque: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

impl std::fmt::Debug for ASurfaceControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ASurfaceControl").finish_non_exhaustive()
    }
}

/// `ASurfaceTransaction`.
#[repr(C)]
pub struct ASurfaceTransaction {
    _opaque: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

impl std::fmt::Debug for ASurfaceTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ASurfaceTransaction").finish_non_exhaustive()
    }
}

/// `ASurfaceTransactionStats`.
#[repr(C)]
pub struct ASurfaceTransactionStats {
    _opaque: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

impl std::fmt::Debug for ASurfaceTransactionStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ASurfaceTransactionStats").finish_non_exhaustive()
    }
}

type OnComplete = unsafe extern "C" fn(context: *mut c_void, stats: *mut ASurfaceTransactionStats);

const VISIBILITY_HIDE: i8 = 0;
const VISIBILITY_SHOW: i8 = 1;
const TRANSPARENCY_TRANSLUCENT: i8 = 1;
const TRANSPARENCY_OPAQUE: i8 = 2;

#[link(name = "android")]
unsafe extern "C" {
    fn ASurfaceControl_createFromWindow(
        parent: *mut ANativeWindow,
        debug_name: *const core::ffi::c_char,
    ) -> *mut ASurfaceControl;
    fn ASurfaceControl_create(
        parent: *mut ASurfaceControl,
        debug_name: *const core::ffi::c_char,
    ) -> *mut ASurfaceControl;
    fn ASurfaceControl_release(surface_control: *mut ASurfaceControl);

    fn ASurfaceTransaction_create() -> *mut ASurfaceTransaction;
    fn ASurfaceTransaction_delete(transaction: *mut ASurfaceTransaction);
    fn ASurfaceTransaction_apply(transaction: *mut ASurfaceTransaction);
    fn ASurfaceTransaction_setOnComplete(
        transaction: *mut ASurfaceTransaction,
        context: *mut c_void,
        func: OnComplete,
    );
    fn ASurfaceTransaction_reparent(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        new_parent: *mut ASurfaceControl,
    );
    fn ASurfaceTransaction_setVisibility(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        visibility: i8,
    );
    fn ASurfaceTransaction_setZOrder(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        z_order: i32,
    );
    fn ASurfaceTransaction_setBuffer(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        buffer: *mut AHardwareBuffer,
        acquire_fence_fd: i32,
    );
    // `const ARect&` in the C++ header: a pointer at the ABI.
    fn ASurfaceTransaction_setGeometry(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        source: *const ARect,
        destination: *const ARect,
        transform: i32,
    );
    fn ASurfaceTransaction_setBufferTransparency(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        transparency: i8,
    );
    fn ASurfaceTransaction_setBufferAlpha(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        alpha: f32,
    );
    fn ASurfaceTransaction_setBufferDataSpace(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        data_space: ADataSpace,
    );
    fn ASurfaceTransaction_setHdrMetadata_smpte2086(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        metadata: *mut AHdrMetadata_smpte2086,
    );
    fn ASurfaceTransaction_setHdrMetadata_cta861_3(
        transaction: *mut ASurfaceTransaction,
        surface_control: *mut ASurfaceControl,
        metadata: *mut AHdrMetadata_cta861_3,
    );

    fn ASurfaceTransactionStats_getASurfaceControls(
        stats: *mut ASurfaceTransactionStats,
        out_surface_controls: *mut *mut *mut ASurfaceControl,
        out_size: *mut usize,
    );
    fn ASurfaceTransactionStats_releaseASurfaceControls(
        surface_controls: *mut *mut ASurfaceControl,
    );
    fn ASurfaceTransactionStats_getPreviousReleaseFenceFd(
        stats: *mut ASurfaceTransactionStats,
        surface_control: *mut ASurfaceControl,
    ) -> i32;
}

/// Why a surface control could not be created.
#[derive(Debug, thiserror::Error)]
#[error("the system could not create the surface control {0:?}")]
pub struct CreateFailed(pub &'static CStr);

/// One owned reference to an `ASurfaceControl`; dropping it releases the
/// reference. The layer leaves the display only when a transaction
/// reparents it away, never on drop.
#[derive(Debug)]
pub struct SurfaceControl(NonNull<ASurfaceControl>);

// SAFETY: an `ASurfaceControl` is a strong reference to a reference-counted
// `SurfaceControl`, whose NDK entry points are callable from any thread.
unsafe impl Send for SurfaceControl {}

impl SurfaceControl {
    /// Takes ownership of one reference to `raw`.
    ///
    /// # Safety
    /// `raw` must be a live `ASurfaceControl` reference the caller owns (for
    /// example the result of `ASurfaceControl_create`); it is released when
    /// the returned value drops.
    #[must_use]
    pub const unsafe fn from_raw(raw: NonNull<ASurfaceControl>) -> Self {
        Self(raw)
    }

    /// Creates a surface control that is a child of `window`'s surface.
    ///
    /// # Errors
    /// [`CreateFailed`] when the system refuses.
    ///
    /// # Safety
    /// `window` must be a live `ANativeWindow` backed by a surface.
    pub unsafe fn from_window(
        window: NonNull<ANativeWindow>,
        name: &'static CStr,
    ) -> Result<Self, CreateFailed> {
        let raw = unsafe { ASurfaceControl_createFromWindow(window.as_ptr(), name.as_ptr()) };
        NonNull::new(raw).map(Self).ok_or(CreateFailed(name))
    }

    /// Creates a child of this surface control, shown at z 0 until a
    /// transaction says otherwise.
    ///
    /// # Errors
    /// [`CreateFailed`] when the system refuses.
    pub fn child(&self, name: &'static CStr) -> Result<Self, CreateFailed> {
        let raw = unsafe { ASurfaceControl_create(self.0.as_ptr(), name.as_ptr()) };
        NonNull::new(raw).map(Self).ok_or(CreateFailed(name))
    }

    /// The raw handle, borrowed.
    #[must_use]
    pub const fn as_ptr(&self) -> *mut ASurfaceControl {
        self.0.as_ptr()
    }
}

impl Drop for SurfaceControl {
    fn drop(&mut self) {
        unsafe { ASurfaceControl_release(self.0.as_ptr()) };
    }
}

/// One `ASurfaceTransaction` being built. Applying it consumes it; dropping
/// it unapplied discards every change.
pub struct Transaction(NonNull<ASurfaceTransaction>);

impl Transaction {
    /// An empty transaction.
    ///
    /// # Panics
    /// When the system cannot allocate one.
    #[must_use]
    pub fn new() -> Self {
        Self(
            NonNull::new(unsafe { ASurfaceTransaction_create() })
                .expect("ASurfaceTransaction_create returned null"),
        )
    }

    /// Shows `buffer` on `surface` once `acquire` (if any) signals; the
    /// system takes its own reference to the buffer and ownership of the
    /// fence.
    ///
    /// # Safety
    /// `buffer` must be a live `AHardwareBuffer` allocated with
    /// `GPU_SAMPLED_IMAGE` usage.
    pub unsafe fn set_buffer(
        &mut self,
        surface: &SurfaceControl,
        buffer: NonNull<AHardwareBuffer>,
        acquire: Option<OwnedFd>,
    ) {
        let fence = acquire.map_or(-1, IntoRawFd::into_raw_fd);
        unsafe {
            ASurfaceTransaction_setBuffer(
                self.0.as_ptr(),
                surface.as_ptr(),
                buffer.as_ptr(),
                fence,
            );
        }
    }

    /// Moves `surface` under `parent`, or off the display with `None` —
    /// which releases its buffer in this transaction's completion.
    pub fn reparent(&mut self, surface: &SurfaceControl, parent: Option<&SurfaceControl>) {
        unsafe {
            ASurfaceTransaction_reparent(
                self.0.as_ptr(),
                surface.as_ptr(),
                parent.map_or(core::ptr::null_mut(), SurfaceControl::as_ptr),
            );
        }
    }

    /// Sets one planned property on `surface`.
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the NDK mutates the transaction through its raw handle"
    )]
    pub fn set(&mut self, surface: &SurfaceControl, op: Op) {
        let (t, sc) = (self.0.as_ptr(), surface.as_ptr());
        unsafe {
            match op {
                Op::Z(z) => ASurfaceTransaction_setZOrder(t, sc, z),
                Op::Visible(shown) => ASurfaceTransaction_setVisibility(
                    t,
                    sc,
                    if shown {
                        VISIBILITY_SHOW
                    } else {
                        VISIBILITY_HIDE
                    },
                ),
                Op::Geometry(geometry) => {
                    let (source, destination) =
                        (arect(geometry.source), arect(geometry.destination));
                    ASurfaceTransaction_setGeometry(
                        t,
                        sc,
                        &raw const source,
                        &raw const destination,
                        buffer_transform(geometry.transform),
                    );
                }
                Op::Alpha(alpha) => ASurfaceTransaction_setBufferAlpha(t, sc, alpha),
                Op::Opaque(opaque) => ASurfaceTransaction_setBufferTransparency(
                    t,
                    sc,
                    if opaque {
                        TRANSPARENCY_OPAQUE
                    } else {
                        TRANSPARENCY_TRANSLUCENT
                    },
                ),
                Op::Dataspace(dataspace) => {
                    let space = data_space(dataspace);
                    tracing::debug!(
                        target: "cherenkov::planes",
                        dataspace = %format_args!("0x{:08x}", space.0),
                        "plane dataspace"
                    );
                    ASurfaceTransaction_setBufferDataSpace(t, sc, space);
                }
                Op::Hdr(hdr) => set_hdr(t, sc, hdr),
            }
        }
    }

    /// Applies the transaction. `on_complete` runs once, on a system
    /// thread, when the frame carrying it has been presented — with the
    /// release fence of every buffer the transaction replaced or removed.
    pub fn apply(self, on_complete: impl FnOnce(&Completion<'_>) + Send + 'static) {
        let context: Box<Callback> = Box::new(Box::new(on_complete));
        unsafe {
            ASurfaceTransaction_setOnComplete(
                self.0.as_ptr(),
                Box::into_raw(context).cast(),
                complete,
            );
            ASurfaceTransaction_apply(self.0.as_ptr());
        }
    }
}

impl Default for Transaction {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        unsafe { ASurfaceTransaction_delete(self.0.as_ptr()) };
    }
}

/// The statistics of one completed transaction, valid during its callback.
pub struct Completion<'a> {
    stats: NonNull<ASurfaceTransactionStats>,
    surfaces: &'a [*mut ASurfaceControl],
}

impl Completion<'_> {
    /// The surface controls the transaction touched, as the raw handles the
    /// transaction was built with.
    #[must_use]
    pub const fn surfaces(&self) -> &[*mut ASurfaceControl] {
        self.surfaces
    }

    /// The fence the buffer `surface` showed before this transaction
    /// releases on; `None` when it is already released.
    ///
    /// # Panics
    /// When `surface` is not one of [`Self::surfaces`] — the system aborts
    /// on such a query.
    #[must_use]
    pub fn previous_release(&self, surface: *mut ASurfaceControl) -> Option<OwnedFd> {
        assert!(
            self.surfaces.contains(&surface),
            "a release fence query names a surface the transaction did not touch"
        );
        let fd = unsafe {
            ASurfaceTransactionStats_getPreviousReleaseFenceFd(self.stats.as_ptr(), surface)
        };
        // SAFETY: a non-negative fd is a new descriptor owned by the caller.
        (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

/// The callback a transaction's completion runs, boxed once more so the
/// context pointer is thin.
type Callback = Box<dyn FnOnce(&Completion<'_>) + Send>;

unsafe extern "C" fn complete(context: *mut c_void, stats: *mut ASurfaceTransactionStats) {
    // SAFETY: `apply` passed a leaked `Box<Callback>` as the context,
    // and the system invokes the callback exactly once per applied
    // transaction.
    let callback = unsafe { Box::from_raw(context.cast::<Callback>()) };
    let stats = NonNull::new(stats).expect("the completion carries statistics");
    let mut list: *mut *mut ASurfaceControl = core::ptr::null_mut();
    let mut len = 0usize;
    unsafe {
        ASurfaceTransactionStats_getASurfaceControls(stats.as_ptr(), &raw mut list, &raw mut len);
    }
    let surfaces = if list.is_null() {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(list, len) }
    };
    callback(&Completion { stats, surfaces });
    if !list.is_null() {
        unsafe { ASurfaceTransactionStats_releaseASurfaceControls(list) };
    }
}

const fn arect(rect: IRect) -> ARect {
    ARect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

#[expect(clippy::cast_possible_wrap, reason = "the transform bits are 0..=7")]
const fn buffer_transform(transform: BufferTransform) -> i32 {
    let mut bits = ANativeWindowTransform::ANATIVEWINDOW_TRANSFORM_IDENTITY.0;
    if transform.mirror_x {
        bits |= ANativeWindowTransform::ANATIVEWINDOW_TRANSFORM_MIRROR_HORIZONTAL.0;
    }
    if transform.mirror_y {
        bits |= ANativeWindowTransform::ANATIVEWINDOW_TRANSFORM_MIRROR_VERTICAL.0;
    }
    if transform.rotate_90 {
        bits |= ANativeWindowTransform::ANATIVEWINDOW_TRANSFORM_ROTATE_90.0;
    }
    bits as i32
}

/// The `ADataSpace` bits of a planned dataspace.
pub const fn data_space(dataspace: Dataspace) -> ADataSpace {
    let standard = match dataspace.standard {
        Standard::Bt709 => ADataSpace::STANDARD_BT709,
        Standard::Bt2020 => ADataSpace::STANDARD_BT2020,
        Standard::DciP3 => ADataSpace::STANDARD_DCI_P3,
    };
    let transfer = match dataspace.transfer {
        TransferFn::Linear => ADataSpace::TRANSFER_LINEAR,
        TransferFn::Srgb => ADataSpace::TRANSFER_SRGB,
        TransferFn::Smpte170M => ADataSpace::TRANSFER_SMPTE_170M,
        TransferFn::St2084 => ADataSpace::TRANSFER_ST2084,
        TransferFn::Hlg => ADataSpace::TRANSFER_HLG,
    };
    let range = match dataspace.range {
        Range::Full => ADataSpace::RANGE_FULL,
        Range::Limited => ADataSpace::RANGE_LIMITED,
        Range::Extended => ADataSpace::RANGE_EXTENDED,
    };
    ADataSpace(standard.0 | transfer.0 | range.0)
}

unsafe fn set_hdr(t: *mut ASurfaceTransaction, sc: *mut ASurfaceControl, hdr: HdrMetadata) {
    let xy = |[x, y]: [f32; 2]| ndk_sys::AColor_xy { x, y };
    let mut mastering = hdr.mastering.map(|m| AHdrMetadata_smpte2086 {
        displayPrimaryRed: xy(m.red),
        displayPrimaryGreen: xy(m.green),
        displayPrimaryBlue: xy(m.blue),
        whitePoint: xy(m.white),
        maxLuminance: m.max_luminance,
        minLuminance: m.min_luminance,
    });
    let mut light = hdr.content_light.map(|l| AHdrMetadata_cta861_3 {
        maxContentLightLevel: l.max_content,
        maxFrameAverageLightLevel: l.max_frame_average,
    });
    // A null pointer clears the metadata.
    unsafe {
        ASurfaceTransaction_setHdrMetadata_smpte2086(
            t,
            sc,
            mastering
                .as_mut()
                .map_or(core::ptr::null_mut(), core::ptr::from_mut),
        );
        ASurfaceTransaction_setHdrMetadata_cta861_3(
            t,
            sc,
            light
                .as_mut()
                .map_or(core::ptr::null_mut(), core::ptr::from_mut),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataspaces_compose_the_ndk_constants() {
        assert_eq!(data_space(Dataspace::SRGB), ADataSpace::ADATASPACE_SRGB);
        assert_eq!(
            data_space(Dataspace {
                standard: Standard::Bt2020,
                transfer: TransferFn::St2084,
                range: Range::Limited,
            }),
            ADataSpace::ADATASPACE_BT2020_ITU_PQ
        );
        assert_eq!(
            data_space(Dataspace {
                standard: Standard::Bt709,
                transfer: TransferFn::Linear,
                range: Range::Extended,
            }),
            ADataSpace::ADATASPACE_SCRGB_LINEAR
        );
    }
}
