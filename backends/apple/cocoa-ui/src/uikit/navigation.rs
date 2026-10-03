//! The `UIKit` navigation stack: a `UINavigationController` of content
//! view controllers.
//!
//! The controller keeps a stack of pages. `push`/`pop` animate the
//! transition; `set_pages` rebuilds the stack when the path changes outside
//! a user gesture. Each page's content lives in a plain content controller
//! so `set_nav_page` can install per-page chrome — leading/trailing bar
//! items, title, subtitle — on its `UINavigationItem`. The back-swipe edge
//! gesture reports through `set_pop_handler`, and interactive-pop
//! completion/cancellation is reported with `pop_completed`, so the model
//! path and the visible stack never disagree.
//!
//! # Safety
//!
//! The `unsafe` here defines `UIViewController` and
//! `UINavigationController` subclasses — the controller is its own
//! `UINavigationControllerDelegate`, watching transitions — and calls
//! `objc2`/`UIKit` bindings marked unsafe because `UIKit` view-controller
//! APIs are main-thread only, which the `MainThreadOnly` thread kind and
//! [`MainThreadMarker`] constructor guarantee.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSObjectProtocol, NSString};
use objc2_ui_kit::{
    UIBarButtonItem, UIBarButtonItemStyle, UIButton, UIControl, UIGestureRecognizer,
    UIGestureRecognizerDelegate, UINavigationBar, UINavigationController,
    UINavigationControllerDelegate, UINavigationItem, UINavigationItemLargeTitleDisplayMode,
    UISearchController, UISearchResultsUpdating, UIViewController,
};

/// `viewWillAppear:` listener, called with `UIKit`'s `animated` flag.
type WillAppearHandler = Rc<dyn Fn(bool)>;

/// A plain content view controller, reporting when its view appears.
pub struct NavContentControllerIvars {
    /// Called when the controller's view appears.
    appear: RefCell<Option<Rc<dyn Fn()>>>,
    /// Called when the controller's view is about to appear.
    will_appear: RefCell<Option<WillAppearHandler>>,
    /// Called when the controller's view disappears.
    disappear: RefCell<Option<Rc<dyn Fn()>>>,
    /// The search field's change sink, reporting the current text.
    search_change: RefCell<Option<SearchChangeHandler>>,
    /// The page's `UISearchResultsUpdating`, retained for the weak outlet.
    search_updater: RefCell<Option<Retained<SearchUpdater>>>,
    /// The page's search drawer, retained for later updates.
    search_controller: RefCell<Option<Retained<UISearchController>>>,
}

impl fmt::Debug for NavContentControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavContentControllerIvars").finish()
    }
}

