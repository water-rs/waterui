//! Window toolbar item promotion — the chrome behaviour
//! `WuiWindowToolbar.setWindowContent` carried, now owned by the backend.
//!
//! A declared window-toolbar child promotes to a real `NSToolbarItem` when
//! it is a button whose label draws a platform symbol or renders into a
//! template image; every other child is hosted as the view it is.

#![cfg(target_os = "macos")]

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use cocoa_ui::Retained;
use cocoa_ui::appkit::{HostedItem, ToolbarChild};
use cocoa_ui::objc2_app_kit::{NSView, NSWindow};
use objc2::msg_send;
use objc2_foundation::NSString;
use waterui_backend_core::Environment;

#[cfg(feature = "gpu_surface")]
use alloc::collections::BTreeMap;
#[cfg(feature = "gpu_surface")]
use core::cell::RefCell;

/// The point size a rendered label rasterizes into a toolbar icon.
const TOOLBAR_ICON_SIZE: f64 = 18.0;

/// Installs `view` — the rendered window-toolbar host — as `window`'s
/// toolbar items: lone-child wrappers are descended and the first
/// multi-child view's children become the items, as
/// `WuiWindowToolbar.setWindowContent` answered them.
///
/// `env` is the environment the toolbar subtree was rendered under; icon
/// rasters resolve their GPU surfaces through it. Returns the mounted
/// jobs handle — the caller keeps it in the window content's keep-alive,
/// so closing or remounting the toolbar drops and cancels every raster
/// this install started.
pub(crate) fn install_toolbar_items(
    window: &NSWindow,
    view: &NSView,
    env: &Environment,
) -> Rc<IconJobs> {
    let mut node = cocoa_ui::view::retain_base(view);
    loop {
        let subs = cocoa_ui::view::subviews(&node);
        if subs.len() != 1 {
            break;
        }
        node = subs[0].clone();
    }
    let subs = cocoa_ui::view::subviews(&node);
    let children: Vec<Retained<NSView>> = if subs.is_empty() {
        alloc::vec![node]
    } else {
        subs
    };
    let children = Rc::new(children);
    let coordinator = cocoa_ui::appkit::WindowToolbar::attached(window);
    let jobs = IconJobs::new(env);
    #[cfg(feature = "gpu_surface")]
    jobs.set_refresh({
        let coordinator = coordinator.clone();
        let children = children.clone();
        let jobs = Rc::downgrade(&jobs);
        move || {
            let Some(jobs) = jobs.upgrade() else {
                return;
            };
            coordinator.set_window_items(
                children
                    .iter()
                    .enumerate()
                    .map(|(index, view)| toolbar_child(view, index, &jobs))
                    .collect(),
            );
        }
    });
    coordinator.set_window_items(
        children
            .iter()
            .enumerate()
            .map(|(index, view)| toolbar_child(view, index, &jobs))
            .collect(),
    );
    jobs
}

/// Describes one declared toolbar child to the kit: a button whose label
/// draws a platform symbol becomes a real `NSToolbarItem` — icon in the
/// capsule, name kept for the overflow menu, tooltip and assistive
/// technology, running the button's action — exactly as a navigation
/// action does. A label that is not a platform symbol renders into a
/// template image the toolbar tints like its own items, and any other
/// child is hosted as the view it is.
fn toolbar_child(view: &Retained<NSView>, index: usize, jobs: &Rc<IconJobs>) -> ToolbarChild {
    let hosted = |view: &Retained<NSView>| HostedItem {
        view: view.clone(),
        size: view.fittingSize().into(),
    };
    let Some(button) = cocoa_ui::appkit::first_button(view) else {
        return ToolbarChild {
            view: hosted(view),
            icon: None,
            label: String::new(),
            bordered: false,
            action: None,
        };
    };
    // The button's label view is its sibling: the button leaf mounts the
    // button and its label container into the same parent.
    let label_view = {
        let button_view: *const NSView = &raw const ****button;
        // SAFETY: `superview`/`subviews` are ordinary main-thread reads.
        unsafe { button.superview() }.and_then(|parent| {
            parent
                .subviews()
                .into_iter()
                .find(|subview| !core::ptr::eq::<NSView>(&raw const **subview, button_view))
        })
    };
    let icon = label_view
        .as_ref()
        .and_then(|label| cocoa_ui::appkit::first_symbol_view(label))
        .and_then(|symbol_view| symbol_view.symbol_name())
        .and_then(|name| cocoa_ui::appkit::symbol_image(&name))
        .or_else(|| {
            label_view
                .as_ref()
                .and_then(|label| raster_label_icon(index, label, jobs))
        });
    // SAFETY: `accessibilityLabel` is a plain getter on the main thread.
    let text: Option<Retained<NSString>> = unsafe { msg_send![&button, accessibilityLabel] };
    let label = text.map_or_else(String::new, |label| label.to_string());
    // The label's own button carries the handler, so the toolbar item runs
    // the same action the view would have.
    let action = Rc::new(move || {
        // SAFETY: toolbar actions fire on the main thread.
        unsafe { cocoa_ui::appkit::activate(button.control()) };
    });
    ToolbarChild {
        view: hosted(view),
        icon,
        label,
        bordered: true,
        action: Some(action),
    }
}

