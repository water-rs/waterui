//! `Native<NavigationStack<(),()>>` — the navigation stack.
//!
//! One driver owns the model side (`NavigationController` transactions,
//! `NavigationDestinationState` callbacks); each platform projects pages
//! into its native container — `UINavigationController` on `UIKit`, a view
//! stack plus the window's toolbar coordinator on `AppKit`. Transactions
//! replace a suffix of the page list; user pops (interactive gesture, back
//! button) complete against the model through `popped()` +
//! `complete_native_pop`.
//!
//! `UIKit`: each page is a `NavContentController` holding the rendered
//! content; `set_page` applies the `Bar` chrome to `UINavigationItem` —
//! the chrome `WuiNavigationStack` configured in `viewWillAppear`.
//! `AppKit`: each page is an `NSView`; the bar chrome publishes into the
//! window's toolbar while the window carries a titlebar — the
//! `WuiWindowToolbar.attached(to:)` + `topEntry` wiring.

use alloc::rc::{Rc, Weak};

use waterui::navigation::{
    AnyNavigationTransition, CustomNavigationController, NativeNavigationTransition,
    NavigationController, NavigationStack, NavigationTransaction, resolve_navigation_root,
};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;

/// Installs the stack's claim on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<NavigationStack<(), ()>>(stack_leaf);
}

/// The stack's leaf: a `NavigationController` is inserted into the child
/// environment so `NavigationLink`/path bindings reach this driver, the
/// deferred root resolves against that environment, and the platform
/// driver is retained by the leaf's keep-alive.
fn stack_leaf(stack: NavigationStack<(), ()>, ctx: &RenderContext<'_>) -> NativeLeaf {
    let driver = Rc::new(platform::Stack::new(stack.transition_style().clone()));
    let erased: Rc<dyn Sink> = driver.clone();
    let mut env = ctx.env().clone();
    let controller = NavigationController::new(Receiver {
        driver: Rc::downgrade(&erased),
    });
    env.insert(controller);
    driver.prepare(ctx, env.clone());
    let root = resolve_navigation_root(stack.into_inner(), &env);
    let mut leaf = driver.build(root);
    leaf.keep(driver);
    leaf
}

/// What each platform driver answers to transactions.
pub(super) trait Sink {
    /// Applies one suffix-replacing transaction.
    fn apply(&self, transaction: NavigationTransaction);
}

/// The controller side of the model-to-native handoff: transactions land
/// here from `NavigationController` and forward into the driver while it
/// lives.
#[derive(Debug)]
struct Receiver {
    /// The owning driver; transactions die with it.
    driver: Weak<dyn Sink>,
}

impl CustomNavigationController for Receiver {
    fn apply(&mut self, transaction: NavigationTransaction) {
        if let Some(driver) = self.driver.upgrade() {
            driver.apply(transaction);
        }
    }
}

/// The resolved transition kind for one page: the page's declared
/// transition, falling back to the stack's.
fn page_transition(
    declared: Option<&AnyNavigationTransition>,
    stack: &AnyNavigationTransition,
) -> NativeNavigationTransition {
    declared.unwrap_or(stack).native()
}

/// The stack's layout: fills its parent — a navigation container claims
/// the whole space it is offered.
#[derive(Debug)]
struct Fill;

impl SubView for Fill {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(0.0, 0.0))
    }
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(target_os = "ios")]
mod platform {
    use alloc::rc::Rc;
    use alloc::string::ToString;
    use alloc::vec::Vec;
    use core::cell::{OnceCell, RefCell};
    use core::fmt;

    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::Retained;
    use cocoa_ui::geometry::Rect as KitRect;
    use cocoa_ui::objc2_ui_kit::{UIBarButtonItem, UIControlEvents};
    use cocoa_ui::uikit::{
        HostView, LargeTitle, NavContentController, NavPage, NavSearch, bar_item, first_button,
    };
    use cocoa_ui::view;
    use waterui::Environment;
    use waterui::Str;
    use waterui::navigation::{
        AnyNavigationTransition, NativeNavigationTransition, NavigationController,
        NavigationDestinationState, NavigationSearch, NavigationTitleDisplayMode,
        NavigationTransaction, NavigationTransactionId, NavigationView,
    };
    use waterui::reactive::Signal;
    use waterui_core::layout::ProposalSize;

    use crate::contract::{KeepAlive, NativeLeaf, RenderContext, Renderer};

    use super::super::bar::{BarItem, BarItemIcon, BarState, bar_item_frame, search_prompt};
    use super::{Fill, Sink, page_transition};

