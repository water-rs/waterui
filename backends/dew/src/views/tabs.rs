//! Tabs: one row of destinations along the foot of the panel.
//!
//! A tab bar is the one navigation container that fits a small display
//! sideways — a handful of peers, each its own screen — so dew draws
//! [`TabsLayout`] as a bottom row of items above the selected tab's page.
//!
//! Two contract points, both consequences of the ones in
//! [`super::navigation`]:
//!
//! - **A tab's page is built when it is first selected and retained
//!   afterwards.** Coming back to a tab shows it exactly where it was left,
//!   which is what the semantics ask for, and a tab never opened costs
//!   nothing — on a device with a few hundred kilobytes of RAM, building four
//!   screens to show one is not a trade worth making.
//! - **Switching tabs is instantaneous**, for the reason a push is: any
//!   cross-fade would redraw the whole panel on every frame it ran.
//!
//! An item's tint comes from a colour signal installed into that item's own
//! environment, derived from the selection. Selecting a different tab
//! therefore recolours two labels through the ordinary reactive path, with no
//! part of the tree rebuilt.

use core::cell::RefCell;

use nami::{Binding, Computed, Signal, SignalExt};
use waterui_core::env::Store;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_core::layout::{
    Point, ProposalSize, Rect as LayoutRect, Size, StretchAxis, ViewDimensions,
};
use waterui_core::{AnyView, Environment};
use waterui_graphics::color::{AccentColor, ForegroundColor, MutedForegroundColor, ResolvedColor};
use waterui_navigation::tab::{NativeTabStyle, TabIcon};
use waterui_navigation::{NavigationView, Tab, TabsLayout};
use waterui_text::Text;
use waterui_text::styled::StyledStr;

use crate::dispatch::{DewNode, DewRenderer, RenderContext, WatchedSignal};
use crate::pointer::{PointerHandler, PointerTargetHandle};
use crate::text::DewState;
use crate::theme;
use crate::views::to_f32;

/// Vertical inset of a tab item from the bar's edges.
const ITEM_PADDING_Y: f64 = 4.0;
/// Gap between a tab's icon and its title.
const ICON_SPACING: f64 = 2.0;
/// Gap between a tab's title and its badge.
const BADGE_SPACING: f64 = 4.0;
/// Shortest a tab bar is drawn.
const MIN_BAR_HEIGHT: f64 = 32.0;
/// Width of the hairline above the bar.
const HAIRLINE: f64 = 1.0;

/// A tab's page: built the first time the tab is selected, retained after.
enum Page {
    Unopened(AnyViewBuilder<NavigationView>),
    Open(Box<dyn DewNode>),
}

struct TabItem {
    id: Id,
    icon: Option<Box<dyn DewNode>>,
    title: Box<dyn DewNode>,
    badge: Option<Box<dyn DewNode>>,
    page: Page,
    /// The environment the *page* is built in — untinted. The tint belongs to
    /// the bar item, not to the screen the tab opens.
    env: Environment,
    pointer: PointerTargetHandle,
}

/// Sets the selection when its slot is tapped, unless the tab is disabled.
struct TabPointer {
    selection: Binding<Id>,
    id: Id,
    enabled: Computed<bool>,
    armed: bool,
}

impl PointerHandler for TabPointer {
    fn pointer_down(&mut self, _point: kurbo::Point, _bounds: kurbo::Rect) -> bool {
        self.armed = self.enabled.get();
        false
    }

    fn pointer_up(&mut self, point: kurbo::Point, bounds: kurbo::Rect) -> bool {
        let select =
            core::mem::take(&mut self.armed) && bounds.contains(point) && self.enabled.get();
        if select && self.selection.get() != self.id {
            self.selection.set(self.id);
            return true;
        }
        false
    }

    fn pointer_cancel(&mut self) -> bool {
        self.armed = false;
        false
    }
}

struct TabsNode {
    selection: WatchedSignal<Binding<Id>>,
    items: Vec<TabItem>,
}

