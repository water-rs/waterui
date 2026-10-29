use crate::renderer::bounded_proposal;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::{
    Edge, HydroNativeView, HydroState, RetainedSubview, WidgetRenderContext, measure_tabs_layout,
    tabs_button_rect, tabs_content_proposal,
};
use crate::theme::{TabItemGeometry, TabItemLayout};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Signal};
use waterui::navigation::tab::{NativeTabStyle, Tab, TabIcon, TabsLayout};
use waterui_controls::label::LabelDisplayMode;
use waterui_core::id::Id;
use waterui_core::layout::{LayoutDirection, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::views::{AnyViewsSnapshot, ViewSnapshot, Views};
use waterui_core::{AnyView, Environment, Native};

#[cfg(feature = "accessibility")]
use crate::widgets::util::widget_disabled;

/// The retained render state of one tab. Its `label` is a move-only `AnyView`, so
/// it is held as a [`RetainedSubview`] built once and re-flushed each frame; its
/// `content` is a cloneable `Rc`-backed builder rebuilt fresh each frame; `tag`
/// drives selection.
struct TabRenderState<E: crate::text::TextEngine> {
    tag: Id,
    label: RetainedSubview<E>,
    /// The label's icon, lifted out so the tab item can place it above the
    /// label (vertical) or beside it (horizontal).
    icon: Option<RetainedSubview<E>>,
    content: RetainedSubview<E>,
    enabled: nami::Computed<bool>,
    /// The reactive badge count drawn on the item; zero draws nothing.
    badge: Option<nami::Computed<i32>>,
}

impl<E: crate::text::TextEngine> TabRenderState<E> {
    fn from_tab(tag: Id, tab: Tab) -> Self {
        let (label, icon, content, badge, enabled, _) = tab.into_parts();
        Self {
            tag,
            label: RetainedSubview::new(label.build()),
            icon: icon.map(|icon| {
                RetainedSubview::new(match icon {
                    TabIcon::System(icon) => AnyView::new(icon),
                    TabIcon::View(builder) => builder.build(),
                })
            }),
            content: RetainedSubview::new(AnyView::new(content.build())),
            enabled,
            badge,
        }
    }
}

/// The retained render state of a `TabsLayout` container. The selection `Binding` and tab
/// native style are kept by value; each tab is a [`TabRenderState`].
///
/// `tabs` mirrors the layout's collection: a membership change the watcher
/// reports sets `tabs_dirty` and stores the event's snapshot; `reconcile`
/// then applies it, so a tab whose id survives keeps its retained sub-views —
/// navigation state, focus, in-flight gestures — while only inserted or
/// removed tabs materialize or drop theirs.
pub struct TabsRenderState<E: crate::text::TextEngine> {
    selection: Binding<Id>,
    style: NativeTabStyle,
    tabs: Vec<TabRenderState<E>>,
    /// The collection snapshot the last applied change reported — every
    /// reconcile works this membership, never the live collection.
    snapshot: Rc<RefCell<AnyViewsSnapshot<Tab>>>,
    /// A relocation [`reconcile`] decided but has not published: callers run
    /// it under a `borrow_mut` of the whole state, and writing `selection`
    /// there would let a user watcher re-enter the borrowed state. The id
    /// resolves reads immediately; the binding write is flushed after the
    /// borrow drops.
    pending_selection: Cell<Option<Id>>,
    tabs_dirty: Rc<Cell<bool>>,
    _watch: BoxWatcherGuard,
}

impl<E: crate::text::TextEngine> TabsRenderState<E> {
    pub(crate) fn from_tabs(tabs: TabsLayout, renderer: &crate::renderer::SemanticCore<E>) -> Self {
        let (selection, tabs, style, ..) = tabs.into_parts();
        let snapshot = Rc::new(RefCell::new(tabs.snapshot()));
        // Membership changes mark the owning widget's cell — the node whose
        // read built this state, or the window root when built unowned.
        let tabs_dirty = Rc::new(Cell::new(true));
        let cell = Rc::downgrade(
            &renderer
                .reader_cell()
                .unwrap_or_else(|| Rc::clone(renderer.root_cell())),
        );
        let watch = {
            let snapshot = Rc::clone(&snapshot);
            let tabs_dirty = Rc::clone(&tabs_dirty);
            tabs.watch(.., move |ctx, _change| {
                tabs_dirty.set(true);
                *snapshot.borrow_mut() = ctx.into_value();
                if let Some(cell) = cell.upgrade() {
                    cell.mark(crate::renderer::Dirty::STRUCTURE);
                }
            })
        };
        let mut state = Self {
            selection,
            style,
            tabs: Vec::new(),
            snapshot,
            pending_selection: Cell::new(None),
            tabs_dirty,
            _watch: watch,
        };
        state.reconcile();
        state.flush_pending_selection();
        state
    }

    /// Applies the membership change the watcher reported, when there is
    /// one: states are re-keyed by tab id into the snapshot's order — a
    /// surviving id keeps its [`RetainedSubview`]s untouched, an inserted id
    /// materializes, a removed one drops — and the selection relocates when
    /// the selected tab left the set.
    fn reconcile(&mut self) {
        if !self.tabs_dirty.replace(false) {
            return;
        }
        let snapshot = self.snapshot.borrow().clone();
        // A relocation a previous reconcile queued counts as selected until
        // the binding write is flushed — reads must not see the stale value.
        let selected = self
            .pending_selection
            .get()
            .unwrap_or_else(|| self.selection.snapshot());
        let old_index = self.tabs.iter().position(|tab| tab.tag == selected);
        let mut old: std::collections::BTreeMap<Id, TabRenderState<E>> =
            self.tabs.drain(..).map(|tab| (tab.tag, tab)).collect();
        let mut tabs = Vec::with_capacity(snapshot.len());
        for index in 0..snapshot.len() {
            let tag = snapshot
                .get_id(index)
                .expect("a collection snapshot reports an id for every index");
            let tab = old.remove(&tag).unwrap_or_else(|| {
                TabRenderState::from_tab(
                    tag,
                    snapshot
                        .get_view(index)
                        .expect("a collection snapshot reports a view for every index"),
                )
            });
            tabs.push(tab);
        }
        self.tabs = tabs;
        // The selected tab left the set: queue the tab now at its index —
        // the new last tab when it was trailing — for the post-borrow flush
        // to publish. An empty set leaves the binding untouched; a selection
        // that never named a tab keeps its existing read-time failure.
        if let Some(old_index) = old_index
            && !self.tabs.is_empty()
            && !self.tabs.iter().any(|tab| tab.tag == selected)
        {
            let index = old_index.min(self.tabs.len() - 1);
            self.pending_selection.set(Some(self.tabs[index].tag));
        }
    }

    /// Publishes a queued relocation to the selection binding. Call only
    /// while no `RefCell<TabsRenderState>` borrow is held — the write runs
    /// watchers synchronously, and a watcher mutating the collection must
    /// be free to re-enter the membership watch.
    pub(crate) fn flush_pending_selection(&self) {
        if let Some(id) = self.pending_selection.take() {
            self.selection.set(id);
        }
    }

    /// The `impl_widget_behavior` `prepare` hook: reconciles a pending
    /// membership change before measure and render touch `self.tabs`, and
    /// builds the inserted tabs' sub-views — the measure pass that follows
    /// has no renderer to build on.
    pub(crate) fn prepare_tabs(
        &mut self,
        renderer: &mut crate::renderer::HydrolysisRenderer<E>,
        env: &Environment,
    ) {
        self.reconcile();
        self.prebuild_labels(renderer, env);
    }

    /// Eagerly build the tab-label and icon sub-views (the measure path has
    /// no renderer to build on). The label builds title-only: the bar places
    /// the icon itself, so the label must not draw it a second time.
    pub(crate) fn prebuild_labels(
        &mut self,
        renderer: &mut crate::renderer::SemanticCore<E>,
        env: &Environment,
    ) {
        self.reconcile();
        let label_env = tab_label_env(env);
        for tab in &mut self.tabs {
            tab.label.ensure_built(renderer, &label_env);
            if let Some(icon) = &mut tab.icon {
                icon.ensure_built(renderer, env);
            }
            tab.content.ensure_built(renderer, env);
        }
    }

    /// The selected tab's index, or `None` when the set is empty. A
    /// selection naming no tab of a non-empty set fails as it always has.
    fn selected_index(&self, selected_id: Id) -> Option<usize> {
        if self.tabs.is_empty() {
            return None;
        }
        let selected_id = self.pending_selection.get().unwrap_or(selected_id);
        Some(
            self.tabs
                .iter()
                .position(|tab| tab.tag == selected_id)
                .unwrap_or_else(|| panic!("hydrolysis Tabs selection is not present in tabs")),
        )
    }

    /// Each tab's live badge count, read through `renderer` so a change
    /// schedules a frame. A tab without a badge reads zero.
    fn badge_counts(&self, renderer: &mut crate::renderer::SemanticCore<E>) -> Vec<i32> {
        self.tabs
            .iter()
            .map(|tab| {
                tab.badge
                    .as_ref()
                    .map_or(0, |badge| renderer.read_signal(badge))
            })
            .collect()
    }

    /// Consume every tab's label/icon/content layout-dirty marks: their
    /// measured sizes feed the bar's, so an enclosing retained sub-view
    /// must see them (water-rs/waterui#1957).
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        // No reconcile here: membership changes materialize sub-views, which
        // only `prepare_tabs` can build — this hook has no renderer. The
        // prepare pass applies the change before measure ever reads it.
        self.tabs.iter_mut().fold(false, |dirty, tab| {
            dirty
                | tab.label.take_layout_dirty_if_built()
                | tab
                    .icon
                    .as_mut()
                    .is_some_and(RetainedSubview::take_layout_dirty_if_built)
                | tab.content.take_layout_dirty_if_built()
        })
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<TabsLayout> {
    fn intrinsic(
        state: &mut HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        measure_tabs_layout(
            view.as_inner(),
            ProposalSize::UNSPECIFIED,
            state,
            env,
            theme,
        )
    }

    fn dimensions(
        state: &mut HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        ViewDimensions::new(measure_tabs_layout(
            view.as_inner(),
            proposal,
            state,
            env,
            theme,
        ))
    }
}

/// One tab's accessibility inputs: `(tag, interaction_key, default_label,
/// is_selected, badge_count)`; a zero badge count is not announced.
#[cfg(feature = "accessibility")]
type TabAccessibilityEntry = (
    Id,
    crate::renderer::InteractionKey,
    Option<String>,
    bool,
    i32,
);

/// Emits a tab list's accessibility tree from per-tab [`TabAccessibilityEntry`]
/// tuples. Shared by the dispatch path and the
/// retained `Widget`-node path (which extracts each default label from its
/// tab's [`RetainedSubview`]). `bar_rect` carries the bar's bounds-space
/// rect — the docked band [`WidgetRenderContext::chrome_splits`] carves out;
/// the semantic walk has no bounds and passes `None`.
#[cfg(feature = "accessibility")]
pub fn tabs_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    bar_rect: Option<kurbo::Rect>,
    selection: &Binding<Id>,
    style: NativeTabStyle,
    labels: &[TabAccessibilityEntry],
    env: &Environment,
) {
    let disabled = renderer.read_signal(&widget_disabled(env));
    let mut tab_list = AccessibilityNode::new(
        crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::TabList,
        ),
    );
    let tab_list_label = renderer.resolve_accessibility_label(env, None);
    if let Some(label) = tab_list_label {
        tab_list.set_label(label);
    }
    if let Some(value) = renderer.resolve_accessibility_value(env, None) {
        tab_list.set_value(value);
    }
    for (index, (tag, interaction_key, default_label, is_selected, badge)) in
        labels.iter().enumerate()
    {
        let mut tab_node = AccessibilityNode::new(
            crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::Tab,
            ),
        );
        let label = renderer.resolve_accessibility_label(env, default_label.clone());
        if let Some(label) = label {
            tab_node.set_label(label);
        }
        tab_node.set_selected(*is_selected);
        if *badge != 0 {
            tab_node.set_value(badge.to_string());
        }
        tab_node.add_action(AccessibilityAction::Focus);
        if disabled {
            tab_node.set_disabled();
        } else {
            tab_node.add_action(AccessibilityAction::Click);
        }
        let key = i64::from(i32::from(*tag));
        let target = (!disabled).then(|| AccessibilityActionTarget::PickerSelect {
            selection: selection.clone(),
            target: *tag,
        });
        let tab_node_id = match bar_rect {
            Some(bar_rect) => renderer.register_accessibility_child_node_with_key(
                key,
                tab_node,
                tabs_button_rect(bar_rect, labels.len(), index, style),
                env,
                target,
            ),
            None => renderer
                .register_accessibility_child_node_with_key_semantic(key, tab_node, env, target),
        };
        if let Some(tab_node_id) = tab_node_id {
            tab_list.push_child(tab_node_id);
            renderer.register_accessibility_focus_link(interaction_key, tab_node_id);
        }
    }
    match bar_rect {
        Some(bar_rect) => {
            // The bounds-space band resolves through the current placement
            // inside `register_accessibility_node`, the same anchor the
            // `tabs_button_rect` locals above resolve through.
            let _ = renderer.register_accessibility_node(tab_list, bar_rect, env, None);
        }
        None => {
            let _ = renderer.register_accessibility_node_semantic(tab_list, env, None);
        }
    }
}

