//! The window toolbar coordinator: one `NSToolbar` per window, shared by
//! everything that contributes chrome to it.
//!
//! A window has exactly one toolbar, and more than one thing wants to put
//! something in it: an app-level tab container offers its tabs, while
//! whichever navigation page is on screen offers its back button, title,
//! actions and search field. `AppKit` models this as a single `NSToolbar`
//! with one delegate, so a coordinator owns it and each contributor hands
//! over what it wants shown.
//!
//! Going through `NSToolbarItem` rather than a titlebar accessory view is
//! what gives the chrome its system appearance — the capsule around a
//! toolbar button, the spacing between items and the overflow menu when the
//! window is too narrow. The one exception is the search field: a window
//! without a sidebar draws search in a titlebar accessory row below the
//! toolbar.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSObject` subclass conforming to
//! `NSToolbarDelegate`, calls `objc2`/`AppKit` bindings marked unsafe
//! because `AppKit` window and toolbar APIs are main-thread only — the
//! `MainThreadOnly` thread kind and [`MainThreadMarker`] constructor
//! guarantee the main thread — and uses raw KVO
//! (`addObserver:forKeyPath:options:context:`) to watch the sidebar item's
//! collapse state, pairing each registration with removal when the sidebar
//! is replaced or withdrawn.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSImage, NSLayoutAttribute, NSLayoutConstraint, NSSearchToolbarItem, NSSplitViewController,
    NSSplitViewItem, NSSplitViewItemBehavior, NSTitlebarAccessoryViewController, NSToolbar,
    NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarItem, NSToolbarItemIdentifier,
    NSTrackingSeparatorToolbarItem, NSView, NSWindow, NSWindowStyleMask, NSWindowTitleVisibility,
    NSWindowToolbarStyle,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{
    NSArray, NSKeyValueChangeKey, NSKeyValueObservingOptions,
    NSObjectNSKeyValueObserverRegistration, NSObjectProtocol, NSSet, NSString,
};

use crate::appkit::search_field::SearchField;
use crate::callback::guarded;
use crate::geometry::Size;

/// A view the toolbar hosts as-is, at the size its owner measured.
pub struct HostedItem {
    /// The view to host.
    pub view: Retained<NSView>,
    /// The size the owner measured for it.
    pub size: Size,
}

impl fmt::Debug for HostedItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostedItem")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// One navigation action offered to the toolbar.
///
/// An action carrying an icon becomes a real `NSToolbarItem` — the icon in a
/// capsule, the name in the overflow menu and tooltip. An action with no
/// icon is hosted as the view it is: a text-only action like "Edit" is a
/// bordered toolbar button showing its text, which is what the platform does
/// too.
pub struct ToolbarChild {
    /// The action's rendered view, hosted when there is no icon.
    pub view: HostedItem,
    /// The action's icon, when one was resolved.
    pub icon: Option<Retained<NSImage>>,
    /// The action's title, for the overflow menu, palette and tooltip.
    pub label: String,
    /// Whether the label's own button draws a bezel; a borderless button
    /// stays bare in the toolbar.
    pub bordered: bool,
    /// The handler the item runs when invoked — the same one the hosted
    /// view's own control would run.
    pub action: Option<Rc<dyn Fn()>>,
}

impl fmt::Debug for ToolbarChild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolbarChild")
            .field("label", &self.label)
            .field("has_icon", &self.icon.is_some())
            .field("bordered", &self.bordered)
            .field("has_action", &self.action.is_some())
            .field("view", &self.view)
            .finish()
    }
}

/// A search field offered to the toolbar.
pub struct HostedSearch {
    /// The field to show.
    pub field: Retained<SearchField>,
    /// An identity token for the search source, so a rebuild offering the
    /// same search keeps the field — and its focus — instead of replacing
    /// the row.
    pub source_id: usize,
}

impl fmt::Debug for HostedSearch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostedSearch")
            .field("field", &self.field)
            .field("source_id", &self.source_id)
            .finish()
    }
}

/// The toolbar contribution of one navigation page.
#[derive(Default)]
pub struct ToolbarContent {
    /// Whether a back control precedes the page's items.
    pub shows_back: bool,
    /// What the back control runs.
    pub on_back: Option<Rc<dyn Fn()>>,
    /// The page title, when plain text — painted as the window's title.
    pub title: Option<String>,
    /// The page title as a view, when the title is not plain text.
    pub title_item: Option<HostedItem>,
    /// The semantic leading action.
    pub leading: Option<ToolbarChild>,
    /// The semantic trailing action.
    pub trailing: Option<ToolbarChild>,
    /// The page's status item: informational, centred beside the tabs.
    pub status: Option<HostedItem>,
    /// The page's search field.
    pub search: Option<HostedSearch>,
}