/// Builds the retained node for a tab container.
pub fn build(
    renderer: &mut DewRenderer,
    layout: TabsLayout,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    let TabsLayout {
        selection,
        tabs,
        style,
        ..
    } = layout;
    assert!(
        !matches!(style, NativeTabStyle::Sidebar),
        "dew does not implement the sidebar tab style: a panel has room for one column, \
         so tabs are drawn as a bottom bar"
    );
    let items = tabs
        .into_iter()
        .map(|tab| build_item(renderer, tab, &selection, env, depth))
        .collect();
    Box::new(TabsNode {
        selection: WatchedSignal::new(selection, renderer.signals()),
        items,
    })
}

fn build_item(
    renderer: &mut DewRenderer,
    tab: Tab<Id>,
    selection: &Binding<Id>,
    env: &Environment,
    depth: usize,
) -> TabItem {
    let Tab {
        id,
        label,
        icon,
        content,
        badge,
        enabled,
    } = tab;
    let tinted = tinted_environment(selection, id, env);
    let icon = icon.map(|icon| match icon {
        TabIcon::View(view) => crate::dispatch::build_node(renderer, view.build(), &tinted, depth),
        TabIcon::System(icon) => panic!(
            "dew cannot draw the system icon `{}` on a tab: a panel has no OS icon catalog, \
             so use a packaged WaterUI icon set instead",
            icon.name.as_str()
        ),
    });
    let badge = badge.map(|count| {
        // Reactive, and empty below one: a badge that reads "0" is not a badge.
        let text = Text::computed(count.map(|count| {
            if count > 0 {
                StyledStr::plain(count.to_string())
            } else {
                StyledStr::empty()
            }
        }));
        crate::dispatch::build_node(renderer, AnyView::new(text), &tinted, depth)
    });
    TabItem {
        id,
        icon,
        title: crate::dispatch::build_node(renderer, label, &tinted, depth),
        badge,
        page: Page::Unopened(content),
        pointer: PointerTargetHandle::new(TabPointer {
            selection: selection.clone(),
            id,
            enabled,
            armed: false,
        }),
        env: env.clone(),
    }
}

/// An environment whose foreground follows this tab's selection state.
fn tinted_environment(selection: &Binding<Id>, id: Id, env: &Environment) -> Environment {
    let accent = theme::slot::<AccentColor>(env, theme::ACCENT);
    let muted = theme::slot::<MutedForegroundColor>(env, theme::MUTED_FOREGROUND);
    let tint = accent
        .zip(&muted)
        .zip(selection)
        .map(move |((accent, muted), selected)| if selected == id { accent } else { muted });
    let mut item_env = env.clone();
    item_env.insert(Store::<ForegroundColor, Computed<ResolvedColor>>::new(
        tint.computed(),
    ));
    item_env
}

/// The measured parts of one tab item.
struct ItemLayout {
    icon: Size,
    title: Size,
    badge: Size,
    height: f64,
}

impl TabItem {
    fn layout(&self, state: &RefCell<DewState>) -> ItemLayout {
        let measure = |node: &dyn DewNode| node.measure(state, ProposalSize::UNSPECIFIED).size;
        let icon = self
            .icon
            .as_ref()
            .map_or_else(Size::zero, |node| measure(node.as_ref()));
        let title = measure(self.title.as_ref());
        let badge = self
            .badge
            .as_ref()
            .map_or_else(Size::zero, |node| measure(node.as_ref()));
        let icon_block = if icon.height > 0.0 {
            f64::from(icon.height) + ICON_SPACING
        } else {
            0.0
        };
        ItemLayout {
            icon,
            title,
            badge,
            height: icon_block + f64::from(title.height),
        }
    }