define_class!(
    // SAFETY: `UIViewController`'s designated initializer is
    // `initWithNibName:bundle:`; `NavContentController::new` passes nil/None,
    // and the class does not implement `Drop`.
    #[unsafe(super(UIViewController))]
    #[name = "CocoaUiNavContentController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = NavContentControllerIvars]
    #[derive(Debug)]
    /// A `UIViewController` hosting one navigation page's content.
    pub struct NavContentController;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIViewController`.
    unsafe impl NSObjectProtocol for NavContentController {}

    impl NavContentController {
        // SAFETY: overriding `viewWillAppear:` carries no obligations.
        #[unsafe(method(viewWillAppear:))]
        fn view_will_appear(&self, animated: bool) {
            // SAFETY: see the module safety note.
            let _: () = unsafe { msg_send![super(self), viewWillAppear: animated] };
            let handler = self.ivars().will_appear.borrow().clone();
            if let Some(handler) = handler {
                handler(animated);
            }
        }

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

impl NavContentController {
    /// A controller hosting `view` — added as a subview covering the
    /// controller's view, laid out at `viewDidLayoutSubviews`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, view: &objc2_ui_kit::UIView) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavContentControllerIvars {
            appear: RefCell::new(None),
            will_appear: RefCell::new(None),
            disappear: RefCell::new(None),
            search_change: RefCell::new(None),
            search_updater: RefCell::new(None),
            search_controller: RefCell::new(None),
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
        // A nested `UINavigationController`'s view (a stack inside a page)
        // still needs the containment chain for its appearance callbacks;
        // capture the owner before `setView` installs `this` as the view's
        // responder delegate.
        let owner = crate::uikit::view_controller::owning_controller(view);
        this.setView(Some(view));
        if let Some(child) = owner
            && Retained::as_ptr(&child) != Retained::as_ptr(&this).cast()
        {
            crate::uikit::view_controller::add_child(&this, &child);
            crate::uikit::view_controller::did_move_to_parent(&child);
        }
        this
    }

    /// Runs `handler` each time the page's view appears.
    pub fn set_appear_handler(&self, handler: impl Fn() + 'static) {
        self.ivars().appear.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` each time the page's view is about to appear, with
    /// `UIKit`'s `animated` argument.
    pub fn set_will_appear_handler(&self, handler: impl Fn(bool) + 'static) {
        self.ivars().will_appear.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` each time the page's view disappears.
    pub fn set_disappear_handler(&self, handler: impl Fn() + 'static) {
        self.ivars().disappear.replace(Some(Rc::new(handler)));
    }

    /// The navigation item chrome this page installs.
    ///
    /// `UINavigationItem` is a property the controller itself owns, not part
    /// of the view hierarchy, so the page's chrome applies through it rather
    /// than through a subview.
    pub fn set_page(&self, page: &NavPage) {
        // SAFETY: `navigationItem` is a `UIViewController` getter on the
        // main thread; the returned item is owned by the controller.
        let item: Retained<objc2_ui_kit::UINavigationItem> =
            unsafe { msg_send![self, navigationItem] };
        item.setTitle(Some(&NSString::from_str(&page.title)));
        item.setTitleView(page.title_view.as_deref());
        item.setSubtitle(page.subtitle.as_deref().map(NSString::from_str).as_deref());
        let mtm = self.mtm();
        if page.leading.is_empty() {
            item.setLeftBarButtonItems(None);
        } else {
            item.setLeftBarButtonItems(Some(&NSArray::from_retained_slice(&page.leading)));
        }
        item.setLeftItemsSupplementBackButton(!page.leading.is_empty());
        if page.trailing.is_empty() {
            item.setRightBarButtonItems(None);
        } else {
            item.setRightBarButtonItems(Some(&NSArray::from_retained_slice(&page.trailing)));
        }
        item.setLargeTitleDisplayMode(page.large_title.native());
        // The page controls its own back affordance: a semantic back action,
        // or no back at all (a root page, or a page that declared none).
        item.setHidesBackButton(page.hides_back);
        if let Some(on_back) = &page.on_back {
            let on_back = on_back.clone();
            // SAFETY: `UIAction::actionWithHandler:` retains the block, which
            // owns the `Rc` for the action's life.
            let action = unsafe {
                objc2_ui_kit::UIAction::actionWithHandler(
                    block2::RcBlock::into_raw(RcBlock::new(
                        move |_: core::ptr::NonNull<objc2_ui_kit::UIAction>| {
                            on_back();
                        },
                    )),
                    mtm,
                )
            };
            item.setBackAction(Some(&action));
        }
        if page.bottom.is_empty() {
            self.setToolbarItems(None);
            if let Some(nav) = self.navigationController() {
                nav.setToolbarHidden_animated(true, false);
            }
        } else {
            self.setToolbarItems(Some(&NSArray::from_retained_slice(&page.bottom)));
            if let Some(nav) = self.navigationController() {
                nav.setToolbarHidden_animated(false, false);
            }
        }
        self.ivars().search_updater.replace(None);
        self.ivars().search_controller.replace(None);
        if let Some(search) = &page.search {
            // SAFETY: `initWithSearchResultsController:` is
            // `UISearchController`'s designated initializer; a nil
            // results controller keeps the current content — a
            // results-less search controller draws over the page
            // itself.
            let controller: Retained<UISearchController> = unsafe {
                msg_send![
                    mtm.alloc::<UISearchController>(),
                    initWithSearchResultsController: Option::<&UIViewController>::None
                ]
            };
            let bar = controller.searchBar();
            bar.setPlaceholder(Some(&NSString::from_str(&search.placeholder)));
            bar.setText(Some(&NSString::from_str(&search.text)));
            let updater = SearchUpdater::new(mtm);
            updater.set_change_handler({
                let this = Retained::<Self>::from(self);
                move |text| {
                    if let Some(handler) = this.ivars().search_change.borrow().as_ref() {
                        handler(text);
                    }
                }
            });
            controller.setSearchResultsUpdater(Some(ProtocolObject::from_ref(&*updater)));
            if let Some(placement) = search.placement {
                item.setPreferredSearchBarPlacement(placement.native());
            }
            if let Some(hides) = search.hides_when_scrolling {
                item.setHidesSearchBarWhenScrolling(hides);
            }
            item.setSearchController(Some(&controller));
            // `searchResultsUpdater` is a weak outlet — the controller
            // keeps the updater and drawer alive for the page's life.
            self.ivars().search_updater.replace(Some(updater));
            self.ivars().search_controller.replace(Some(controller));
            self.setDefinesPresentationContext(true);
        } else {
            item.setSearchController(None);
        }
        if let Some(nav) = self.navigationController() {
            nav.setNavigationBarHidden_animated(page.hidden, false);
        }
    }

    /// Runs `handler` with the search field's text when the user edits it.
    /// Runs `handler` with the search field's current text on each edit.
    pub fn set_search_change_handler(&self, handler: impl Fn(String) + 'static) {
        self.ivars().search_change.replace(Some(Rc::new(handler)));
    }

    /// The current text of the page's search field, when it has one.
    ///
    /// Drives the binding→platform direction without going through the
    /// change handler.
    pub fn set_search_text(&self, text: &str) {
        // SAFETY: `navigationItem` is a `UIViewController` getter on the
        // main thread.
        let item: Retained<objc2_ui_kit::UINavigationItem> =
            unsafe { msg_send![self, navigationItem] };
        if let Some(controller) = item.searchController() {
            controller
                .searchBar()
                .setText(Some(&NSString::from_str(text)));
        }
    }

    /// The search drawer's placeholder, when one is attached.
    pub fn set_search_placeholder(&self, placeholder: &str) {
        // SAFETY: `navigationItem` is a `UIViewController` getter on the
        // main thread.
        let item: Retained<objc2_ui_kit::UINavigationItem> =
            unsafe { msg_send![self, navigationItem] };
        if let Some(controller) = item.searchController() {
            controller
                .searchBar()
                .setPlaceholder(Some(&NSString::from_str(placeholder)));
        }
    }
}

/// The text-change reporter a page's search drawer fires through —
/// `UISearchResultsUpdating`'s hook, driving the search binding like
/// the search drawer's coordinator.
pub struct SearchUpdaterIvars {
    /// Called with the search bar's current text when it changes.
    change: RefCell<Option<SearchChangeHandler>>,
}

impl fmt::Debug for SearchUpdaterIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchUpdaterIvars").finish()
    }
}

define_class!(
    // SAFETY: `NSObject`'s `init` is its designated initializer; the class
    // does not implement `Drop`.
    #[unsafe(super(objc2::runtime::NSObject))]
    #[name = "CocoaUiSearchUpdater"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SearchUpdaterIvars]
    #[derive(Debug)]
    /// Reports `UISearchController` text edits.
    pub struct SearchUpdater;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject`.
    unsafe impl NSObjectProtocol for SearchUpdater {}

    // SAFETY: `updateSearchResultsForSearchController:` carries
    // `UISearchResultsUpdating`'s signature.
    unsafe impl UISearchResultsUpdating for SearchUpdater {
        #[unsafe(method(updateSearchResultsForSearchController:))]
        fn update_search_results_for_search_controller(
            &self,
            search_controller: &UISearchController,
        ) {
            let handler = self.ivars().change.borrow().clone();
            if let Some(handler) = handler {
                let text = search_controller
                    .searchBar()
                    .text()
                    .unwrap_or_default()
                    .to_string();
                handler(text);
            }
        }
    }
);

impl SearchUpdater {
    /// An updater with no handler yet.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SearchUpdaterIvars {
            change: RefCell::new(None),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// Runs `handler` with the search field's current text on each edit.
    pub fn set_change_handler(&self, handler: impl Fn(String) + 'static) {
        self.ivars().change.replace(Some(Rc::new(handler)));
    }
}

/// How a page's title draws in the bar.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LargeTitle {
    /// The platform chooses — a large title at the stack's root under
    /// `prefersLargeTitles`, inline elsewhere.
    #[default]
    Automatic,
    /// Always inline.
    Inline,
    /// Always large.
    Large,
}

