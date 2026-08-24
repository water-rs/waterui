//! Navigation: what a stack, a destination and a bar mean on a panel.
//!
//! # Dew's navigation contract
//!
//! **A stack shows exactly one destination.** The top entry is measured,
//! rendered and hit-tested; the entries below it are retained — node tree,
//! shaped text and all — but do no per-frame work. Retaining them is what
//! makes going back cheap and, more importantly, correct: a list scrolled
//! halfway down is still scrolled halfway down when the pushed screen is
//! dismissed. Embedded stacks are two to four screens deep, so the memory this
//! costs is bounded and small, and rebuilding a screen on every pop would cost
//! a full body evaluation plus re-shaping every string on it.
//!
//! **A destination presents instantly. Dew animates no navigation
//! transition,** and resolves every declared one — automatic, fade, zoom,
//! none — to an immediate switch. This is a property of the renderer, not a
//! shortcut. Dew's frame economy is a pairwise diff of the retained display
//! list: a command that draws the same thing in the same place costs nothing,
//! and only the pixels that actually changed are re-rasterized and clocked out
//! to the panel. A slide or a fade moves or recolours *every* command on
//! screen, so each of its frames dirties the whole screen. At 60fps for 250ms
//! that is fifteen full-screen repaints — on a 240×240 RGB565 panel over a
//! 40MHz SPI bus, ~23ms of wire time each, or about 350ms of bus occupancy for
//! one push that could have cost 23ms. Presenting instantly makes a push
//! exactly one full-screen frame, which is the same cost the destination's
//! first paint has anyway. The asymmetry with hydrolysis — which redraws the
//! whole scene every frame by design and animates freely — is documented here
//! rather than hidden behind a degraded imitation.
//!
//! **The bar is chrome the stack owns, not a widget each screen carries.** A
//! destination declares a [`Bar`]; the stack draws the current one across the
//! top: leading items (with the back affordance first), the title, trailing
//! items, an optional search field beneath, and any bottom-bar items pinned to
//! the foot of the screen. A [`NavigationView`] rendered *outside* a stack
//! draws its own bar, which is what makes a bare navigation view usable as a
//! whole screen.
//!
//! **Accessibility.** Dew has no accessibility tree at all today — no backend
//! code produces one, and `waterui-testing` reads hydrolysis'. Navigation
//! therefore contributes no semantic surface of its own, and this is a gap to
//! close rather than a decision: every control the chrome creates (the back
//! affordance, toolbar items, the search field) is an ordinary `WaterUI`
//! control built from a semantic [`Label`](waterui_controls::label::Label), so
//! each already carries the name an accessibility tree would publish once Dew
//! grows one.
//!
//! **Split views are not implemented.** [`NavigationSplitLayout`] wants two or
//! three side-by-side columns, and the panels Dew targets have room for one;
//! reaching one arrives as an explicit panic rather than a silently collapsed
//! layout.
//!
//! [`NavigationSplitLayout`]: waterui_navigation::NavigationSplitLayout

use core::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use kurbo::Rect;
use nami::Computed;
use waterui_controls::button::{ButtonStyle, button};
use waterui_controls::text_field::TextField;
use waterui_core::layout::{
    Point, ProposalSize, Rect as LayoutRect, Size, StretchAxis, ViewDimensions,
};
use waterui_core::{AnyView, Environment};
use waterui_graphics::color::ResolvedColor;
use waterui_navigation::{
    Bar, CustomNavigationController, NavigationController, NavigationDestinationState,
    NavigationStack, NavigationTitleDisplayMode, NavigationToolbarPlacement, NavigationTransaction,
    NavigationView, navigation_back_label, resolve_navigation_root,
};

use crate::dispatch::{DewNode, DewRenderer, RenderContext, WatchedSignal, build_node};
use crate::text::DewState;
use crate::views::to_f32;

