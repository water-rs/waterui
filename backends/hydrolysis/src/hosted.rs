//! Main-thread content installed on a mandatory system-compositor plane.

use std::{cell::RefCell, fmt, rc::Rc};

use waterui_core::{Computed, NativeView, layout::StretchAxis};

/// The engine's platform-selected hosted object: an `NSView` on macOS, a
/// `CALayer` on iOS, a `SurfaceControl` on Android and an `HtmlElement` on
/// wasm32.
pub type HostedObject = <cherenkov_gpu::Gpu as cherenkov::HostedLayers>::Object;

/// Where Hydrolysis paints interactive content above a hosted plane.
///
/// The renderer republishes the rectangles every frame, in window hit-test
/// space (logical points, y down from the window content's top-left). A host
/// refuses a hit that [`covers`](Self::covers) reports, so the event reaches
/// Hydrolysis's own hit testing instead.
#[derive(Clone, Default)]
pub struct HostedOcclusion {
    rects: Rc<RefCell<Vec<kurbo::Rect>>>,
}

impl HostedOcclusion {
    /// Whether Hydrolysis content painted above the plane takes a hit at
    /// `point`, in window hit-test space.
    #[must_use]
    pub fn covers(&self, point: kurbo::Point) -> bool {
        self.rects.borrow().iter().any(|rect| rect.contains(point))
    }

    /// The renderer's write side, published by the hit-test materialization.
    pub(crate) fn sink(&self) -> Rc<RefCell<Vec<kurbo::Rect>>> {
        Rc::clone(&self.rects)
    }
}

impl fmt::Debug for HostedOcclusion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("HostedOcclusion")
            .field(&self.rects.borrow())
            .finish()
    }
}

/// The input and lifecycle half of hosted content.
///
/// Geometry belongs exclusively
/// to Cherenkov: the node supplies its local extent and the retained layer tree
/// supplies transforms, clipping, visibility and paint order.
///
/// There are deliberately no `Send`/`Sync` bounds: both DOM and `AppKit` objects live
/// on the UI thread. A content instance can be mounted in only one node.
pub trait HostedContent: 'static {
    /// Mounts this instance and returns its platform object.
    ///
    /// The host refuses every hit `occlusion` covers, converting the hit's
    /// point into window hit-test space through its platform view hierarchy.
    fn mount(&self, occlusion: HostedOcclusion) -> HostedObject;

    /// Detaches the platform object and releases its focus observation. Called
    /// explicitly when the retained node retires, before engine binding cleanup.
    fn unmount(&self);

    /// Whether this object or a descendant currently owns platform focus.
    fn focused(&self) -> Computed<bool>;

    /// Moves platform focus into this content.
    fn request_focus(&self);
}

/// A hosted leaf. Wrap it in `Native::new` to insert it into a `WaterUI` tree.
///
/// It fills its proposal and requires a surface with a system-compositor parent;
/// unsupported transforms/effects fail with Cherenkov's Unplaceable error.
pub struct HostedView {
    pub(crate) content: Rc<dyn HostedContent>,
}

impl HostedView {
    /// Creates a leaf whose content will mount when its retained node is built.
    pub fn new(content: impl HostedContent) -> Self {
        Self {
            content: Rc::new(content),
        }
    }
}

impl fmt::Debug for HostedView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostedView").finish_non_exhaustive()
    }
}

impl NativeView for HostedView {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

pub struct HostedRuntime {
    pub content: Rc<dyn HostedContent>,
    pub hosted: cherenkov::Hosted<cherenkov_gpu::Gpu>,
    pub focused: Computed<bool>,
    pub occlusion: HostedOcclusion,
    pub binding: RefCell<Option<(cherenkov::LayerId, kurbo::Size)>>,
}

impl fmt::Debug for HostedRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostedRuntime")
            .field("binding", &self.binding.borrow())
            .finish_non_exhaustive()
    }
}

impl HostedRuntime {
    pub fn new(view: HostedView) -> Rc<Self> {
        let occlusion = HostedOcclusion::default();
        let object = view.content.mount(occlusion.clone());
        Rc::new(Self {
            focused: view.content.focused(),
            content: view.content,
            hosted: cherenkov::Hosted::new(object),
            occlusion,
            binding: RefCell::new(None),
        })
    }
}
