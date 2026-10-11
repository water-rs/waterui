//! `CAMetalLayer` presentation driven by a `CAMetalDisplayLink` — the
//! platform's own presentation primitive for Metal content.
//!
//! A [`MetalPresenter`] owns the presentation `CAMetalLayer` and one
//! `CAMetalDisplayLink` on the main run loop. Drawables arrive only through
//! link updates — `nextDrawable` is never called, so there is never a
//! blocking wait, a polling clock, a timer fallback, or a CPU pixel transfer
//! on the presentation path.
//!
//! The lease: at most one drawable is outstanding. A link update that arrives
//! while a [`DrawableFrame`] lives drops the delivered drawable unpresented
//! and keeps demand; dropping the frame releases the lease without
//! presenting. `present` consumes a frame only while its generation is
//! current and its recorded drawable size still matches the layer's —
//! everything else settles the lease and shows nothing.
//!
//! # Safety
//!
//! All types here are main-thread only (`MainThreadOnly`, !Send/!Sync via
//! `Rc`/`Cell`): `CAMetalLayer`, `CAMetalDisplayLink` and the
//! delegate are used on the main thread, and the link posts updates on the
//! run loop it was added to — the main one.

use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGFloat, CGSize};
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes};
use objc2_metal::{MTLDevice, MTLDrawable, MTLTexture};
use objc2_quartz_core::{
    CAFrameRateRange, CAMetalDisplayLink, CAMetalDisplayLinkDelegate, CAMetalDisplayLinkUpdate,
    CAMetalDrawable, CAMetalLayer,
};

use crate::callback::guarded;

/// The link's delegate; holds the presenter state weakly so an update
/// landing after the owner dropped the presenter fires nothing.
struct LinkDelegateIvars {
    inner: Weak<PresenterInner>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the ivar is a weak
    // presenter and the class implements no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiMetalLinkDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = LinkDelegateIvars]
    /// The `CAMetalDisplayLinkDelegate` of a [`MetalPresenter`]'s link.
    struct LinkDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for LinkDelegate {}

    // SAFETY: the delegate method fires on the run loop the link was added
    // to — the main run loop — and only reads main-thread ivars.
    unsafe impl CAMetalDisplayLinkDelegate for LinkDelegate {
        #[unsafe(method(metalDisplayLink:needsUpdate:))]
        fn metal_display_link_needs_update(
            &self,
            _link: &CAMetalDisplayLink,
            update: &CAMetalDisplayLinkUpdate,
        ) {
            let Some(inner) = self.ivars().inner.upgrade() else {
                return;
            };
            guarded("metal display link update", move || {
                inner.deliver_update(update);
            });
        }
    }
);

impl LinkDelegate {
    /// A delegate targeting `inner`, upgraded per update.
    fn new(mtm: MainThreadMarker, inner: &Rc<PresenterInner>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(LinkDelegateIvars {
            inner: Rc::downgrade(inner),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this
    }
}

/// One frame lease on the presenter's drawable pool, issued by a link update.
///
/// Dropping it releases the lease without presenting — a stale or
/// superseded frame never reaches the screen and never blocks the next one.
/// Only [`DrawableFrame::present`] turns a frame into a
/// [`PresentedFrame`].
pub struct DrawableFrame {
    /// The drawable this frame renders into.
    drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>,
    /// The update's target presentation timestamp, in `CACurrentMediaTime`
    /// seconds — handed to the renderer so animations time to display.
    target_time: f64,
    /// The pixel size the delivered drawable's texture reports — the
    /// contract the completion's size check compares against the layer's
    /// `drawableSize`, not a copy of the configured size.
    drawable_size: CGSize,
    /// The presenter generation that issued this frame.
    generation: u64,
    /// The issuing presenter — the lease release drops through it.
    owner: Weak<PresenterInner>,
}

impl DrawableFrame {
    /// The `MTLTexture` the frame renders into — drawable textures are
    /// render targets only (`framebufferOnly`).
    #[must_use]
    pub fn texture(&self) -> Retained<ProtocolObject<dyn MTLTexture>> {
        self.drawable.texture()
    }

    /// The update's target presentation timestamp, `CACurrentMediaTime`
    /// seconds.
    #[must_use]
    pub const fn target_time(&self) -> f64 {
        self.target_time
    }

    /// The recorded drawable size, in pixels.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a drawable texture fits in u32 and cannot be negative"
    )]
    pub const fn drawable_size(&self) -> (u32, u32) {
        (
            self.drawable_size.width as u32,
            self.drawable_size.height as u32,
        )
    }