impl LargeTitle {
    const fn native(self) -> UINavigationItemLargeTitleDisplayMode {
        match self {
            Self::Automatic => UINavigationItemLargeTitleDisplayMode::Automatic,
            Self::Inline => UINavigationItemLargeTitleDisplayMode::Never,
            Self::Large => UINavigationItemLargeTitleDisplayMode::Always,
        }
    }
}

/// Where the navigation bar places the page's search field —
/// `UINavigationItemSearchBarPlacement`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SearchBarPlacement {
    /// The platform chooses — `.automatic`.
    #[default]
    Automatic,
    /// Stacked below the bar's other content — `.stacked`.
    Stacked,
}

impl SearchBarPlacement {
    const fn native(self) -> objc2_ui_kit::UINavigationItemSearchBarPlacement {
        match self {
            Self::Automatic => objc2_ui_kit::UINavigationItemSearchBarPlacement::Automatic,
            Self::Stacked => objc2_ui_kit::UINavigationItemSearchBarPlacement::Stacked,
        }
    }
}

/// A page's search drawer: the field's placeholder, its text at install
/// time, and where the field sits in the bar.
#[derive(Debug, Default)]
pub struct NavSearch {
    /// The placeholder text.
    pub placeholder: String,
    /// The text the field starts with — the binding's current value.
    pub text: String,
    /// The field's placement in the bar — `preferredSearchBarPlacement`.
    /// `None` leaves the platform default in place.
    pub placement: Option<SearchBarPlacement>,
    /// `hidesSearchBarWhenScrolling`: when true the bar's height tracks the
    /// content scroll offset — a bar `UIKit` never sees scroll begins at
    /// zero height. `None` leaves the platform default in place.
    pub hides_when_scrolling: Option<bool>,
}

