//! The `UIKit` split view: a `UISplitViewController` of sidebar and detail
//! columns.
//!
//! The controller shows `sidebar` and `detail` columns (a supplementary
//! column joins them for triple-column style). Collapsed-column reports
//! arrive through `set_collapse_handler` via `UISplitViewControllerDelegate`.
//!
//! # Safety
//!
//! The `unsafe` here defines `UIViewController` and `UISplitViewController`
//! subclasses — the controller is its own `UISplitViewControllerDelegate` —
//! and calls `objc2`/`UIKit` bindings marked unsafe because `UIKit`
//! view-controller APIs are main-thread only, which the `MainThreadOnly`
//! thread kind and [`MainThreadMarker`] constructor guarantee.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::NSObjectProtocol;
use objc2_foundation::NSString;
use objc2_ui_kit::{
    UISplitViewController, UISplitViewControllerColumn, UISplitViewControllerDelegate,
    UISplitViewControllerStyle, UIView, UIViewController,
};

/// The per-column preferred width range.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColumnWidth {
    /// The width the column settles at.
    pub preferred: Option<f64>,
    /// The narrowest the column may be.
    pub minimum: Option<f64>,
    /// The widest the column may be.
    pub maximum: Option<f64>,
}

/// A plain content view controller for a split column.
pub struct SplitColumnControllerIvars {
    /// Called when the controller's view appears.
    appear: RefCell<Option<Rc<dyn Fn()>>>,
    /// Called when the controller's view disappears.
    disappear: RefCell<Option<Rc<dyn Fn()>>>,
}

impl fmt::Debug for SplitColumnControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SplitColumnControllerIvars").finish()
    }
}

define_class!(
    // SAFETY: `UIViewController`'s designated initializer is
    // `initWithNibName:bundle:`; `SplitColumnController::new` passes
    // nil/None, and the class does not implement `Drop`.
    #[unsafe(super(UIViewController))]
    #[name = "CocoaUiSplitColumnController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SplitColumnControllerIvars]
    #[derive(Debug)]
    /// A `UIViewController` wrapping one column's content view.
    pub struct SplitColumnController;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIViewController`.
    unsafe impl NSObjectProtocol for SplitColumnController {}

    impl SplitColumnController {
        // SAFETY: overriding `viewDidAppear:` carries no obligations.
        #[unsafe(method(viewDidAppear:))]
        fn view_did_appear(&self, animated: bool) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewDidAppear: animated] };
            let handler = self.ivars().appear.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }

        // SAFETY: overriding `viewDidDisappear:` carries no obligations.
        #[unsafe(method(viewDidDisappear:))]
        fn view_did_disappear(&self, animated: bool) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewDidDisappear: animated] };
            let handler = self.ivars().disappear.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }
    }
);

impl SplitColumnController {
    /// A controller hosting `view`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, view: &UIView) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SplitColumnControllerIvars {
            appear: RefCell::new(None),
            disappear: RefCell::new(None),
        });
        // SAFETY: `initWithNibName:bundle:` is `UIViewController`'s
        // designated initializer; nil names and bundles load nothing.
        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithNibName: Option::<&NSString>::None,
                bundle: Option::<&objc2_foundation::NSBundle>::None
            ]
        };
        this.setView(Some(view));
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

/// The split controller's collapse state.
/// Called when a column collapses or expands: `(column, collapsed)`.
type CollapseHandler = Rc<dyn Fn(isize, bool)>;

/// The split controller's collapse state.
pub struct SplitControllerIvars {
    /// Called when a column collapses or expands: `(column, collapsed)`.
    collapse: RefCell<Option<CollapseHandler>>,
    /// The column a collapsed split shows first; `None` keeps the proposed
    /// column.
    collapsed_top: RefCell<Option<UISplitViewControllerColumn>>,
}

impl fmt::Debug for SplitControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SplitControllerIvars").finish()
    }
}

define_class!(
    // SAFETY: `UISplitViewController`'s designated initializer is
    // `initWithStyle:`; `SplitController::new` calls it with the column
    // count, and the class does not implement `Drop`.
    #[unsafe(super(UISplitViewController))]
    #[name = "CocoaUiSplitController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SplitControllerIvars]
    #[derive(Debug)]
    /// A `UISplitViewController` reporting column collapse.
    pub struct SplitController;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UISplitViewController`.
    unsafe impl NSObjectProtocol for SplitController {}

    // SAFETY: the delegate methods carry `UISplitViewControllerDelegate`'s
    // signatures.
    unsafe impl UISplitViewControllerDelegate for SplitController {
        // SAFETY: see the module safety note.
        #[unsafe(method(splitViewController:didHideColumn:))]
        fn split_view_controller_did_hide_column(
            &self,
            _svc: &UISplitViewController,
            column: UISplitViewControllerColumn,
        ) {
            let handler = self.ivars().collapse.borrow().clone();
            if let Some(handler) = handler {
                handler(column.0, true);
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(splitViewController:didShowColumn:))]
        fn split_view_controller_did_show_column(
            &self,
            _svc: &UISplitViewController,
            column: UISplitViewControllerColumn,
        ) {
            let handler = self.ivars().collapse.borrow().clone();
            if let Some(handler) = handler {
                handler(column.0, false);
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(splitViewController:topColumnForCollapsingToProposedTopColumn:))]
        fn split_view_controller_top_column_for_collapsing_to_proposed_top_column(
            &self,
            _svc: &UISplitViewController,
            proposed_top_column: UISplitViewControllerColumn,
        ) -> UISplitViewControllerColumn {
            self.ivars()
                .collapsed_top
                .borrow()
                .unwrap_or(proposed_top_column)
        }
    }
);