    /// Test seam: a frame around a drawable the caller checked out from
    /// its own layer — `owner` is empty, so `present` can never mint a
    /// receipt and `Drop` settles no lease on any presenter. Exists only
    /// under `native-test`; production code cannot mint an ownerless
    /// frame.
    #[cfg(feature = "native-test")]
    #[doc(hidden)]
    #[must_use]
    pub fn unowned_for_test(
        drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>,
        target_time: f64,
    ) -> Self {
        let texture = drawable.texture();
        Self {
            drawable,
            target_time,
            #[expect(
                clippy::cast_precision_loss,
                reason = "a drawable texture fits well inside f64's mantissa"
            )]
            drawable_size: CGSize::new(texture.width() as f64, texture.height() as f64),
            generation: u64::MAX,
            owner: Weak::new(),
        }
    }

    /// Presents the frame through its issuing presenter — only while the
    /// frame's generation is still current and its recorded drawable size
    /// still matches the layer's. Returns the [`PresentedFrame`] receipt;
    /// any other frame is dropped, which settles its lease without
    /// presenting.
    #[must_use]
    pub fn present(self) -> Option<PresentedFrame> {
        let inner = self.owner.upgrade()?;
        if self.generation != inner.generation.get()
            || self.drawable_size != inner.drawable_size.get()
        {
            return None;
        }
        self.drawable.present();
        Some(PresentedFrame {
            target_time: self.target_time,
            generation: self.generation,
            main_thread: PhantomData,
        })
    }
}

impl fmt::Debug for DrawableFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DrawableFrame")
            .field("target_time", &self.target_time)
            .field("drawable_size", &self.drawable_size)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Drop for DrawableFrame {
    /// Dropping releases the lease unpresented: the next link update may
    /// issue a fresh frame, and nothing stale ever shows.
    fn drop(&mut self) {
        if let Some(inner) = self.owner.upgrade() {
            inner.lease.set(false);
        }
    }
}

/// The receipt [`DrawableFrame::present`] returns.
///
/// Only ever minted by an actual `[CAMetalDrawable present]`. Readiness
/// waiters and the runtime's productive-generation accounting consume this
/// and nothing else. Main-thread only like its siblings — the `Rc` marker
/// keeps it
/// `!Send`/`!Sync`, and it is not `Clone`.
pub struct PresentedFrame {
    /// The presented frame's target presentation timestamp.
    target_time: f64,
    /// The generation the frame was issued under.
    generation: u64,
    /// The main-thread witness — the receipt never leaves the thread that
    /// minted it.
    main_thread: PhantomData<Rc<()>>,
}