impl fmt::Debug for ToolbarContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolbarContent")
            .field("shows_back", &self.shows_back)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

const BACK_IDENTIFIER: &str = "dev.cocoaui.navigation.back";
const TITLE_IDENTIFIER: &str = "dev.cocoaui.navigation.title";
const LEADING_IDENTIFIER: &str = "dev.cocoaui.navigation.leading";
const TRAILING_IDENTIFIER: &str = "dev.cocoaui.navigation.trailing";
const STATUS_IDENTIFIER: &str = "dev.cocoaui.navigation.status";
const SEARCH_IDENTIFIER: &str = "dev.cocoaui.navigation.search";
const TABS_IDENTIFIER: &str = "dev.cocoaui.tabs";
const WINDOW_ITEM_PREFIX: &str = "dev.cocoaui.window.item.";
const SIDEBAR_SEPARATOR_IDENTIFIER: &str = "dev.cocoaui.sidebar.separator";
const FLEXIBLE_SPACE_IDENTIFIER: &str = "NSToolbarFlexibleSpaceItem";
const TOGGLE_SIDEBAR_IDENTIFIER: &str = "NSToolbarToggleSidebarItem";

/// The widest search field beside a sidebar: measured off the platform's
/// reference window at 800, 900, 1000 and 1200 pt — the field is 232, 307,
/// 325 and 325 pt, and `AppKit` narrows it below this preferred width as the
/// detail section shrinks.
const SEARCH_ITEM_WIDTH: f64 = 325.0;

/// The search accessory row's height, measured off the platform's window.
const SEARCH_ROW_HEIGHT: f64 = 38.0;

thread_local! {
    /// The live coordinators, keyed by the window pointer.
    static COORDINATORS: RefCell<HashMap<usize, Weak<WindowToolbar>>> =
        RefCell::new(HashMap::new());
}

fn window_key(window: &NSWindow) -> usize {
    std::ptr::from_ref(window) as usize
}

/// The sidebar-collapse key-path observation's context token.
static COLLAPSE_CONTEXT: u8 = 0;

/// The coordinator's retained state.
pub struct WindowToolbarIvars {
    /// The window this coordinator serves — weak, as the window retains the
    /// toolbar which retains the coordinator.
    window: RefCell<Weak<NSWindow>>,
    /// The toolbar owned by this coordinator.
    toolbar: Retained<NSToolbar>,
    /// Identity of the page currently claiming the chrome; `None` when no
    /// page does.
    content_owner: RefCell<Option<usize>>,
    /// The claiming page's chrome.
    content: RefCell<ToolbarContent>,
    /// The window's own toolbar content — `Window::toolbar` children, one
    /// item each.
    window_items: RefCell<Vec<ToolbarChild>>,
    /// The app-level tab control, when a tab container offers one.
    tabs_view: RefCell<Option<Retained<NSView>>>,
    /// The split view controller whose sidebar the toolbar aligns with.
    sidebar_split: RefCell<Option<Weak<NSSplitViewController>>>,
    /// The sidebar toggle item, for its collapse-following label.
    sidebar_toggle: RefCell<Option<Retained<NSToolbarItem>>>,
    /// The handlers promoted `NSToolbarItem`s run, by item identifier.
    item_actions: RefCell<HashMap<String, Rc<dyn Fn()>>>,
    /// The accessory row showing the search field, while a search is offered.
    search_accessory: RefCell<Option<Retained<NSTitlebarAccessoryViewController>>>,
    /// Identity of the search the accessory is attached to.
    search_source: RefCell<Option<usize>>,
    /// The row height currently charged to the window's frame.
    charged_search_row_height: Cell<f64>,
}

impl fmt::Debug for WindowToolbarIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowToolbarIvars")
            .field("window", &self.window.borrow().load().is_some())
            .field("toolbar", &self.toolbar)
            .field("content_owner", &self.content_owner.borrow())
            .field("content", &*self.content.borrow())
            .field("tabs_view", &self.tabs_view.borrow().is_some())
            .field("has_sidebar_split", &self.sidebar_split.borrow().is_some())
            .field(
                "has_sidebar_toggle",
                &self.sidebar_toggle.borrow().is_some(),
            )
            .field("item_actions", &self.item_actions.borrow().len())
            .field(
                "has_search_accessory",
                &self.search_accessory.borrow().is_some(),
            )
            .field("window_items", &self.window_items.borrow().len())
            .field("search_source", &self.search_source.borrow())
            .field(
                "charged_search_row_height",
                &self.charged_search_row_height.get(),
            )
            .finish()
    }
}

