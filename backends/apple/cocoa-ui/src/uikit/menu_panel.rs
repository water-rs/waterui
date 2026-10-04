//! The presented panel a context menu with an accessory uses instead of a
//! `UIContextMenuInteraction`.
//!
//! `UIKit`'s context-menu interaction hands the preview to a rendered
//! portal: the preview's view is parked off-screen and delivers no touch
//! events, so an accessory can never share its space and no public API
//! reports the presented rect. A menu carrying an accessory is instead
//! realized as one owned [`UIViewController`] presented
//! `UIModalPresentationPopover`, anchored to the host view. The panel's own
//! view hierarchy composes the mounted preview leaf, the mounted accessory
//! leaf and the command rows built from the menu tree, so every part shares
//! one coordinate owner, real touch delivery and a live measure: a bound
//! value that resizes a leaf marks this view dirty through
//! `view::invalidate_layout`, the pass re-measures and the popover repacks.
//!
//! Rows are `UIButton`s under `UIButtonConfiguration`, so activation,
//! highlight and accessibility traits come from the control itself;
//! keyboard navigation is a focus ring driven by `UIKeyCommand`s on the
//! controller — arrows move it, Return or Right activates, Left pops a
//! submenu page and Escape dismisses, matching the canonical menu's keys.
//!
//! # Safety
//!
//! The `unsafe` here defines the `UIViewController` subclass `UIKit`
//! presents, its popover delegate and the `UIKeyCommand`s it answers. All
//! calls are main-thread calls: the classes are `MainThreadOnly` and the
//! panel lives for a presentation, owned by [`ContextMenuPopover`].

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSArray, NSObjectProtocol, NSString};
use objc2_ui_kit::{
    NSDirectionalEdgeInsets, NSDirectionalRectEdge, NSObjectUIAccessibility,
    UIAccessibilityTraitNotEnabled, UIAccessibilityTraitSelected,
    UIAdaptivePresentationControllerDelegate, UIButton, UIButtonConfiguration,
    UIButtonConfigurationTitleAlignment, UIColor, UIImage, UIImageView, UIKeyCommand,
    UIKeyInputDownArrow, UIKeyInputEscape, UIKeyInputLeftArrow, UIKeyInputRightArrow,
    UIKeyInputUpArrow, UIKeyModifierFlags, UILayoutPriorityFittingSizeLevel,
    UILayoutPriorityRequired, UIModalPresentationStyle, UIPopoverArrowDirection,
    UIPopoverPresentationControllerDelegate, UIPresentationController, UIScrollView, UIView,
    UIViewAnimationOptions, UIViewController,
};

use crate::action::{ActionTarget, ControlEvents};
use crate::callback::guarded;
use crate::geometry::MeasureProposal;
use crate::menu::{Command, MenuTreeNode};
use crate::uikit::host_view::HostView;

/// The colors a panel draws its chrome with.
///
/// The owner resolves them from the theme the menu presents in —
/// `Foreground`, `MutedForeground`, `Error`, `Border`,
/// `SelectionContainer` and `Surface` — and pushes a new palette through
/// [`ContextMenuPopover::apply_palette`] whenever a token changes, so a
/// panel already on screen repaints in place.
#[derive(Debug, Clone)]
pub struct PanelPalette {
    /// Row titles, subtitles' base and non-destructive symbols.
    pub label: Retained<UIColor>,
    /// Disclosure chevrons and other secondary chrome.
    pub muted: Retained<UIColor>,
    /// Destructive rows and their chrome.
    pub destructive: Retained<UIColor>,
    /// The hairline between row groups.
    pub separator: Retained<UIColor>,
    /// The keyboard-focus fill.
    pub focus_fill: Retained<UIColor>,
    /// The content surface the panel presents over its popover chrome.
    pub surface: Retained<UIColor>,
}

/// The gap between panel sections — preview to accessory to commands.
const SECTION_GAP: f64 = 12.0;
/// Padding above the first and below the last section.
const EDGE_PAD: f64 = 8.0;
/// The total height a separator row occupies, hairline included.
const SEPARATOR_HEIGHT: f64 = 9.0;
/// A command row is never shorter than the platform's touch target.
const ROW_MIN_HEIGHT: f64 = 44.0;
/// The leading gutter a selected row reserves for its checkmark, on top
/// of the plain content inset.
const CHECK_GUTTER: f64 = 22.0;
/// Where a checkmark's glyph sits inside the gutter.
const ROW_CHECK_INSET: f64 = 14.0;
/// The right margin a disclosure chevron's glyph keeps.
const ROW_CHEVRON_INSET: f64 = 12.0;
/// The gap between a row's content and its disclosure chevron.
const CHEVRON_GAP: f64 = 8.0;
/// The panel never grows past this fraction of its window, leaving room
/// for the dimmed backdrop a popover presents over.
const MAX_HEIGHT_FRACTION: f64 = 0.7;
const MAX_WIDTH_FRACTION: f64 = 0.85;
/// The corner radius of the keyboard-focus fill on a row.
const FOCUS_CORNER_RADIUS: f64 = 8.0;
/// Duration of a submenu push/pop crossfade.
const PAGE_TRANSITION_SECONDS: f64 = 0.2;

/// Which composited leaf a [`ContextMenuPopover::set_slot`] call places.
#[derive(Debug, Clone, Copy)]
pub enum PanelSlot {
    /// The lifted preview, shown above the accessory.
    Preview,
    /// The accessory, shown between the preview and the commands.
    Accessory,
}

/// What a row is: a separator, or a button carrying a command.
enum PanelRowKind {
    /// A hairline between row groups; kept so a palette change re-tints it.
    Separator { hairline: Retained<UIView> },
    Activatable {
        button: Retained<UIButton>,
        command: Command,
        /// The symbol the row draws leading its title — a command's own
        /// symbol, or the back row's `chevron.left`.
        leading_symbol: Option<String>,
        /// The checkmark glyph drawn when the command is `selected`.
        checkmark: Option<Retained<UIImageView>>,
        /// The disclosure glyph drawn for a submenu.
        chevron: Option<Retained<UIImageView>>,
        /// The trailing extent the row's chrome occupies — measured from
        /// the disclosure glyph — so the button's content never runs
        /// under it.
        trailing_reserve: f64,
        /// Whether the page reserves the checkmark gutter on every row —
        /// set when any command on the page is `selected`.
        check_gutter: bool,
        action: Rc<dyn Fn()>,
    },
}

