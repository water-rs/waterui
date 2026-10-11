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

use waterui::navigation::{
    Bar, BarColor, NavigationSearch, NavigationTitleDisplayMode, NavigationToolbarItem,
    NavigationToolbarPlacement, ToolbarItemIcon,
};
use waterui::reactive::Computed;
use waterui_backend_core::Environment;
use waterui_core::layout::ProposalSize;

use crate::contract::{NativeLeaf, RenderContext};

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
    pub fn new(bar: &mut Bar, ctx: &RenderContext<'_>) -> Self {
        let title = bar_title(&mut bar.title, ctx);
        let subtitle = bar_title(&mut bar.subtitle, ctx);
        let items = bar
            .toolbar
            .items
            .drain(..)
            .map(|item| bar_item(item, ctx))
            .collect();
        Self {
            title,
            subtitle,
            items,
            search: bar.search.take(),
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
fn bar_title(view: &mut waterui_backend_core::AnyView, ctx: &RenderContext<'_>) -> BarTitle {
    let leaf = ctx.render(core::mem::take(view));
    let text = extract_title_text(leaf.view());
    BarTitle { leaf, text }
}

/// A semantic toolbar item rendered with its name and icon apart.
fn bar_item(item: NavigationToolbarItem, ctx: &RenderContext<'_>) -> BarItem {
    let leaf = ctx.render(item.content);
    let title = item.title.map(|title| title.resolve(ctx.env()).content);
    let icon = item.icon.map(|icon| match icon {
        ToolbarItemIcon::System(icon) => BarItemIcon::System(icon.name.to_string()),
        ToolbarItemIcon::View(builder) => BarItemIcon::View(ctx.render(builder.build())),
    });
    BarItem {
        placement: item.placement,
        leaf,
        title,
        icon,
    }
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

/// The search drawer's placeholder signal.
pub fn search_prompt(
    search: &NavigationSearch,
    env: &Environment,
) -> Computed<waterui::text::StyledStr> {
    search.prompt.resolve(env).content
}

/// A resolved color signal, when the bar color has resolved.
pub fn bar_color(bar: &BarState) -> Option<&Computed<waterui::graphics::color::WorkingColor>> {
    bar.color.as_ref().and_then(|color| color.resolved())
}