/// Measures a retained tabs leaf from its [`TabsRenderState`]: each tab's
/// retained content answers the pane minus the tab bar (see
/// [`tabs_content_proposal`]) — a measurement proposal, not the rendered
/// content rect: the rendered content keeps the whole frame only once the
/// keyboard covers the band outright (keyboard deeper than container inset
/// plus bar height). Mirrors `measure_tabs_layout`.
pub fn measure_tabs_node<E: crate::text::TextEngine>(
    state: &TabsRenderState<E>,
    proposal: ProposalSize,
    hydro: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    let label_env = tab_label_env(env);
    let item_sizes: Vec<(LayoutSize, Option<LayoutSize>)> = state
        .tabs
        .iter()
        .map(|tab| {
            let label_size = tab.label.measure_built(hydro, &label_env, theme);
            let icon_size = tab
                .icon
                .as_ref()
                .map(|icon| icon.measure_built(hydro, env, theme));
            (label_size, icon_size)
        })
        .collect();
    // Decide the layout once from the bar's own extent so the measured bar
    // and the drawn bar answer the same layout (see `tabs_decide_layout`).
    let (layout, metrics) = tabs_decide_layout(
        theme,
        state.style,
        proposal.width.map(f64::from),
        &item_sizes,
    );
    let content_proposal = tabs_content_proposal(proposal, state.style, metrics.bar_height);
    let mut max_content_width: f64 = 0.0;
    let mut max_content_height: f64 = 0.0;
    let mut bar_width = 0.0;
    for (tab, (label_size, icon_size)) in state.tabs.iter().zip(item_sizes.iter()) {
        bar_width += tabs_item_natural_width(*label_size, *icon_size, &metrics, layout);

        let content_size =
            tab.content
                .measure_built_with_proposal(hydro, env, theme, content_proposal);
        max_content_width = max_content_width.max(f64::from(content_size.width));
        max_content_height = max_content_height.max(f64::from(content_size.height));
    }
    let (width, height) = match state.style {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => (
            max_content_width.max(bar_width),
            max_content_height + metrics.bar_height,
        ),
        NativeTabStyle::Sidebar => (
            max_content_width + metrics.bar_height,
            max_content_height
                .max(metrics.button_min_width * crate::num_cast::usize_as_f64(state.tabs.len())),
        ),
    };
    ViewDimensions::new(LayoutSize::new(
        proposal
            .width
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(width)),
        proposal
            .height
            .unwrap_or_else(|| crate::num_cast::f64_as_f32(height)),
    ))
}

