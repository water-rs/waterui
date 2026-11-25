//! The `WuiNavigationBarState` port: a resolved [`Bar`] as renderable
//! chrome — title and subtitle rendered once with their extracted plain
//! text, semantic toolbar items partitioned by placement, the search drawer
//! config, and the bar's color and hidden signals.
//!
//! One item feeds each bar slot on `AppKit` — `leadingItem`/`trailingItem`/
//! `statusItem` — while `UIKit` lays out every leading and trailing item,
//! matching the baseline's placement maps.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use waterui::component::LabelDisplayMode;
use waterui::navigation::{
    Bar, BarColor, NavigationSearch, NavigationTitleDisplayMode, NavigationToolbarPlacement,
    ResolvedNavigationToolbar, ResolvedNavigationToolbarItem, SlotBuilder,
};
use waterui::reactive::Computed;
use waterui_backend_core::Environment;
use waterui_core::layout::ProposalSize;

use crate::contract::{Held, NativeLeaf, RenderContext};
use crate::sf_symbol::SfSymbolProbe;

use super::extract_title_text;

/// A rendered title with its extracted plain-text form — what native chrome
/// shows when it draws a string rather than hosting the view.
pub struct BarTitle {
    /// The rendered title view; `AppKit` hosts it when it is not plain text,
    /// `UIKit` when a principal item or a non-text title claims the slot.
    pub leaf: NativeLeaf,
    /// The text the subtree's labels carry, when any — `isPlainText` in the
    /// baseline.
    pub text: Option<String>,
}

impl fmt::Debug for BarTitle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarTitle")
            .field("text", &self.text)
            .finish_non_exhaustive()
    }
}

/// One rendered semantic toolbar item.
pub struct BarItem {
    /// The item's declared placement.
    pub placement: NavigationToolbarPlacement,
    /// The rendered content view.
    pub leaf: NativeLeaf,
    /// The item's name as resolved content, when declared.
    pub title: Option<Computed<waterui::text::StyledStr>>,
    /// The item's icon: a platform symbol name, or a rendered icon view.
    pub icon: Option<BarItemIcon>,
}

impl fmt::Debug for BarItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarItem")
            .field("placement", &self.placement)
            .finish_non_exhaustive()
    }
}

/// A toolbar item's icon, rendered or symbolic.
#[derive(Debug)]
pub enum BarItemIcon {
    /// A symbol the platform knows by name.
    System(String),
    /// A rendered icon — a packaged icon set's view.
    View(NativeLeaf),
}

/// A resolved [`Bar`]: chrome the navigation containers publish.
#[derive(Debug)]
pub struct BarState {
    /// The title.
    pub title: BarTitle,
    /// The subtitle — `UIKit`'s `navigationItem.subtitle`, unused on
    /// `AppKit`.
    #[allow(dead_code)]
    pub subtitle: BarTitle,
    /// The semantic toolbar items in declaration order.
    pub items: Vec<BarItem>,
    /// The search drawer config.
    pub search: Option<NavigationSearch>,
    /// Optional search suggestions builder lowered into the Apple slot.
    pub search_suggestions: Option<SlotBuilder<Held>>,
    /// The bar's background color, resolved or unset (the theme's `Surface`).
    pub color: Option<BarColor>,
    /// Whether the bar hides.
    pub hidden: Computed<bool>,
    /// The title's display mode.
    #[allow(dead_code)]
    pub display_mode: NavigationTitleDisplayMode,
}

impl BarState {
    /// Builds the bar state: environment-dependent fields must already be
    /// resolved (`resolve_native_fields` runs in `NavigationView::body`), so
    /// every view renders against `ctx`'s environment directly.
    pub fn new(bar: &mut Bar<ResolvedNavigationToolbar<Held>>, ctx: &RenderContext<'_>) -> Self {
        let title = bar_title(bar.title.take(), ctx);
        let subtitle = bar_title(bar.subtitle.take(), ctx);
        let items = bar
            .toolbar
            .take_items()
            .into_iter()
            .map(|item| bar_item(item, ctx))
            .collect();
        Self {
            title,
            subtitle,
            items,
            search: bar.search.take(),
            search_suggestions: bar.toolbar.take_search_suggestions(),
            color: bar.color.take(),
            hidden: bar.hidden.clone(),
            display_mode: bar.display_mode,
        }
    }

    /// The first item placed for the bar's leading slot — `leadingItem`:
    /// `Cancellation` or `TopBarLeading`.
    #[allow(dead_code)]
    pub fn leading(&self) -> Option<&BarItem> {
        self.items.iter().find(|item| {
            matches!(
                item.placement,
                NavigationToolbarPlacement::Cancellation
                    | NavigationToolbarPlacement::TopBarLeading
            )
        })
    }

    /// The first item placed for the bar's trailing slot — `trailingItem`:
    /// `PrimaryAction`, `SecondaryAction`, `Confirmation` or
    /// `TopBarTrailing`.
    #[allow(dead_code)]
    pub fn trailing(&self) -> Option<&BarItem> {
        self.items.iter().find(|item| {
            matches!(
                item.placement,
                NavigationToolbarPlacement::PrimaryAction
                    | NavigationToolbarPlacement::SecondaryAction
                    | NavigationToolbarPlacement::Confirmation
                    | NavigationToolbarPlacement::TopBarTrailing
            )
        })
    }