/// Horizontal inset of bar content from the screen edge.
const BAR_PADDING_X: f64 = 8.0;
/// Vertical inset of bar content from the bar's own edges.
const BAR_PADDING_Y: f64 = 4.0;
/// Gap between adjacent bar items.
const BAR_SPACING: f64 = 6.0;
/// Shortest a bar is drawn, however small its content.
const MIN_BAR_HEIGHT: f64 = 28.0;
/// Width of the hairline separating chrome from content.
const HAIRLINE: f64 = 1.0;

/// One destination: its chrome, its content, and its lifecycle handlers.
struct Entry {
    chrome: Chrome,
    content: Box<dyn DewNode>,
    state: NavigationDestinationState,
}

impl Entry {
    fn build(
        renderer: &mut DewRenderer,
        view: NavigationView,
        env: &Environment,
        back: Option<Box<dyn DewNode>>,
    ) -> Self {
        let NavigationView {
            bar,
            content,
            state,
            ..
        } = view;
        Self {
            chrome: Chrome::build(renderer, bar, env, back),
            content: build_node(renderer, content, env, 0),
            state,
        }
    }
}

/// The retained navigation bar of one destination.
struct Chrome {
    hidden: WatchedSignal<Computed<bool>>,
    color: Option<WatchedSignal<Computed<ResolvedColor>>>,
    display_mode: NavigationTitleDisplayMode,
    title: Box<dyn DewNode>,
    subtitle: Box<dyn DewNode>,
    /// The back affordance, when this destination sits above the root, then
    /// the destination's own leading toolbar items.
    leading: Vec<Box<dyn DewNode>>,
    trailing: Vec<Box<dyn DewNode>>,
    bottom: Vec<Box<dyn DewNode>>,
    search: Option<Box<dyn DewNode>>,
}

/// Every measurement the bar's placement needs, computed once per frame.
struct BarLayout {
    height: f64,
    row_height: f64,
    leading: Vec<Size>,
    trailing: Vec<Size>,
    title: Size,
    subtitle: Size,
    search: Option<Size>,
    /// Whether the title sits on its own row beneath the item row.
    stacked_title: bool,
}

impl Chrome {
    fn build(
        renderer: &mut DewRenderer,
        bar: Bar,
        env: &Environment,
        back: Option<Box<dyn DewNode>>,
    ) -> Self {
        let Bar {
            title,
            subtitle,
            toolbar,
            search,
            color,
            hidden,
            display_mode,
        } = bar;
        let mut leading: Vec<Box<dyn DewNode>> = back.into_iter().collect();
        let mut trailing = Vec::new();
        let mut bottom = Vec::new();
        let mut principal = None;
        for item in toolbar.items {
            let node = build_node(renderer, item.content, env, 0);
            match item.placement {
                NavigationToolbarPlacement::Principal => principal = Some(node),
                NavigationToolbarPlacement::Cancellation
                | NavigationToolbarPlacement::TopBarLeading => leading.push(node),
                NavigationToolbarPlacement::BottomBar | NavigationToolbarPlacement::Status => {
                    bottom.push(node);
                }
                NavigationToolbarPlacement::PrimaryAction
                | NavigationToolbarPlacement::SecondaryAction
                | NavigationToolbarPlacement::Confirmation
                | NavigationToolbarPlacement::TopBarTrailing => trailing.push(node),
            }
        }
        Self {
            hidden: WatchedSignal::new(hidden, renderer.signals()),
            color: color.map(|color| {
                WatchedSignal::new(color.expect_resolved().clone(), renderer.signals())
            }),
            display_mode,
            // Principal content replaces the title outright, which is what the
            // placement means: it is the title area's content.
            title: principal.unwrap_or_else(|| build_node(renderer, title, env, 0)),
            subtitle: build_node(renderer, subtitle, env, 0),
            leading,
            trailing,
            bottom,
            search: search.map(|search| {
                let field = TextField::new(search.prompt.clone(), &search.text)
                    .hide_label()
                    .prompt(search.prompt);
                build_node(renderer, AnyView::new(field), env, 0)
            }),
        }
    }

