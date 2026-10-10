//! Main-thread content installed on a mandatory system-compositor plane.

use std::{cell::RefCell, fmt, rc::Rc};

use waterui_core::{Computed, NativeView, layout::StretchAxis};

/// The engine's platform-selected hosted object (`NSView` on macOS,
/// `SurfaceControl` on Android, `HtmlElement` on wasm32).
pub type HostedObject = <cherenkov_gpu::Gpu as cherenkov::HostedLayers>::Object;

/// The input and lifecycle half of hosted content.
///
/// Geometry belongs exclusively
/// to Cherenkov: the node supplies its local extent and the retained layer tree
/// supplies transforms, clipping, visibility and paint order.
///
/// There are deliberately no `Send`/`Sync` bounds: both DOM and `AppKit` objects live
/// on the UI thread. A content instance can be mounted in only one node.
pub trait HostedContent: 'static {
    /// Mounts this instance and returns its platform object. The published
    /// rectangles are in window logical coordinates and identify interactive
    /// Hydrolysis content painted above this plane. The host must reject hits
    /// in these rectangles, converting through its platform view hierarchy.
    fn mount(&self, occlusion: Rc<RefCell<Vec<kurbo::Rect>>>) -> HostedObject;

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
    pub occlusion: Rc<RefCell<Vec<kurbo::Rect>>>,
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
        let occlusion = Rc::new(RefCell::new(Vec::new()));
        let object = view.content.mount(Rc::clone(&occlusion));
        Rc::new(Self {
            focused: view.content.focused(),
            content: view.content,
            hosted: cherenkov::Hosted::new(object),
            occlusion,
            binding: RefCell::new(None),
        })
    }
}