define_class!(
    // SAFETY: `NSObject`'s designated initializer is `init`, which
    // `WindowToolbar::attached` calls, and the class does not implement
    // `Drop`. KVO deregistration happens in `set_sidebar_split` when the
    // sidebar is withdrawn or replaced.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[name = "CocoaUiWindowToolbar"]
    #[thread_kind = MainThreadOnly]
    #[ivars = WindowToolbarIvars]
    #[derive(Debug)]
    /// The window toolbar's owning delegate and registry of contributors.
    pub struct WindowToolbar;

    // SAFETY: `NSObjectProtocol` asks nothing extra of an `NSObject`.
    unsafe impl NSObjectProtocol for WindowToolbar {}

    impl WindowToolbar {
        // SAFETY: see the module safety note.
        #[unsafe(method(backInvoked))]
        fn back_invoked(&self) {
            guarded("WindowToolbar backInvoked", || {
                let on_back = self.ivars().content.borrow().on_back.clone();
                if let Some(on_back) = on_back {
                    on_back();
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(actionInvoked:))]
        fn action_invoked(&self, sender: &NSToolbarItem) {
            guarded("WindowToolbar actionInvoked:", || {
                let key = sender.itemIdentifier().to_string();
                let action = self.ivars().item_actions.borrow().get(&key).cloned();
                if let Some(action) = action {
                    action();
                }
            });
        }

        // SAFETY: the raw-KVO callback's signature matches
        // `NSKeyValueObserving`'s; only the registered key path's context is
        // answered.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value_for_key_path(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            change: Option<&objc2_foundation::NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            context: *mut std::ffi::c_void,
        ) {
            if context != std::ptr::from_ref(&COLLAPSE_CONTEXT).cast_mut().cast() {
                return;
            }
            // SAFETY: `NSKeyValueChangeNewKey` is an immutable extern
            // constant.
            let collapsed = change
                .and_then(|change| unsafe {
                    change.objectForKey(objc2_foundation::NSKeyValueChangeNewKey)
                })
                .and_then(|value| value.downcast::<objc2_foundation::NSNumber>().ok())
                .is_some_and(|number| number.as_bool());
            self.update_sidebar_toggle_label(collapsed);
        }
    }

    // SAFETY: `NSToolbarDelegate` is adopted because this object owns the
    // toolbar's item set; the methods carry the protocol's signatures.
    unsafe impl NSToolbarDelegate for WindowToolbar {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn toolbar_default_item_identifiers(
            &self,
            _toolbar: &NSToolbar,
        ) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            let identifiers = self.current_identifiers();
            NSArray::from_slice(&identifiers.iter().map(|s| &**s).collect::<Vec<_>>())
        }

        // SAFETY: see the module safety note.
        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn toolbar_allowed_item_identifiers(
            &self,
            _toolbar: &NSToolbar,
        ) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            let identifiers = self.current_identifiers();
            NSArray::from_slice(&identifiers.iter().map(|s| &**s).collect::<Vec<_>>())
        }

        // SAFETY: see the module safety note.
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item_for_item_identifier(
            &self,
            _toolbar: &NSToolbar,
            item_identifier: &NSToolbarItemIdentifier,
            _flag: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            self.item_for_identifier(&item_identifier.to_string())
        }
    }
);