    fn hidden(&self) -> bool {
        self.hidden.get()
    }

    /// Measures every part of the bar against `width`.
    fn layout(&self, state: &RefCell<DewState>, width: f64) -> BarLayout {
        let measure = |node: &dyn DewNode| node.measure(state, ProposalSize::UNSPECIFIED).size;
        let leading: Vec<Size> = self
            .leading
            .iter()
            .map(|node| measure(node.as_ref()))
            .collect();
        let trailing: Vec<Size> = self
            .trailing
            .iter()
            .map(|node| measure(node.as_ref()))
            .collect();
        let title = measure(self.title.as_ref());
        let subtitle = measure(self.subtitle.as_ref());
        let stacked_title = matches!(
            self.display_mode,
            NavigationTitleDisplayMode::Medium | NavigationTitleDisplayMode::Large
        );
        let tallest_item = leading
            .iter()
            .chain(&trailing)
            .map(|size| f64::from(size.height))
            .fold(0.0_f64, f64::max);
        let title_block = f64::from(title.height) + f64::from(subtitle.height);
        let row_content = if stacked_title {
            tallest_item
        } else {
            tallest_item.max(title_block)
        };
        let row_height = BAR_PADDING_Y.mul_add(2.0, row_content).max(MIN_BAR_HEIGHT);
        let search = self.search.as_ref().map(|field| {
            field
                .measure(
                    state,
                    ProposalSize::new(
                        Some(to_f32(BAR_PADDING_X.mul_add(-2.0, width).max(0.0))),
                        None,
                    ),
                )
                .size
        });
        let mut height = row_height;
        if stacked_title {
            height += title_block + BAR_PADDING_Y;
        }
        if let Some(search) = search {
            height += BAR_PADDING_Y.mul_add(2.0, f64::from(search.height));
        }
        BarLayout {
            height: height + HAIRLINE,
            row_height,
            leading,
            trailing,
            title,
            subtitle,
            search,
            stacked_title,
        }
    }

    /// Draws the bar into the top of `bar_rect` and returns nothing: the
    /// caller already knows the height from [`Chrome::layout`].
    fn render(
        &mut self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        bar_rect: Rect,
        layout: &BarLayout,
    ) {
        let background = self.color.as_ref().map_or_else(
            || renderer.theme().surface(),
            |color| to_peniko(color.get()),
        );
        renderer.list_mut().fill(&bar_rect, ctx.transform, background);
        let hairline = Rect::new(
            bar_rect.x0,
            bar_rect.y1 - HAIRLINE,
            bar_rect.x1,
            bar_rect.y1,
        );
        let border = renderer.theme().border();
        renderer.list_mut().fill(&hairline, ctx.transform, border);

        let row = Rect::new(
            bar_rect.x0,
            bar_rect.y0,
            bar_rect.x1,
            bar_rect.y0 + layout.row_height,
        );
        let mut leading_edge = row.x0 + BAR_PADDING_X;
        for (node, size) in self.leading.iter_mut().zip(&layout.leading) {
            let frame = centered_in(row, leading_edge, *size);
            node.render(renderer, ctx.child(frame));
            leading_edge += f64::from(size.width) + BAR_SPACING;
        }
        let mut trailing_edge = row.x1 - BAR_PADDING_X;
        for (node, size) in self.trailing.iter_mut().rev().zip(layout.trailing.iter().rev()) {
            let x = trailing_edge - f64::from(size.width);
            let frame = centered_in(row, x, *size);
            node.render(renderer, ctx.child(frame));
            trailing_edge = x - BAR_SPACING;
        }

        let (title_area, title_centered) = if layout.stacked_title {
            (
                Rect::new(
                    bar_rect.x0 + BAR_PADDING_X,
                    row.y1,
                    bar_rect.x1 - BAR_PADDING_X,
                    row.y1 + f64::from(layout.title.height) + f64::from(layout.subtitle.height),
                ),
                false,
            )
        } else {
            (
                Rect::new(
                    leading_edge,
                    row.y0,
                    trailing_edge.max(leading_edge),
                    row.y1,
                ),
                true,
            )
        };
        let title_block =
            f64::from(layout.title.height) + f64::from(layout.subtitle.height);
        let mut title_y = title_area.y0 + (title_area.height() - title_block).max(0.0) / 2.0;
        for (node, size) in [
            (&mut self.title, layout.title),
            (&mut self.subtitle, layout.subtitle),
        ] {
            if size.height <= 0.0 {
                continue;
            }
            let width = f64::from(size.width).min(title_area.width().max(0.0));
            let x = if title_centered {
                title_area.x0 + (title_area.width() - width).max(0.0) / 2.0
            } else {
                title_area.x0
            };
            node.render(
                renderer,
                ctx.child(LayoutRect::new(
                    Point::new(to_f32(x), to_f32(title_y)),
                    Size::new(to_f32(width), size.height),
                )),
            );
            title_y += f64::from(size.height);
        }

        if let (Some(field), Some(size)) = (self.search.as_mut(), layout.search) {
            let y = bar_rect.y1 - HAIRLINE - BAR_PADDING_Y - f64::from(size.height);
            field.render(
                renderer,
                ctx.child(LayoutRect::new(
                    Point::new(to_f32(bar_rect.x0 + BAR_PADDING_X), to_f32(y)),
                    Size::new(
                        to_f32(BAR_PADDING_X.mul_add(-2.0, bar_rect.width()).max(0.0)),
                        size.height,
                    ),
                )),
            );
        }
    }