    /// Draws the item centred in `slot`.
    fn render(
        &mut self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        slot: kurbo::Rect,
        layout: &ItemLayout,
    ) {
        let mut y = slot.y0 + (slot.height() - layout.height).max(0.0) / 2.0;
        if let Some(icon) = self.icon.as_mut() {
            let x = slot.x0 + (slot.width() - f64::from(layout.icon.width)).max(0.0) / 2.0;
            icon.render(renderer, ctx.child(frame(x, y, layout.icon)));
            y += f64::from(layout.icon.height) + ICON_SPACING;
        }
        let title_row = f64::from(layout.title.width)
            + if layout.badge.width > 0.0 {
                f64::from(layout.badge.width) + BADGE_SPACING
            } else {
                0.0
            };
        let mut x = slot.x0 + (slot.width() - title_row).max(0.0) / 2.0;
        self.title
            .render(renderer, ctx.child(frame(x, y, layout.title)));
        x += f64::from(layout.title.width) + BADGE_SPACING;
        if let Some(badge) = self.badge.as_mut()
            && layout.badge.width > 0.0
        {
            badge.render(renderer, ctx.child(frame(x, y, layout.badge)));
        }
        renderer.register_pointer_target(
            ctx.transform.transform_rect_bbox(slot),
            self.pointer.clone(),
        );
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        let mut changed = self.title.patch(renderer);
        for node in self.icon.iter_mut().chain(&mut self.badge) {
            changed |= node.patch(renderer);
        }
        if let Page::Open(page) = &mut self.page {
            changed |= page.patch(renderer);
        }
        changed
    }
}

const fn frame(x: f64, y: f64, size: Size) -> LayoutRect {
    LayoutRect::new(Point::new(to_f32(x), to_f32(y)), size)
}

impl TabsNode {
    /// The index of the selected tab.
    ///
    /// # Panics
    ///
    /// Panics when the selection names a tab this container does not hold.
    /// The tabs and the selection binding share one `Mapping`, so that cannot
    /// happen by accident, and showing some other tab instead would hide the
    /// mistake behind a screen the app never asked for.
    fn selected(&self) -> usize {
        let selected = self.selection.get();
        self.items
            .iter()
            .position(|item| item.id == selected)
            .unwrap_or_else(|| {
                panic!("dew tab selection {selected:?} names no tab in this container")
            })
    }

    /// Opens the selected tab's page if this is the first time it is shown.
    fn open_selected(&mut self, renderer: &mut DewRenderer) {
        let index = self.selected();
        let Some(item) = self.items.get_mut(index) else {
            return;
        };
        if let Page::Unopened(builder) = &item.page {
            let view = builder.build();
            let env = item.env.clone();
            let node = crate::views::navigation::build_view(renderer, view, &env, 0);
            item.page = Page::Open(node);
        }
    }
}

impl DewNode for TabsNode {
    fn measure(&self, _state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        if self.items.is_empty() {
            return;
        }
        let bounds = ctx.bounds;
        let layouts: Vec<ItemLayout> = self
            .items
            .iter()
            .map(|item| item.layout(renderer.state_cell()))
            .collect();
        let tallest = layouts
            .iter()
            .map(|layout| layout.height)
            .fold(0.0_f64, f64::max);
        let bar_height = ITEM_PADDING_Y.mul_add(2.0, tallest).max(MIN_BAR_HEIGHT) + HAIRLINE;
        let bar = kurbo::Rect::new(bounds.x0, bounds.y1 - bar_height, bounds.x1, bounds.y1);

        let selected = self.selected();
        if let Some(Page::Open(page)) = self.items.get_mut(selected).map(|item| &mut item.page) {
            page.render(
                renderer,
                ctx.child(LayoutRect::new(
                    Point::new(to_f32(bounds.x0), to_f32(bounds.y0)),
                    Size::new(
                        to_f32(bounds.width()),
                        to_f32((bounds.height() - bar_height).max(0.0)),
                    ),
                )),
            );
        }

        let surface = renderer.theme().surface();
        renderer.list_mut().fill(&bar, ctx.transform, surface);
        let hairline = kurbo::Rect::new(bar.x0, bar.y0, bar.x1, bar.y0 + HAIRLINE);
        let border = renderer.theme().border();
        renderer.list_mut().fill(&hairline, ctx.transform, border);

        let slot_width = bar.width() / f64::from(u32::try_from(self.items.len()).unwrap_or(1));
        for (index, (item, layout)) in self.items.iter_mut().zip(&layouts).enumerate() {
            let x = bar.x0 + slot_width * f64::from(u32::try_from(index).unwrap_or(0));
            let slot = kurbo::Rect::new(x, bar.y0 + HAIRLINE, x + slot_width, bar.y1);
            item.render(renderer, ctx, slot, layout);
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.open_selected(renderer);
        self.items
            .iter_mut()
            .fold(false, |changed, item| item.patch(renderer) | changed)
    }
}