/// Renders a retained tabs leaf every flush: emits the tab-list a11y (unless
/// hidden) then the bar + selected content, reading the selection signal live.
/// The docked split is derived once per flush — the a11y tree's bar node and
/// the painted bar land on the same band (§7.1).
pub fn render_tabs_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<TabsRenderState<E>>>,
    env: &Environment,
) {
    let style = state.borrow().style;
    // The bar's layout and metrics are decided once with the chrome
    // split: `render_tabs_parts` draws the bar the split carved, so the
    // layout it places items under is the one the band was derived from.
    let dock_edge = tabs_dock_edge(style, waterui_layout::layout_direction(env).snapshot());
    let (chrome, layout, theme_metrics) = {
        let theme = ctx.theme();
        let extent = tabs_bar_item_extent(ctx.bounds.width(), style, &theme);
        let layout = theme.tabs_item_layout(extent, state.borrow().tabs.len());
        let metrics = theme.tabs_metrics(layout);
        (
            ctx.chrome_splits([(dock_edge, metrics.bar_height)]),
            layout,
            metrics,
        )
    };
    #[cfg(feature = "accessibility")]
    {
        let [bar] = &chrome.bars;
        let hidden = env
            .get::<waterui::accessibility::AccessibilityHidden>()
            .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
        if !hidden {
            let selected_id = ctx.renderer_mut().read_signal(&state.borrow().selection);
            let (selection, labels, relocation) = {
                let mut st = state.borrow_mut();
                st.reconcile();
                st.prebuild_labels(ctx.renderer_mut(), env);
                let selected_index = st.selected_index(selected_id);
                let badges = st.badge_counts(ctx.renderer_mut());
                let labels: Vec<TabAccessibilityEntry> = st
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(index, tab)| {
                        (
                            tab.tag,
                            crate::renderer::InteractionKey::for_rc(
                                state,
                                crate::num_cast::i32_as_u32(i32::from(tab.tag)) as usize,
                            ),
                            tab.label.default_a11y_label(),
                            Some(index) == selected_index,
                            badges[index],
                        )
                    })
                    .collect();
                (st.selection.clone(), labels, st.pending_selection.take())
            };
            // The relocation publishes after the borrow drops — a watcher
            // mutating the collection re-enters the membership watch freely.
            if let Some(id) = relocation {
                selection.set(id);
            }
            tabs_accessibility(
                ctx.renderer_mut(),
                Some(bar.band),
                &selection,
                style,
                &labels,
                env,
            );
        }
    }
    render_tabs_parts(ctx, state, env, chrome, layout, theme_metrics, dock_edge);
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
fn render_tabs_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<TabsRenderState<E>>>,
    env: &Environment,
    chrome: crate::renderer::ChromeGroup<1>,
    layout: TabItemLayout,
    theme_metrics: crate::theme::TabsMetrics,
    dock_edge: Edge,
) {
    let (selection, style, tab_count, relocation) = {
        let mut st = state.borrow_mut();
        st.reconcile();
        st.prebuild_labels(ctx.renderer_mut(), env);
        (
            st.selection.clone(),
            st.style,
            st.tabs.len(),
            st.pending_selection.take(),
        )
    };
    // Publish the relocation outside the borrow — a selection watcher may
    // mutate the collection synchronously.
    if let Some(id) = relocation {
        selection.set(id);
    }
    let selected_id = ctx.renderer_mut().read_signal(&selection);
    let selected_index = state.borrow().selected_index(selected_id);

    // §7.1's chrome split — derived once by `render_tabs_node` together
    // with `layout` and `theme_metrics`: the bar docks to its edge clear
    // of the container region only, so the keyboard covers it instead of
    // lifting it, while the content keeps the laid-out frame — clear of
    // both regions — with the bar's edge docked on the band's inner edge.
    let [bar] = &chrome.bars;
    let (bar_rect, content_rect) = (bar.band, chrome.content);
    let label_env = tab_label_env(env);

    // §7.1 "Chrome": the bar's surface extends through the regions of the
    // edges it touches to the window edge — a leading-docked strip reaches
    // the top, leading and bottom edges, a bottom bar the leading, bottom
    // and trailing ones. The divider stays on the bar's inner edge — the
    // top row of a bottom bar, the trailing column of a sidebar strip —
    // spanning the surface's reach, so a strip's divider runs from the
    // window's top edge to its bottom one.
    let bar_surface = bar.surface(dock_edge);
    {
        let theme = ctx.theme();
        ctx.draw_context(|draw| {
            theme.draw_tabs_bar(&mut *draw, bar_surface);
            let divider = match style {
                NativeTabStyle::Sidebar => {
                    // One pixel on the band's inner edge — the right edge
                    // of a left-docked strip, the left edge of a
                    // right-docked one under right-to-left.
                    let inner = dock_edge.inner_edge(bar_rect);
                    match dock_edge {
                        Edge::Leading => kurbo::Rect::new(
                            (inner - 1.0).max(bar_rect.x0),
                            bar_surface.y0,
                            inner,
                            bar_surface.y1,
                        ),
                        _ => kurbo::Rect::new(
                            inner,
                            bar_surface.y0,
                            (inner + 1.0).min(bar_rect.x1),
                            bar_surface.y1,
                        ),
                    }
                }
                NativeTabStyle::Automatic | NativeTabStyle::TabBar => kurbo::Rect::new(
                    bar_surface.x0,
                    bar_rect.y0,
                    bar_surface.x1,
                    (bar_rect.y0 + 1.0).min(bar_rect.y1),
                ),
            };
            theme.draw_tabs_bar_divider(&mut *draw, divider);
        });
    }

    for index in 0..tab_count {
        let button_rect = tabs_button_rect(bar_rect, tab_count, index, style);
        let tab_id = state.borrow().tabs[index].tag;
        let interaction_key = crate::renderer::InteractionKey::for_rc(
            state,
            crate::num_cast::i32_as_u32(i32::from(tab_id)) as usize,
        );
        // The label and icon sub-views are prebuilt (node path:
        // `prebuild_labels`; dispatch path: `render` calls `prebuild_labels`),
        // so measure them directly for placement.
        let (label_size, icon_size) = {
            let cell = state.borrow();
            let theme = ctx.theme();
            let label_size =
                cell.tabs[index]
                    .label
                    .measure_built(ctx.state_mut(), &label_env, &theme);
            let icon_size = cell.tabs[index]
                .icon
                .as_ref()
                .map(|icon| icon.measure_built(ctx.state_mut(), env, &theme));
            (label_size, icon_size)
        };
        let (icon_rect, label_rect) =
            tabs_item_content_rects(button_rect, icon_size, label_size, &theme_metrics, layout);
        {
            let hit_transform = ctx.renderer_mut().current_hit_transform();
            let hit_bounds = button_rect;
            let (interaction, press_slot, _) =
                ctx.renderer_mut()
                    .bind_interaction_target(interaction_key, hit_bounds, env);
            let interaction = crate::renderer::local_interaction_state(interaction, hit_transform);
            let is_selected = Some(index) == selected_index;
            // A horizontal item's indicator and state layer hug the icon+label
            // content grown by the button inset, not the whole button share;
            // vertical items keep the button-wide bounds. The hit target stays
            // the full button rect either way.
            let chrome_bounds = match layout {
                TabItemLayout::Horizontal => {
                    let content_x0 =
                        icon_rect.map_or(label_rect.x0, |rect| rect.x0.min(label_rect.x0));
                    let content_x1 =
                        icon_rect.map_or(label_rect.x1, |rect| rect.x1.max(label_rect.x1));
                    let inset = theme_metrics.button_horizontal_inset;
                    kurbo::Rect::new(
                        (content_x0 - inset).max(button_rect.x0),
                        button_rect.y0,
                        (content_x1 + inset).min(button_rect.x1),
                        button_rect.y1,
                    )
                }
                TabItemLayout::Vertical => button_rect,
            };
            {
                let theme = ctx.theme();
                let item = TabItemGeometry {
                    bounds: chrome_bounds,
                    icon: icon_rect,
                    title: label_rect,
                    layout,
                    placement: style,
                };
                ctx.draw_context(|draw| {
                    if is_selected {
                        theme.draw_tabs_highlight(&mut *draw, item);
                    }
                    theme.draw_tabs_button_state_layer(&mut *draw, item, is_selected, interaction);
                });
            }
            let selection_binding = selection.clone();
            let enabled = {
                let st = state.borrow();
                ctx.renderer_mut().read_signal(&st.tabs[index].enabled)
            };
            if enabled {
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, _env| {
                        if selection_binding.snapshot() != tab_id {
                            selection_binding.set(tab_id);
                        }
                        true
                    },
                );
            }
        }
        let has_label = label_rect.width() > 0.0 && label_rect.height() > 0.0;
        let badge = {
            let badge = state.borrow().tabs[index].badge.clone();
            badge.map_or(0, |badge| ctx.renderer_mut().read_signal(&badge))
        };
        if icon_rect.is_some() || has_label {
            // The tab item's a11y is emitted by `tabs_accessibility`, so suppress
            // the sub-views' own a11y (matching the dispatch path's
            // `dispatch_in_rect_without_accessibility`).
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().push_accessibility_suppression();
            // A tab gets an equal share of the bar and no more. Without this a
            // long label drew straight over its neighbour and off the edge of
            // the bar, since the label lays out at its natural width.
            ctx.with_scope(
                crate::renderer::mount::ScopeKey {
                    role: "tab",
                    item: index as u64,
                },
                1.0,
                button_rect,
                |ctx| {
                    let render_ctx = ctx.render_context();
                    let mut st = state.borrow_mut();
                    // The icon draws whether or not the label has text to show.
                    if let (Some(icon), Some(icon_rect)) = (&mut st.tabs[index].icon, icon_rect) {
                        let icon_area = bar.area_for(icon_rect);
                        icon.place(
                            ctx.renderer_mut(),
                            render_ctx,
                            env,
                            ProposalSize::UNSPECIFIED,
                            icon_rect,
                            icon_area,
                        );
                    }
                    if has_label {
                        let label_area = bar.area_for(label_rect);
                        st.tabs[index].label.place(
                            ctx.renderer_mut(),
                            render_ctx,
                            &label_env,
                            ProposalSize::UNSPECIFIED,
                            label_rect,
                            label_area,
                        );
                    }
                    drop(st);
                    // The count anchors on the icon, on the title without one.
                    if badge != 0 {
                        let anchor = icon_rect.unwrap_or(label_rect);
                        crate::widgets::layout::badge::draw_badge_indicator(
                            ctx, anchor, badge, env,
                        );
                    }
                },
            );
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().pop_accessibility_suppression();
        }
    }

    if let Some(selected_index) = selected_index
        && content_rect.width() > 0.0
        && content_rect.height() > 0.0
    {
        let mut st = state.borrow_mut();
        let render_ctx = ctx.render_context();
        // §7.1: tab content is chrome-hosted — it inherits the widget's
        // boundaries on the edges the tab bar leaves reachable, with the
        // bar's edge docked on the band's inner edge.
        let content_area = chrome.content_area;
        st.tabs[selected_index].content.place(
            ctx.renderer_mut(),
            render_ctx,
            env,
            bounded_proposal(content_rect),
            content_rect,
            content_area,
        );
    }
}

