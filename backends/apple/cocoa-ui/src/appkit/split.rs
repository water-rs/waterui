//! The `AppKit` split view: an `NSSplitViewController` of typed columns.
//!
//! The controller owns up to three columns — sidebar, supplementary,
//! detail — each an `NSSplitViewItem` wrapping a plain-content
//! `NSViewController`. The sidebar item is created `allowsFullHeightLayout`
//! so it runs the window's full height. Column collapse reports through
//! `set_collapse_handler`, watched via raw KVO on each item's `collapsed`.
//!
//! # Safety
//!
//! The `unsafe` here defines `NSViewController` and `NSSplitViewController`
//! subclasses, registers them as KVO observers of the columns' `collapsed`
//! — each registration is removed before the column set is replaced — and
//! calls `objc2`/`AppKit` bindings marked unsafe because `AppKit`
//! view-controller APIs are main-thread only, which the `MainThreadOnly`
//! thread kind and [`MainThreadMarker`] constructor guarantee.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSSplitViewController, NSSplitViewItem, NSSplitViewItemBehavior, NSView, NSViewController,
};
use objc2_foundation::{
    NSDictionary, NSKeyValueChangeKey, NSKeyValueObservingOptions,
    NSObjectNSKeyValueObserverRegistration, NSObjectProtocol, NSString,
};

/// Which column a collapse event concerns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    /// The leading sidebar column.
    Sidebar,
    /// The middle supplementary column.
    Supplementary,
    /// The trailing detail column.
    Detail,
}

/// The per-column preferred width range.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColumnWidth {
    /// The content extent the column settles at: the divider is placed at
    /// this width plus whatever insets the pane applies around the item's
    /// view (the sidebar's concentric-glass margin).
    pub preferred: Option<f64>,
    /// The narrowest the column may be.
    pub minimum: Option<f64>,
    /// The widest the column may be.
    pub maximum: Option<f64>,
}

/// A plain-content view controller for a split column, reporting when its
/// view moves on and off screen.
pub struct ColumnControllerIvars {
    /// Called when the controller's view appears on screen.
    appear: RefCell<Option<Rc<dyn Fn()>>>,
    /// Called when the controller's view leaves the screen.
    disappear: RefCell<Option<Rc<dyn Fn()>>>,
}

impl fmt::Debug for ColumnControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColumnControllerIvars").finish()
    }
}

define_class!(
    // SAFETY: `NSViewController`'s designated initializer is
    // `initWithNibName:bundle:`; `ColumnController::new` passes nil/None, and
    // the class does not implement `Drop`.
    #[unsafe(super(NSViewController))]
    #[name = "CocoaUiSplitColumnController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ColumnControllerIvars]
    #[derive(Debug)]
    /// An `NSViewController` wrapping one column's content view.
    pub struct ColumnController;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSViewController`.
    unsafe impl NSObjectProtocol for ColumnController {}

    impl ColumnController {
        // SAFETY: overriding `viewDidAppear` carries no obligations.
        #[unsafe(method(viewDidAppear))]
        fn view_did_appear(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewDidAppear] };
            let handler = self.ivars().appear.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: overriding `viewDidDisappear` carries no obligations.
        #[unsafe(method(viewDidDisappear))]
        fn view_did_disappear(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewDidDisappear] };
            let handler = self.ivars().disappear.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }
    }
);

impl ColumnController {
    /// A controller hosting `view`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, view: &NSView) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ColumnControllerIvars {
            appear: RefCell::new(None),
            disappear: RefCell::new(None),
        });
        // SAFETY: `initWithNibName:bundle:` is `NSViewController`'s
        // designated initializer; nil names and bundles load nothing.
        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithNibName: Option::<&NSString>::None,
                bundle: Option::<&objc2_foundation::NSBundle>::None
            ]
        };
        this.setView(view);
        this
    }

    /// Runs `handler` each time the column's view appears.
    pub fn set_appear_handler(&self, handler: impl Fn() + 'static) {
        self.ivars().appear.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` each time the column's view disappears.
    pub fn set_disappear_handler(&self, handler: impl Fn() + 'static) {
        self.ivars().disappear.replace(Some(Rc::new(handler)));
    }
}

/// The `collapsed` key-path observation's context token.
static COLLAPSE_CONTEXT: u8 = 1;

/// The split view controller's state: collapse reporting, the columns'
/// retained content controllers and their declared widths.
/// Called when a column collapses or expands: `(column, collapsed)`.
type CollapseHandler = Rc<dyn Fn(Column, bool)>;

/// The split view controller's state: collapse reporting, the columns'
/// retained content controllers and their declared widths.
pub struct SplitViewControllerIvars {
    /// Called when a column collapses or expands: `(column, collapsed)`.
    collapse: RefCell<Option<CollapseHandler>>,
    /// The columns' retained content controllers.
    controllers: RefCell<Vec<Retained<ColumnController>>>,
    /// Whether the preferred column widths have been applied — the columns
    /// settle at them once, then the user's widths win.
    widths_applied: Cell<bool>,
    /// The columns' declared widths, until the preferred widths land.
    preferred_widths: RefCell<Vec<ColumnWidth>>,
}

