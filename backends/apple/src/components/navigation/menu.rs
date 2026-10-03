//! `Native<ResolvedMenu>` rendered as the platform's menu trigger.
//!
//! Mirrors `WuiMenu`: a slim trigger hosting the label view — a plain,
//! accent-tinted `UIButton` with `showsMenuAsPrimaryAction` on `UIKit`, a
//! pull-down `NSPopUpButton` with the label overlaid on `AppKit` — whose
//! menu rebuilds whenever the resolved items, or any command's label,
//! disabled or selected signal, change. On `UIKit` the label resolves
//! `Foreground` to `Accent`, as `makeMenuLabelEnvironment` overlays it; on
//! `AppKit` it keeps the surrounding foreground.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::RefCell;
use core::fmt;

use waterui::component::menu::{ResolvedMenu, ResolvedMenuItem};
use waterui::reactive::{Computed, Signal};
use waterui_backend_core::Environment;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

use crate::menus::menu_tree;

/// Watches every signal a resolved item collection owns and calls `rebuild`
/// on change — `WuiMenuTree`'s per-node observations (label, disabled,
/// selected). Guards live in `guards`; replacing them drops the old arm.
fn arm_items(items: &[ResolvedMenuItem], guards: &mut Vec<Box<dyn Any>>, rebuild: &Rc<dyn Fn()>) {
    let arm = |signal: &Computed<bool>| -> Box<dyn Any> {
        let rebuild = Rc::downgrade(rebuild);
        Box::new(signal.watch(move |_| {
            if let Some(rebuild) = rebuild.upgrade() {
                rebuild();
            }
        }))
    };
    for item in items {
        match item {
            ResolvedMenuItem::Divider => {}
            ResolvedMenuItem::Command(command) => {
                guards.push(arm(&command.disabled));
                guards.push(arm(&command.selected));
            }
            ResolvedMenuItem::Menu(menu) => {
                arm_items(&menu.items.snapshot(), guards, rebuild);
            }
        }
    }
}

/// The resolved items, their environment, and the call that applies a fresh
/// menu tree to the platform object. Rebuilds on the collection itself and
/// on every per-item signal, re-arming after each rebuild.
struct MenuDriver {
    items: Computed<Vec<ResolvedMenuItem>>,
    env: Environment,
    apply: Rc<dyn Fn(Vec<cocoa_ui::menu::MenuTreeNode>)>,
    guards: RefCell<Vec<Box<dyn Any>>>,
}

impl fmt::Debug for MenuDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuDriver").finish_non_exhaustive()
    }
}

impl MenuDriver {
    /// Builds the native menu from the snapshot and re-arms the item
    /// signals. Called once at install, then on every change.
    fn rebuild(self: &Rc<Self>) {
        let items = self.items.snapshot();
        (self.apply)(menu_tree(&items, &self.env));
        let mut guards = Vec::new();
        let rebuild: Rc<dyn Fn()> = {
            let driver = self.clone();
            Rc::new(move || driver.rebuild())
        };
        arm_items(&items, &mut guards, &rebuild);
        *self.guards.borrow_mut() = guards;
    }
}

/// The menu trigger's layout face: the label's measured size plus the
/// platform's trigger chrome — the pull-down face's padding on `AppKit`,
/// none on `UIKit`.
struct MenuSubView {
    label: NativeLeaf,
    horizontal_padding: f32,
    vertical_padding: f32,
}

impl fmt::Debug for MenuSubView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuSubView").finish_non_exhaustive()
    }
}

