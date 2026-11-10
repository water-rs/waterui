//! The `menu` leaf: `Native<ResolvedMenu>` rendered as a pull-down menu
//! trigger — an `NSPopUpButton` on `AppKit`, a `UIButton` presenting a
//! `UIMenu` on `UIKit` — with the resolved label drawn inside the button.
//!
//! The trigger is drawn in the menu's resolved `ButtonStyle` and
//! `ControlSize` exactly as a `Button` of that style and size is
//! ([`button_chrome`]): the bezel and control size on `AppKit`, the chrome
//! on `UIKit`, the accent tint, and the label's `Foreground` and `Body`.
//! The label is an arbitrary child view framed inside the trigger — under
//! the chrome's content insets on `UIKit`, and on `AppKit` 12pt leading +
//! 36pt chevron + 4pt vertical inside a bezel and nothing around a
//! transparent trigger — measured under the offer minus that padding.
//! `accessibility_label` speaks on the trigger.
//!
//! The items collection is a `Computed<Vec<ResolvedMenuItem>>`: the whole
//! menu rebuilds on every change — the collection itself, or any item's
//! label / `disabled` / `selected` signal — matching `WuiMenuTree`'s
//! rebuild-everything semantics. Rebuilds run inside the platform
//! animation the watcher's metadata carries.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use cocoa_ui::{PlatformView, Rect, view};
use waterui::component::button::ButtonStyle;
use waterui::component::menu::{ResolvedMenu, ResolvedMenuItem};
use waterui::reactive::Signal;
use waterui::reactive::watcher::{BoxWatcherGuard, Metadata};
use waterui_backend_core::Environment;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};

use crate::components::button_chrome;
use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal::{self, Placement};

#[cfg(target_os = "macos")]
use super::menu_items::append_items;
#[cfg(target_os = "ios")]
use super::menu_items::build_menu;
use super::menu_items::{collect_item_watchers, item_title};

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{HitTest, HostView, Menu, MenuButton, MenuItem};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::{HitTest, HostView, MenuButton};
}

use platform::{HostView, MenuButton};

/// The padding the trigger keeps around its label, in points: `leading`
/// and `trailing` along the reading direction, `vertical` above and below.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LabelPadding {
    leading: f32,
    trailing: f32,
    vertical: f32,
}

impl LabelPadding {
    /// A trigger whose label fills it edge to edge.
    #[cfg(target_os = "macos")]
    const NONE: Self = Self {
        leading: 0.0,
        trailing: 0.0,
        vertical: 0.0,
    };

    /// Both horizontal insets together.
    fn horizontal(self) -> f32 {
        self.leading + self.trailing
    }

    /// The offer the embedded label measures under: the trigger's proposal
    /// minus the padding on each axis, never negative — `labelOffer`.
    fn label_offer(self, proposal: ProposalSize) -> ProposalSize {
        ProposalSize {
            width: proposal
                .width
                .map(|width| (width - self.horizontal()).max(0.0)),
            height: proposal
                .height
                .map(|height| self.vertical.mul_add(-2.0, height).max(0.0)),
        }
    }

    /// The label's frame inside a `width` × `height` trigger, in the
    /// trigger's own space: the leading inset sits on the right when
    /// `right_to_left`.
    fn label_frame(self, width: f64, height: f64, right_to_left: bool) -> Rect {
        let leading = if right_to_left {
            self.trailing
        } else {
            self.leading
        };
        let vertical = f64::from(self.vertical);
        Rect::new(
            f64::from(leading),
            vertical,
            (width - f64::from(self.horizontal())).max(0.0),
            vertical.mul_add(-2.0, height).max(0.0),
        )
    }
}

/// The padding a pull-down trigger in `style` keeps around its label: a
/// bezel keeps the 12pt leading text inset, the 36pt trailing chevron
/// column and 4pt above and below; a transparent trigger, which draws
/// neither bezel nor chevron, keeps nothing — as a `Button` of that style
/// pads.
#[cfg(target_os = "macos")]
const fn label_padding(style: ButtonStyle) -> LabelPadding {
    if button_chrome::draws_chrome(style) {
        LabelPadding {
            leading: 12.0,
            trailing: 36.0,
            vertical: 4.0,
        }
    } else {
        LabelPadding::NONE
    }
}