    /// Measures the bottom bar, which is drawn at the foot of the screen.
    fn bottom_layout(&self, state: &RefCell<DewState>) -> Option<(f64, Vec<Size>)> {
        if self.bottom.is_empty() {
            return None;
        }
        let sizes: Vec<Size> = self
            .bottom
            .iter()
            .map(|node| node.measure(state, ProposalSize::UNSPECIFIED).size)
            .collect();
        let tallest = sizes
            .iter()
            .map(|size| f64::from(size.height))
            .fold(0.0_f64, f64::max);
        Some((
            BAR_PADDING_Y.mul_add(2.0, tallest).max(MIN_BAR_HEIGHT) + HAIRLINE,
            sizes,
        ))
    }

    fn render_bottom(
        &mut self,
        renderer: &mut DewRenderer,
        ctx: RenderContext,
        rect: Rect,
        sizes: &[Size],
    ) {
        let surface = renderer.theme().surface();
        renderer.list_mut().fill(&rect, ctx.transform, surface);
        let hairline = Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + HAIRLINE);
        let border = renderer.theme().border();
        renderer.list_mut().fill(&hairline, ctx.transform, border);
        let gaps = u32::try_from(sizes.len().saturating_sub(1)).unwrap_or(u32::MAX);
        let total: f64 = BAR_SPACING.mul_add(
            f64::from(gaps),
            sizes.iter().map(|size| f64::from(size.width)).sum::<f64>(),
        );
        let mut x = rect.x0 + (rect.width() - total).max(0.0) / 2.0;
        let row = Rect::new(rect.x0 + HAIRLINE, rect.y0 + HAIRLINE, rect.x1, rect.y1);
        for (node, size) in self.bottom.iter_mut().zip(sizes) {
            let frame = centered_in(row, x, *size);
            node.render(renderer, ctx.child(frame));
            x += f64::from(size.width) + BAR_SPACING;
        }
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        let mut changed = self.title.patch(renderer) | self.subtitle.patch(renderer);
        for node in self
            .leading
            .iter_mut()
            .chain(&mut self.trailing)
            .chain(&mut self.bottom)
            .chain(&mut self.search)
        {
            changed |= node.patch(renderer);
        }
        changed
    }
}

/// Places `size` at `x`, vertically centred in `row`.
fn centered_in(row: Rect, x: f64, size: Size) -> LayoutRect {
    let y = row.y0 + (row.height() - f64::from(size.height)).max(0.0) / 2.0;
    LayoutRect::new(Point::new(to_f32(x), to_f32(y)), size)
}