/// The environment a tab's label builds under: the bar owns the icon's
/// placement, so the label always renders its title alone.
fn tab_label_env(env: &Environment) -> Environment {
    env.extending(LabelDisplayMode::TitleOnly)
}

/// The edge a style's tab bar docks to — the bottom edge for
/// `Automatic`/`TabBar`, the *leading* edge for a `Sidebar` strip (§7.1's
/// chrome split docks a bar to the edge it touches). `Edge`'s variants
/// are physical, so the leading dock resolves under `direction` — a
/// sidebar lands on the right edge under right-to-left
/// (water-rs/waterui#1827).
const fn tabs_dock_edge(style: NativeTabStyle, direction: LayoutDirection) -> Edge {
    match style {
        NativeTabStyle::Sidebar => {
            if direction.is_right_to_left() {
                Edge::Trailing
            } else {
                Edge::Leading
            }
        }
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => Edge::Bottom,
    }
}

/// The extent a style's tab bar reports to `WidgetTheme::tabs_item_layout`
/// when the container is `container_width` wide: a bottom bar's extent is the
/// container's width; a sidebar strip reports its own thickness — the
/// vertical metrics' `bar_height` — which never widens into the pane.
pub fn tabs_bar_item_extent(
    container_width: f64,
    style: NativeTabStyle,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> f64 {
    match style {
        NativeTabStyle::Sidebar => theme.tabs_metrics(TabItemLayout::Vertical).bar_height,
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => container_width,
    }
}

/// Decides the bar's item layout and metrics once for a measure pass: the
/// theme answers `tabs_item_layout` from the bar's own extent (see
/// [`tabs_bar_item_extent`]) computed from `item_sizes`, so the bar this pass
/// measures and the one the render pass draws under the same bounds agree.
#[expect(
    clippy::option_if_let_else,
    reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
)]
pub fn tabs_decide_layout(
    theme: &Rc<dyn crate::theme::WidgetTheme>,
    style: NativeTabStyle,
    proposed_width: Option<f64>,
    item_sizes: &[(LayoutSize, Option<LayoutSize>)],
) -> (TabItemLayout, crate::theme::TabsMetrics) {
    let extent = match proposed_width {
        Some(width) => tabs_bar_item_extent(width, style, theme),
        None => {
            if style == NativeTabStyle::Sidebar {
                tabs_bar_item_extent(0.0, style, theme)
            } else {
                let vertical_metrics = theme.tabs_metrics(TabItemLayout::Vertical);
                item_sizes
                    .iter()
                    .map(|(label_size, icon_size)| {
                        tabs_item_natural_width(
                            *label_size,
                            *icon_size,
                            &vertical_metrics,
                            TabItemLayout::Vertical,
                        )
                    })
                    .sum()
            }
        }
    };
    let layout = theme.tabs_item_layout(extent, item_sizes.len());
    (layout, theme.tabs_metrics(layout))
}