    /// The item placed for the bar's status slot.
    #[allow(dead_code)]
    pub fn status(&self) -> Option<&BarItem> {
        self.items
            .iter()
            .find(|item| item.placement == NavigationToolbarPlacement::Status)
    }

    /// The `UIKit` leading group: every `Cancellation`/`TopBarLeading` item.
    #[cfg(target_os = "ios")]
    pub fn leading_items(&self) -> impl Iterator<Item = &BarItem> {
        self.items.iter().filter(|item| {
            matches!(
                item.placement,
                NavigationToolbarPlacement::Cancellation
                    | NavigationToolbarPlacement::TopBarLeading
            )
        })
    }

    /// The `UIKit` trailing group: every action/`TopBarTrailing` item.
    #[cfg(target_os = "ios")]
    pub fn trailing_items(&self) -> impl Iterator<Item = &BarItem> {
        self.items.iter().filter(|item| {
            matches!(
                item.placement,
                NavigationToolbarPlacement::PrimaryAction
                    | NavigationToolbarPlacement::SecondaryAction
                    | NavigationToolbarPlacement::Confirmation
                    | NavigationToolbarPlacement::TopBarTrailing
            )
        })
    }

    /// The `UIKit` bottom-bar items: `BottomBar` and `Status` content.
    #[cfg(target_os = "ios")]
    pub fn bottom_items(&self) -> impl Iterator<Item = &BarItem> {
        self.items.iter().filter(|item| {
            matches!(
                item.placement,
                NavigationToolbarPlacement::BottomBar | NavigationToolbarPlacement::Status
            )
        })
    }

    /// The `UIKit` principal view: the first `Principal` item's content.
    #[cfg(target_os = "ios")]
    pub fn principal(&self) -> Option<&BarItem> {
        self.items
            .iter()
            .find(|item| item.placement == NavigationToolbarPlacement::Principal)
    }
}

/// A title rendered and read back — the view plus the text its leaf's
/// labels carry.
fn bar_title(view: Option<waterui::text::Text>, ctx: &RenderContext<'_>) -> BarTitle {
    let leaf = ctx.render(view);
    let text = extract_title_text(leaf.view());
    BarTitle { leaf, text }
}

/// A semantic toolbar item rendered with its name and icon apart.
///
/// The item renders under the toolbar's environment — the slot's captured
/// context included — so its labels default to icon-only presentation, as
/// the platform's toolbars present them, while explicit label preferences
/// keep precedence. That environment is also the toolbar's
/// ([`ToolbarChrome`](crate::toolbar_chrome::ToolbarChrome)), so the item's
/// button styles resolve the way the platform's toolbar draws them.
fn bar_item(item: ResolvedNavigationToolbarItem<Held>, ctx: &RenderContext<'_>) -> BarItem {
    let (placement, content, title, icon, item_env) = item.into_parts();
    let toolbar_env = toolbar_environment(item_env.as_ref().unwrap_or_else(|| ctx.env()));
    let ctx = ctx.with_env(&toolbar_env);

    let leaf = ctx.realize(content);
    let title = title.map(|title| title.resolve(ctx.env()).into_content());
    let icon = icon.map(|slot| {
        let mut probe = SfSymbolProbe::default();
        if slot.probe(&mut probe, ctx.env())
            && let Some(symbol) = probe.name
        {
            BarItemIcon::System(String::from(symbol))
        } else {
            BarItemIcon::View(ctx.realize(slot.build()))
        }
    });
    BarItem {
        placement,
        leaf,
        title,
        icon,
    }
}

/// `env` with the toolbar defaults added without overriding an explicit
/// label context.
fn toolbar_environment(env: &Environment) -> Environment {
    let mut env = crate::toolbar_chrome::toolbar_environment(env);
    if env
        .get::<LabelDisplayMode>()
        .is_none_or(|mode| matches!(mode, LabelDisplayMode::Automatic))
    {
        env.insert(LabelDisplayMode::IconOnly);
    }
    env
}

/// The intrinsic size the chrome offers a hosted item: its measured size at
/// the unspecified proposal — `setPlacementProposal(WuiProposalSize())`
/// followed by `sizeThatFits`.
pub fn bar_item_frame(item: &BarItem) -> cocoa_ui::Rect {
    let dimensions = item.leaf.layout().measure(ProposalSize {
        width: None,
        height: None,
    });
    cocoa_ui::Rect::new(
        0.0,
        0.0,
        f64::from(dimensions.size.width),
        f64::from(dimensions.size.height),
    )
}

/// The search drawer's placeholder signal, when a prompt was declared.
pub fn search_prompt(
    search: &NavigationSearch,
    env: &Environment,
) -> Option<Computed<waterui::text::StyledStr>> {
    search
        .prompt()
        .map(|prompt| prompt.resolve(env).into_content())
}