fn to_peniko(color: ResolvedColor) -> peniko::Color {
    let srgb = color.to_srgb_with_headroom();
    peniko::Color::new([srgb.red, srgb.green, srgb.blue, color.opacity])
}

/// The buffer a controller writes into and [`StackNode::patch`] drains.
///
/// A transaction arrives from a button action, in the middle of pointer
/// dispatch: the node it would mutate is being rendered from. Buffering it and
/// applying it in the patch phase is the same seam `Dynamic` uses, and it
/// guarantees a newly exposed destination is patched — measurement caches
/// cleared — before it is measured.
#[derive(Default)]
struct Pending {
    transactions: RefCell<VecDeque<NavigationTransaction>>,
    back_requested: Cell<bool>,
    /// Set while [`StackNode::patch`] is draining, so a transaction produced
    /// by the very frame that is applying one does not ask for another.
    applying: Cell<bool>,
}

struct StackReceiver {
    pending: Rc<Pending>,
    signals: waterui_backend_core::frame_signals::FrameSignals,
}

impl CustomNavigationController for StackReceiver {
    fn apply(&mut self, transaction: NavigationTransaction) {
        self.pending.transactions.borrow_mut().push_back(transaction);
        if !self.pending.applying.get() {
            self.signals.request_refresh();
        }
    }
}

/// The retained node behind a navigation stack.
struct StackNode {
    controller: NavigationController,
    env: Environment,
    entries: Vec<Entry>,
    pending: Rc<Pending>,
}

/// Builds the retained node for a navigation stack.
pub fn build_stack(
    renderer: &mut DewRenderer,
    stack: NavigationStack<(), ()>,
    env: &Environment,
) -> Box<dyn DewNode> {
    let pending = Rc::new(Pending::default());
    let controller = NavigationController::new(StackReceiver {
        pending: Rc::clone(&pending),
        signals: renderer.signals(),
    });
    let mut child_env = env.clone();
    child_env.insert(controller.clone());

    let root = resolve_navigation_root(stack.into_inner(), &child_env);
    let mut entry = Entry::build(renderer, root, &child_env, None);
    entry.state.appeared(&child_env);
    Box::new(StackNode {
        controller,
        env: child_env,
        entries: vec![entry],
        pending,
    })
}

impl StackNode {
    fn top_mut(&mut self) -> &mut Entry {
        self.entries
            .last_mut()
            .expect("a navigation stack always retains its root")
    }

    /// The back affordance for a destination pushed above the root.
    fn back_button(&self, renderer: &mut DewRenderer) -> Box<dyn DewNode> {
        let pending = Rc::clone(&self.pending);
        let signals = renderer.signals();
        let view = button(navigation_back_label())
            .style(ButtonStyle::Plain)
            .action(move || {
                // The destination decides whether the pop may start, and it is
                // the patch phase that asks it; recording the request here
                // keeps the answer out of the middle of pointer dispatch.
                pending.back_requested.set(true);
                signals.request_refresh();
            });
        build_node(renderer, AnyView::new(view), &self.env, 0)
    }

    /// Asks the current destination whether a user pop may start, and starts
    /// it when it may.
    fn resolve_back_request(&mut self) {
        if !self.pending.back_requested.replace(false) {
            return;
        }
        let env = self.env.clone();
        if self.top_mut().state.attempt_pop(&env) {
            self.controller.request_pop(1);
        }
    }