/// A tab item's natural width for the bar's item layout: icon above the label
/// takes the wider of the two; icon beside the label adds them.
pub fn tabs_item_natural_width(
    label_size: LayoutSize,
    icon_size: Option<LayoutSize>,
    metrics: &crate::theme::TabsMetrics,
    layout: TabItemLayout,
) -> f64 {
    let content_width = match (layout, icon_size) {
        (TabItemLayout::Horizontal, Some(icon_size)) => {
            f64::from(icon_size.width) + metrics.icon_label_spacing + f64::from(label_size.width)
        }
        (_, Some(icon_size)) => f64::from(label_size.width).max(f64::from(icon_size.width)),
        (_, None) => f64::from(label_size.width),
    };
    metrics
        .button_horizontal_inset
        .mul_add(2.0, content_width)
        .max(metrics.button_min_width)
}

/// Places a tab item's icon and label inside its button rect. Vertical stacks
/// the icon above the label; horizontal puts the icon beside the label.
fn tabs_item_content_rects(
    button_rect: kurbo::Rect,
    icon_size: Option<LayoutSize>,
    label_size: LayoutSize,
    metrics: &crate::theme::TabsMetrics,
    layout: TabItemLayout,
) -> (Option<kurbo::Rect>, kurbo::Rect) {
    let Some(icon_size) = icon_size else {
        return (None, tabs_label_rect(button_rect, label_size, metrics));
    };
    let max_width = metrics
        .button_horizontal_inset
        .mul_add(-2.0, button_rect.width())
        .max(0.0);
    match layout {
        TabItemLayout::Vertical => {
            let icon_width = f64::from(icon_size.width).min(max_width);
            let icon_height = f64::from(icon_size.height).min(button_rect.height());
            let label_width = f64::from(label_size.width).min(max_width);
            let label_height = f64::from(label_size.height)
                .min((button_rect.height() - icon_height - metrics.icon_label_spacing).max(0.0));
            let total_height = icon_height + metrics.icon_label_spacing + label_height;
            let y0 = (button_rect.height() - total_height)
                .max(0.0)
                .mul_add(0.5, button_rect.y0);
            (
                Some(kurbo::Rect::new(
                    (button_rect.width() - icon_width).mul_add(0.5, button_rect.x0),
                    y0,
                    button_rect.x0 + f64::midpoint(button_rect.width(), icon_width),
                    y0 + icon_height,
                )),
                kurbo::Rect::new(
                    (button_rect.width() - label_width).mul_add(0.5, button_rect.x0),
                    y0 + icon_height + metrics.icon_label_spacing,
                    button_rect.x0 + f64::midpoint(button_rect.width(), label_width),
                    y0 + icon_height + metrics.icon_label_spacing + label_height,
                ),
            )
        }
        TabItemLayout::Horizontal => {
            let icon_width = f64::from(icon_size.width).min(max_width);
            let label_width = f64::from(label_size.width)
                .min((max_width - icon_width - metrics.icon_label_spacing).max(0.0));
            let icon_height = f64::from(icon_size.height).min(button_rect.height());
            let label_height = f64::from(label_size.height).min(button_rect.height());
            let total_width = icon_width + metrics.icon_label_spacing + label_width;
            let x0 = (button_rect.width() - total_width)
                .max(0.0)
                .mul_add(0.5, button_rect.x0);
            (
                Some(kurbo::Rect::new(
                    x0,
                    (button_rect.height() - icon_height).mul_add(0.5, button_rect.y0),
                    x0 + icon_width,
                    button_rect.y0 + f64::midpoint(button_rect.height(), icon_height),
                )),
                kurbo::Rect::new(
                    x0 + icon_width + metrics.icon_label_spacing,
                    (button_rect.height() - label_height).mul_add(0.5, button_rect.y0),
                    x0 + icon_width + metrics.icon_label_spacing + label_width,
                    button_rect.y0 + f64::midpoint(button_rect.height(), label_height),
                ),
            )
        }
    }
}

