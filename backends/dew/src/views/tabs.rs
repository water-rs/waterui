//! Tabs: retained peer destinations in a bottom bar or a sidebar rail.
//!
//! Dew honors the requested native style. A tab bar places peers along the
//! foot of the available bounds; a sidebar places them down the leading edge.
//! Automatic style selects the sidebar in landscape bounds and the tab bar in
//! portrait bounds, so the same retained container adapts without rebuilding
//! any page.
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

use accesskit::{Action as AccessibilityAction, Node as AccessibilityNode, NodeId, Role};
use nami::{Binding, Computed, Signal, SignalExt};
use waterui_controls::label::Label;
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

use crate::accessibility::ActionTarget;
use crate::dispatch::{DewNode, DewRenderer, RenderContext, WatchedSignal};
use crate::pointer::{PointerHandler, PointerTargetHandle};
use crate::text::DewState;
use crate::theme;
use crate::views::to_f32;

/// Vertical inset of a tab item from the bar's edges.
const ITEM_PADDING_Y: f64 = 4.0;
/// Horizontal inset of a tab item from a sidebar edge.
const ITEM_PADDING_X: f64 = 8.0;
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
    selection: Binding<Id>,
    semantic_label: WatchedSignal<Computed<StyledStr>>,
    enabled: WatchedSignal<Computed<bool>>,
    icon: Option<Box<dyn DewNode>>,
    title: Box<dyn DewNode>,
    badge: Option<Box<dyn DewNode>>,
    page: Page,
    /// The environment the *page* is built in — untinted. The tint belongs to
    /// the bar item, not to the screen the tab opens.
    env: Environment,
    pointer: PointerTargetHandle,
    accessibility_id: NodeId,
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
    style: NativeTabStyle,
    accessibility_id: NodeId,
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
    let items = tabs
        .into_iter()
        .map(|tab| build_item(renderer, tab, &selection, env, depth))
        .collect();
    Box::new(TabsNode {
        selection: WatchedSignal::new(selection, renderer.signals()),
        items,
        style,
        accessibility_id: renderer.allocate_accessibility_id(),
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
    let semantic_label = label
        .downcast_ref::<Label>()
        .expect("a tab's erased semantic label must contain Label")
        .semantic_text()
        .resolve(env)
        .content;
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
        selection: selection.clone(),
        semantic_label: WatchedSignal::new(semantic_label, renderer.signals()),
        enabled: WatchedSignal::new(enabled.clone(), renderer.signals()),
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
        accessibility_id: renderer.allocate_accessibility_id(),
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
    row_width: f64,
    row_height: f64,
}

impl TabItem {
    fn register_accessibility(
        &self,
        renderer: &mut DewRenderer,
        bounds: kurbo::Rect,
        selected: bool,
    ) {
        if !renderer.accessibility_enabled() {
            return;
        }
        let mut node = AccessibilityNode::new(Role::Tab);
        node.set_label(self.semantic_label.get().to_plain().to_string());
        node.set_selected(selected);
        node.add_action(AccessibilityAction::Focus);
        let target = if self.enabled.get() {
            node.add_action(AccessibilityAction::Click);
            Some(ActionTarget::Select {
                selection: self.selection.clone(),
                value: self.id,
            })
        } else {
            node.set_disabled();
            None
        };
        renderer.register_accessibility_node(self.accessibility_id, node, bounds, target);
    }

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
            row_width: f64::from(icon.width)
                + if icon.width > 0.0 { ICON_SPACING } else { 0.0 }
                + f64::from(title.width)
                + if badge.width > 0.0 {
                    BADGE_SPACING + f64::from(badge.width)
                } else {
                    0.0
                },
            row_height: f64::from(icon.height.max(title.height).max(badge.height)),
        }
    }

    /// Draws an item centred in one slot of the bottom tab bar.
    fn render_tab_bar(
        &mut self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        slot: kurbo::Rect,
        layout: &ItemLayout,
        selected: bool,
    ) {
        let window_bounds = ctx.transform.transform_rect_bbox(slot);
        self.register_accessibility(renderer, window_bounds, selected);
        renderer.push_accessibility_suppression();
        let mut y = slot.y0 + (slot.height() - layout.height).max(0.0) / 2.0;
        if let Some(icon) = self.icon.as_mut() {
            let width = f64::from(layout.icon.width).min(slot.width());
            let height = f64::from(layout.icon.height).min(slot.height());
            let x = slot.x0 + (slot.width() - width) / 2.0;
            icon.render(
                renderer,
                ctx.child(LayoutRect::new(
                    Point::new(to_f32(x), to_f32(y)),
                    Size::new(to_f32(width), to_f32(height)),
                )),
            );
            y += height + ICON_SPACING;
        }
        let title_row = f64::from(layout.title.width)
            + if layout.badge.width > 0.0 {
                f64::from(layout.badge.width) + BADGE_SPACING
            } else {
                0.0
            };
        let mut x = slot.x0 + (slot.width() - title_row).max(0.0) / 2.0;
        let badge_width = if layout.badge.width > 0.0 {
            f64::from(layout.badge.width) + BADGE_SPACING
        } else {
            0.0
        };
        let title_width = f64::from(layout.title.width).min((slot.width() - badge_width).max(0.0));
        let title_height = f64::from(layout.title.height).min((slot.y1 - y).max(0.0));
        self.title.render(
            renderer,
            ctx.child(LayoutRect::new(
                Point::new(to_f32(x), to_f32(y)),
                Size::new(to_f32(title_width), to_f32(title_height)),
            )),
        );
        x += title_width + BADGE_SPACING;
        if let Some(badge) = self.badge.as_mut()
            && layout.badge.width > 0.0
        {
            let width = f64::from(layout.badge.width).min((slot.x1 - x).max(0.0));
            let height = f64::from(layout.badge.height).min((slot.y1 - y).max(0.0));
            badge.render(
                renderer,
                ctx.child(LayoutRect::new(
                    Point::new(to_f32(x), to_f32(y)),
                    Size::new(to_f32(width), to_f32(height)),
                )),
            );
        }
        renderer.register_pointer_target(
            ctx.transform.transform_rect_bbox(slot),
            self.pointer.clone(),
        );
        renderer.pop_accessibility_suppression();
    }

    /// Draws an item in one full-width sidebar row.
    fn render_sidebar(
        &mut self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        slot: kurbo::Rect,
        layout: &ItemLayout,
        selected: bool,
    ) {
        let window_bounds = ctx.transform.transform_rect_bbox(slot);
        self.register_accessibility(renderer, window_bounds, selected);
        renderer.push_accessibility_suppression();
        let mut x = slot.x0 + ITEM_PADDING_X;
        let y = slot.y0 + (slot.height() - layout.row_height).max(0.0) / 2.0;
        if let Some(icon) = self.icon.as_mut() {
            let width = f64::from(layout.icon.width).min((slot.x1 - x).max(0.0));
            let height = f64::from(layout.icon.height).min(slot.height());
            icon.render(
                renderer,
                ctx.child(LayoutRect::new(
                    Point::new(to_f32(x), to_f32(y)),
                    Size::new(to_f32(width), to_f32(height)),
                )),
            );
            x += width + ICON_SPACING;
        }
        let badge_width = if layout.badge.width > 0.0 {
            f64::from(layout.badge.width) + BADGE_SPACING
        } else {
            0.0
        };
        let title_width = f64::from(layout.title.width)
            .min((slot.x1 - ITEM_PADDING_X - x - badge_width).max(0.0));
        let title_height = f64::from(layout.title.height).min(slot.height());
        self.title.render(
            renderer,
            ctx.child(LayoutRect::new(
                Point::new(to_f32(x), to_f32(y)),
                Size::new(to_f32(title_width), to_f32(title_height)),
            )),
        );
        x += title_width + BADGE_SPACING;
        if let Some(badge) = self.badge.as_mut()
            && layout.badge.width > 0.0
        {
            let width = f64::from(layout.badge.width).min((slot.x1 - ITEM_PADDING_X - x).max(0.0));
            let height = f64::from(layout.badge.height).min(slot.height());
            badge.render(
                renderer,
                ctx.child(LayoutRect::new(
                    Point::new(to_f32(x), to_f32(y)),
                    Size::new(to_f32(width), to_f32(height)),
                )),
            );
        }
        renderer.register_pointer_target(
            ctx.transform.transform_rect_bbox(slot),
            self.pointer.clone(),
        );
        renderer.pop_accessibility_suppression();
    }

    fn patch_chrome(&mut self, renderer: &mut DewRenderer) -> bool {
        let mut changed = self.title.patch(renderer);
        for node in self.icon.iter_mut().chain(&mut self.badge) {
            changed |= node.patch(renderer);
        }
        changed
    }
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
        let item = &mut self.items[index];
        if let Page::Unopened(builder) = &item.page {
            let view = builder.build();
            let env = item.env.clone();
            let node = crate::views::navigation::build_view(renderer, view, &env, 0);
            item.page = Page::Open(node);
        }
    }

    fn uses_sidebar(&self, bounds: kurbo::Rect) -> bool {
        match self.style {
            NativeTabStyle::Automatic => bounds.width() > bounds.height(),
            NativeTabStyle::TabBar => false,
            NativeTabStyle::Sidebar => true,
        }
    }

    fn render_selected_page(
        &mut self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        bounds: kurbo::Rect,
    ) {
        let selected = self.selected();
        let Page::Open(page) = &mut self.items[selected].page else {
            panic!("the selected dew tab must be opened during the patch phase")
        };
        page.render(renderer, ctx.child(rect_frame(bounds)));
    }

    fn render_tab_bar(
        &mut self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        bounds: kurbo::Rect,
        layouts: &[ItemLayout],
    ) {
        let tallest = layouts
            .iter()
            .map(|layout| layout.height)
            .fold(0.0_f64, f64::max);
        let desired_height = ITEM_PADDING_Y.mul_add(2.0, tallest).max(MIN_BAR_HEIGHT) + HAIRLINE;
        let bar_height = desired_height.min(bounds.height() / 2.0);
        let bar = kurbo::Rect::new(bounds.x0, bounds.y1 - bar_height, bounds.x1, bounds.y1);
        self.render_selected_page(
            renderer,
            ctx,
            kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x1, bar.y0),
        );

        let surface = renderer.theme().surface();
        renderer.list_mut().fill(&bar, ctx.transform, surface);
        let hairline = kurbo::Rect::new(bar.x0, bar.y0, bar.x1, bar.y0 + HAIRLINE);
        let border = renderer.theme().border();
        renderer.list_mut().fill(&hairline, ctx.transform, border);

        if renderer.accessibility_enabled() {
            renderer.register_accessibility_node(
                self.accessibility_id,
                AccessibilityNode::new(Role::TabList),
                ctx.transform.transform_rect_bbox(bar),
                None,
            );
            renderer.push_accessibility_parent(self.accessibility_id);
        }

        let count = u32::try_from(self.items.len())
            .expect("a dew tab container cannot hold more than u32::MAX items");
        let slot_width = bar.width() / f64::from(count);
        let selected = self.selection.get();
        for (index, (item, layout)) in self.items.iter_mut().zip(layouts).enumerate() {
            let index = u32::try_from(index).expect("a dew tab index must fit in u32");
            let x = bar.x0 + slot_width * f64::from(index);
            let slot = kurbo::Rect::new(x, bar.y0 + HAIRLINE, x + slot_width, bar.y1);
            item.render_tab_bar(renderer, ctx, slot, layout, item.id == selected);
        }
        if renderer.accessibility_enabled() {
            renderer.pop_accessibility_parent();
        }
    }

    fn render_sidebar(
        &mut self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        bounds: kurbo::Rect,
        layouts: &[ItemLayout],
    ) {
        let desired_width = ITEM_PADDING_X.mul_add(
            2.0,
            layouts
                .iter()
                .map(|layout| layout.row_width)
                .fold(0.0_f64, f64::max)
                + HAIRLINE,
        );
        let sidebar_width = desired_width.min(bounds.width() / 2.0);
        let sidebar = kurbo::Rect::new(bounds.x0, bounds.y0, bounds.x0 + sidebar_width, bounds.y1);
        self.render_selected_page(
            renderer,
            ctx,
            kurbo::Rect::new(sidebar.x1, bounds.y0, bounds.x1, bounds.y1),
        );

        let surface = renderer.theme().surface();
        renderer.list_mut().fill(&sidebar, ctx.transform, surface);
        let hairline = kurbo::Rect::new(sidebar.x1 - HAIRLINE, sidebar.y0, sidebar.x1, sidebar.y1);
        let border = renderer.theme().border();
        renderer.list_mut().fill(&hairline, ctx.transform, border);

        if renderer.accessibility_enabled() {
            renderer.register_accessibility_node(
                self.accessibility_id,
                AccessibilityNode::new(Role::TabList),
                ctx.transform.transform_rect_bbox(sidebar),
                None,
            );
            renderer.push_accessibility_parent(self.accessibility_id);
        }

        let count = u32::try_from(self.items.len())
            .expect("a dew tab container cannot hold more than u32::MAX items");
        let slot_height = sidebar.height() / f64::from(count);
        let selected = self.selection.get();
        for (index, (item, layout)) in self.items.iter_mut().zip(layouts).enumerate() {
            let index = u32::try_from(index).expect("a dew tab index must fit in u32");
            let y = sidebar.y0 + slot_height * f64::from(index);
            let slot = kurbo::Rect::new(sidebar.x0, y, sidebar.x1 - HAIRLINE, y + slot_height);
            item.render_sidebar(renderer, ctx, slot, layout, item.id == selected);
        }
        if renderer.accessibility_enabled() {
            renderer.pop_accessibility_parent();
        }
    }
}

const fn rect_frame(rect: kurbo::Rect) -> LayoutRect {
    LayoutRect::new(
        Point::new(to_f32(rect.x0), to_f32(rect.y0)),
        Size::new(to_f32(rect.width()), to_f32(rect.height())),
    )
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
        if self.uses_sidebar(bounds) {
            self.render_sidebar(renderer, ctx, bounds, &layouts);
        } else {
            self.render_tab_bar(renderer, ctx, bounds, &layouts);
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.open_selected(renderer);
        let selected = self.selected();
        let mut changed = self
            .items
            .iter_mut()
            .fold(false, |changed, item| item.patch_chrome(renderer) | changed);
        let Page::Open(page) = &mut self.items[selected].page else {
            panic!("the selected dew tab must be opened before it is patched")
        };
        changed |= page.patch(renderer);
        changed
    }
}