/// One page's navigation-bar chrome.
#[derive(Default)]
pub struct NavPage {
    /// The page's title.
    pub title: String,
    /// A view replacing the title text, or `None` for the plain title.
    pub title_view: Option<Retained<objc2_ui_kit::UIView>>,
    /// The page's subtitle, when its bar state declares one.
    pub subtitle: Option<String>,
    /// Leading bar items, left to right.
    pub leading: Vec<Retained<UIBarButtonItem>>,
    /// Trailing bar items, right to left.
    pub trailing: Vec<Retained<UIBarButtonItem>>,
    /// Bottom-bar items — the toolbar the page presents under its content.
    pub bottom: Vec<Retained<UIBarButtonItem>>,
    /// Whether the page hides the back button.
    pub hides_back: bool,
    /// The semantic back action the page's back affordance runs — `None`
    /// leaves the bar's default (root pages still hide it via
    /// `hides_back`).
    pub on_back: Option<Rc<dyn Fn()>>,
    /// The page's search drawer, when its bar state declares one.
    pub search: Option<NavSearch>,
    /// How the page's title displays.
    pub large_title: LargeTitle,
    /// Whether the navigation bar hides while this page is topmost.
    pub hidden: bool,
}

impl fmt::Debug for NavPage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavPage")
            .field("title", &self.title)
            .field("title_view", &self.title_view)
            .field("subtitle", &self.subtitle)
            .field("leading", &self.leading)
            .field("trailing", &self.trailing)
            .field("bottom", &self.bottom)
            .field("hides_back", &self.hides_back)
            .field("has_back_action", &self.on_back.is_some())
            .field("search", &self.search)
            .field("large_title", &self.large_title)
            .field("hidden", &self.hidden)
            .finish()
    }
}

/// A bar button wrapping a hosted view — the `WaterUI` item model hands the
/// platform a view, not an icon-and-action pair, so chrome items are
/// `initWithCustomView:`.
///
/// The action the page's search drawer reports text changes through.
type SearchChangeHandler = Rc<dyn Fn(String)>;

/// The gate an interactive pop consults — the top page's pop verdict.
type PopGate = Rc<dyn Fn() -> bool>;

