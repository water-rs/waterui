//! The `UIKit` tab bar: a `UITabBarController` reporting tab selection.
//!
//! Tabs carry a label and an optional symbol icon; `is_search` marks the
//! tab the system presents as the search tab. Selection is two-way:
//! `select` moves the highlight and `on_select` reports the tab the user
//! picked, including when the search tab is selected.
//!
//! # Safety
//!
//! The `unsafe` here defines `UIViewController` and `UITabBarController`
//! subclasses — the controller is its own `UITabBarControllerDelegate`,
//! watching `didSelectViewController` — and calls `objc2`/`UIKit` bindings
//! marked unsafe because `UIKit` view-controller APIs are main-thread only,
//! which the `MainThreadOnly` thread kind and [`MainThreadMarker`]
//! constructor guarantee.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSObjectProtocol, NSString};
use objc2_ui_kit::{
    UIImage, UITabBarController, UITabBarControllerDelegate, UITabBarItem, UIView, UIViewController,
};

/// One tab's content.
#[derive(Clone, Debug, Default)]
pub struct TabSpec {
    /// The tab's label.
    pub label: String,
    /// A system-symbol icon name.
    pub symbol: Option<String>,
    /// The tab's badge text.
    pub badge: Option<String>,
    /// Whether this tab is the search tab.
    pub is_search: bool,
    /// Whether the tab is enabled.
    pub enabled: bool,
}

/// A plain content view controller hosting one tab's content.
pub struct TabContentControllerIvars {}

impl fmt::Debug for TabContentControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TabContentControllerIvars").finish()
    }
}

define_class!(
    // SAFETY: `UIViewController`'s designated initializer is
    // `initWithNibName:bundle:`; `TabContentController::new` passes nil/None,
    // and the class does not implement `Drop`.
    #[unsafe(super(UIViewController))]
    #[name = "CocoaUiTabContentController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TabContentControllerIvars]
    #[derive(Debug)]
    /// A `UIViewController` hosting one tab's content view.
    pub struct TabContentController;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIViewController`.
    unsafe impl NSObjectProtocol for TabContentController {}
);

impl TabContentController {
    /// A controller hosting `view`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, view: &UIView) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TabContentControllerIvars {});
        // SAFETY: `initWithNibName:bundle:` is `UIViewController`'s
        // designated initializer; nil names and bundles load nothing.
        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithNibName: Option::<&NSString>::None,
                bundle: Option::<&objc2_foundation::NSBundle>::None
            ]
        };
        // `view` can itself be a controller's root — a `NavigationStack`
        // pane is a `UINavigationController`'s view. Capture that owner
        // before `setView` installs `this` as the view's responder
        // delegate, or the walk would return `this` and self-child.
        let owner = crate::uikit::view_controller::owning_controller(view);
        this.setView(Some(view));
        // UIKit only forwards appearance and layout callbacks down a real
        // containment chain, so the pane controller must become this
        // controller's child.
        if let Some(child) = owner
            && Retained::as_ptr(&child) != Retained::as_ptr(&this).cast()
        {
            crate::uikit::view_controller::add_child(&this, &child);
            crate::uikit::view_controller::did_move_to_parent(&child);
        }
        this
    }
}

/// The tab bar controller's selection state.
/// Called with the selected tab's index.
type SelectHandler = Rc<dyn Fn(usize)>;

/// The tab bar controller's selection state.
pub struct TabsControllerIvars {
    /// Called with the selected tab's index.
    select: RefCell<Option<SelectHandler>>,
}

impl fmt::Debug for TabsControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TabsControllerIvars").finish()
    }
}

define_class!(
    // SAFETY: `UITabBarController`'s designated initializer is
    // `initWithNibName:bundle:`; `TabsController::new` passes nil/None, and
    // the class does not implement `Drop`.
    #[unsafe(super(UITabBarController))]
    #[name = "CocoaUiTabsController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TabsControllerIvars]
    #[derive(Debug)]
    /// A `UITabBarController` reporting tab selection.
    pub struct TabsController;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UITabBarController`.
    unsafe impl NSObjectProtocol for TabsController {}

    // SAFETY: `tabBarController:didSelectViewController:` carries
    // `UITabBarControllerDelegate`'s signature.
    unsafe impl UITabBarControllerDelegate for TabsController {
        // SAFETY: see the module safety note.
        #[unsafe(method(tabBarController:didSelectViewController:))]
        fn tab_bar_controller_did_select_view_controller(
            &self,
            _tab_bar_controller: &UITabBarController,
            _view_controller: &UIViewController,
        ) {
            let index = self.selectedIndex();
            if index == usize::MAX {
                return;
            }
            let handler = self.ivars().select.borrow().clone();
            if let Some(handler) = handler {
                handler(index);
            }
        }
    }
);