impl SubView for MenuSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let label_proposal = ProposalSize {
            width: proposal
                .width
                .map(|width| (width - self.horizontal_padding).max(0.0)),
            height: proposal
                .height
                .map(|height| (height - self.vertical_padding).max(0.0)),
        };
        let dimensions = self.label.layout().measure(label_proposal);
        ViewDimensions::new(Size::new(
            dimensions.size.width + self.horizontal_padding,
            dimensions.size.height + self.vertical_padding,
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(target_os = "macos")]
fn install_menu(menu: ResolvedMenu, ctx: &crate::contract::RenderContext<'_>) -> NativeLeaf {
    use cocoa_ui::appkit::{Menu, MenuItem, PopUpButton};

    let mtm = ctx.mtm();
    let pop_up = PopUpButton::new(mtm);

    // `AppKit`'s trigger draws the label in the label color — no tint.
    let label = ctx.render(menu.label);
    let label_view = cocoa_ui::view::retain_base(label.view());
    pop_up.set_label_view(&label_view);

    let driver = Rc::new(MenuDriver {
        apply: Rc::new({
            let pop_up = pop_up.clone();
            move |nodes| {
                let native = Menu::new(mtm, "");
                // A pull-down list takes its face title from the first item;
                // the overlaid label draws the trigger, so it stays empty.
                native.add_item(MenuItem::new(mtm, "", None, ""));
                native.set_nodes(&nodes);
                pop_up.set_menu_items(native.menu());
            }
        }),
        env: ctx.env().clone(),
        items: menu.items,
        guards: RefCell::new(Vec::new()),
    });
    driver.rebuild();
    let items = driver.items.clone();
    let mut leaf = NativeLeaf::new(
        &*pop_up.clone().into_super(),
        MenuSubView {
            label,
            horizontal_padding: 48.0,
            vertical_padding: 8.0,
        },
    );
    leaf.keep(driver.clone());
    leaf.watch(&items, move |_| driver.rebuild());
    let trigger = pop_up.into_super();
    leaf.bind(&menu.accessibility_label, move |label| {
        let text = cocoa_ui::text::strip_bidi_controls(label.to_plain().as_str());
        cocoa_ui::view::set_accessibility_label(&trigger, &text);
    });
    leaf
}

#[cfg(target_os = "ios")]
fn install_menu(menu: ResolvedMenu, ctx: &crate::contract::RenderContext<'_>) -> NativeLeaf {
    use waterui::reactive::SignalExt;
    use waterui::resolve::Resolvable;
    use waterui::theme::color::{Accent, Foreground};
    use waterui::theme::install_color_signal;

    let mtm = ctx.mtm();
    let button = cocoa_ui::uikit::Button::new(mtm);
    button.set_chrome(cocoa_ui::uikit::button::Chrome::Plain, mtm);
    button.set_shows_menu_as_primary_action(true);

    // `UIKit`'s trigger tints its label with the accent.
    let mut label_env = ctx.env().clone();
    install_color_signal::<Foreground>(&mut label_env, Accent.resolve(ctx.env()).computed());
    let label = ctx.with_env(&label_env).render(menu.label);
    let label_view = cocoa_ui::view::retain_base(label.view());
    cocoa_ui::view::add_subview(
        AsRef::<cocoa_ui::objc2_ui_kit::UIButton>::as_ref(&button),
        &label_view,
    );
    // SAFETY: an ordinary main-thread `UIKit` setter.
    label_view.setUserInteractionEnabled(false);

    let driver = Rc::new(MenuDriver {
        apply: Rc::new({
            let button = button.clone();
            move |nodes| {
                let menu = cocoa_ui::uikit::menu(mtm, &cocoa_ui::menu::Command::default(), &nodes);
                button.set_menu(Some(&menu));
            }
        }),
        env: ctx.env().clone(),
        items: menu.items,
        guards: RefCell::new(Vec::new()),
    });
    driver.rebuild();
    let items = driver.items.clone();
    let mut leaf = NativeLeaf::new(
        AsRef::<cocoa_ui::objc2_ui_kit::UIButton>::as_ref(&button),
        MenuSubView {
            label,
            horizontal_padding: 0.0,
            vertical_padding: 0.0,
        },
    );
    leaf.keep(driver.clone());
    leaf.watch(&items, move |_| driver.rebuild());
    let trigger = cocoa_ui::view::retain_base(button.as_ref());
    leaf.bind(&menu.accessibility_label, move |label| {
        let text = cocoa_ui::text::strip_bidi_controls(label.to_plain().as_str());
        cocoa_ui::view::set_accessibility_label(&trigger, &text);
    });
    leaf
}

/// Installs the `ResolvedMenu` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ResolvedMenu>(install_menu);
}