    /// A pending transaction: it completes when `UINavigationController`
    /// settles to the expected depth — `didShow` reconciles the way the
    /// baseline's `expectedNativeStack` comparison did.
    struct Pending {
        /// The transaction's identifier.
        id: NavigationTransactionId,
        /// The depth that settles it.
        expected: usize,
        /// The states it removed — `popped` when it completes.
        removed: Vec<Rc<RefCell<NavigationDestinationState>>>,
    }

    impl fmt::Debug for Pending {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Pending")
                .field("id", &self.id)
                .field("expected", &self.expected)
                .field("removed", &self.removed.len())
                .finish()
        }
    }

    /// One page on the stack.
    struct Entry {
        /// The page's view controller.
        controller: Retained<NavContentController>,
        /// The rendered content — kept for its watchers.
        #[allow(dead_code)]
        leaf: NativeLeaf,
        /// The rendered bar — kept for its leaves.
        #[allow(dead_code)]
        bar: BarState,
        /// The destination's reactive state.
        state: Rc<RefCell<NavigationDestinationState>>,
        /// The transition this page arrives and leaves with.
        transition: NativeNavigationTransition,
        /// The entry's own watchers.
        #[allow(dead_code)]
        keep: KeepAlive,
    }

    impl fmt::Debug for Entry {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Entry")
                .field("transition", &self.transition)
                .finish_non_exhaustive()
        }
    }

    /// The `UIKit` driver.
    pub(super) struct Stack {
        /// The environment children resolve against — carries this stack's
        /// model controller.
        env: OnceCell<Environment>,
        /// The render capability for inserted pages.
        renderer: OnceCell<Renderer>,
        /// The model controller — retained to answer `transition_*` and
        /// `complete_native_pop` after `NavigationController::new` hands
        /// the receiver out.
        model: OnceCell<NavigationController>,
        /// The native `UINavigationController` subclass.
        nav: OnceCell<Retained<cocoa_ui::uikit::NavigationController>>,
        /// The stack's declared transition.
        transition: AnyNavigationTransition,
        /// The pages — entry 0 is the root.
        pages: RefCell<Vec<Rc<Entry>>>,
        /// The outstanding transaction, when any.
        pending: RefCell<Option<Pending>>,
        /// The main-thread proof.
        mtm: MainThreadMarker,
    }

    impl fmt::Debug for Stack {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Stack")
                .field("pages", &self.pages.borrow().len())
                .finish_non_exhaustive()
        }
    }

    impl Stack {
        /// A driver holding only the declared transition; `prepare` wires
        /// the environment, `build` renders the root.
        pub(super) fn new(transition: AnyNavigationTransition) -> Self {
            Self {
                env: OnceCell::new(),
                renderer: OnceCell::new(),
                model: OnceCell::new(),
                nav: OnceCell::new(),
                transition,
                pages: RefCell::new(Vec::new()),
                pending: RefCell::new(None),
                mtm: MainThreadMarker::new().expect("views resolve on the main thread"),
            }
        }

        /// Captures the environment the children resolve against and the
        /// render capability matching it.
        pub(super) fn prepare(&self, ctx: &RenderContext<'_>, env: Environment) {
            let _ = self.renderer.set(ctx.with_env(&env).renderer());
            let _ = self.model.set(
                env.get::<NavigationController>()
                    .expect("installed")
                    .clone(),
            );
            let _ = self.env.set(env);
        }

        /// The environment children render under.
        fn env(&self) -> &Environment {
            self.env.get().expect("prepared")
        }

        /// Renders the root page, installs the native controller's handlers
        /// and returns the stack's leaf.
        pub(super) fn build(self: &Rc<Self>, root: NavigationView) -> NativeLeaf {
            let root = self.make_entry(root, true);
            let nav = cocoa_ui::uikit::NavigationController::new(self.mtm, &root.controller);
            nav.set_prefers_large_titles(true);
            let driver = Rc::downgrade(self);
            nav.set_pop_handler(move |count| {
                if let Some(driver) = driver.upgrade() {
                    driver.native_pop(count);
                }
            });
            let driver = Rc::downgrade(self);
            nav.set_pop_gate(move || {
                driver
                    .upgrade()
                    .is_some_and(|driver| driver.top_attempt_pop())
            });
            let driver = Rc::downgrade(self);
            nav.set_show_handler(move |depth| {
                if let Some(driver) = driver.upgrade() {
                    driver.settled(depth);
                }
            });
            self.pages.borrow_mut().push(Rc::new(root));
            let _ = self.nav.set(nav);
            // A `UINavigationController` owns the bar and the scroll insets of
            // the content it hosts — `WuiSafeAreaManaging` in the baseline —
            // so the leaf reports it through a kit host and the window root
            // hands it the whole window, not the safe-area rect.
            let host = HostView::new(self.mtm, KitRect::ZERO);
            host.set_manages_safe_area(true);
            let nav_view = self
                .nav
                .get()
                .expect("installed")
                .view()
                .expect("navigation view");
            view::add_subview(&host, &nav_view);
            host.set_primary_content_handler({
                let nav_view = nav_view.clone();
                move |_| Some(nav_view.clone())
            });
            host.set_layout_handler(|host| {
                let bounds = view::bounds(host);
                if let Some(sub) = view::subviews(host).first() {
                    view::set_frame(sub, bounds);
                }
            });
            let mut leaf = NativeLeaf::new(&*host, Fill);
            leaf.keep(host);
            leaf.keep(nav_view);
            leaf
        }

        /// Renders one page: content plus `Bar` chrome on `navigationItem`.
        fn make_entry(&self, mut view: NavigationView, root: bool) -> Entry {
            let renderer = self.renderer.get().expect("prepared");
            let ctx = renderer.context();
            let leaf = ctx.render(core::mem::take(&mut view.content));
            let bar = BarState::new(&mut view.bar, &ctx);
            let controller = NavContentController::new(self.mtm, leaf.view());
            let state = Rc::new(RefCell::new(core::mem::take(&mut view.state)));
            let transition = page_transition(view.transition.as_ref(), &self.transition);
            let mut keep = KeepAlive::default();
            let mut page = NavPage {
                title: bar.title.text.clone().unwrap_or_default(),
                subtitle: bar.subtitle.text.clone(),
                ..NavPage::default()
            };
            if let Some(principal) = bar.principal() {
                page.title_view = Some(cocoa_ui::view::retain_base(principal.leaf.view()));
            } else if bar.title.text.is_none() {
                page.title_view = Some(cocoa_ui::view::retain_base(bar.title.leaf.view()));
            }
            page.leading = bar
                .leading_items()
                .map(|item| self.button(item, &mut keep))
                .collect();
            page.trailing = bar
                .trailing_items()
                .map(|item| self.button(item, &mut keep))
                .collect();
            page.bottom = bar
                .bottom_items()
                .map(|item| self.button(item, &mut keep))
                .collect();
            page.large_title = match bar.display_mode {
                NavigationTitleDisplayMode::Automatic => LargeTitle::Automatic,
                NavigationTitleDisplayMode::Inline => LargeTitle::Inline,
                NavigationTitleDisplayMode::Medium | NavigationTitleDisplayMode::Large => {
                    LargeTitle::Large
                }
            };
            page.hides_back = root;
            if !root {
                let state = Rc::clone(&state);
                let env = self.env().clone();
                let model = self.model.get().expect("prepared").clone();
                page.on_back = Some(Rc::new(move || {
                    if state.borrow_mut().attempt_pop(&env) {
                        model.request_pop(1);
                    }
                }));
            }
            if let Some(search) = bar.search.as_ref() {
                page.search = Some(attach_search(search, ctx.env(), &controller, &mut keep));
            }
            page.hidden = bar.hidden.snapshot();
            controller.set_page(&page);
            {
                let appear_state = Rc::clone(&state);
                let env = self.env().clone();
                controller.set_appear_handler(move || {
                    appear_state.borrow_mut().appeared(&env);
                });
                let disappear_state = Rc::clone(&state);
                let env = self.env().clone();
                controller.set_disappear_handler(move || {
                    disappear_state.borrow_mut().disappeared(&env);
                });
            }
            {
                let nav = self.nav.get().cloned();
                keep.bind(&bar.hidden, move |hidden| {
                    if let Some(nav) = nav.as_ref() {
                        nav.set_bar_hidden(hidden, false);
                    }
                });
            }
            Entry {
                controller,
                leaf,
                bar,
                state,
                transition,
                keep,
            }
        }

        /// One semantic item as a `UIBarButtonItem`: an icon item carries a
        /// symbol plus an action forwarded to the rendered content's first
        /// button — `firstButton.sendActions(.primaryActionTriggered)` —
        /// while a plain item hosts the content itself. The title signal
        /// feeds `accessibilityLabel`.
        fn button(&self, item: &BarItem, keep: &mut KeepAlive) -> Retained<UIBarButtonItem> {
            let action = first_button(item.leaf.view()).map(|button| {
                Rc::new(move || {
                    button.sendActionsForControlEvents(UIControlEvents::PrimaryActionTriggered);
                }) as Rc<dyn Fn()>
            });
            let object = match item.icon.as_ref() {
                Some(BarItemIcon::System(name)) => bar_item(self.mtm, Some(name), None, action),
                Some(BarItemIcon::View(icon)) => {
                    // The declared icon draws inside the bar's chrome as a
                    // template image the bar tints; a view that cannot
                    // render stays hosted as the item's custom view.
                    let size = icon
                        .layout()
                        .measure(ProposalSize {
                            width: None,
                            height: None,
                        })
                        .size;
                    cocoa_ui::view::set_frame(
                        icon.view(),
                        cocoa_ui::geometry::Rect::new(
                            0.0,
                            0.0,
                            f64::from(size.width),
                            f64::from(size.height),
                        ),
                    );
                    if let Some(image) = cocoa_ui::bitmap::view_template_image(icon.view(), 24.0) {
                        cocoa_ui::uikit::image_bar_item(self.mtm, Some(&image), action)
                    } else {
                        cocoa_ui::view::set_frame(item.leaf.view(), bar_item_frame(item));
                        bar_item(self.mtm, None, Some(item.leaf.view()), action)
                    }
                }
                None => {
                    // A hosted item must arrive with a real frame: the bar
                    // wraps it as the item's customView and never lays it
                    // out itself.
                    cocoa_ui::view::set_frame(item.leaf.view(), bar_item_frame(item));
                    bar_item(self.mtm, None, Some(item.leaf.view()), None)
                }
            };
            if let Some(title) = item.title.clone() {
                let object = object.clone();
                keep.bind(&title, move |title| {
                    let text = title.to_plain().to_string();
                    cocoa_ui::uikit::bar_item_accessibility_label(&object, &text);
                });
            }
            object
        }

        /// `didShow` reconcile: the pending transaction's removed states
        /// hear `popped` and the transaction is acknowledged — or cancelled
        /// on a mismatch, the baseline's `expectedNativeStack` check.
        fn settled(&self, depth: usize) {
            let Some(pending) = self.pending.borrow_mut().take() else {
                return;
            };
            let model = self.model.get().expect("prepared");
            if depth == pending.expected {
                let _ = model.transition_completed(pending.id);
            } else {
                let _ = model.transition_cancelled(pending.id);
            }
            let env = self.env().clone();
            for state in pending.removed {
                state.borrow_mut().popped(&env);
            }
        }

        /// A user pop — gesture or the bar's back button — pops the last
        /// `count` states and tells the model.
        fn native_pop(&self, count: usize) {
            let env = self.env().clone();
            let popped: Vec<Rc<Entry>> = {
                let pages = self.pages.borrow();
                pages
                    .iter()
                    .skip(pages.len().saturating_sub(count))
                    .cloned()
                    .collect()
            };
            for entry in &popped {
                entry.state.borrow_mut().popped(&env);
            }
            self.model
                .get()
                .expect("prepared")
                .complete_native_pop(count);
            let mut pages = self.pages.borrow_mut();
            let keep = pages.len().saturating_sub(count);
            pages.truncate(keep);
        }

        /// The top page's pop verdict for the interactive gesture.
        fn top_attempt_pop(&self) -> bool {
            let env = self.env().clone();
            self.pages
                .borrow()
                .last()
                .is_none_or(|entry| entry.state.borrow_mut().attempt_pop(&env))
        }
    }

    impl Sink for Stack {
        fn apply(&self, transaction: NavigationTransaction) {
            // A superseded transaction is cancelled; its removed states
            // still hear `popped` — they are off the stack either way.
            if let Some(pending) = self.pending.borrow_mut().take() {
                let _ = self
                    .model
                    .get()
                    .expect("prepared")
                    .transition_cancelled(pending.id);
                let env = self.env().clone();
                for state in pending.removed {
                    state.borrow_mut().popped(&env);
                }
            }
            let prefix = transaction.retained_prefix.saturating_add(1);
            let mut pages = self.pages.borrow_mut();
            let cut = prefix.min(pages.len());
            let removed: Vec<Rc<Entry>> = pages.split_off(cut);
            let removed: Vec<Rc<RefCell<NavigationDestinationState>>> = removed
                .iter()
                .map(|entry| Rc::clone(&entry.state))
                .collect();
            let mut inserted = Vec::new();
            for builder in transaction.inserted {
                inserted.push(Rc::new(self.make_entry(builder.build(), false)));
            }
            let animated = inserted
                .last()
                .or_else(|| pages.last())
                .is_some_and(|entry| entry.transition != NativeNavigationTransition::None);
            pages.extend(inserted);
            let controllers: Vec<Retained<NavContentController>> =
                pages.iter().map(|entry| entry.controller.clone()).collect();
            self.nav
                .get()
                .expect("built")
                .set_pages(&controllers, animated);
            *self.pending.borrow_mut() = Some(Pending {
                id: transaction.id,
                expected: pages.len(),
                removed,
            });
        }
    }

    /// The page's search config plus the live bindings that keep the
    /// drawer's text and placeholder signals flowing both ways.
    fn attach_search(
        search: &NavigationSearch,
        env: &Environment,
        controller: &Retained<NavContentController>,
        keep: &mut KeepAlive,
    ) -> NavSearch {
        let config = NavSearch {
            placeholder: search_prompt(search, env).snapshot().to_plain().to_string(),
            text: search.text.snapshot().to_string(),
            placement: Some(cocoa_ui::uikit::SearchBarPlacement::Stacked),
            hides_when_scrolling: Some(false),
        };
        let binding = search.text.clone();
        controller.set_search_change_handler(move |text| {
            binding.set(Str::from(text));
        });
        let target = controller.clone();
        keep.bind(&search.text.clone(), move |text| {
            target.set_search_text(text.as_ref());
        });
        let target = controller.clone();
        keep.bind(&search_prompt(search, env), move |prompt| {
            target.set_search_placeholder(&prompt.to_plain());
        });
        config
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use alloc::rc::Rc;
    use alloc::string::ToString;
    use alloc::vec::Vec;
    use core::cell::{Cell, OnceCell, RefCell};
    use core::fmt;

    use cocoa_ui::Retained;
    use cocoa_ui::appkit::{
        HostView, HostedItem, HostedSearch, SearchField, ToolbarChild, ToolbarContent,
        WindowToolbar, activate, first_button, run_animation, set_animated_alpha,
    };
    use cocoa_ui::geometry::Size as KitSize;
    use cocoa_ui::objc2_app_kit::{NSImage, NSView, NSWindowStyleMask};
    use cocoa_ui::view::{bounds, set_frame};
    use waterui::Environment;
    use waterui::Str;
    use waterui::navigation::{
        AnyNavigationTransition, NativeNavigationTransition, NavigationController,
        NavigationDestinationState, NavigationTransaction, NavigationView,
    };
    use waterui::reactive::Signal;
    use waterui_core::layout::ProposalSize;

    use crate::contract::{KeepAlive, NativeLeaf, RenderContext, Renderer};

    use super::super::bar::{BarItem, BarItemIcon, BarState, search_prompt};
    use super::{Fill, Sink, page_transition};

    /// One page on the stack.
    struct Entry {
        /// The page's host view.
        view: Retained<NSView>,
        /// The rendered content — kept for its watchers.
        #[allow(dead_code)]
        leaf: NativeLeaf,
        /// The rendered bar — kept for its leaves.
        bar: BarState,
        /// The destination's reactive state.
        state: Rc<RefCell<NavigationDestinationState>>,
        /// The transition this page arrives and leaves with.
        transition: NativeNavigationTransition,
        /// The entry's own watchers.
        #[allow(dead_code)]
        keep: KeepAlive,
    }

    impl fmt::Debug for Entry {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Entry")
                .field("transition", &self.transition)
                .finish_non_exhaustive()
        }
    }

    /// The `AppKit` driver.
    pub(super) struct Stack {
        /// The environment children resolve against — carries this stack's
        /// model controller.
        env: OnceCell<Environment>,
        /// The render capability for inserted pages.
        renderer: OnceCell<Renderer>,
        /// The model controller.
        model: OnceCell<NavigationController>,
        /// The stack's host view.
        host: Retained<HostView>,
        /// The stack's declared transition.
        transition: AnyNavigationTransition,
        /// The pages — entry 0 is the root.
        entries: RefCell<Vec<Rc<Entry>>>,
        /// The window's toolbar coordinator, while a titled window hosts us.
        toolbar: RefCell<Option<Retained<WindowToolbar>>>,
        /// The search field published for the current top page.
        search_field: RefCell<Option<Retained<SearchField>>>,
        /// Whether the stack currently owns its chrome — `false` inside a
        /// hidden tab page.
        chrome_active: Cell<bool>,
        /// The current top entry's identity — `report_top_change`'s
        /// previous page.
        top_view: Cell<Option<usize>>,
        /// The main-thread proof.
        mtm: cocoa_ui::MainThreadMarker,
        /// A weak self-reference for callbacks — set once the driver sits
        /// inside its `Rc`.
        self_weak: OnceCell<std::rc::Weak<Self>>,
    }

    impl fmt::Debug for Stack {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Stack")
                .field("entries", &self.entries.borrow().len())
                .finish_non_exhaustive()
        }
    }

    impl Stack {
        /// A driver holding the declared transition and its host view.
        pub(super) fn new(transition: AnyNavigationTransition) -> Self {
            let mtm = cocoa_ui::MainThreadMarker::new().expect("main thread");
            Self {
                env: OnceCell::new(),
                renderer: OnceCell::new(),
                model: OnceCell::new(),
                host: HostView::new(mtm, cocoa_ui::geometry::Rect::ZERO),
                transition,
                entries: RefCell::new(Vec::new()),
                toolbar: RefCell::new(None),
                search_field: RefCell::new(None),
                chrome_active: Cell::new(true),
                top_view: Cell::new(None),
                mtm,
                self_weak: OnceCell::new(),
            }
        }

        /// Captures the environment and render capability.
        pub(super) fn prepare(&self, ctx: &RenderContext<'_>, env: Environment) {
            let _ = self.renderer.set(ctx.with_env(&env).renderer());
            let _ = self.model.set(
                env.get::<NavigationController>()
                    .expect("installed")
                    .clone(),
            );
            let _ = self.env.set(env);
        }

        /// The environment children render under.
        fn env(&self) -> &Environment {
            self.env.get().expect("prepared")
        }

        /// Builds the leaf: root entry pushed, layout/window/hidden
        /// handlers wired.
        pub(super) fn build(self: &Rc<Self>, root: NavigationView) -> NativeLeaf {
            let _ = self.self_weak.set(Rc::downgrade(self));
            let driver = Rc::downgrade(self);
            self.host.set_layout_handler(move |host| {
                if let Some(driver) = driver.upgrade() {
                    driver.layout(host);
                }
            });
            let driver = Rc::downgrade(self);
            self.host.set_window_handler(move |host| {
                if let Some(driver) = driver.upgrade() {
                    driver.window_changed(host);
                }
            });
            let driver = Rc::downgrade(self);
            self.host.set_hidden_handler(move |_host, hidden| {
                if let Some(driver) = driver.upgrade() {
                    driver.set_chrome_active(!hidden);
                }
            });
            let entry = self.make_entry(root);
            self.entries.borrow_mut().push(Rc::new(entry));
            self.place_entries();
            NativeLeaf::new(&*self.host, Fill)
        }

        /// Renders one page: content in a host view, bar state and the
        /// hidden watcher that republishes chrome — `refreshChromeIfTop`.
        fn make_entry(&self, mut view: NavigationView) -> Entry {
            let renderer = self.renderer.get().expect("prepared");
            let ctx = renderer.context();
            let leaf = ctx.render(core::mem::take(&mut view.content));
            let bar = BarState::new(&mut view.bar, &ctx);
            let host = HostView::new(self.mtm, cocoa_ui::geometry::Rect::ZERO);
            host.add_subview(leaf.view());
            host.set_layout_handler(|host| {
                let rect = bounds(host);
                for subview in cocoa_ui::view::subviews(host) {
                    set_frame(&subview, rect);
                }
            });
            let state = Rc::new(RefCell::new(core::mem::take(&mut view.state)));
            let transition = page_transition(view.transition.as_ref(), &self.transition);
            let mut keep = KeepAlive::default();
            {
                let driver = self.self_weak.get().expect("built").clone();
                keep.watch(&bar.hidden, move |_| {
                    if let Some(driver) = driver.upgrade() {
                        driver.publish();
                    }
                });
            }
            Entry {
                view: host.into_super(),
                leaf,
                bar,
                state,
                transition,
                keep,
            }
        }

        /// Frames every entry inside `host`: each fills bounds minus the
        /// titlebar's safe-area inset — `layoutEntries`.
        fn layout(&self, host: &HostView) {
            let mut rect = bounds(host);
            let inset = self.host.safe_area_insets().top;
            rect.origin.y += inset;
            rect.size.height -= inset;
            for entry in self.entries.borrow().iter() {
                cocoa_ui::view::set_frame(&entry.view, rect);
            }
        }

        /// Window attach/detach: a titled window gets the toolbar
        /// coordinator — `WuiWindowToolbar.attached(to:)`.
        fn window_changed(&self, host: &HostView) {
            let toolbar = cocoa_ui::view::window(host)
                .filter(|window| window.styleMask().contains(NSWindowStyleMask::Titled))
                .map(|window| WindowToolbar::attached(&window));
            *self.toolbar.borrow_mut() = toolbar;
            self.publish();
        }

        /// `setNavigationChromeActive(_:)`: a containing tab hides us, so
        /// the toolbar contribution is withdrawn.
        fn set_chrome_active(&self, active: bool) {
            self.chrome_active.set(active);
            self.publish();
        }

        /// Replaces the page views to match `entries`: only the top entry
        /// shows; a `Fade` transition cross-fades — `applyEntryChange`.
        fn place_entries(&self) {
            let previous_top = self.top_view.get();
            let entries = self.entries.borrow();
            let top = entries.last().cloned();
            let fade = top
                .as_ref()
                .is_some_and(|entry| entry.transition == NativeNavigationTransition::Fade);
            let mut hide_after = Vec::new();
            for (index, entry) in entries.iter().enumerate() {
                let is_top = index + 1 == entries.len();
                if !cocoa_ui::view::subviews(&self.host)
                    .iter()
                    .any(|sub| **sub == *entry.view)
                {
                    self.host.add_subview(&entry.view);
                }
                if fade && is_top {
                    cocoa_ui::view::set_alpha(&entry.view, 0.0);
                    cocoa_ui::view::set_hidden(&entry.view, false);
                } else if fade {
                    // The outgoing page stays visible until the fade ends.
                    hide_after.push(entry.view.clone());
                } else {
                    cocoa_ui::view::set_hidden(&entry.view, !is_top);
                }
            }
            drop(entries);
            self.layout(&self.host);
            if fade && let Some(entry) = top {
                let shown = entry.view.clone();
                run_animation(
                    Rc::new(move || {
                        set_animated_alpha(&shown, 1.0);
                    }),
                    Rc::new(move || {
                        for view in &hide_after {
                            cocoa_ui::view::set_hidden(view, true);
                        }
                    }),
                );
            }
            self.report_top_change(previous_top);
            self.publish();
        }

        /// `appear`/`disappear` on the top edge — `pageTransitions`.
        fn report_top_change(&self, previous: Option<usize>) {
            let env = self.env().clone();
            let entries = self.entries.borrow();
            let current = entries
                .last()
                .map(|entry| std::ptr::from_ref(&**entry) as usize);
            match (previous, current) {
                (Some(previous), Some(current)) if previous == current => {}
                (_, current) => {
                    if let Some(previous) = previous
                        && let Some(old) = entries
                            .iter()
                            .find(|entry| std::ptr::from_ref(&**entry) as usize == previous)
                    {
                        old.state.borrow_mut().disappeared(&env);
                    }
                    if let Some(current) = current
                        && let Some(new) = entries
                            .iter()
                            .find(|entry| std::ptr::from_ref(&**entry) as usize == current)
                    {
                        new.state.borrow_mut().appeared(&env);
                    }
                }
            }
            self.top_view.set(current);
        }

        /// Publishes the top page's chrome into the window toolbar, or
        /// withdraws it — `updateWindowToolbar()`'s `topEntry` snapshot.
        fn publish(&self) {
            let toolbar = self.toolbar.borrow().clone();
            let Some(toolbar) = toolbar else {
                return;
            };
            let owner = std::ptr::from_ref(self) as usize;
            let withdraw = |stack: &Self| {
                toolbar.clear_content(owner);
                stack.search_field.replace(None);
            };
            if !self.chrome_active.get() {
                withdraw(self);
                return;
            }
            let entries = self.entries.borrow();
            let Some(top) = entries.last() else {
                drop(entries);
                withdraw(self);
                return;
            };
            if top.bar.hidden.snapshot() {
                drop(entries);
                withdraw(self);
                return;
            }
            let mut content = ToolbarContent {
                shows_back: entries.len() > 1,
                ..ToolbarContent::default()
            };
            if content.shows_back {
                let state = Rc::clone(&top.state);
                let env = self.env().clone();
                let model = self.model.get().expect("prepared").clone();
                content.on_back = Some(Rc::new(move || {
                    if state.borrow_mut().attempt_pop(&env) {
                        model.request_pop(1);
                    }
                }));
            }
            if let Some(text) = top.bar.title.text.clone() {
                content.title = Some(text);
            } else {
                content.title_item = Some(HostedItem {
                    view: cocoa_ui::view::retain_base(top.bar.title.leaf.view()),
                    size: measure(&top.bar.title.leaf),
                });
            }
            content.leading = top.bar.leading().map(Self::child);
            content.trailing = top.bar.trailing().map(Self::child);
            content.status = top.bar.status().map(|item| HostedItem {
                view: cocoa_ui::view::retain_base(item.leaf.view()),
                size: measure(&item.leaf),
            });
            if let Some(search) = top.bar.search.as_ref() {
                let field = SearchField::new(self.mtm);
                field.set_placeholder(
                    search_prompt(search, self.env())
                        .snapshot()
                        .to_plain()
                        .as_ref(),
                );
                field.set_text(search.text.snapshot().as_ref());
                let binding = search.text.clone();
                field.set_change_handler(move |field| {
                    binding.set(Str::from(field.text()));
                });
                let source_id = std::ptr::from_ref(&**top) as usize;
                self.search_field.replace(Some(field.clone()));
                content.search = Some(HostedSearch { field, source_id });
            } else {
                self.search_field.replace(None);
            }
            drop(entries);
            toolbar.set_content(content, owner);
        }

        /// One semantic item as a `ToolbarChild`: a `System` icon turns the
        /// item into a capsule whose action forwards to the content's first
        /// control — `firstButton`'s `performClick` — and a `View` icon or
        /// no icon hosts the content itself.
        fn child(item: &BarItem) -> ToolbarChild {
            // The item's action and its chrome both come from the button
            // inside it, the way `firstButton` informed `actionItem`.
            let button = first_button(item.leaf.view());
            let action = button.as_ref().map(|button| {
                let button = button.clone();
                Rc::new(move || {
                    // SAFETY: toolbar actions fire on the main thread.
                    unsafe { activate(button.control()) };
                }) as Rc<dyn Fn()>
            });
            let bordered = button.as_ref().is_none_or(|button| !button.is_borderless());
            let label = item
                .title
                .as_ref()
                .map(|title| title.snapshot().to_plain().to_string())
                .unwrap_or_default();
            match item.icon.as_ref() {
                Some(BarItemIcon::System(name)) => ToolbarChild {
                    view: HostedItem {
                        view: cocoa_ui::view::retain_base(item.leaf.view()),
                        size: measure(&item.leaf),
                    },
                    icon: NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        &objc2_foundation::NSString::from_str(name),
                        None,
                    ),
                    label,
                    bordered,
                    action,
                },
                Some(BarItemIcon::View(icon)) => {
                    let size = measure(icon);
                    set_frame(
                        icon.view(),
                        cocoa_ui::geometry::Rect::new(0.0, 0.0, size.width, size.height),
                    );
                    ToolbarChild {
                        view: HostedItem {
                            view: cocoa_ui::view::retain_base(item.leaf.view()),
                            size: measure(&item.leaf),
                        },
                        icon: cocoa_ui::bitmap::view_template_image(icon.view(), 18.0),
                        label,
                        bordered,
                        action,
                    }
                }
                None => ToolbarChild {
                    view: HostedItem {
                        view: cocoa_ui::view::retain_base(item.leaf.view()),
                        size: measure(&item.leaf),
                    },
                    icon: None,
                    label,
                    bordered,
                    action,
                },
            }
        }
    }

    impl Sink for Stack {
        fn apply(&self, transaction: NavigationTransaction) {
            let prefix = transaction.retained_prefix.saturating_add(1);
            let mut entries = self.entries.borrow_mut();
            let cut = prefix.min(entries.len());
            let removed: Vec<Rc<Entry>> = entries.split_off(cut);
            let env = self.env().clone();
            for entry in &removed {
                cocoa_ui::view::remove_from_superview(&entry.view);
                entry.state.borrow_mut().popped(&env);
            }
            for builder in transaction.inserted {
                entries.push(Rc::new(self.make_entry(builder.build())));
            }
            drop(entries);
            self.place_entries();
            // `AppKit` mounts synchronously (the fade animates in place, the
            // views are already laid out) — the transaction settles now.
            let _ = self
                .model
                .get()
                .expect("prepared")
                .transition_completed(transaction.id);
        }
    }

    /// A `BarTitle`/`BarItem` leaf measured at the unspecified proposal —
    /// `sizeThatFits(WuiProposalSize())`.
    fn measure(leaf: &NativeLeaf) -> KitSize {
        let size = leaf
            .layout()
            .measure(ProposalSize {
                width: None,
                height: None,
            })
            .size;
        KitSize::new(f64::from(size.width), f64::from(size.height))
    }
}