/// One row of a [`PanelPage`].
struct PanelRow {
    view: Retained<UIView>,
    kind: PanelRowKind,
}

/// One menu page of rows; submenus push further pages. The action targets
/// the rows use live and die with their page — repeated push/pop cycles
/// keep nothing behind.
struct PanelPage {
    container: Retained<HostView>,
    /// The widest row's natural width — the page's content width before
    /// the viewport's clamp applies.
    width: f64,
    rows: Vec<PanelRow>,
    /// Storage for the rows' action targets; never read, dropped with the
    /// page.
    _keepalive: Vec<Box<dyn Any>>,
}

/// The ivars of a [`MenuPanelController`].
pub struct MenuPanelControllerIvars {
    /// The scroll view wrapping the column when the panel overflows.
    scroll: RefCell<Option<Retained<UIScrollView>>>,
    /// The stacked content the scroll view holds.
    column: RefCell<Option<Retained<HostView>>>,
    /// The mounted preview leaf's view.
    preview: RefCell<Option<Retained<UIView>>>,
    /// The mounted accessory leaf's view.
    accessory: RefCell<Option<Retained<UIView>>>,
    /// The page stack; page zero is the menu's top level.
    pages: RefCell<Vec<PanelPage>>,
    /// The keyboard-focused row's index into the focusable rows, if any.
    focused: Cell<Option<usize>>,
    /// The palette the rows and chrome draw with — resolved by the owner
    /// at construction and pushed again on every token change.
    palette: RefCell<PanelPalette>,
    /// The key commands the controller answers, built once.
    key_commands: RefCell<Option<Retained<NSArray<UIKeyCommand>>>>,
    /// The preferred size last reported — a same-size set is a no-op.
    reported_size: Cell<CGSize>,
    /// The window-derived caps applied to the preferred size.
    max_height: Cell<f64>,
    max_width: Cell<f64>,
    /// The teardown the owner runs once when the presentation ends.
    on_dismiss: RefCell<Option<Rc<dyn Fn()>>>,
}

impl fmt::Debug for MenuPanelControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuPanelControllerIvars")
            .field("pages", &self.pages.borrow().len())
            .field("focused", &self.focused.get())
            .finish_non_exhaustive()
    }
}

impl MenuPanelControllerIvars {
    /// The ivars a controller starts with: every slot empty except the
    /// palette the owner already resolved.
    fn new(palette: PanelPalette) -> Self {
        Self {
            scroll: RefCell::new(None),
            column: RefCell::new(None),
            preview: RefCell::new(None),
            accessory: RefCell::new(None),
            pages: RefCell::new(Vec::new()),
            focused: Cell::new(None),
            palette: RefCell::new(palette),
            key_commands: RefCell::new(None),
            reported_size: Cell::new(CGSize::new(-1.0, -1.0)),
            max_height: Cell::new(f64::MAX),
            max_width: Cell::new(f64::MAX),
            on_dismiss: RefCell::new(None),
        }
    }
}