/// The label raster for one toolbar child under `gpu_surface`: the slot
/// answers `None` until the async raster lands, at which point the jobs'
/// refresh republishes the items with the image.
#[cfg(feature = "gpu_surface")]
fn raster_label_icon(
    index: usize,
    label: &Retained<NSView>,
    jobs: &Rc<IconJobs>,
) -> Option<Retained<cocoa_ui::objc2_app_kit::NSImage>> {
    jobs.image(index, label, TOOLBAR_ICON_SIZE)
}

/// The label raster in a native-only build: the plain bitmap path, which
/// is complete because no GPU surface can exist in the subtree.
#[cfg(not(feature = "gpu_surface"))]
fn raster_label_icon(
    _index: usize,
    label: &Retained<NSView>,
    _jobs: &Rc<IconJobs>,
) -> Option<Retained<cocoa_ui::objc2_app_kit::NSImage>> {
    cocoa_ui::bitmap::view_template_image(label, TOOLBAR_ICON_SIZE)
}

/// The shared icon-raster slots chrome publishers keep: a `View` icon —
/// any declared view, including one hosting GPU content — cannot be drawn
/// synchronously under `CAMetalLayer`, so each item publishes iconless and
/// the job below fills the slot once the central capture lands, then
/// republishes through `refresh`.
///
/// Keyed by the caller's own item identity (a child index or an item
/// address): each publisher owns one `IconJobs`, installs one `refresh`,
/// and keeps the `Rc` in its keep-alive or state so unmounting cancels
/// every outstanding raster.
#[cfg(feature = "gpu_surface")]
pub(crate) struct IconJobs {
    /// The environment the rasters resolve their surfaces through.
    env: Environment,
    /// The republish fired when an icon lands.
    refresh: RefCell<Option<Rc<dyn Fn()>>>,
    /// Per-key slots: the landed image, or the task still in flight.
    slots: RefCell<BTreeMap<usize, IconSlot>>,
}

/// One key's raster state.
#[cfg(feature = "gpu_surface")]
struct IconSlot {
    /// The landed image — republished reads take it.
    image: Option<Retained<cocoa_ui::objc2_app_kit::NSImage>>,
    /// The in-flight raster; dropping it cancels the work.
    task: Option<executor_core::AnyLocalExecutorTask<()>>,
}

/// The marker the same publishers hold in a native-only build: with no
/// `gpu_surface` feature there is nothing to raster asynchronously.
#[cfg(not(feature = "gpu_surface"))]
pub(crate) struct IconJobs;

#[cfg(feature = "gpu_surface")]
impl IconJobs {
    /// Empty icon jobs resolving against `env`.
    pub(crate) fn new(env: &Environment) -> Rc<Self> {
        Rc::new(Self {
            env: env.clone(),
            refresh: RefCell::new(None),
            slots: RefCell::new(BTreeMap::new()),
        })
    }

    /// Arms the republish a landing icon triggers — set once, before the
    /// first [`image`](Self::image) call.
    pub(crate) fn set_refresh(self: &Rc<Self>, refresh: impl Fn() + 'static) {
        *self.refresh.borrow_mut() = Some(Rc::new(refresh));
    }

    /// `key`'s raster for `view`: `Some` once landed; otherwise the raster
    /// task is started on first call and the slot answers `None` until the
    /// image arrives and `refresh` republishes.
    pub(crate) fn image(
        self: &Rc<Self>,
        key: usize,
        view: &NSView,
        max_side: f64,
    ) -> Option<Retained<cocoa_ui::objc2_app_kit::NSImage>> {
        let mut slots = self.slots.borrow_mut();
        let slot = slots.entry(key).or_insert(IconSlot {
            image: None,
            task: None,
        });
        if let Some(image) = &slot.image {
            return Some(image.clone());
        }
        if slot.task.is_none() {
            let view = cocoa_ui::view::retain_base(view);
            let env = self.env.clone();
            let jobs = Rc::downgrade(self);
            slot.task = Some(executor_core::spawn_local(async move {
                let image = crate::capture_image::template_image(&view, &env, max_side).await;
                let Some(jobs) = jobs.upgrade() else {
                    return;
                };
                {
                    let mut slots = jobs.slots.borrow_mut();
                    if let Some(slot) = slots.get_mut(&key) {
                        slot.image = Some(image);
                    }
                }
                if let Some(refresh) = jobs.refresh.borrow().clone() {
                    refresh();
                }
            }));
        }
        None
    }
}

#[cfg(not(feature = "gpu_surface"))]
impl IconJobs {
    /// The empty marker.
    pub(crate) fn new(_env: &Environment) -> Rc<Self> {
        Rc::new(Self)
    }
}