/// The padding a trigger in `style` keeps around its label: the chrome's
/// own content insets, as a `Button` of that style pads.
#[cfg(target_os = "ios")]
fn label_padding(style: ButtonStyle, mtm: cocoa_ui::MainThreadMarker) -> LabelPadding {
    let (horizontal, vertical) = button_chrome::chrome_padding(style, mtm);
    LabelPadding {
        leading: horizontal,
        trailing: horizontal,
        vertical,
    }
}

/// The leaf's live state: the trigger, the latest resolved items, the
/// watchers keeping them observed, and the environment commands run
/// against.
struct MenuState {
    /// The pull-down trigger.
    button: MenuButton,
    /// The host view, for layout invalidation after a rebuild.
    host: cocoa_ui::Retained<HostView>,
    /// The environment item actions run against — `env.inner` in Swift.
    env: Environment,
    /// The resolved items the live menu was built from.
    items: Vec<ResolvedMenuItem>,
    /// Watchers on every item signal; replaced wholesale on rebuild.
    item_watchers: Vec<BoxWatcherGuard>,
    /// The placement a Rust parent last gave this leaf — its proposal is
    /// `selectedProposal`.
    placement: Cell<Placement>,
    /// The scope the live menu's key commands are armed in, replaced with
    /// the menu on every rebuild.
    #[cfg(target_os = "ios")]
    key_commands: Option<cocoa_ui::uikit::KeyCommands>,
    /// Main-thread proof, captured for menu rebuilds inside watchers.
    mtm: cocoa_ui::MainThreadMarker,
}

impl core::fmt::Debug for MenuState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MenuState").finish_non_exhaustive()
    }
}

/// Re-snapshots `items` onto the state, installs a fresh watcher per item
/// signal, and rebuilds the platform menu.
fn apply_items(state: &Rc<RefCell<MenuState>>, items: Vec<ResolvedMenuItem>) {
    let resync: Rc<dyn Fn(&Metadata)> = {
        // Weak: `item_watchers` lives inside the state — a strong capture
        // here would keep `MenuState` alive forever.
        let state = Rc::downgrade(state);
        Rc::new(move |metadata| {
            let Some(state) = state.upgrade() else {
                return;
            };
            crate::animation::with_platform_animation(metadata, move || {
                let items = state.borrow().items.clone();
                apply_items(&state, items);
            });
        })
    };
    let mut watchers = Vec::new();
    collect_item_watchers(&items, &resync, &mut watchers);
    {
        let mut state = state.borrow_mut();
        state.items = items;
        state.item_watchers = watchers;
    }
    rebuild_menu(state);
    // `invalidateCapturedRendering`: a rebuilt menu may measure
    // differently.
    view::invalidate_layout(&state.borrow().host);
    crate::measure_memo::invalidate();
}

/// Rebuilds the trigger's menu from the live items.
#[cfg(target_os = "macos")]
fn rebuild_menu(state: &Rc<RefCell<MenuState>>) {
    let (button, env, items, mtm) = {
        let state = state.borrow();
        (
            state.button.clone(),
            state.env.clone(),
            state.items.clone(),
            state.mtm,
        )
    };
    // Pull-down menus keep a blank first item the button draws as its
    // face title; the real label is the mounted child.
    let menu = platform::Menu::new(mtm, "");
    menu.add_item(platform::MenuItem::new(mtm, "", None, ""));
    append_items(mtm, &menu, &items, &env);
    button.set_menu(&menu);
}

/// Rebuilds the trigger's menu from the live items.
#[cfg(target_os = "ios")]
fn rebuild_menu(state: &Rc<RefCell<MenuState>>) {
    let (button, env, items, mtm) = {
        let state = state.borrow();
        (
            state.button.clone(),
            state.env.clone(),
            state.items.clone(),
            state.mtm,
        )
    };
    let key_commands = button.menu_commands();
    button.set_menu(&build_menu(mtm, &key_commands, "", None, &items, &env));
    // `setMenu:` has replaced the rows of a menu that is open, so the
    // previous menu's commands are unreachable and retire with its scope.
    state.borrow_mut().key_commands = Some(key_commands);
}

/// Ends the trigger's open menu when the leaf's state goes, ahead of the
/// state's `KeyCommands`: under Catalyst the menu is an `NSMenu` popup that
/// a leaf retiring behind a still-mounted ghost view would otherwise leave
/// offering the retired commands.
#[cfg(all(target_os = "ios", target_abi = "macabi"))]
struct EndOpenMenu(MenuButton);