impl fmt::Debug for SplitViewControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SplitViewControllerIvars")
            .field("controllers", &self.controllers.borrow().len())
            .field("widths_applied", &self.widths_applied.get())
            .field("preferred_widths", &self.preferred_widths.borrow())
            .field("has_collapse", &self.collapse.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSSplitViewController`'s designated initializer is
    // `initWithNibName:bundle:`; `SplitViewController::new` passes nil/None,
    // and the class does not implement `Drop`. KVO registrations are removed
    // in `set_columns` before the items are replaced.
    #[unsafe(super(NSSplitViewController))]
    #[name = "CocoaUiSplitViewController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SplitViewControllerIvars]
    #[derive(Debug)]
    /// An `NSSplitViewController` reporting column collapse.
    pub struct SplitViewController;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSSplitViewController`.
    unsafe impl NSObjectProtocol for SplitViewController {}

    impl SplitViewController {
        // SAFETY: overriding `viewDidLayout` carries no obligations.
        #[unsafe(method(viewDidLayout))]
        fn view_did_layout(&self) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewDidLayout] };
            self.apply_preferred_widths_once();
        }

        // SAFETY: the raw-KVO callback's signature matches
        // `NSKeyValueObserving`'s; only the registered context is answered.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value_for_key_path(
            &self,
            _key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            context: *mut std::ffi::c_void,
        ) {
            if context != std::ptr::from_ref(&COLLAPSE_CONTEXT).cast_mut().cast() {
                return;
            }
            let Some(object) = object else { return };
            // SAFETY: the observed object is the `NSSplitViewItem` the KVO
            // registration was placed on.
            let Some(item) = (unsafe {
                Retained::retain(std::ptr::from_ref(object).cast_mut().cast::<NSSplitViewItem>())
            }) else {
                return;
            };
            // SAFETY: `NSKeyValueChangeNewKey` is an immutable extern
            // constant.
            let collapsed = change
                .and_then(|change| unsafe {
                    change.objectForKey(objc2_foundation::NSKeyValueChangeNewKey)
                })
                .and_then(|value| value.downcast::<objc2_foundation::NSNumber>().ok())
                .is_some_and(|number| number.as_bool());
            self.report_collapse(&item, collapsed);
        }
    }
);