    fn apply_transaction(
        &mut self,
        renderer: &mut DewRenderer,
        transaction: NavigationTransaction,
    ) {
        let NavigationTransaction {
            id,
            retained_prefix,
            removed,
            inserted,
        } = transaction;
        assert_eq!(
            retained_prefix + removed + 1,
            self.entries.len(),
            "a dew navigation transaction must replace the stack's current suffix"
        );
        let env = self.env.clone();
        for mut entry in self.entries.drain(retained_prefix + 1..) {
            entry.state.disappeared(&env);
            entry.state.popped(&env);
        }
        if !inserted.is_empty() {
            // A push over a destination that is still there covers it; a
            // replace already reported the one it removed.
            if removed == 0 {
                self.top_mut().state.disappeared(&env);
            }
            for builder in inserted {
                let back = self.back_button(renderer);
                let entry = Entry::build(renderer, builder.build(), &env, Some(back));
                self.entries.push(entry);
            }
        }
        // Whatever ends up on top is now the active destination, whether a
        // pop uncovered it or a push created it.
        self.top_mut().state.appeared(&env);
        // Presentation is instantaneous, so the transaction is complete the
        // moment it is applied; nothing is left running to acknowledge later.
        let _ = self.controller.transition_completed(id);
    }
}

impl DewNode for StackNode {
    fn measure(&self, _state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        render_destination(self.entries.last_mut(), renderer, ctx);
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        // Everything a transaction sets off — a lifecycle handler that pushes
        // another destination, a path mutation — is drained by this same loop,
        // so one frame settles the whole cascade.
        self.pending.applying.set(true);
        self.resolve_back_request();
        let mut structural = false;
        loop {
            let next = self.pending.transactions.borrow_mut().pop_front();
            let Some(transaction) = next else { break };
            structural = true;
            self.apply_transaction(renderer, transaction);
        }
        self.pending.applying.set(false);
        // Only the visible destination is patched: the ones beneath it neither
        // measure nor draw this frame, so clearing their caches would be work
        // spent on a screen nobody can see.
        let top = self.top_mut();
        let changed = top.chrome.patch(renderer) | top.content.patch(renderer);
        changed | structural
    }
}

/// Draws one destination — its bar, its bottom bar, and its content — into
/// `ctx`.
fn render_destination(entry: Option<&mut Entry>, renderer: &mut DewRenderer, ctx: RenderContext) {
    let entry = entry.expect("a navigation stack always retains its root");
    let bounds = ctx.bounds;
    let bar = (!entry.chrome.hidden())
        .then(|| entry.chrome.layout(renderer.state_cell(), bounds.width()));
    let bottom = entry.chrome.bottom_layout(renderer.state_cell());

    let bar_height = bar.as_ref().map_or(0.0, |layout| layout.height);
    let bottom_height = bottom.as_ref().map_or(0.0, |(height, _)| *height);
    if let Some(layout) = &bar {
        let rect = Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y0 + bar_height);
        entry.chrome.render(renderer, ctx, rect, layout);
    }
    if let Some((height, sizes)) = &bottom {
        let rect = Rect::new(bounds.x0, bounds.y1 - height, bounds.x1, bounds.y1);
        entry.chrome.render_bottom(renderer, ctx, rect, sizes);
    }
    let content = LayoutRect::new(
        Point::new(to_f32(bounds.x0), to_f32(bounds.y0 + bar_height)),
        Size::new(
            to_f32(bounds.width()),
            to_f32((bounds.height() - bar_height - bottom_height).max(0.0)),
        ),
    );
    entry.content.render(renderer, ctx.child(content));
}

/// The retained node behind a [`NavigationView`] used on its own.
struct DestinationNode {
    entry: Entry,
}

/// Builds the node for a navigation view.
///
/// Inside a stack the surrounding stack owns the chrome, so the view
/// contributes its content alone; on its own it draws its own bar.
pub fn build_view(
    renderer: &mut DewRenderer,
    view: NavigationView,
    env: &Environment,
    depth: usize,
) -> Box<dyn DewNode> {
    if env.get::<NavigationController>().is_some() {
        return build_node(renderer, view.content, env, depth);
    }
    Box::new(DestinationNode {
        entry: Entry::build(renderer, view, env, None),
    })
}

impl DewNode for DestinationNode {
    fn measure(&self, _state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        render_destination(Some(&mut self.entry), renderer, ctx);
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.entry.chrome.patch(renderer) | self.entry.content.patch(renderer)
    }
}