/// The transition-settled sink a navigation controller reports through.
type ShowHandler = Rc<dyn Fn(usize)>;

/// A bar button wrapping a hosted view — the `WaterUI` item model hands the
/// platform a view, not an icon-and-action pair, so chrome items are
/// `initWithCustomView:`.
pub struct BarButtonIvars {
    /// Called when the button is tapped.
    action: RefCell<Option<Rc<dyn Fn()>>>,
}

impl fmt::Debug for BarButtonIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarButtonIvars").finish()
    }
}

define_class!(
    // SAFETY: `UIControl`'s designated initializer is `initWithFrame:`,
    // which `BarButton::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UIControl))]
    #[name = "CocoaUiBarButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = BarButtonIvars]
    #[derive(Debug)]
    /// A `UIControl` hosting a view for a bar button item.
    pub struct BarButton;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIControl`.
    unsafe impl NSObjectProtocol for BarButton {}

    impl BarButton {
        // SAFETY: `tapped` is this class's own `.touchUpInside` action.
        #[unsafe(method(tapped))]
        fn tapped(&self) {
            let handler = self.ivars().action.borrow().clone();
            if let Some(handler) = handler {
                handler();
            }
        }
    }
);

impl BarButton {
    /// A bar button hosting `view`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, view: &objc2_ui_kit::UIView) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(BarButtonIvars {
            action: RefCell::new(None),
        });
        // SAFETY: `initWithFrame:` is `UIControl`'s designated initializer;
        // the button is measured by its hosted view.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: view.frame()] };
        this.addSubview(view);
        // SAFETY: `self` is the target of `tapped`.
        unsafe {
            this.addTarget_action_forControlEvents(
                Some(
                    std::ptr::from_ref::<Self>(this.as_ref())
                        .cast::<AnyObject>()
                        .as_ref()
                        .unwrap_unchecked(),
                ),
                objc2::sel!(tapped),
                objc2_ui_kit::UIControlEvents::TouchUpInside,
            );
        }
        this
    }

    /// Runs `handler` when the button is tapped.
    pub fn set_action(&self, handler: impl Fn() + 'static) {
        self.ivars().action.replace(Some(Rc::new(handler)));
    }
}

/// A `UIBarButtonItem` from a symbol name and/or a hosted view.
#[must_use]
pub fn bar_item(
    mtm: MainThreadMarker,
    symbol: Option<&str>,
    view: Option<&objc2_ui_kit::UIView>,
    action: Option<Rc<dyn Fn()>>,
) -> Retained<UIBarButtonItem> {
    if let Some(view) = view {
        let button = BarButton::new(mtm, view);
        button.set_action(move || {
            if let Some(action) = action.as_ref() {
                action();
            }
        });
        return UIBarButtonItem::initWithCustomView(mtm.alloc(), &button);
    }
    let image = symbol
        .and_then(|symbol| objc2_ui_kit::UIImage::systemImageNamed(&NSString::from_str(symbol)));
    // SAFETY: `initWithImage:style:target:action:` is a `UIBarButtonItem`
    // designated initializer; nil target/action are valid.
    let item = unsafe {
        UIBarButtonItem::initWithImage_style_target_action(
            mtm.alloc(),
            image.as_deref(),
            UIBarButtonItemStyle::Plain,
            None,
            None,
        )
    };
    if let Some(action) = action {
        // SAFETY: `UIAction::actionWithHandler:` retains the block, which
        // owns the `Rc` for the item's life.
        let ui_action = unsafe {
            objc2_ui_kit::UIAction::actionWithHandler(
                block2::RcBlock::into_raw(RcBlock::new(
                    move |_: core::ptr::NonNull<objc2_ui_kit::UIAction>| {
                        action();
                    },
                )),
                mtm,
            )
        };
        item.setPrimaryAction(Some(&ui_action));
    }
    item
}

/// A standalone `UINavigationBar` — the in-content bar
/// when no `UINavigationController` hosts the page.
pub struct NavBarIvars {
    /// The one item the bar shows.
    item: Retained<UINavigationItem>,
}

impl fmt::Debug for NavBarIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavBarIvars").finish()
    }
}

define_class!(
    // SAFETY: `UINavigationBar` initializes through `init`, which
    // `NavBar::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UINavigationBar))]
    #[name = "CocoaUiNavBar"]
    #[thread_kind = MainThreadOnly]
    #[ivars = NavBarIvars]
    #[derive(Debug)]
    /// An in-content navigation bar presenting one page's chrome.
    pub struct NavBar;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UINavigationBar`.
    unsafe impl NSObjectProtocol for NavBar {}
);

impl NavBar {
    /// A bar with one empty item.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: `UINavigationItem` initializes through `init`.
        let item: Retained<UINavigationItem> =
            unsafe { msg_send![mtm.alloc::<UINavigationItem>(), init] };
        let this = Self::alloc(mtm).set_ivars(NavBarIvars { item });
        // SAFETY: `init` is `UIView`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let items = NSArray::from_retained_slice(&[this.ivars().item.clone()]);
        this.setItems(Some(&items));
        this
    }

    /// Applies a page's chrome to the bar's item — the subset of
    /// [`NavContentController::set_page`] an in-content bar draws:
    /// title, title view, leading/trailing items and the back affordance.
    pub fn set_page(&self, page: &NavPage) {
        let item = &*self.ivars().item;
        item.setTitle(Some(&NSString::from_str(&page.title)));
        item.setTitleView(page.title_view.as_deref());
        item.setSubtitle(page.subtitle.as_deref().map(NSString::from_str).as_deref());
        if page.leading.is_empty() {
            item.setLeftBarButtonItems(None);
        } else {
            item.setLeftBarButtonItems(Some(&NSArray::from_retained_slice(&page.leading)));
        }
        item.setLeftItemsSupplementBackButton(false);
        if page.trailing.is_empty() {
            item.setRightBarButtonItems(None);
        } else {
            item.setRightBarButtonItems(Some(&NSArray::from_retained_slice(&page.trailing)));
        }
        item.setHidesBackButton(page.hides_back);
        if let Some(on_back) = &page.on_back {
            let on_back = on_back.clone();
            // SAFETY: `UIAction::actionWithHandler:` retains the block, which
            // owns the `Rc` for the action's life.
            let action = unsafe {
                objc2_ui_kit::UIAction::actionWithHandler(
                    block2::RcBlock::into_raw(RcBlock::new(
                        move |_: core::ptr::NonNull<objc2_ui_kit::UIAction>| {
                            on_back();
                        },
                    )),
                    self.mtm(),
                )
            };
            item.setBackAction(Some(&action));
        }
    }
}

/// The navigation stack's model: the pages and the current pop state.
/// Called when the user pops one or more pages by gesture or back button:
/// the count of pages to drop.
type PopHandler = Rc<dyn Fn(usize)>;

/// The first `UIControl` in `view`'s subtree, depth-first — the control a
/// chrome item forwards its activation to, as `firstButton` did for the
/// `WaterUI` bar item's action.
#[must_use]
pub fn first_control(view: &objc2_ui_kit::UIView) -> Option<Retained<UIControl>> {
    if let Some(control) = view.downcast_ref::<UIControl>() {
        return Some(Retained::from(control));
    }
    for subview in &view.subviews() {
        if let Some(control) = first_control(&subview) {
            return Some(control);
        }
    }
    None
}

/// The first `UIButton` in `view`'s subtree, depth-first.
#[must_use]
pub fn first_button(view: &objc2_ui_kit::UIView) -> Option<Retained<UIButton>> {
    if let Some(button) = view.downcast_ref::<UIButton>() {
        return Some(Retained::from(button));
    }
    for subview in &view.subviews() {
        if let Some(button) = first_button(&subview) {
            return Some(button);
        }
    }
    None
}

/// A `UIBarButtonItem` drawing `image` inside the bar's chrome — the symbol
/// and view-rendered icons alike; `action` fires on tap.
#[must_use]
pub fn image_bar_item(
    mtm: MainThreadMarker,
    image: Option<&objc2_ui_kit::UIImage>,
    action: Option<Rc<dyn Fn()>>,
) -> Retained<UIBarButtonItem> {
    // SAFETY: `initWithImage:style:target:action:` is a `UIBarButtonItem`
    // designated initializer; nil target/action are valid.
    let item = unsafe {
        UIBarButtonItem::initWithImage_style_target_action(
            mtm.alloc(),
            image,
            UIBarButtonItemStyle::Plain,
            None,
            None,
        )
    };
    if let Some(action) = action {
        // SAFETY: `UIAction::actionWithHandler:` retains the block, which
        // owns the `Rc` for the item's life.
        let ui_action = unsafe {
            objc2_ui_kit::UIAction::actionWithHandler(
                block2::RcBlock::into_raw(RcBlock::new(
                    move |_: core::ptr::NonNull<objc2_ui_kit::UIAction>| {
                        action();
                    },
                )),
                mtm,
            )
        };
        item.setPrimaryAction(Some(&ui_action));
    }
    item
}

/// The navigation stack's model: the pages and the current pop state.
pub struct NavigationControllerIvars {
    /// Called when the user pops one or more pages by gesture or back
    /// button: the count of pages to drop.
    pop: RefCell<Option<PopHandler>>,
    /// Whether a user-driven pop may begin — the top page's
    /// `pop_enabled`/`pop_attempted` gate `gestureRecognizerShouldBegin`
    /// applies.
    pop_gate: RefCell<Option<PopGate>>,
    /// Called when a transition settles: the depth now visible. Model-driven
    /// transactions complete here — `transition_completed` on the `WaterUI`
    /// side — so the model only observes a pop once the stack matches.
    show: RefCell<Option<ShowHandler>>,
    /// The pages the model asked to show — a user-driven pop is reported
    /// only when the visible stack still matches it.
    expected_depth: Cell<usize>,
}

impl fmt::Debug for NavigationControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationControllerIvars")
            .field("has_pop", &self.pop.borrow().is_some())
            .field("has_pop_gate", &self.pop_gate.borrow().is_some())
            .field("has_show", &self.show.borrow().is_some())
            .field("expected_depth", &self.expected_depth.get())
            .finish()
    }
}

define_class!(
    // SAFETY: `UINavigationController`'s designated initializer is
    // `initWithNibName:bundle:`; `NavigationController::new` passes nil/None,
    // and the class does not implement `Drop`.
    #[unsafe(super(UINavigationController))]
    #[name = "CocoaUiNavigationController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = NavigationControllerIvars]
    #[derive(Debug)]
    /// A `UINavigationController` reporting user-driven pops.
    pub struct NavigationController;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UINavigationController`.
    unsafe impl NSObjectProtocol for NavigationController {}

    // SAFETY: `gestureRecognizerShouldBegin:` carries
    // `UIGestureRecognizerDelegate`'s signature.
    unsafe impl UIGestureRecognizerDelegate for NavigationController {
        #[unsafe(method(gestureRecognizerShouldBegin:))]
        fn gesture_recognizer_should_begin(
            &self,
            gesture_recognizer: &UIGestureRecognizer,
        ) -> bool {
            let is_pop = self
                .interactivePopGestureRecognizer()
                .is_some_and(|pop| *pop == *gesture_recognizer);
            if is_pop && self.viewControllers().count() > 1 {
                self.ivars()
                    .pop_gate
                    .borrow()
                    .as_ref()
                    .is_none_or(|gate| gate())
            } else {
                !is_pop
            }
        }
    }

    // SAFETY: `navigationController:didShowViewController:animated:` carries
    // `UINavigationControllerDelegate`'s signature.
    unsafe impl UINavigationControllerDelegate for NavigationController {
        // SAFETY: see the module safety note.
        #[unsafe(method(navigationController:didShowViewController:animated:))]
        fn navigation_controller_did_show_view_controller_animated(
            &self,
            _navigation_controller: &UINavigationController,
            _view_controller: &UIViewController,
            _animated: bool,
        ) {
            let depth = self.viewControllers().count();
            let expected = self.ivars().expected_depth.replace(depth);
            if let Some(show) = self.ivars().show.borrow().as_ref() {
                show(depth);
            }
            if depth < expected {
                let handler = self.ivars().pop.borrow().clone();
                if let Some(handler) = handler {
                    handler(expected - depth);
                }
            }
        }
    }
);