/// Wires a macOS [`cocoa_ui::appkit::SearchField`] to a [`NavigationSearch`] and
/// optional suggestions panel, keeping watchers and panel state inside `keep`.
#[cfg(target_os = "macos")]
pub fn bind_macos_search_field(
    mtm: cocoa_ui::MainThreadMarker,
    field: &cocoa_ui::Retained<cocoa_ui::appkit::SearchField>,
    search: &NavigationSearch,
    suggestions: Option<&SlotBuilder<Held>>,
    renderer: &crate::contract::Renderer,
    env: &Environment,
    keep: &mut crate::contract::KeepAlive,
) {
    use alloc::rc::Rc;

    use waterui::Str;
    use waterui::reactive::Signal;

    field.set_text(search.text().snapshot().as_ref());
    field.set_change_handler({
        let text = search.text().clone();
        move |field| text.set(Str::from(field.text()))
    });
    keep.bind(search.text(), {
        let field = field.clone();
        move |text: Str| field.set_text(text.as_ref())
    });
    if let Some(prompt) = search_prompt(search, env) {
        keep.bind(&prompt, {
            let field = field.clone();
            move |prompt| field.set_placeholder(prompt.to_plain().as_ref())
        });
    }
    if let Some(on_submit) = search.on_submit().cloned() {
        let env = env.clone();
        field.set_submit_handler(move |_| {
            let _ = on_submit.call(&env);
        });
    }
    let sync_suggestions = make_macos_suggestions_sync(mtm, field, suggestions, renderer, keep);
    let active = search.active().cloned();
    field.set_begin_editing_handler({
        let active = active.clone();
        let sync_suggestions = Rc::clone(&sync_suggestions);
        move |_| {
            if let Some(active) = active.as_ref()
                && !active.snapshot()
            {
                active.set(true);
            }
            sync_suggestions(true);
        }
    });
    field.set_end_editing_handler({
        let active = active.clone();
        let sync_suggestions = Rc::clone(&sync_suggestions);
        move |_| {
            if let Some(active) = active.as_ref()
                && active.snapshot()
            {
                active.set(false);
            }
            sync_suggestions(false);
        }
    });
    if let Some(active) = active {
        keep.bind(&active, {
            let field = field.clone();
            move |is_active| {
                field.set_active(is_active);
                sync_suggestions(is_active);
            }
        });
    }
}

#[cfg(target_os = "macos")]
fn make_macos_suggestions_sync(
    mtm: cocoa_ui::MainThreadMarker,
    field: &cocoa_ui::Retained<cocoa_ui::appkit::SearchField>,
    suggestions: Option<&SlotBuilder<Held>>,
    renderer: &crate::contract::Renderer,
    keep: &mut crate::contract::KeepAlive,
) -> alloc::rc::Rc<dyn Fn(bool)> {
    use alloc::rc::Rc;
    use core::cell::RefCell;

    use cocoa_ui::appkit::{Panel, convert_to_screen, window_of};
    use cocoa_ui::geometry::Rect;
    use cocoa_ui::view;

    use crate::transition::live::LiveChild;

    struct SuggestionsPanel {
        panel: Panel,
        _live: LiveChild,
    }

    impl Drop for SuggestionsPanel {
        fn drop(&mut self) {
            self.panel.detach();
        }
    }

    let Some(suggestions) = suggestions.cloned() else {
        return Rc::new(|_| {});
    };
    let panel_slot: Rc<RefCell<Option<SuggestionsPanel>>> = Rc::new(RefCell::new(None));
    keep.keep(Rc::clone(&panel_slot));
    let field = field.clone();
    let renderer = renderer.clone();
    Rc::new(move |shown: bool| {
        if !shown {
            *panel_slot.borrow_mut() = None;
            return;
        }
        if panel_slot.borrow().is_some() {
            return;
        }
        let Some(parent) = window_of(&field) else {
            return;
        };
        let leaf = renderer.realize(suggestions.build());
        let measured = leaf
            .layout()
            .measure(ProposalSize {
                width: None,
                height: None,
            })
            .size;
        let anchor = view::bounds_in_window(&field);
        let width = f64::from(measured.width).max(anchor.size.width).max(160.0);
        let height = f64::from(measured.height).max(1.0);
        let panel = Panel::new(mtm);
        panel.attach(&parent);
        let container = panel.content_view(mtm);
        let live = LiveChild::mount(leaf, &container, None);
        view::set_autoresizing_flexible_size(live.view());
        let frame_window = Rect::new(
            anchor.origin.x,
            anchor.origin.y - height - 4.0,
            width,
            height,
        );
        panel.set_frame(convert_to_screen(&parent, frame_window));
        *panel_slot.borrow_mut() = Some(SuggestionsPanel { panel, _live: live });
    })
}

/// A resolved color signal, when the bar color has resolved.
pub fn bar_color(bar: &BarState) -> Option<&Computed<waterui::graphics::color::WorkingColor>> {
    bar.color.as_ref().and_then(|color| color.resolved())
}