define_class!(
    // SAFETY: `UIViewController` asks a subclass to initialize through
    // `initWithNibName:bundle:`, which `MenuPanelController::new` does, and
    // the class does not implement `Drop`.
    #[unsafe(super(UIViewController))]
    #[name = "CocoaUiMenuPanelController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuPanelControllerIvars]
    #[derive(Debug)]
    /// The popover's content controller: composes the mounted leaves and
    /// the command rows, and owns the keyboard focus ring.
    struct MenuPanelController;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIViewController`
    // subclass.
    unsafe impl NSObjectProtocol for MenuPanelController {}

    impl MenuPanelController {
        // SAFETY: see the module safety note.
        #[unsafe(method(canBecomeFirstResponder))]
        fn can_become_first_responder(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidAppear:))]
        fn view_did_appear(&self, animated: bool) {
            guarded("MenuPanelController viewDidAppear", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), viewDidAppear: animated] };
                // Hardware-keyboard navigation needs the panel in the
                // responder chain; a presented controller takes it here.
                self.becomeFirstResponder();
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewDidDisappear:))]
        fn view_did_disappear(&self, animated: bool) {
            guarded("MenuPanelController viewDidDisappear", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), viewDidDisappear: animated] };
                // The presentation can also end without the panel's own
                // `dismiss` or the adaptive delegate: a presenter-side
                // `dismissViewController`, the presenter being dismissed
                // or popped, or the scene tearing its window down removes
                // the view without either callback, and the panel would
                // stay presented in the owner's state — the next open
                // refused. `isBeingDismissed` reports a dismissal of the
                // controller or an ancestor;
                // `isMovingFromParentViewController` covers container
                // removal; a vanished presenter means the presentation is
                // already gone. A temporary cover — a presented child —
                // reports none of these, so it is not a dismissal.
                if self.isBeingDismissed()
                    || self.isMovingFromParentViewController()
                    || self.presentingViewController().is_none()
                {
                    self.run_on_dismiss();
                }
            });
        }

        // SAFETY: `keyCommands` is a plain responder-chain query the
        // controller answers with its stored commands.
        #[unsafe(method_id(keyCommands))]
        fn key_commands_override(&self) -> Option<Retained<NSArray<UIKeyCommand>>> {
            self.ivars().key_commands.borrow().clone()
        }

        // SAFETY: `cocoaUiMenuUp:` is the action the up-arrow command
        // declares; `UIKeyCommand` sends it with itself as sender.
        #[unsafe(method(cocoaUiMenuUp:))]
        fn menu_up(&self, _command: &UIKeyCommand) {
            guarded("MenuPanelController cocoaUiMenuUp:", || self.move_focus(-1));
        }

        // SAFETY: see `menu_up`.
        #[unsafe(method(cocoaUiMenuDown:))]
        fn menu_down(&self, _command: &UIKeyCommand) {
            guarded("MenuPanelController cocoaUiMenuDown:", || self.move_focus(1));
        }

        // SAFETY: see `menu_up`.
        #[unsafe(method(cocoaUiMenuBack:))]
        fn menu_back(&self, _command: &UIKeyCommand) {
            guarded("MenuPanelController cocoaUiMenuBack:", || self.pop_page());
        }

        // SAFETY: see `menu_up`.
        #[unsafe(method(cocoaUiMenuActivate:))]
        fn menu_activate(&self, _command: &UIKeyCommand) {
            guarded("MenuPanelController cocoaUiMenuActivate:", || {
                self.activate_focused();
            });
        }

        // SAFETY: see `menu_up`.
        #[unsafe(method(cocoaUiMenuDismiss:))]
        fn menu_dismiss_key(&self, _command: &UIKeyCommand) {
            guarded("MenuPanelController cocoaUiMenuDismiss:", || self.dismiss_menu());
        }
    }
);

impl MenuPanelController {
    /// A panel controller with its column, scroll view and key commands
    /// built but no content placed yet. `palette` is the owner's resolved
    /// theme — the panel has no chrome colors of its own.
    fn new(mtm: MainThreadMarker, palette: PanelPalette) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuPanelControllerIvars {
            key_commands: RefCell::new(Some(key_commands(mtm))),
            ..MenuPanelControllerIvars::new(palette)
        });
        // SAFETY: `initWithNibName:bundle:` is `UIViewController`'s
        // designated initializer; no nib means the view comes from us.
        let this: Retained<Self> = unsafe {
            msg_send![super(this), initWithNibName: None::<&NSString>, bundle: None::<&objc2_foundation::NSBundle>]
        };

        let root = HostView::new(mtm, crate::geometry::Rect::ZERO);
        let scroll = UIScrollView::new(mtm);
        let column = HostView::new(mtm, crate::geometry::Rect::ZERO);
        scroll.addSubview(&column);
        root.add_subview(&scroll);

        this.ivars().scroll.replace(Some(scroll));
        this.ivars().column.replace(Some(column));

        let weak = Weak::new(&*this);
        root.set_layout_handler(move |view| {
            if let Some(this) = weak.load() {
                if let Some(scroll) = this.ivars().scroll.borrow().as_ref() {
                    scroll.setFrame(view.bounds());
                }
                // The viewport changed; the column re-measures against it.
                if let Some(column) = this.ivars().column.borrow().as_ref() {
                    column.set_needs_layout();
                }
            }
        });
        let weak = Weak::new(&*this);
        if let Some(column) = this.ivars().column.borrow().as_ref() {
            column.set_layout_handler(move |_view| {
                if let Some(this) = weak.load() {
                    this.layout_column();
                }
            });
        }
        // SAFETY: `root` is a live view.
        this.setView(Some(&root));
        this
    }

    /// The view a mounted leaf is added to: the column the panel stacks.
    fn column_view(&self) -> Retained<UIView> {
        self.ivars()
            .column
            .borrow()
            .clone()
            .expect("column built")
            .into_super()
    }

    /// The size the content measures to, capped by the window fractions
    /// recorded at `present`. Heights are measured under the width the
    /// panel would actually get — the clamped natural width — so a leaf
    /// that wraps or grows when narrowed reports its true height.
    fn natural_size(&self) -> CGSize {
        let ivars = self.ivars();
        let parts: Vec<Retained<UIView>> = [
            ivars.preview.borrow().clone(),
            ivars.accessory.borrow().clone(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let pages = ivars.pages.borrow();
        let mut width = 0.0_f64;
        for part in &parts {
            width = width.max(fitting_size(part).width);
        }
        if let Some(page) = pages.last() {
            width = width.max(page.width);
        }
        let width = width.min(ivars.max_width.get());
        let mut height = EDGE_PAD;
        let mut sections = false;
        for part in &parts {
            height += if sections { SECTION_GAP } else { 0.0 } + measure_height_at(part, width);
            sections = true;
        }
        if let Some(page) = pages.last() {
            height += if sections { SECTION_GAP } else { 0.0 }
                + page
                    .rows
                    .iter()
                    .map(|row| row_height_for(row, width))
                    .sum::<f64>();
        }
        height += EDGE_PAD;
        CGSize::new(width, height.min(ivars.max_height.get()))
    }

    /// Places the column's parts and reports the natural size to the
    /// presentation — a size change repacks the popover. Runs on every
    /// layout pass, so a bound value that resizes a leaf lands here through
    /// the ancestor `setNeedsLayout` `invalidate_layout` performs.
    ///
    /// The column's width is the presented viewport's: the scroll view's
    /// bounds, which the root's layout pass has already sized to the
    /// `preferredContentSize` the popover adopted.
    fn layout_column(&self) {
        let ivars = self.ivars();
        let Some(column) = ivars.column.borrow().clone() else {
            return;
        };
        let width = {
            let scroll = ivars.scroll.borrow();
            scroll.as_ref().map(|scroll| scroll.bounds().size.width)
        }
        .filter(|width| *width > 0.0)
        .or_else(|| {
            self.view()
                .map(|view| view.bounds().size.width)
                .filter(|width| *width > 0.0)
        });
        let Some(width) = width else {
            return;
        };
        let mut y = EDGE_PAD;
        for part in [
            ivars.preview.borrow().clone(),
            ivars.accessory.borrow().clone(),
        ]
        .into_iter()
        .flatten()
        {
            let height = measure_height_at(&part, width);
            part.setFrame(CGRect::new(
                CGPoint::new(0.0, y),
                CGSize::new(width, height),
            ));
            y += height + SECTION_GAP;
        }
        if let Some(page) = ivars.pages.borrow().last() {
            let page_height: f64 = page.rows.iter().map(|row| row_height_for(row, width)).sum();
            page.container.setFrame(CGRect::new(
                CGPoint::new(0.0, y),
                CGSize::new(width, page_height),
            ));
            y += page_height;
        }
        let content_height = y + EDGE_PAD;
        column.setFrame(CGRect::new(
            CGPoint::ZERO,
            CGSize::new(width, content_height),
        ));
        if let Some(scroll) = ivars.scroll.borrow().as_ref() {
            scroll.setContentSize(CGSize::new(width, content_height));
        }
        let natural = self.natural_size();
        if natural != ivars.reported_size.get() {
            ivars.reported_size.set(natural);
            self.setPreferredContentSize(natural);
        }
    }

    /// The current page's focusable rows: indices of enabled buttons.
    fn focusables(&self) -> Vec<usize> {
        let pages = self.ivars().pages.borrow();
        let Some(page) = pages.last() else {
            return Vec::new();
        };
        page.rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match &row.kind {
                PanelRowKind::Activatable { button, .. } if button.isEnabled() => Some(index),
                _ => None,
            })
            .collect()
    }

    /// Moves the focus ring through the focusable rows, wrapping at the
    /// ends like a tracked menu does.
    fn move_focus(&self, delta: isize) {
        let focusables = self.focusables();
        if focusables.is_empty() {
            return;
        }
        let len = focusables.len().cast_signed();
        let next = self.ivars().focused.get().map_or_else(
            || {
                if delta >= 0 { 0 } else { focusables.len() - 1 }
            },
            |current| (current.cast_signed() + delta).rem_euclid(len) as usize,
        );
        self.set_focus(Some(next));
    }

    /// Draws the ring on the focusable row at `focus` (an index into the
    /// page's focusable rows), clears it elsewhere, and moves `VoiceOver`.
    fn set_focus(&self, focus: Option<usize>) {
        self.ivars().focused.set(focus);
        let ivars = self.ivars();
        let mut focused_view: Option<Retained<UIButton>> = None;
        let mut focused_rect: Option<CGRect> = None;
        {
            let pages = ivars.pages.borrow();
            let Some(page) = pages.last() else {
                return;
            };
            let focusables: Vec<usize> = page
                .rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| match &row.kind {
                    PanelRowKind::Activatable { button, .. } if button.isEnabled() => Some(index),
                    _ => None,
                })
                .collect();
            let focused_row = focus.and_then(|focus| focusables.get(focus).copied());
            let palette = ivars.palette.borrow().clone();
            for (index, row) in page.rows.iter().enumerate() {
                let PanelRowKind::Activatable {
                    button,
                    command,
                    leading_symbol,
                    trailing_reserve,
                    check_gutter,
                    ..
                } = &row.kind
                else {
                    continue;
                };
                let is_focused = focused_row == Some(index);
                button.setConfiguration(Some(&row_configuration(
                    button.mtm(),
                    command,
                    leading_symbol.as_deref(),
                    *trailing_reserve,
                    *check_gutter,
                    is_focused,
                    &palette,
                )));
                if is_focused {
                    focused_view = Some(button.clone());
                    let mut rect = row.view.frame();
                    rect.origin.y += page.container.frame().origin.y;
                    focused_rect = Some(rect);
                }
            }
        }
        if let (Some(rect), Some(scroll)) = (focused_rect, ivars.scroll.borrow().as_ref()) {
            scroll.scrollRectToVisible_animated(rect, false);
        }
        if let Some(view) = focused_view {
            // SAFETY: `UIAccessibilityPostNotification` is a main-thread
            // call and `view` is a live control.
            unsafe {
                objc2_ui_kit::UIAccessibilityPostNotification(
                    objc2_ui_kit::UIAccessibilityLayoutChangedNotification,
                    Some(&*Retained::as_ptr(&view).cast::<AnyObject>()),
                );
            }
        }
    }

    /// Runs the focused row's action — a command picks and dismisses, a
    /// submenu row pushes, a back row pops.
    fn activate_focused(&self) {
        let action = {
            let pages = self.ivars().pages.borrow();
            let Some(focused) = self.ivars().focused.get() else {
                return;
            };
            let Some(page) = pages.last() else {
                return;
            };
            let focusables: Vec<usize> = page
                .rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| match &row.kind {
                    PanelRowKind::Activatable { button, .. } if button.isEnabled() => Some(index),
                    _ => None,
                })
                .collect();
            focusables
                .get(focused)
                .and_then(|row| match &page.rows[*row].kind {
                    PanelRowKind::Activatable { action, .. } => Some(action.clone()),
                    PanelRowKind::Separator { .. } => None,
                })
        };
        if let Some(action) = action {
            action();
        }
    }

    /// Builds a page's rows: a leading back row for submenu pages, then a
    /// button per command, a hairline per divider and a chevron row per
    /// submenu.
    #[expect(
        clippy::too_many_lines,
        reason = "each node kind maps to one row builder; splitting them would scatter the column bookkeeping"
    )]
    fn build_page(&self, back: Option<&str>, nodes: &[MenuTreeNode]) -> PanelPage {
        let mtm = self.mtm();
        let container = HostView::new(mtm, crate::geometry::Rect::ZERO);
        let mut rows: Vec<PanelRow> = Vec::new();
        let mut keepalive: Vec<Box<dyn Any>> = Vec::new();
        let mut width = 0.0_f64;

        // A menu page shows one checkmark column: when any row is
        // selected every row reserves the gutter so titles stay aligned,
        // like the native menus do.
        let check_gutter = nodes.iter().any(|node| {
            matches!(node, MenuTreeNode::Command(command, _) if command.selected)
        });

        let mut push_row = |view: Retained<UIView>, kind: PanelRowKind| {
            width = match &kind {
                PanelRowKind::Separator { .. } => width,
                PanelRowKind::Activatable { button, .. } => {
                    width.max(button.intrinsicContentSize().width)
                }
            };
            container.add_subview(&view);
            rows.push(PanelRow { view, kind });
        };

        if let Some(title) = back {
            let command = Command {
                label: String::from(title),
                ..Command::default()
            };
            let weak = Weak::new(self);
            let action: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(this) = weak.load() {
                    this.pop_page();
                }
            });
            let row = row_button(
                mtm,
                &command,
                Some("chevron.left"),
                false,
                check_gutter,
                &self.ivars().palette.borrow(),
            );
            keepalive.push(Box::new(ActionTarget::new(
                &row.button,
                ControlEvents::TOUCH_UP_INSIDE,
                {
                    let action = action.clone();
                    move |_| action()
                },
            )));
            push_row(
                row.view,
                PanelRowKind::Activatable {
                    button: row.button,
                    command,
                    leading_symbol: Some(String::from("chevron.left")),
                    checkmark: row.checkmark,
                    chevron: row.chevron,
                    trailing_reserve: row.trailing_reserve,
                    check_gutter,
                    action,
                },
            );
        }

        for node in nodes {
            match node {
                MenuTreeNode::Divider => {
                    let separator = HostView::new(mtm, crate::geometry::Rect::ZERO);
                    separator.set_layout_handler(|view| {
                        for subview in view.subviews().to_vec() {
                            subview.setFrame(CGRect::new(
                                CGPoint::new(16.0, view.bounds().size.height / 2.0 - 0.25),
                                CGSize::new(view.bounds().size.width - 32.0, 0.5),
                            ));
                        }
                    });
                    let hairline = UIView::new(mtm);
                    hairline.setBackgroundColor(Some(&separator_color(self)));
                    separator.add_subview(&hairline);
                    push_row(separator.into_super(), PanelRowKind::Separator { hairline });
                }
                MenuTreeNode::Command(command, run) => {
                    let row = row_button(
                        mtm,
                        command,
                        command.symbol.as_deref(),
                        false,
                        check_gutter,
                        &self.ivars().palette.borrow(),
                    );
                    row.button.setEnabled(command.enabled);
                    let weak = Weak::new(self);
                    let action: Rc<dyn Fn()> = Rc::new({
                        let run = run.clone();
                        move || {
                            run();
                            if let Some(this) = weak.load() {
                                this.dismiss_menu();
                            }
                        }
                    });
                    keepalive.push(Box::new(ActionTarget::new(
                        &row.button,
                        ControlEvents::TOUCH_UP_INSIDE,
                        {
                            let action = action.clone();
                            move |_| action()
                        },
                    )));
                    push_row(
                        row.view,
                        PanelRowKind::Activatable {
                            button: row.button,
                            command: command.clone(),
                            leading_symbol: command.symbol.clone(),
                            checkmark: row.checkmark,
                            chevron: row.chevron,
                            trailing_reserve: row.trailing_reserve,
                            check_gutter,
                            action,
                        },
                    );
                }
                MenuTreeNode::Submenu(command, children) => {
                    let row = row_button(
                        mtm,
                        command,
                        command.symbol.as_deref(),
                        true,
                        check_gutter,
                        &self.ivars().palette.borrow(),
                    );
                    row.button.setEnabled(command.enabled);
                    let weak = Weak::new(self);
                    let title = command.label.clone();
                    let children = children.clone();
                    let action: Rc<dyn Fn()> = Rc::new(move || {
                        if let Some(this) = weak.load() {
                            this.push_page(&title, &children);
                        }
                    });
                    keepalive.push(Box::new(ActionTarget::new(
                        &row.button,
                        ControlEvents::TOUCH_UP_INSIDE,
                        {
                            let action = action.clone();
                            move |_| action()
                        },
                    )));
                    push_row(
                        row.view,
                        PanelRowKind::Activatable {
                            button: row.button,
                            command: command.clone(),
                            leading_symbol: command.symbol.clone(),
                            checkmark: row.checkmark,
                            chevron: row.chevron,
                            trailing_reserve: row.trailing_reserve,
                            check_gutter,
                            action,
                        },
                    );
                }
            }
        }

        // Each row stretches to the container's width and re-measures its
        // height under it, so a title or subtitle that wraps when the
        // panel is narrower than the natural width still draws whole.
        let placements: Vec<(Retained<UIView>, Option<Retained<UIButton>>)> = rows
            .iter()
            .map(|row| {
                let button = match &row.kind {
                    PanelRowKind::Activatable { button, .. } => Some(button.clone()),
                    PanelRowKind::Separator { .. } => None,
                };
                (row.view.clone(), button)
            })
            .collect();
        container.set_layout_handler(move |view| {
            let width = view.bounds().size.width;
            let mut y = 0.0;
            for (row, button) in &placements {
                let height = button
                    .as_ref()
                    .map_or(SEPARATOR_HEIGHT, |button| row_height_at(button, width));
                row.setFrame(CGRect::new(
                    CGPoint::new(0.0, y),
                    CGSize::new(width, height),
                ));
                y += height;
            }
        });
        PanelPage {
            container,
            width,
            rows,
            _keepalive: keepalive,
        }
    }

    /// Pushes a submenu page with a leading back row.
    fn push_page(&self, title: &str, children: &[MenuTreeNode]) {
        let page = self.build_page(Some(title), children);
        let old = self
            .ivars()
            .pages
            .borrow()
            .last()
            .map(|page| page.container.clone());
        if let Some(column) = self.ivars().column.borrow().as_ref() {
            column.add_subview(&page.container);
        }
        self.ivars().pages.borrow_mut().push(page);
        self.swap_page(old);
        self.set_focus(None);
    }

    /// Pops back to the parent page; a no-op at the root. The parent page's
    /// container left the hierarchy when the child pushed, so it is added
    /// back before the old page crossfades away.
    fn pop_page(&self) {
        {
            let mut pages = self.ivars().pages.borrow_mut();
            if pages.len() <= 1 {
                return;
            }
            let old = pages.pop().map(|page| page.container);
            if let (Some(column), Some(top)) = (self.ivars().column.borrow().as_ref(), pages.last())
            {
                column.add_subview(&top.container);
            }
            drop(pages);
            self.swap_page(old);
        }
        self.set_focus(None);
    }

    /// Removes the page that was on top, crossfading to the new top.
    fn swap_page(&self, old: Option<Retained<HostView>>) {
        let Some(column) = self.ivars().column.borrow().clone() else {
            return;
        };
        let animations = RcBlock::new(move || {
            if let Some(old) = &old {
                old.removeFromSuperview();
            }
        });
        UIView::transitionWithView_duration_options_animations_completion(
            &column,
            PAGE_TRANSITION_SECONDS,
            UIViewAnimationOptions::TransitionCrossDissolve,
            Some(&*animations),
            None,
        );
        column.set_needs_layout();
    }

    /// Closes the panel the way a picked command or Escape does.
    /// `presentationControllerDidDismiss` does not run for a programmatic
    /// dismissal, so the completion carries the same once-only teardown.
    fn dismiss_menu(&self) {
        let weak = Weak::new(self);
        let completion = RcBlock::new(move || {
            if let Some(this) = weak.load() {
                this.run_on_dismiss();
            }
        });
        self.dismissViewControllerAnimated_completion(true, Some(&*completion));
    }

    /// Runs the owner's teardown once, whichever path dismissed the panel.
    /// The callback is taken before it runs — it may borrow the panel's
    /// state itself and must not find this borrow held.
    fn run_on_dismiss(&self) {
        let on_dismiss = self.ivars().on_dismiss.borrow_mut().take();
        if let Some(on_dismiss) = on_dismiss {
            on_dismiss();
        }
    }
}

/// The key commands the panel answers while it is first responder.
fn key_commands(mtm: MainThreadMarker) -> Retained<NSArray<UIKeyCommand>> {
    let return_key = NSString::from_str("\r");
    let commands = [
        // SAFETY: each call binds a `MenuPanelController` selector to a
        // `UIKit` input constant on the main thread.
        unsafe {
            UIKeyCommand::keyCommandWithInput_modifierFlags_action(
                UIKeyInputUpArrow,
                UIKeyModifierFlags(0),
                sel!(cocoaUiMenuUp:),
                mtm,
            )
        },
        // SAFETY: see above.
        unsafe {
            UIKeyCommand::keyCommandWithInput_modifierFlags_action(
                UIKeyInputDownArrow,
                UIKeyModifierFlags(0),
                sel!(cocoaUiMenuDown:),
                mtm,
            )
        },
        // SAFETY: see above.
        unsafe {
            UIKeyCommand::keyCommandWithInput_modifierFlags_action(
                UIKeyInputLeftArrow,
                UIKeyModifierFlags(0),
                sel!(cocoaUiMenuBack:),
                mtm,
            )
        },
        // SAFETY: see above.
        unsafe {
            UIKeyCommand::keyCommandWithInput_modifierFlags_action(
                UIKeyInputRightArrow,
                UIKeyModifierFlags(0),
                sel!(cocoaUiMenuActivate:),
                mtm,
            )
        },
        // SAFETY: see above.
        unsafe {
            UIKeyCommand::keyCommandWithInput_modifierFlags_action(
                &return_key,
                UIKeyModifierFlags(0),
                sel!(cocoaUiMenuActivate:),
                mtm,
            )
        },
        // SAFETY: see above.
        unsafe {
            UIKeyCommand::keyCommandWithInput_modifierFlags_action(
                UIKeyInputEscape,
                UIKeyModifierFlags(0),
                sel!(cocoaUiMenuDismiss:),
                mtm,
            )
        },
    ];
    NSArray::from_retained_slice(&commands)
}

/// A row's height under `width`: the control's own fitting measure —
/// required horizontally, free vertically — never below the touch
/// target.
fn row_height_at(button: &UIButton, width: f64) -> f64 {
    button
        .systemLayoutSizeFittingSize_withHorizontalFittingPriority_verticalFittingPriority(
            CGSize::new(width, 0.0),
            UILayoutPriorityRequired,
            UILayoutPriorityFittingSizeLevel,
        )
        .height
        .max(ROW_MIN_HEIGHT)
}

/// A row's height under `width` — a separator's fixed band or the
/// button's width-aware measure.
fn row_height_for(row: &PanelRow, width: f64) -> f64 {
    match &row.kind {
        PanelRowKind::Separator { .. } => SEPARATOR_HEIGHT,
        PanelRowKind::Activatable { button, .. } => row_height_at(button, width),
    }
}

/// A mounted part's size under `proposal`. A `HostView` answers through
/// its installed measure handler — the same query `sizeThatFits` and
/// `intrinsicContentSize` forward, so a `None` axis reaches the leaf
/// truly unbounded; any other view answers `intrinsicContentSize`.
fn measure_view(view: &UIView, proposal: MeasureProposal) -> CGSize {
    if let Some(host) = AnyObject::downcast_ref::<HostView>(view)
        && let Some(size) = host.measure(proposal)
    {
        return size.into();
    }
    let size = view.intrinsicContentSize();
    let bounds = view.bounds().size;
    CGSize::new(
        if size.width >= 0.0 { size.width } else { bounds.width },
        if size.height >= 0.0 { size.height } else { bounds.height },
    )
}

/// A mounted part's height under `width` — the leaf's measure with the
/// width bound and the height unbounded.
fn measure_height_at(view: &UIView, width: f64) -> f64 {
    measure_view(view, MeasureProposal::width(width)).height
}

/// A view's natural size — the leaf's measure unbounded on both axes.
fn fitting_size(view: &UIView) -> CGSize {
    measure_view(view, MeasureProposal::UNBOUNDED)
}

/// The separator color the panel draws with — the palette's `Border`
/// token.
fn separator_color(panel: &MenuPanelController) -> Retained<UIColor> {
    panel.ivars().palette.borrow().separator.clone()
}

/// The pieces [`row_button`] assembles for [`build_page`].
struct BuiltRow {
    view: Retained<UIView>,
    button: Retained<UIButton>,
    checkmark: Option<Retained<UIImageView>>,
    chevron: Option<Retained<UIImageView>>,
    trailing_reserve: f64,
}

/// A system-symbol image view for a row's own chrome — checkmark or
/// disclosure chevron — tinted like the row's text.
fn symbol_image_view(
    mtm: MainThreadMarker,
    name: &str,
    tint: &UIColor,
) -> Option<Retained<UIImageView>> {
    let image = UIImage::systemImageNamed(&NSString::from_str(name))?;
    let view = UIImageView::new(mtm);
    // SAFETY: `view` is a live image view on the main thread.
    unsafe {
        view.setImage(Some(&image));
        view.setTintColor(Some(tint));
    }
    Some(view)
}

/// A command row: a container holding a `UIButton` for the title,
/// subtitle and leading symbol, plus the row's own chrome — a leading
/// checkmark when `selected` and a trailing disclosure chevron for a
/// submenu — so symbol, selection and disclosure stay independent of one
/// another. Selection and enabled state surface on the control and in
/// the accessibility traits.
fn row_button(
    mtm: MainThreadMarker,
    command: &Command,
    leading_symbol: Option<&str>,
    disclosure: bool,
    check_gutter: bool,
    palette: &PanelPalette,
) -> BuiltRow {
    let row = HostView::new(mtm, crate::geometry::Rect::ZERO);
    let button = UIButton::new(mtm);
    // The disclosure glyph is measured before the button is configured so
    // the content insets can reserve its real extent — title text then
    // never runs under the chevron, however long it is.
    let chevron = if disclosure {
        symbol_image_view(mtm, "chevron.right", &palette.muted)
    } else {
        None
    };
    let trailing_reserve = chevron.as_ref().map_or(0.0, |chevron| {
        ROW_CHEVRON_INSET + chevron.intrinsicContentSize().width + CHEVRON_GAP
    });
    button.setConfiguration(Some(&row_configuration(
        mtm,
        command,
        leading_symbol,
        trailing_reserve,
        check_gutter,
        false,
        palette,
    )));
    button.setSelected(command.selected);
    // SAFETY: the trait constants are `extern static` reads on the main
    // thread.
    unsafe {
        let mut traits = button.accessibilityTraits(mtm);
        if command.selected {
            traits |= UIAccessibilityTraitSelected;
        }
        if !command.enabled {
            traits |= UIAccessibilityTraitNotEnabled;
        }
        button.setAccessibilityTraits(traits, mtm);
    }
    row.add_subview(&button);
    let checkmark = if command.selected {
        symbol_image_view(
            mtm,
            "checkmark",
            if command.destructive {
                &palette.destructive
            } else {
                &palette.label
            },
        )
    } else {
        None
    };
    if let Some(checkmark) = &checkmark {
        row.add_subview(checkmark);
    }
    if let Some(chevron) = &chevron {
        row.add_subview(chevron);
    }
    let button_ret = button.clone();
    let checkmark_ret = checkmark.clone();
    let chevron_ret = chevron.clone();
    row.set_layout_handler(move |view| {
        let size = view.bounds().size;
        button.setFrame(CGRect::new(CGPoint::ZERO, size));
        if let Some(checkmark) = &checkmark {
            let glyph = checkmark.intrinsicContentSize();
            checkmark.setFrame(CGRect::new(
                CGPoint::new(ROW_CHECK_INSET, (size.height - glyph.height) / 2.0),
                glyph,
            ));
        }
        if let Some(chevron) = &chevron {
            let glyph = chevron.intrinsicContentSize();
            chevron.setFrame(CGRect::new(
                CGPoint::new(
                    size.width - ROW_CHEVRON_INSET - glyph.width,
                    (size.height - glyph.height) / 2.0,
                ),
                glyph,
            ));
        }
    });
    BuiltRow {
        view: row.into_super(),
        button: button_ret,
        checkmark: checkmark_ret,
        chevron: chevron_ret,
        trailing_reserve,
    }
}

/// The `UIButtonConfiguration` a row shows — title and subtitle leading,
/// destructive red, its leading symbol, and the keyboard-focus fill. The
/// selected gutter and disclosure chrome live on the row itself.
fn row_configuration(
    mtm: MainThreadMarker,
    command: &Command,
    leading_symbol: Option<&str>,
    trailing_reserve: f64,
    check_gutter: bool,
    focused: bool,
    palette: &PanelPalette,
) -> Retained<UIButtonConfiguration> {
    let config = UIButtonConfiguration::plainButtonConfiguration(mtm);
    config.setTitle(Some(&NSString::from_str(&command.label)));
    if let Some(subtitle) = &command.subtitle {
        config.setSubtitle(Some(&NSString::from_str(subtitle)));
    }
    config.setTitleAlignment(UIButtonConfigurationTitleAlignment::Leading);
    config.setContentInsets(NSDirectionalEdgeInsets {
        top: 10.0,
        leading: 16.0 + if check_gutter { CHECK_GUTTER } else { 0.0 },
        bottom: 10.0,
        trailing: 16.0 + trailing_reserve,
    });
    if command.destructive {
        config.setBaseForegroundColor(Some(&palette.destructive));
    } else {
        config.setBaseForegroundColor(Some(&palette.label));
    }
    if let Some(name) = leading_symbol
        && let Some(image) = UIImage::systemImageNamed(&NSString::from_str(name))
    {
        config.setImage(Some(&image));
        config.setImagePlacement(NSDirectionalRectEdge::Leading);
        config.setImagePadding(8.0);
    }
    if focused {
        config.setBaseBackgroundColor(Some(&palette.focus_fill));
        config.background().setCornerRadius(FOCUS_CORNER_RADIUS);
    }
    config
}

/// The ivars of a [`MenuPanelDelegate`]: the controller it reports for.
#[derive(Default)]
pub struct MenuPanelDelegateIvars {
    panel: RefCell<Option<Weak<MenuPanelController>>>,
}

impl fmt::Debug for MenuPanelDelegateIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuPanelDelegateIvars").finish()
    }
}

define_class!(
    // SAFETY: `NSObject` asks a subclass to initialize through `init`,
    // which `MenuPanelDelegate::new` does, and the class does not implement
    // `Drop`.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[name = "CocoaUiMenuPanelDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuPanelDelegateIvars]
    #[derive(Debug)]
    /// The `UIPopoverPresentationControllerDelegate` of a presented panel:
    /// keeps the popover floating on compact widths and reports dismissal.
    struct MenuPanelDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for MenuPanelDelegate {}

    // SAFETY: the adaptive-delegate methods `UIKit` calls through the
    // popover's delegate are implemented with their declared signatures.
    unsafe impl UIAdaptivePresentationControllerDelegate for MenuPanelDelegate {
        // SAFETY: see the module safety note.
        #[unsafe(method(adaptivePresentationStyleForPresentationController:))]
        fn adaptive_presentation_style(
            &self,
            _controller: &UIPresentationController,
        ) -> UIModalPresentationStyle {
            // The contract a context menu needs: a floating panel on every
            // width, not the fullscreen sheet `.popover` adapts to on a
            // compact phone.
            UIModalPresentationStyle::None
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(presentationControllerDidDismiss:))]
        fn presentation_did_dismiss(&self, _controller: &UIPresentationController) {
            guarded("MenuPanelDelegate presentationControllerDidDismiss", || {
                if let Some(panel) = self.ivars().panel.borrow().as_ref().and_then(Weak::load) {
                    panel.run_on_dismiss();
                }
            });
        }
    }

    // SAFETY: the popover delegate protocol's own methods are all
    // optional; the adaptive answers it inherits are implemented above.
    unsafe impl UIPopoverPresentationControllerDelegate for MenuPanelDelegate {}
);

impl MenuPanelDelegate {
    fn new(mtm: MainThreadMarker, panel: &Retained<MenuPanelController>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuPanelDelegateIvars {
            panel: RefCell::new(Some(Weak::new(&**panel))),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// An assembled popover panel: the controller `UIKit` presents.
///
/// `UIKit` holds the popover's delegate weakly, so this value owns it for
/// the presentation. A clone shares the same presented panel — the owner
/// keeps one for borrowing-free dismissal.
#[derive(Debug, Clone)]
pub struct ContextMenuPopover {
    controller: Retained<MenuPanelController>,
    delegate: Retained<MenuPanelDelegate>,
}

impl ContextMenuPopover {
    /// A panel with its geometry owner — scroll view and column — built
    /// and its chrome colors resolved from the owner's theme.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, palette: &PanelPalette) -> Self {
        let controller = MenuPanelController::new(mtm, palette.clone());
        let delegate = MenuPanelDelegate::new(mtm, &controller);
        let this = Self {
            controller,
            delegate,
        };
        this.apply_palette(palette);
        this
    }

    /// Replaces the palette the rows and chrome draw with — a theme token
    /// changed while the panel is open — repainting every row, checkmark,
    /// chevron and hairline in place and re-measuring the column.
    pub fn apply_palette(&self, palette: &PanelPalette) {
        let ivars = self.controller.ivars();
        ivars.palette.replace(palette.clone());
        if let Some(scroll) = ivars.scroll.borrow().as_ref() {
            scroll.setBackgroundColor(Some(&palette.surface));
        }
        for page in ivars.pages.borrow().iter() {
            for row in &page.rows {
                match &row.kind {
                    PanelRowKind::Separator { hairline } => {
                        hairline.setBackgroundColor(Some(&palette.separator));
                    }
                    PanelRowKind::Activatable {
                        checkmark,
                        chevron,
                        command,
                        ..
                    } => {
                        if let Some(checkmark) = checkmark {
                            let tint: &UIColor = if command.destructive {
                                &palette.destructive
                            } else {
                                &palette.label
                            };
                            // SAFETY: `checkmark` is a live image view on
                            // the main thread.
                            unsafe {
                                checkmark.setTintColor(Some(tint));
                            }
                        }
                        if let Some(chevron) = chevron {
                            // SAFETY: `chevron` is a live image view on
                            // the main thread.
                            unsafe {
                                chevron.setTintColor(Some(&palette.muted));
                            }
                        }
                    }
                }
            }
        }
        // `set_focus` re-renders each row's configuration with the new
        // palette, preserving which row held the ring.
        self.controller.set_focus(ivars.focused.get());
        if let Some(column) = ivars.column.borrow().as_ref() {
            column.set_needs_layout();
        }
    }

    /// The view a content leaf mounts into: the column the panel stacks.
    /// Mounting a leaf here installs its intrinsic measure, which is how
    /// the panel re-measures it every layout pass.
    #[must_use]
    pub fn mount_target(&self) -> Retained<UIView> {
        self.controller.column_view()
    }

    /// Registers a mounted leaf's view in `slot` so the layout pass places
    /// it above the command rows.
    pub fn set_slot(&self, slot: PanelSlot, view: &UIView) {
        let ivars = self.controller.ivars();
        match slot {
            PanelSlot::Preview => ivars.preview.replace(Some(Retained::from(view))),
            PanelSlot::Accessory => ivars.accessory.replace(Some(Retained::from(view))),
        };
        if let Some(column) = ivars.column.borrow().as_ref() {
            column.set_needs_layout();
        }
    }

    /// Builds the command rows — buttons for commands, hairlines for
    /// dividers, chevron rows that push submenu pages — from `nodes`.
    ///
    /// May only be called once per panel: the nodes travel with the
    /// presentation.
    pub fn set_commands(&self, nodes: &[MenuTreeNode]) {
        let page = self.controller.build_page(None, nodes);
        if let Some(column) = self.controller.ivars().column.borrow().as_ref() {
            column.add_subview(&page.container);
        }
        self.controller.ivars().pages.borrow_mut().push(page);
    }

    /// Presents the panel as a popover anchored to `host`'s bounds. The
    /// adaptive style is pinned to `.none` so the panel floats on compact
    /// phones; an outside tap, a picked command, `dismiss` or Escape all
    /// end the presentation and run `on_dismiss` once.
    ///
    /// Returns `false` when `host` is outside a window or has no
    /// presenting controller — the menu simply does not open.
    pub fn present(&self, host: &UIView, on_dismiss: Rc<dyn Fn()>) -> bool {
        let Some(window) = host.window() else {
            return false;
        };
        let Some(root) = window.rootViewController() else {
            return false;
        };
        let mut presenter = root;
        while let Some(next) = presenter.presentedViewController() {
            presenter = next;
        }
        let bounds = window.bounds();
        let ivars = self.controller.ivars();
        ivars
            .max_height
            .set(bounds.size.height * MAX_HEIGHT_FRACTION);
        ivars.max_width.set(bounds.size.width * MAX_WIDTH_FRACTION);
        ivars.on_dismiss.replace(Some(on_dismiss));
        self.controller
            .setModalPresentationStyle(UIModalPresentationStyle::Popover);
        let Some(popover) = self.controller.popoverPresentationController() else {
            return false;
        };
        popover.setSourceView(Some(host));
        popover.setSourceRect(host.bounds());
        popover.setPermittedArrowDirections(UIPopoverArrowDirection::Any);
        // SAFETY: `delegate` answers both the popover and adaptive
        // protocols; `self` retains it since the property is weak.
        unsafe {
            popover.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate)));
        }
        let natural = self.controller.natural_size();
        ivars.reported_size.set(natural);
        self.controller.setPreferredContentSize(natural);
        presenter.presentViewController_animated_completion(&self.controller, true, None);
        true
    }

    /// Dismisses the panel programmatically — a `DismissContextMenu`
    /// request lands here; the teardown runs once through whichever path
    /// lands first: this dismissal's completion, the adaptive delegate on
    /// an outside tap, or `viewDidDisappear` when the presenter ends the
    /// presentation itself.
    pub fn dismiss(&self) {
        self.controller.dismiss_menu();
    }

    /// The current page stack depth, rows and focus for the `native` test
    /// suite — `(depth, [(label, activatable)], focused)` of the visible
    /// page. Exists only under `native-test`; the panel keeps no public
    /// introspection API.
    #[cfg(feature = "native-test")]
    #[must_use]
    pub fn page_probe_for_test(&self) -> (usize, Vec<(String, bool)>, Option<usize>) {
        let ivars = self.controller.ivars();
        let pages = ivars.pages.borrow();
        let Some(page) = pages.last() else {
            return (0, Vec::new(), None);
        };
        let rows = page
            .rows
            .iter()
            .map(|row| match &row.kind {
                PanelRowKind::Separator { .. } => (String::new(), false),
                PanelRowKind::Activatable { command, .. } => (command.label.clone(), true),
            })
            .collect();
        (pages.len(), rows, ivars.focused.get())
    }

    /// Feeds the test suite's key press through the same paths the
    /// `UIKeyCommand`s invoke.
    #[cfg(feature = "native-test")]
    pub fn press_key_for_test(&self, input: &str) {
        match input {
            "up" => self.controller.move_focus(-1),
            "down" => self.controller.move_focus(1),
            "left" => self.controller.pop_page(),
            "right" | "return" => self.controller.activate_focused(),
            "escape" => self.controller.dismiss_menu(),
            _ => {}
        }
    }
}