impl WindowToolbar {
    /// The coordinator attached to `window`, created on first use.
    ///
    /// Attaching installs an `NSToolbar` on the window, switches the window
    /// to a unified titlebar, and enables full-size content — what lets a
    /// sidebar run the window's full height while everything else places
    /// itself below the toolbar through the safe area.
    ///
    /// # Panics
    ///
    /// Panics if `window` is not a live `NSWindow` — impossible in practice.
    #[must_use]
    pub fn attached(window: &NSWindow) -> Retained<Self> {
        let key = window_key(window);
        if let Some(existing) = COORDINATORS.with(|map| map.borrow().get(&key).and_then(Weak::load))
        {
            return existing;
        }
        let mtm = MainThreadMarker::from(window);
        let this = Self::alloc(mtm).set_ivars(WindowToolbarIvars {
            // SAFETY: `window` is a live `NSWindow`; `Weak` takes its retain
            // out immediately.
            window: RefCell::new(Weak::from_retained(&unsafe {
                Retained::retain(std::ptr::from_ref(window).cast_mut()).expect("live window")
            })),
            toolbar: NSToolbar::initWithIdentifier(
                mtm.alloc(),
                &NSString::from_str("dev.cocoaui.window"),
            ),
            content_owner: RefCell::new(None),
            content: RefCell::new(ToolbarContent::default()),
            window_items: RefCell::new(Vec::new()),
            tabs_view: RefCell::new(None),
            sidebar_split: RefCell::new(None),
            sidebar_toggle: RefCell::new(None),
            item_actions: RefCell::new(HashMap::new()),
            search_accessory: RefCell::new(None),
            search_source: RefCell::new(None),
            charged_search_row_height: Cell::new(0.0),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        let toolbar = this.ivars().toolbar.clone();
        toolbar.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
        toolbar.setAllowsUserCustomization(false);
        window.setToolbar(Some(&toolbar));
        window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        window.setStyleMask(window.styleMask() | NSWindowStyleMask::FullSizeContentView);
        COORDINATORS.with(|map| {
            map.borrow_mut().insert(key, Weak::from_retained(&this));
        });
        this
    }

    /// The underlying `NSToolbar`.
    #[must_use]
    pub fn toolbar(&self) -> &NSToolbar {
        &self.ivars().toolbar
    }

    fn window(&self) -> Option<Retained<NSWindow>> {
        self.ivars().window.borrow().load()
    }

    /// Offers the window's own toolbar content, or withdraws it when empty.
    ///
    /// Each child is described the way a navigation action is: an icon and
    /// label become a real `NSToolbarItem`, a child with no icon is hosted
    /// as the view it is.
    pub fn set_window_items(&self, items: Vec<ToolbarChild>) {
        *self.ivars().window_items.borrow_mut() = items;
        self.rebuild();
    }

    /// Offers the app-level tab control, or withdraws it with `None`.
    pub fn set_tabs(&self, tabs: Option<Retained<NSView>>) {
        *self.ivars().tabs_view.borrow_mut() = tabs;
        self.update_title_visibility();
        self.rebuild();
    }

    /// Aligns the toolbar with a full-height sidebar, or withdraws the
    /// alignment with `None`.
    ///
    /// The alignment is two items: the sidebar's collapse control, and a
    /// separator that tracks the split view's divider so everything after it
    /// sits over the detail column.
    ///
    /// # Panics
    ///
    /// Panics if `controller` is not a live `NSSplitViewController`.
    pub fn set_sidebar_split(&self, controller: Option<&NSSplitViewController>) {
        let changed = {
            let slot = self.ivars().sidebar_split.borrow();
            match (slot.as_ref().and_then(objc2::rc::Weak::load), controller) {
                (None, None) => false,
                (Some(old), Some(new)) => !std::ptr::eq(old.as_ref(), new),
                _ => true,
            }
        };
        if !changed {
            return;
        }
        self.unobserve_sidebar();
        self.ivars().sidebar_split.replace(controller.map(|c| {
            // SAFETY: `c` is a live `NSSplitViewController`; `Weak` takes
            // its retain out immediately.
            Weak::from_retained(&unsafe {
                Retained::retain(std::ptr::from_ref(c).cast_mut())
                    .expect("live split view controller")
            })
        }));
        self.observe_sidebar();
        self.update_title_visibility();
        self.rebuild();
    }

    /// Offers one navigation page's chrome, claiming the toolbar for
    /// `owner`.
    ///
    /// `owner` is a stable identity token of the publishing object — the
    /// address of its state — because several pages exist at once and only
    /// the one on screen may contribute; a stack switched away from must not
    /// leave its buttons behind in the toolbar.
    pub fn set_content(&self, content: ToolbarContent, owner: usize) {
        self.ivars().content_owner.replace(Some(owner));
        self.ivars().content.replace(content);
        self.rebuild();
    }

    /// Withdraws `owner`'s chrome, if it still holds the toolbar.
    pub fn clear_content(&self, owner: usize) {
        if self.ivars().content_owner.borrow().as_ref() != Some(&owner) {
            return;
        }
        self.ivars().content_owner.replace(None);
        self.ivars().content.replace(ToolbarContent::default());
        self.rebuild();
    }

    /// Whether the window title is painted in the titlebar.
    ///
    /// A tab control takes the title's place; the window title still exists
    /// for the Window menu and Mission Control. A sidebar does not hide it —
    /// the tracking separator moves the title into the detail column's
    /// section of the titlebar.
    fn update_title_visibility(&self) {
        if let Some(window) = self.window() {
            window.setTitleVisibility(if self.ivars().tabs_view.borrow().is_some() {
                NSWindowTitleVisibility::Hidden
            } else {
                NSWindowTitleVisibility::Visible
            });
        }
    }

    fn sidebar_item(&self) -> Option<Retained<NSSplitViewItem>> {
        self.ivars()
            .sidebar_split
            .borrow()
            .as_ref()
            .and_then(objc2::rc::Weak::load)?
            .splitViewItems()
            .iter()
            .find(|item| item.behavior() == NSSplitViewItemBehavior::Sidebar)
    }

    fn observe_sidebar(&self) {
        let Some(item) = self.sidebar_item() else {
            return;
        };
        // SAFETY: paired with `unobserve_sidebar`'s removal; the context
        // token disambiguates this registration in the callback.
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
        self.update_sidebar_toggle_label(item.isCollapsed());
    }

    fn unobserve_sidebar(&self) {
        let Some(item) = self.sidebar_item() else {
            return;
        };
        // SAFETY: removes the registration `observe_sidebar` added; this runs
        // before the ivar is replaced, so `item` is still the observed one.
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

    /// Names the collapse control after what it will do, as the platform's
    /// own sidebar apps do: "Hide Sidebar" while the sidebar is up, "Show
    /// Sidebar" once it is tucked away.
    fn update_sidebar_toggle_label(&self, collapsed: bool) {
        let label = NSString::from_str(if collapsed {
            "Show Sidebar"
        } else {
            "Hide Sidebar"
        });
        if let Some(toggle) = self.ivars().sidebar_toggle.borrow().as_ref() {
            toggle.setLabel(&label);
            toggle.setPaletteLabel(&label);
            toggle.setToolTip(Some(&label));
        }
    }

    /// Whether the page's search goes in the toolbar: beside a sidebar the
    /// detail section carries the field as a toolbar item at its trailing
    /// edge; without one it is the titlebar accessory row.
    fn search_is_toolbar_item(&self) -> bool {
        self.ivars().content.borrow().search.is_some()
            && self
                .ivars()
                .sidebar_split
                .borrow()
                .as_ref()
                .and_then(objc2::rc::Weak::load)
                .is_some()
    }

    /// The toolbar's items, in order.
    ///
    /// The tab control is anchored to the toolbar's centre slot rather than
    /// placed between flexible spaces, so it stays put while the page's
    /// actions move around it. The sidebar's collapse control hugs the
    /// tracking separator at the sidebar's trailing edge — the flexible
    /// space does the pushing — and the detail section carries no flexible
    /// space of its own beside a sidebar, where `AppKit` keeps items after
    /// the separator at the trailing edge and lets the search item flex.
    fn current_identifiers(&self) -> Vec<Retained<NSToolbarItemIdentifier>> {
        let has_sidebar = self
            .ivars()
            .sidebar_split
            .borrow()
            .as_ref()
            .and_then(objc2::rc::Weak::load)
            .is_some();
        let mut identifiers: Vec<String> = Vec::new();
        {
            let content = self.ivars().content.borrow();
            if has_sidebar {
                identifiers.push(FLEXIBLE_SPACE_IDENTIFIER.to_owned());
                identifiers.push(TOGGLE_SIDEBAR_IDENTIFIER.to_owned());
                identifiers.push(SIDEBAR_SEPARATOR_IDENTIFIER.to_owned());
            }
            if content.shows_back {
                identifiers.push(BACK_IDENTIFIER.to_owned());
            }
            if content.leading.is_some() {
                identifiers.push(LEADING_IDENTIFIER.to_owned());
            }
            if content.title_item.is_some() {
                identifiers.push(TITLE_IDENTIFIER.to_owned());
            }
            if self.ivars().tabs_view.borrow().is_some() {
                identifiers.push(TABS_IDENTIFIER.to_owned());
            }
            if content.status.is_some() {
                identifiers.push(STATUS_IDENTIFIER.to_owned());
            }
            if !has_sidebar {
                identifiers.push(FLEXIBLE_SPACE_IDENTIFIER.to_owned());
            }
        }
        let window_count = self.ivars().window_items.borrow().len();
        for index in 0..window_count {
            identifiers.push(format!("{WINDOW_ITEM_PREFIX}{index}"));
        }
        {
            let content = self.ivars().content.borrow();
            if content.trailing.is_some() {
                identifiers.push(TRAILING_IDENTIFIER.to_owned());
            }
            if self.search_is_toolbar_item() {
                identifiers.push(SEARCH_IDENTIFIER.to_owned());
            }
        }
        identifiers.iter().map(|s| NSString::from_str(s)).collect()
    }

    /// The items the toolbar keeps at its centre: the tabs and the page's
    /// status item share the slot.
    fn centered_identifiers(&self) -> Vec<Retained<NSToolbarItemIdentifier>> {
        let mut identifiers = Vec::new();
        if self.ivars().tabs_view.borrow().is_some() {
            identifiers.push(NSString::from_str(TABS_IDENTIFIER));
        }
        if self.ivars().content.borrow().status.is_some() {
            identifiers.push(NSString::from_str(STATUS_IDENTIFIER));
        }
        identifiers
    }

    fn rebuild(&self) {
        let toolbar = &self.ivars().toolbar;
        while !toolbar.items().is_empty() {
            toolbar.removeItemAtIndex((toolbar.items().count() - 1).cast_signed());
        }
        let identifiers = self.current_identifiers();
        for (index, identifier) in identifiers.iter().enumerate() {
            toolbar.insertItemWithItemIdentifier_atIndex(identifier, index.cast_signed());
        }
        let centered = self.centered_identifiers();
        toolbar.setCenteredItemIdentifiers(&NSSet::from_slice(
            &centered.iter().map(|s| &**s).collect::<Vec<_>>(),
        ));
        if let (Some(window), Some(title)) =
            (self.window(), self.ivars().content.borrow().title.clone())
        {
            window.setTitle(&NSString::from_str(&title));
        }
        self.update_search_accessory();
    }

    /// Keeps the search accessory row in step with the offered content.
    ///
    /// The row is a titlebar accessory pinned to the bottom of the titlebar,
    /// 38 pt tall with the field centred at 41 % of the window's width. Only
    /// a *different* search replaces the row; rebuilding for the same one
    /// leaves it alone so typing never loses focus.
    fn update_search_accessory(&self) {
        let source = if self.search_is_toolbar_item() {
            None
        } else {
            self.ivars()
                .content
                .borrow()
                .search
                .as_ref()
                .map(|search| search.source_id)
        };
        if *self.ivars().search_source.borrow() == source {
            return;
        }
        self.ivars().search_source.replace(source);

        if let Some(accessory) = self.ivars().search_accessory.replace(None)
            && let Some(window) = self.window()
        {
            let index = window
                .titlebarAccessoryViewControllers()
                .indexOfObjectIdenticalTo(&*accessory);
            if index != usize::MAX {
                window.removeTitlebarAccessoryViewControllerAtIndex(index.cast_signed());
            }
        }

        let Some(source) = source else {
            self.charge_search_row_height(0.0);
            return;
        };
        let _ = source;
        let field = {
            let content = self.ivars().content.borrow();
            content
                .search
                .as_ref()
                .map(|search| search.field.clone().into_super().into_super().into_super())
        };
        let Some(field) = field else {
            self.charge_search_row_height(0.0);
            return;
        };
        let mtm = MainThreadMarker::from(self);
        let accessory = NSTitlebarAccessoryViewController::new(mtm);
        accessory.setLayoutAttribute(NSLayoutAttribute::Bottom);
        let container = NSView::new(mtm);
        // SAFETY: `field` is a `SearchField`, an `NSControl` — an `NSView`.
        let field: Retained<NSView> = unsafe { Retained::cast_unchecked::<NSView>(field) };
        field.setTranslatesAutoresizingMaskIntoConstraints(false);
        container.addSubview(&field);
        let constraints: Vec<Retained<NSLayoutConstraint>> = vec![
            field
                .centerXAnchor()
                .constraintEqualToAnchor(&container.centerXAnchor()),
            field
                .centerYAnchor()
                .constraintEqualToAnchor(&container.centerYAnchor()),
            field
                .widthAnchor()
                .constraintEqualToAnchor_multiplier(&container.widthAnchor(), 0.41),
            field.heightAnchor().constraintEqualToConstant(28.0),
            container
                .heightAnchor()
                .constraintEqualToConstant(SEARCH_ROW_HEIGHT),
        ];
        NSLayoutConstraint::activateConstraints(&NSArray::from_slice(
            &constraints.iter().map(Retained::as_ref).collect::<Vec<_>>(),
        ));
        accessory.setView(&container);
        // The titlebar gives the row the height of the frame it is handed at
        // insert; the height constraint alone is not consulted. Assigning the
        // view resets that frame, so it is written after the assignment.
        accessory.view().setFrame(CGRect::new(
            objc2_core_foundation::CGPoint::ZERO,
            objc2_core_foundation::CGSize::new(
                self.window().map_or(0.0, |w| w.frame().size.width),
                SEARCH_ROW_HEIGHT,
            ),
        ));
        if let Some(window) = self.window() {
            window.addTitlebarAccessoryViewController(&accessory);
        }
        self.ivars().search_accessory.replace(Some(accessory));
        self.charge_search_row_height(SEARCH_ROW_HEIGHT);
    }

    /// Spends or returns the search row's height in the window's frame.
    ///
    /// The row is chrome, so its cost comes out of the window's height, not
    /// the content's: the full-size-content collapse runs after the
    /// accessory exists, and the window ends up exactly the row's height
    /// shorter than a toolbar-only one.
    fn charge_search_row_height(&self, height: f64) {
        let Some(window) = self.window() else { return };
        let delta = height - self.ivars().charged_search_row_height.get();
        if delta == 0.0 {
            return;
        }
        self.ivars().charged_search_row_height.set(height);
        if !window
            .styleMask()
            .contains(NSWindowStyleMask::FullSizeContentView)
        {
            return;
        }
        let mut frame = window.frame();
        frame.size.height -= delta;
        window.setFrame_display(frame, true);
    }

    /// Builds the toolbar item an identifier stands for.
    #[allow(clippy::too_many_lines)] // One match arm per identifier.
    fn item_for_identifier(&self, id: &str) -> Option<Retained<NSToolbarItem>> {
        let mtm = MainThreadMarker::from(self);
        let identifier = NSString::from_str(id);
        match id {
            BACK_IDENTIFIER => {
                let item = NSToolbarItem::initWithItemIdentifier(mtm.alloc(), &identifier);
                item.setImage(
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        &NSString::from_str("chevron.backward"),
                        Some(&NSString::from_str("Back")),
                    )
                    .as_deref(),
                );
                item.setLabel(&NSString::from_str("Back"));
                item.setNavigational(true);
                // SAFETY: `self` implements `backInvoked`; the target is an
                // assign reference, not a retain cycle.
                unsafe { item.setTarget(Some(self.as_ref())) };
                // SAFETY: `setAction:` registers `backInvoked`.
                unsafe { item.setAction(Some(sel!(backInvoked))) };
                Some(item)
            }
            TOGGLE_SIDEBAR_IDENTIFIER => {
                // A plain item rather than the system-vended one: the
                // system's own toggle names itself "Sidebar" forever, while
                // the platform's sidebar apps name the control after what it
                // does — "Hide Sidebar" up, "Show Sidebar" down.
                let item = NSToolbarItem::initWithItemIdentifier(mtm.alloc(), &identifier);
                item.setImage(
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        &NSString::from_str("sidebar.leading"),
                        None,
                    )
                    .as_deref(),
                );
                item.setNavigational(true);
                if let Some(split) = self
                    .ivars()
                    .sidebar_split
                    .borrow()
                    .as_ref()
                    .and_then(objc2::rc::Weak::load)
                {
                    // SAFETY: `NSSplitViewController` implements
                    // `toggleSidebar:`.
                    unsafe { item.setTarget(Some(split.as_ref())) };
                    // SAFETY: `setAction:` registers `toggleSidebar:`.
                    unsafe { item.setAction(Some(sel!(toggleSidebar:))) };
                }
                self.ivars().sidebar_toggle.replace(Some(item.clone()));
                self.update_sidebar_toggle_label(
                    self.sidebar_item().is_some_and(|item| item.isCollapsed()),
                );
                Some(item)
            }
            SIDEBAR_SEPARATOR_IDENTIFIER => {
                let split = self
                    .ivars()
                    .sidebar_split
                    .borrow()
                    .as_ref()
                    .and_then(objc2::rc::Weak::load)?;
                Some(
                    NSTrackingSeparatorToolbarItem::trackingSeparatorToolbarItemWithIdentifier_splitView_dividerIndex(
                        &identifier,
                        &split.splitView(),
                        0,
                    )
                    .into_super(),
                )
            }
            TABS_IDENTIFIER => {
                let view = self.ivars().tabs_view.borrow().clone()?;
                Some(self.hosting_item(&identifier, &view, None))
            }
            TITLE_IDENTIFIER => {
                let title = self
                    .ivars()
                    .content
                    .borrow()
                    .title_item
                    .as_ref()
                    .map(|i| (i.view.clone(), i.size))?;
                Some(self.hosting_item(&identifier, &title.0, Some(title.1)))
            }
            LEADING_IDENTIFIER | TRAILING_IDENTIFIER => {
                let content = self.ivars().content.borrow();
                let slot = if id == LEADING_IDENTIFIER {
                    content.leading.as_ref()
                } else {
                    content.trailing.as_ref()
                };
                let child = slot?;
                let item = self.action_item(&identifier, child);
                if id == LEADING_IDENTIFIER {
                    item.setNavigational(true);
                }
                Some(item)
            }
            STATUS_IDENTIFIER => {
                let status = self
                    .ivars()
                    .content
                    .borrow()
                    .status
                    .as_ref()
                    .map(|i| (i.view.clone(), i.size))?;
                Some(self.hosting_item(&identifier, &status.0, Some(status.1)))
            }
            SEARCH_IDENTIFIER => {
                let field = self
                    .ivars()
                    .content
                    .borrow()
                    .search
                    .as_ref()
                    .map(|search| search.field.clone())?;
                let item = NSSearchToolbarItem::initWithItemIdentifier(mtm.alloc(), &identifier);
                item.setPreferredWidthForSearchField(SEARCH_ITEM_WIDTH);
                item.setSearchField(&field);
                Some(item.into_super())
            }
            _ => {
                let index = id.strip_prefix(WINDOW_ITEM_PREFIX)?.parse::<usize>().ok()?;
                let items = self.ivars().window_items.borrow();
                let item = items.get(index)?;
                Some(self.action_item(&identifier, item))
            }
        }
    }

    /// Builds a toolbar item for one navigation action.
    fn action_item(
        &self,
        identifier: &NSToolbarItemIdentifier,
        action: &ToolbarChild,
    ) -> Retained<NSToolbarItem> {
        let Some(icon) = &action.icon else {
            let item = self.hosting_item(identifier, &action.view.view, Some(action.view.size));
            item.setBordered(action.bordered);
            return item;
        };
        let item =
            NSToolbarItem::initWithItemIdentifier(MainThreadMarker::from(self).alloc(), identifier);
        item.setImage(Some(icon));
        let label = NSString::from_str(&action.label);
        item.setLabel(&label);
        item.setPaletteLabel(&label);
        if !action.label.is_empty() {
            item.setToolTip(Some(&label));
        }
        item.setBordered(action.bordered);
        // SAFETY: `self` implements `actionInvoked:`; the target is an assign
        // reference, not a retain cycle.
        unsafe { item.setTarget(Some(self.as_ref())) };
        // SAFETY: `setAction:` registers `actionInvoked:`.
        unsafe { item.setAction(Some(sel!(actionInvoked:))) };
        if let Some(handler) = &action.action {
            self.ivars()
                .item_actions
                .borrow_mut()
                .insert(identifier.to_string(), handler.clone());
        }
        item
    }

    /// Wraps a view in a toolbar item at the size its owner measured.
    ///
    /// The toolbar measures an item through the constraint system, so the
    /// size is stated as constraints rather than through the item's
    /// long-deprecated size bounds.
    fn hosting_item(
        &self,
        identifier: &NSToolbarItemIdentifier,
        view: &Retained<NSView>,
        size: Option<Size>,
    ) -> Retained<NSToolbarItem> {
        let size = size.unwrap_or_else(|| view.fittingSize().into());
        view.removeFromSuperview();
        view.setFrame(CGRect::new(
            objc2_core_foundation::CGPoint::ZERO,
            size.into(),
        ));
        view.setTranslatesAutoresizingMaskIntoConstraints(false);
        let constraints: Vec<Retained<NSLayoutConstraint>> = vec![
            view.widthAnchor().constraintEqualToConstant(size.width),
            view.heightAnchor().constraintEqualToConstant(size.height),
        ];
        NSLayoutConstraint::activateConstraints(&NSArray::from_slice(
            &constraints.iter().map(Retained::as_ref).collect::<Vec<_>>(),
        ));
        let item =
            NSToolbarItem::initWithItemIdentifier(MainThreadMarker::from(self).alloc(), identifier);
        item.setView(Some(&**view));
        item
    }
}