fn tabs_label_rect(
    button_rect: kurbo::Rect,
    label_size: waterui_core::layout::Size,
    metrics: &crate::theme::TabsMetrics,
) -> kurbo::Rect {
    let max_width = metrics
        .button_horizontal_inset
        .mul_add(-2.0, button_rect.width())
        .max(0.0);
    let width = f64::from(label_size.width).min(max_width);
    let height = f64::from(label_size.height).min(button_rect.height());
    let x0 = (button_rect.width() - width).mul_add(0.5, button_rect.x0);
    let y0 = (button_rect.height() - height).mul_add(0.5, button_rect.y0);
    kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}

/// Emits a retained tabs layout's accessibility tree for the semantic walk:
/// the tab bar and tab nodes `tabs_accessibility` registers (labels are
/// suppressed in the sub-view flush, so they emit nothing themselves), then
/// the selected tab's content subtree — it flushes unsuppressed in the
/// rendered path, so it emits its own nodes here too.
#[cfg(feature = "accessibility")]
pub fn emit_tabs_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<TabsRenderState<E>>>,
    env: &Environment,
) {
    let owner = state;
    let relocation = {
        let mut st = state.borrow_mut();
        st.reconcile();
        st.prebuild_labels(renderer, env);
        st.pending_selection.take()
    };
    // Publish outside the borrow so a watcher mutating the collection can
    // re-enter the membership watch.
    if let Some(id) = relocation {
        let selection = state.borrow().selection.clone();
        selection.set(id);
    }
    let mut state = state.borrow_mut();
    let selected_id = renderer.read_signal(&state.selection);
    let (selection, style, labels) = {
        let selected_index = state.selected_index(selected_id);
        let badges = state.badge_counts(renderer);
        let labels: Vec<TabAccessibilityEntry> = state
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                (
                    tab.tag,
                    crate::renderer::InteractionKey::for_rc(
                        owner,
                        crate::num_cast::i32_as_u32(i32::from(tab.tag)) as usize,
                    ),
                    tab.label.default_a11y_label(),
                    Some(index) == selected_index,
                    badges[index],
                )
            })
            .collect();
        (state.selection.clone(), state.style, labels)
    };
    tabs_accessibility(renderer, None, &selection, style, &labels, env);
    if let Some(selected_index) = state.selected_index(selected_id) {
        state.tabs[selected_index]
            .content
            .emit_accessibility(renderer, env);
    }
}