impl NavigationController {
    /// A navigation stack with a single root page.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, root: &NavContentController) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavigationControllerIvars {
            pop: RefCell::new(None),
            pop_gate: RefCell::new(None),
            show: RefCell::new(None),
            expected_depth: Cell::new(1),
        });
        // SAFETY: `initWithRootViewController:` is a `UINavigationController`
        // initializer on an allocated instance.
        let this: Retained<Self> =
            unsafe { msg_send![super(this), initWithRootViewController: root] };
        // SAFETY: the controller conforms to `UINavigationControllerDelegate`;
        // the delegate is an assign reference.
        unsafe { this.setDelegate(Some(ProtocolObject::from_ref(&*this))) };
        // The edge-swipe gesture asks the delegate whether a pop may begin —
        // the top page's pop gate — and the controller answers as its own
        // `UIGestureRecognizerDelegate`.
        if let Some(gesture) = this.interactivePopGestureRecognizer() {
            gesture.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        }
        this
    }

    /// Whether a user-initiated pop may begin: `handler` reports the top
    /// page's verdict at gesture start.
    pub fn set_pop_gate(&self, gate: impl Fn() -> bool + 'static) {
        self.ivars().pop_gate.replace(Some(Rc::new(gate)));
    }

    /// Runs `handler` with the visible depth each time a transition
    /// settles — model and native pops alike.
    pub fn set_show_handler(&self, handler: impl Fn(usize) + 'static) {
        self.ivars().show.replace(Some(Rc::new(handler)));
    }

    /// Whether the navigation bar is hidden, optionally animating the
    /// change. The top page's `hidden` flag applies through
    /// [`NavContentController::set_page`]; this is the direct override.
    pub fn set_bar_hidden(&self, hidden: bool, animated: bool) {
        self.setNavigationBarHidden_animated(hidden, animated);
    }

    /// `navigationBar.prefersLargeTitles`: the gate each page's
    /// `largeTitleDisplayMode` consults — without it the per-item mode is
    /// inert and every title draws inline.
    pub fn set_prefers_large_titles(&self, prefers: bool) {
        self.navigationBar().setPrefersLargeTitles(prefers);
    }

    /// The pages currently on the stack, root first.
    #[must_use]
    pub fn pages(&self) -> Vec<Retained<NavContentController>> {
        self.viewControllers()
            .iter()
            .filter_map(|controller| controller.downcast::<NavContentController>().ok())
            .collect()
    }

    /// Replaces the pages below the topmost with `under` — the model's
    /// retained prefix — and pushes `new_pages` for `apply(inserted)`. The
    /// caller passes the stack it computed from the transaction.
    pub fn set_pages(&self, pages: &[Retained<NavContentController>], animated: bool) {
        self.ivars().expected_depth.set(pages.len());
        self.setViewControllers_animated(
            &objc2_foundation::NSArray::from_retained_slice(
                &pages
                    .iter()
                    .map(|p| p.clone().into_super())
                    .collect::<Vec<_>>(),
            ),
            animated,
        );
    }

    /// Pops `count` pages, animating the transition; reports `true` when
    /// the visible stack changed.
    pub fn pop_pages(&self, count: usize, animated: bool) -> bool {
        let depth = self.viewControllers().count();
        let Some(_nonempty) = depth.checked_sub(count + 1) else {
            return false;
        };
        let remaining = depth - count;
        let controllers = self.viewControllers();
        let Some(target) = controllers.iter().nth(remaining - 1) else {
            return false;
        };
        // SAFETY: `popToViewController:animated:` is a main-thread stack
        // update on a live navigation controller.
        let _: () = unsafe { msg_send![self, popToViewController: &*target, animated: animated] };
        true
    }

    /// Runs `handler` with the count of pages to drop when the user pops.
    pub fn set_pop_handler(&self, handler: impl Fn(usize) + 'static) {
        self.ivars().pop.replace(Some(Rc::new(handler)));
    }
}

/// The item's accessibility label — `UIBarItem`'s informal conformance has
/// no generated binding.
pub fn bar_item_accessibility_label(item: &UIBarButtonItem, label: &str) {
    let label = NSString::from_str(label);
    // SAFETY: `UIBarItem` conforms to `UIAccessibilityIdentification`; the
    // selector is declared on it.
    let _: () = unsafe { msg_send![item, setAccessibilityLabel: &*label] };
}