impl SplitViewController {
    /// An empty split view controller.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SplitViewControllerIvars {
            collapse: RefCell::new(None),
            controllers: RefCell::new(Vec::new()),
            widths_applied: Cell::new(false),
            preferred_widths: RefCell::new(Vec::new()),
        });
        // SAFETY: `initWithNibName:bundle:` is `NSSplitViewController`'s
        // designated initializer; nil names and bundles load nothing.
        unsafe {
            msg_send![
                super(this),
                initWithNibName: Option::<&NSString>::None,
                bundle: Option::<&objc2_foundation::NSBundle>::None
            ]
        }
    }

    /// Replaces the columns with `sidebar`, optional `supplementary` and
    /// `detail` content views.
    pub fn set_columns(&self, sidebar: &NSView, supplementary: Option<&NSView>, detail: &NSView) {
        let mtm = MainThreadMarker::from(self);
        self.unobserve_items();
        let mut items = Vec::new();
        let mut controllers = Vec::new();

        let sidebar_controller = ColumnController::new(mtm, sidebar);
        let sidebar_item = NSSplitViewItem::sidebarWithViewController(&sidebar_controller);
        // `allowsFullHeightLayout` is what lets the sidebar run the window's
        // full height — traffic lights inside it.
        sidebar_item.setAllowsFullHeightLayout(true);
        sidebar_item.setCanCollapse(true);
        controllers.push(sidebar_controller);
        items.push(sidebar_item);

        if let Some(supplementary) = supplementary {
            let controller = ColumnController::new(mtm, supplementary);
            let item = NSSplitViewItem::contentListWithViewController(&controller);
            controllers.push(controller);
            items.push(item);
        }
        let detail_controller = ColumnController::new(mtm, detail);
        items.push(NSSplitViewItem::splitViewItemWithViewController(
            &detail_controller,
        ));
        controllers.push(detail_controller);

        for item in self.splitViewItems().iter().collect::<Vec<_>>() {
            self.removeSplitViewItem(&item);
        }
        for item in &items {
            self.addSplitViewItem(item);
        }
        self.ivars().controllers.replace(controllers);
        self.ivars().widths_applied.set(false);
        self.observe_items(&items);
    }

    /// Sets the columns' preferred/minimum/maximum widths. The preferred
    /// widths land once at the first layout — the user's widths win after.
    pub fn set_column_widths(&self, widths: &[ColumnWidth]) {
        self.ivars().widths_applied.set(false);
        self.ivars().preferred_widths.replace(widths.to_vec());
        let items = self.splitViewItems();
        for (index, item) in items.iter().enumerate() {
            let Some(width) = widths.get(index) else {
                continue;
            };
            if let Some(minimum) = width.minimum {
                item.setMinimumThickness(minimum);
            }
            if let Some(maximum) = width.maximum {
                item.setMaximumThickness(maximum);
            }
        }
    }

    /// Applies each declared preferred width to its divider, once.
    fn apply_preferred_widths_once(&self) {
        if self.ivars().widths_applied.replace(true) {
            return;
        }
        let widths = self.ivars().preferred_widths.borrow().clone();
        let split_view = self.splitView();
        let arranged = split_view.arrangedSubviews();
        let items = self.splitViewItems();
        for (index, width) in widths.iter().enumerate() {
            let Some(preferred) = width.preferred else {
                continue;
            };
            if index >= arranged.count().saturating_sub(1) {
                break;
            }
            // A preferred width declares the column's content extent. The
            // platform can inset the item's view inside its pane — the
            // sidebar's concentric-glass margin — so the divider lands at
            // the content width plus the insets the pane itself reports.
            let position = if index < items.count() {
                let content = items
                    .objectAtIndex(index)
                    .viewController(MainThreadMarker::from(self))
                    .view();
                let pane = arranged.objectAtIndex(index);
                let in_pane =
                    crate::view::convert_rect(&content, crate::view::bounds(&content), Some(&pane));
                if in_pane.size.width <= 0.0 {
                    preferred
                } else {
                    preferred + pane.frame().size.width - in_pane.size.width
                }
            } else {
                preferred
            };
            split_view.setPosition_ofDividerAtIndex(position, index.cast_signed());
        }
    }

    /// Whether a column is collapsed right now.
    #[must_use]
    pub fn is_collapsed(&self, column: Column) -> bool {
        let index = self.column_index(column);
        self.splitViewItems()
            .iter()
            .nth(index)
            .is_some_and(|item| item.isCollapsed())
    }

    /// Collapses or expands a column.
    pub fn set_collapsed(&self, column: Column, collapsed: bool) {
        let index = self.column_index(column);
        if let Some(item) = self.splitViewItems().iter().nth(index) {
            // SAFETY: `setCollapsed:` is a main-thread property write on a
            // live split view item.
            unsafe {
                let _: () = msg_send![&item, setCollapsed: collapsed];
            }
        }
    }

    fn column_index(&self, column: Column) -> usize {
        let triple = self.splitViewItems().count() == 3;
        match column {
            Column::Sidebar => 0,
            Column::Supplementary => usize::from(triple),
            Column::Detail => usize::from(triple) + 1,
        }
    }

    fn observe_items(&self, items: &[Retained<NSSplitViewItem>]) {
        for item in items {
            // SAFETY: paired with `unobserve_items`, which runs before the
            // items are replaced; the context token disambiguates this
            // registration in the callback.
            unsafe {
                item.addObserver_forKeyPath_options_context(
                    self,
                    &NSString::from_str("collapsed"),
                    NSKeyValueObservingOptions::New,
                    std::ptr::from_ref(&COLLAPSE_CONTEXT)
                        .cast_mut()
                        .cast::<std::ffi::c_void>(),
                );
            }
        }
    }

    fn unobserve_items(&self) {
        for item in self.splitViewItems().iter().collect::<Vec<_>>() {
            // SAFETY: removes the registration `observe_items` added for
            // each live column item.
            unsafe {
                item.removeObserver_forKeyPath_context(
                    self,
                    &NSString::from_str("collapsed"),
                    std::ptr::from_ref(&COLLAPSE_CONTEXT)
                        .cast_mut()
                        .cast::<std::ffi::c_void>(),
                );
            }
        }
    }

    fn report_collapse(&self, item: &NSSplitViewItem, collapsed: bool) {
        let column = match item.behavior() {
            NSSplitViewItemBehavior::Sidebar => Column::Sidebar,
            NSSplitViewItemBehavior::ContentList => Column::Supplementary,
            _ => Column::Detail,
        };
        let handler = self.ivars().collapse.borrow().clone();
        if let Some(handler) = handler {
            handler(column, collapsed);
        }
    }

    /// The column content controllers, in order — for wiring lifecycle.
    #[must_use]
    pub fn column_controllers(&self) -> Vec<Retained<ColumnController>> {
        self.ivars().controllers.borrow().clone()
    }

    /// Runs `handler` when a column collapses or expands.
    pub fn set_collapse_handler(&self, handler: impl Fn(Column, bool) + 'static) {
        self.ivars().collapse.replace(Some(Rc::new(handler)));
    }
}