impl TabsController {
    /// An empty tab bar controller.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TabsControllerIvars {
            select: RefCell::new(None),
        });
        // SAFETY: `initWithNibName:bundle:` is `UITabBarController`'s
        // designated initializer; nil names and bundles load nothing.
        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithNibName: Option::<&NSString>::None,
                bundle: Option::<&objc2_foundation::NSBundle>::None
            ]
        };
        // SAFETY: the controller conforms to `UITabBarControllerDelegate`;
        // the delegate is an assign reference.
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        this
    }

    /// Replaces the tabs: `specs` describe the tabs, `contents` their
    /// content views — the two are equal in length.
    pub fn set_tabs(&self, specs: &[TabSpec], contents: &[Retained<UIView>]) {
        let mtm = MainThreadMarker::from(self);
        let controllers: Vec<Retained<TabContentController>> = specs
            .iter()
            .zip(contents.iter())
            .map(|(spec, view)| {
                let controller = TabContentController::new(mtm, view);
                let title = NSString::from_str(&spec.label);
                let image = spec
                    .symbol
                    .as_ref()
                    .and_then(|symbol| UIImage::systemImageNamed(&NSString::from_str(symbol)));
                if spec.is_search {
                    let item = objc2_ui_kit::UITabBarItem::initWithTabBarSystemItem_tag(
                        mtm.alloc(),
                        objc2_ui_kit::UITabBarSystemItem::Search,
                        0,
                    );
                    // SAFETY: `setTabBarItem:` installs the search tab's item.
                    unsafe {
                        controller.setTabBarItem(Some(&item));
                    }
                } else {
                    controller.setTitle(Some(&title));
                }
                if let Some(item) = controller.tabBarItem() {
                    item.setImage(image.as_deref());
                    if let Some(badge) = &spec.badge {
                        item.setBadgeValue(Some(&NSString::from_str(badge)));
                    }
                    item.setEnabled(spec.enabled);
                }
                controller
            })
            .collect();
        self.setViewControllers_animated(
            Some(&NSArray::from_retained_slice(
                &controllers
                    .iter()
                    .map(|c| c.clone().into_super())
                    .collect::<Vec<_>>(),
            )),
            false,
        );
    }

    /// The `UITabBarItem` of the installed tab at `index`.
    ///
    /// Reactive chrome mutates the item in place: rebuilding the
    /// `viewControllers` array for a badge tick re-parents each pane view,
    /// which `UIViewController` rejects while the view is still another
    /// controller's `view`.
    ///
    /// # Panics
    /// Panics when `index` is out of range.
    #[must_use]
    pub fn tab_item(&self, index: usize) -> Retained<UITabBarItem> {
        let controllers = self.viewControllers().expect("tabs are installed");
        controllers
            .objectAtIndex(index)
            .downcast::<TabContentController>()
            .expect("tab controllers are CocoaUiTabContentController")
            .tabBarItem()
            .expect("a tab content controller always has a tab bar item")
    }

    /// The selected tab's index; `None` when nothing is selected.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        let index = self.selectedIndex();
        (index != usize::MAX).then_some(index)
    }

    /// Selects `index` without firing `on_select`.
    pub fn select(&self, index: usize) {
        self.setSelectedIndex(index);
    }

    /// Runs `handler` with the selected tab's index.
    pub fn set_select_handler(&self, handler: impl Fn(usize) + 'static) {
        self.ivars().select.replace(Some(Rc::new(handler)));
    }
}