impl fmt::Debug for PresentedFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PresentedFrame")
            .field("target_time", &self.target_time)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl PresentedFrame {
    /// The presented frame's target presentation timestamp.
    #[must_use]
    pub const fn target_time(&self) -> f64 {
        self.target_time
    }

    /// The generation the presented frame was issued under.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// What the presenter hands to one link update — the owned state the
/// delegate reaches weakly.
struct PresenterInner {
    /// The presentation layer the link is bound to.
    layer: Retained<CAMetalLayer>,
    /// The issuing generation — advanced on drawable-size/format/colour
    /// changes, context replacement, detach and drop. A frame issued by an
    /// older generation can never present.
    generation: Cell<u64>,
    /// Whether a [`DrawableFrame`] lease is outstanding.
    lease: Cell<bool>,
    /// The drawable size `configure` last established.
    drawable_size: Cell<CGSize>,
    /// The frame sink — the surface's render body, invoked per issued frame.
    on_frame: Rc<dyn Fn(DrawableFrame)>,
    /// Self-reference for lease ownership.
    this: Weak<Self>,
}

impl PresenterInner {
    /// A link update: issue a [`DrawableFrame`] unless a lease is already
    /// outstanding — in that case the delivered drawable is dropped
    /// unpresented and demand stays as it was. The drawable's own texture
    /// carries the frame's real pixel size: a resize/update race cannot
    /// mislabel an old-size drawable as the new configuration.
    fn deliver_update(&self, update: &CAMetalDisplayLinkUpdate) {
        let drawable = update.drawable();
        if self.lease.get() {
            return;
        }
        let texture = drawable.texture();
        self.lease.set(true);
        let frame = DrawableFrame {
            #[expect(
                clippy::cast_precision_loss,
                reason = "a drawable texture fits well inside f64's mantissa"
            )]
            drawable_size: CGSize::new(texture.width() as f64, texture.height() as f64),
            drawable,
            target_time: update.targetPresentationTimestamp(),
            generation: self.generation.get(),
            owner: self.this.clone(),
        };
        (self.on_frame)(frame);
    }
}

/// A `CAMetalLayer` + `CAMetalDisplayLink` presenter: one link on the main
/// run loop in common modes, at most one outstanding drawable lease.
///
/// Not `Clone`; main-thread only.
pub struct MetalPresenter {
    /// The owned state the delegate reaches weakly — `layer`, the
    /// issuing generation, the lease and the frame sink.
    inner: Rc<PresenterInner>,
    /// The display link scheduled on the main run loop — never absent:
    /// `Drop` invalidates it, and nothing else may run it down earlier.
    link: Retained<CAMetalDisplayLink>,
    /// The link's delegate — `CAMetalDisplayLink` keeps it weak, so the
    /// presenter retains it for the link's lifetime.
    #[expect(
        dead_code,
        reason = "the delegate is kept for the link's lifetime, never read"
    )]
    delegate: Retained<LinkDelegate>,
}

impl fmt::Debug for MetalPresenter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetalPresenter")
            .field("generation", &self.inner.generation.get())
            .field("lease", &self.inner.lease.get())
            .field("paused", &self.is_paused())
            .finish_non_exhaustive()
    }
}

impl MetalPresenter {
    /// Creates the presentation layer's presenter: builds the
    /// `CAMetalDisplayLink` on `layer`, adds it to the main run loop in
    /// common modes (paused — the owner unpauses when demand exists), and
    /// wires the delegate that issues [`DrawableFrame`]s to `on_frame`.
    ///
    /// `layer` is retained by the presenter; the caller adds it to the view
    /// hierarchy and owns its frame/hidden/contents-gravity bookkeeping.
    ///
    /// # Panics
    ///
    /// When called off the main thread.
    #[must_use]
    pub fn new(layer: Retained<CAMetalLayer>, on_frame: Rc<dyn Fn(DrawableFrame)>) -> Self {
        let mtm = MainThreadMarker::new().expect("MetalPresenter is created on the main thread");
        let inner = Rc::new_cyclic(|this| PresenterInner {
            layer,
            generation: Cell::new(0),
            lease: Cell::new(false),
            drawable_size: Cell::new(CGSize::ZERO),
            on_frame,
            this: this.clone(),
        });
        // The presenter owns the layer's static contract: drawables are
        // render-targets only, the pool is bounded at two, presenting never
        // blocks on the transaction, and until a new-size frame lands the
        // last frame scales. These setters must run BEFORE the link exists:
        // once a `CAMetalDisplayLink` is bound to the layer, mutating
        // `maximumDrawableCount` raises `CAMetalLayerInvalidOperation`
        // ("should not be called when using CAMetalDisplayLink").
        inner.layer.setFramebufferOnly(true);
        inner.layer.setMaximumDrawableCount(2);
        inner.layer.setPresentsWithTransaction(false);
        // SAFETY: `kCAGravityResize` is a system constant static.
        inner
            .layer
            .setContentsGravity(unsafe { objc2_quartz_core::kCAGravityResize });
        // SAFETY: `initWithMetalLayer:` is `CAMetalDisplayLink`'s designated
        // initializer, binding the link to `inner`'s layer.
        let link =
            CAMetalDisplayLink::initWithMetalLayer(CAMetalDisplayLink::alloc(), &inner.layer);
        let delegate = LinkDelegate::new(mtm, &inner);
        // The delegate is a plain `NSObject` protocol adoption;
        // `CAMetalDisplayLink` keeps it weak, the presenter retains it.
        link.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        link.setPaused(true);
        // SAFETY: the link is scheduled on the main run loop in common
        // modes; `NSRunLoopCommonModes` is a system constant.
        unsafe {
            link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
        }
        Self {
            inner,
            link,
            delegate,
        }
    }