#[cfg(all(target_os = "ios", target_abi = "macabi"))]
impl Drop for EndOpenMenu {
    fn drop(&mut self) {
        self.0.end_open_menu();
    }
}

/// The container's layout face: the label's measurement plus the label
/// padding — `sizeThatFits`.
struct MenuSubView {
    /// The mounted label child.
    label: Mounted,
    /// The padding the trigger keeps around the label.
    padding: LabelPadding,
}

impl core::fmt::Debug for MenuSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MenuSubView").finish_non_exhaustive()
    }
}

impl SubView for MenuSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let measured = self
            .label
            .layout()
            .measure(self.padding.label_offer(proposal));
        ViewDimensions::new(waterui_core::layout::Size::new(
            measured.size.width + self.padding.horizontal(),
            self.padding.vertical.mul_add(2.0, measured.size.height),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `menu` handler on the dispatcher: `Native<ResolvedMenu>`
/// maps to a host view holding the pull-down trigger and its label child.
#[expect(
    clippy::too_many_lines,
    reason = "the handler's wiring is sequential; splitting it would obscure the mount order"
)]
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ResolvedMenu>(|config, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let button = MenuButton::new(mtm);
        let label_container = HostView::new(mtm, Rect::ZERO);
        // The label never intercepts input meant for the trigger.
        label_container.set_hit_test_handler(|_, _| platform::HitTest::Pass);

        let host_view: &PlatformView = &host;
        view::add_subview(host_view, button.view());
        view::add_subview(button.view(), &label_container);

        // The trigger and its label take the menu's style and size the way
        // a `Button` of that style and size does.
        let style = config.style;
        #[cfg(target_os = "macos")]
        let padding = {
            button_chrome::apply_bezel(
                &button.button(),
                style,
                button_chrome::control_size(config.size),
            );
            label_padding(style)
        };
        #[cfg(target_os = "ios")]
        let padding = label_padding(style, mtm);
        let label_env = button_chrome::label_environment(style, config.size, ctx.env());

        let label_leaf = ctx.with_env(&label_env).render(config.label);
        let child_view = view::retain_base(label_leaf.view());
        let mounted = label_leaf.mount(&label_container);
        {
            let child_view = child_view.clone();
            label_container.set_layout_handler(move |host| {
                view::set_frame(&child_view, view::bounds(host));
            });
        }

        let state = Rc::new(RefCell::new(MenuState {
            button,
            host: host.clone(),
            env: ctx.env().clone(),
            items: Vec::new(),
            item_watchers: Vec::new(),
            placement: Cell::new(Placement::UNPLACED),
            #[cfg(target_os = "ios")]
            key_commands: None,
            mtm,
        }));

        // `layoutSubviews` / `layout`: the trigger fills the host's
        // response, the label sits inside it under the platform's padding,
        // and the selected proposal — or the bounds — reaches the label as
        // its offer.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                // The trigger draws in its response rectangle, centred in
                // the frame it was assigned (docs/layout-spec.md §2.4).
                let bounds = proposal::response(host);
                let state = state.borrow();
                view::set_frame(state.button.view(), bounds);
                let width = bounds.size.width;
                let height = bounds.size.height;
                view::set_frame(
                    &label_container,
                    padding.label_frame(width, height, view::is_right_to_left(host)),
                );

                let placement = state.placement.get();
                let base = placement.proposal.unwrap_or({
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "the layout contract is f32; host bounds always fit"
                    )]
                    ProposalSize {
                        width: Some(width as f32),
                        height: Some(height as f32),
                    }
                });
                // The label sits at its layout frame inside the host, so it
                // aligns under the host's own offset.
                proposal::deliver(
                    &child_view,
                    Placement {
                        proposal: Some(padding.label_offer(base)),
                        offset: placement.offset,
                        ..Placement::UNPLACED
                    },
                );
            }
        });

        #[cfg(all(target_os = "ios", target_abi = "macabi"))]
        let button_for_teardown = state.borrow().button.clone();
        let mut leaf = NativeLeaf::new(
            host_view,
            MenuSubView {
                label: mounted,
                padding,
            },
        );
        leaf.keep(label_env);

        // Items → menu: rebuild on every change (`watch` so the rebuild
        // carries the watcher's animation metadata), with the initial list
        // applied up front.
        apply_items(&state, config.items.snapshot());
        leaf.watch(&config.items, {
            let state = Rc::clone(&state);
            move |wctx| {
                let items = wctx.value().clone();
                let state = Rc::clone(&state);
                crate::animation::with_platform_animation(wctx.metadata(), move || {
                    apply_items(&state, items);
                });
            }
        });

        // The proposal a Rust parent selected invalidates placement even
        // when the frame does not move.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            let host = host.clone();
            move |placement| {
                let state = state.borrow();
                if state.placement.get() != placement {
                    state.placement.set(placement);
                    host.set_needs_layout();
                    crate::measure_memo::invalidate();
                }
            }
        });
        leaf.keep(sink_guard);

        // `accessibility_label` speaks on the trigger — and carries its
        // `AppKit` tooltip.
        let trigger = view::retain_base(state.borrow().button.view());
        leaf.bind(&config.accessibility_label, move |styled| {
            view::set_accessibility_label(&trigger, &item_title(&styled));
        });

        // The tint colours the trigger as it colours a `Button` of its style.
        #[cfg(target_os = "macos")]
        let styled = state.borrow().button.button();
        #[cfg(target_os = "ios")]
        let styled = state.borrow().button.button().clone();
        button_chrome::bind_tint(&mut leaf, styled, style, ctx.env(), mtm);

        leaf.keep(state);
        // Kept after the state, so it drops first (reverse order).
        #[cfg(all(target_os = "ios", target_abi = "macabi"))]
        leaf.keep(EndOpenMenu(button_for_teardown));
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::text::StyledStr;

    const PADDING: LabelPadding = LabelPadding {
        leading: 12.0,
        trailing: 36.0,
        vertical: 4.0,
    };

    #[test]
    fn label_offer_subtracts_padding_and_clamps() {
        let offer = PADDING.label_offer(ProposalSize {
            width: Some(120.0),
            height: Some(40.0),
        });
        assert_eq!(offer.width, Some(72.0));
        assert_eq!(offer.height, Some(32.0));

        // Padding larger than the proposal clamps to zero rather than
        // going negative, matching the Swift `max(0, ...)`.
        let clamped = PADDING.label_offer(ProposalSize {
            width: Some(0.0),
            height: Some(0.0),
        });
        assert_eq!(clamped.width, Some(0.0));
        assert_eq!(clamped.height, Some(0.0));
    }

    #[test]
    fn the_label_frame_mirrors_its_leading_inset_right_to_left() {
        assert_eq!(
            PADDING.label_frame(120.0, 40.0, false),
            Rect::new(12.0, 4.0, 72.0, 32.0)
        );
        assert_eq!(
            PADDING.label_frame(120.0, 40.0, true),
            Rect::new(36.0, 4.0, 72.0, 32.0)
        );
    }

    /// A pull-down trigger pads its label as a `Button` of the menu's
    /// style does: inside a bezel the label clears the text inset and the
    /// chevron, and a transparent trigger, drawing neither, pads nothing.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_pull_down_pads_like_a_button_of_its_style() {
        for style in [
            ButtonStyle::Automatic,
            ButtonStyle::Bordered,
            ButtonStyle::BorderedProminent,
            ButtonStyle::Glass,
            ButtonStyle::GlassProminent,
        ] {
            assert_eq!(label_padding(style), PADDING, "{style:?} draws a bezel");
        }
        for style in [
            ButtonStyle::Plain,
            ButtonStyle::Link,
            ButtonStyle::Borderless,
        ] {
            assert_eq!(
                label_padding(style),
                LabelPadding::NONE,
                "{style:?} is transparent"
            );
        }
    }

    #[test]
    fn item_title_strips_bidi_marks() {
        assert_eq!(item_title(&StyledStr::from("a\u{202a}b\u{202c}")), "ab");
    }

    /// Dividers split the item list into groups; consecutive and trailing
    /// dividers produce no empty groups, matching `splitMenuGroups`.
    #[test]
    fn groups_split_on_dividers() {
        fn divider() -> ResolvedMenuItem {
            ResolvedMenuItem::Divider
        }
        let items = [divider(), divider()];
        assert!(
            items
                .split(|item| matches!(item, ResolvedMenuItem::Divider))
                .all(<[ResolvedMenuItem]>::is_empty)
        );
    }
}