impl SplitController {
    /// A double- or triple-column split controller.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, triple: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SplitControllerIvars {
            collapse: RefCell::new(None),
            collapsed_top: RefCell::new(None),
        });
        // SAFETY: `initWithStyle:` is `UISplitViewController`'s designated
        // initializer for a columnar split.
        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithStyle: if triple {
                    UISplitViewControllerStyle::TripleColumn
                } else {
                    UISplitViewControllerStyle::DoubleColumn
                }
            ]
        };
        // SAFETY: the controller conforms to `UISplitViewControllerDelegate`;
        // the delegate is an assign reference.
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        this
    }

    /// Installs `controller` as `column`'s content: `0` sidebar, `1`
    /// supplementary, `2` detail/secondary.
    pub fn set_column(&self, column: UISplitViewControllerColumn, controller: &UIViewController) {
        self.setViewController_forColumn(Some(controller), column);
    }

    /// Whether `column` is collapsed right now.
    #[must_use]
    pub fn is_collapsed(&self, column: UISplitViewControllerColumn) -> bool {
        // SAFETY: `isCollapsed:` reads the column's collapse state.
        unsafe { msg_send![self, isCollapsed: column] }
    }

    /// Collapses or expands `column`: `Shown` to show it, `Hidden`/`Only`
    /// to collapse it.
    pub fn set_collapsed(&self, column: UISplitViewControllerColumn, collapsed: bool) {
        let display = if collapsed {
            objc2_ui_kit::UISplitViewControllerDisplayMode::OneBesideSecondary
        } else {
            objc2_ui_kit::UISplitViewControllerDisplayMode::TwoBesideSecondary
        };
        let _ = column;
        self.setPreferredDisplayMode(display);
    }

    /// Shows `column` — the programmatic counterpart of the sidebar toggle.
    pub fn show_column(&self, column: UISplitViewControllerColumn) {
        // SAFETY: `showColumn:` is a main-thread column transition.
        unsafe {
            let _: () = msg_send![self, showColumn: column];
        }
    }

    /// Hides `column`.
    pub fn hide_column(&self, column: UISplitViewControllerColumn) {
        // SAFETY: `hideColumn:` is a main-thread column transition.
        unsafe {
            let _: () = msg_send![self, hideColumn: column];
        }
    }

    /// Sets a column's width range.
    pub fn set_column_widths(
        &self,
        column: UISplitViewControllerColumn,
        preferred: f64,
        minimum: f64,
        maximum: f64,
    ) {
        if preferred > 0.0 {
            self.setPreferredPrimaryColumnWidth(preferred);
        }
        if minimum > 0.0 {
            self.setMinimumPrimaryColumnWidth(minimum);
        }
        if maximum > 0.0 {
            self.setMaximumPrimaryColumnWidth(maximum);
        }
        let _ = column;
    }

    /// Runs `handler` when a column collapses or expands: `(column,
    /// collapsed)`.
    pub fn set_collapse_handler(&self, handler: impl Fn(isize, bool) + 'static) {
        self.ivars().collapse.replace(Some(Rc::new(handler)));
    }

    /// The column a collapsing split lands on: `Some` overrides the proposed
    /// column in `topColumnForCollapsingToProposedTopColumn:`, `None` accepts
    /// it.
    pub fn set_collapsed_top_column(&self, column: Option<UISplitViewControllerColumn>) {
        self.ivars().collapsed_top.replace(column);
    }
}