    /// The presentation layer — frame, hidden state and colours are applied
    /// to it by the owner.
    #[must_use]
    pub fn layer(&self) -> Retained<CAMetalLayer> {
        self.inner.layer.clone()
    }

    /// The presenter's generation — a [`DrawableFrame`] presents only while
    /// the generation that issued it is still current.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.inner.generation.get()
    }

    /// Advances the generation: every outstanding frame becomes
    /// unpresentable, and its completion settles its lease without
    /// presenting. Called on any `drawableSize`, pixel format or colour-space
    /// change, on context replacement, on detach and on drop.
    pub fn advance_generation(&self) {
        self.inner
            .generation
            .set(self.inner.generation.get().wrapping_add(1));
    }

    /// Whether the link is paused — a paused link delivers no updates.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.link.isPaused()
    }

    /// Unpauses or pauses the link. The owner unpauses only while the view
    /// is attached, effectively visible, in an active scene and has demand;
    /// everything else pauses.
    pub fn set_paused(&self, paused: bool) {
        self.link.setPaused(paused);
    }

    /// Arms the link for the display's cadence: `preferredFrameRateRange`
    /// asks for the screen's real maximum.
    pub fn set_display_rate(&self, maximum_frames_per_second: f32) {
        self.link.setPreferredFrameRateRange(CAFrameRateRange::new(
            maximum_frames_per_second.min(60.0),
            maximum_frames_per_second,
            maximum_frames_per_second,
        ));
    }

    /// Swaps the device the drawable pool renders for — context
    /// replacement keeps the layer and link. Advances the generation.
    pub fn set_device(&self, device: &ProtocolObject<dyn MTLDevice>) {
        self.inner.layer.setDevice(Some(device));
        self.advance_generation();
    }

    /// Sets `drawableSize` from the physical size, in the same layout pass
    /// that changes it; a size change advances the generation so a frame
    /// rendered for the previous size can never present at the new one.
    pub fn set_drawable_size(&self, size: CGSize) {
        if self.inner.drawable_size.get() != size {
            self.inner.drawable_size.set(size);
            self.inner.layer.setDrawableSize(size);
            self.advance_generation();
        }
    }

    /// Retires the link and advances the generation: no further updates
    /// arrive, every outstanding frame becomes unpresentable, and the
    /// delegate's weak target makes a queued update after teardown inert.
    /// `Drop` runs this; nothing else may — teardown has one owner.
    fn invalidate(&self) {
        // SAFETY: the link was added to the main run loop; removing it
        // and invalidating is main-thread work by contract.
        unsafe {
            self.link
                .removeFromRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
        }
        self.link.invalidate();
        self.advance_generation();
    }
}

impl Drop for MetalPresenter {
    /// Detach and drop advance the generation and retire the link — a late
    /// completion settles its lease and presents nothing.
    fn drop(&mut self) {
        self.invalidate();
    }
}

/// One drawable pixel size as a `CGSize` — `drawableSize` is float-valued.
#[must_use]
pub const fn drawable_size(width: u32, height: u32) -> CGSize {
    CGSize::new(width as CGFloat, height as CGFloat)
}
