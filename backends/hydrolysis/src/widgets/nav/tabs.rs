use crate::renderer::bounded_proposal;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::num_cast::usize_as_f64;
#[cfg(feature = "accessibility")]
use crate::renderer::AccessibilityActionTarget;
use crate::renderer::HeldChild;
use crate::renderer::live_tabs::{LiveTabs, LiveTabsParts, TabsFrames};
use crate::renderer::{
    ChromeBar, Edge, HydroState, HydrolysisRenderer, RetainedSubview, SemanticCore, Track,
    TrackAction, WidgetRenderContext, tabs_button_rect, tabs_content_proposal,
};
use crate::text::TextEngine;
use crate::theme::{
    TabBarWidth, TabItemGeometry, TabItemLayout, TabsLook, TabsMetrics, TabsSpec, WidgetTheme,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use kurbo::{Point, Rect, Vec2};
use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Signal, SignalExt as _};
use waterui::component::badge::BadgeValue;
use waterui::navigation::tab::{NativeTabStyle, TabItem, TabsLayout};
use waterui::theme::ForegroundOverride;
use waterui::theme::color::Foreground;
use waterui_controls::label::LabelDisplayMode;
use waterui_core::Environment;
use waterui_core::contract_violation;
use waterui_core::id::Id;
use waterui_core::layout::{LayoutDirection, ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::plugin::Plugin;
use waterui_core::views::AnyViewsSnapshot;
use waterui_graphics::color::{Color, signal_color};

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
    /// The reactive badge value drawn on the item.
    badge: Option<nami::Computed<BadgeValue>>,
}

impl<E: crate::text::TextEngine> TabRenderState<E> {
    fn from_tab(tag: Id, tab: TabItem<HeldChild<E>>) -> Self {
        let (label, icon, content, badge, enabled, _) = tab.into_parts();
        Self {
            tag,
            label: RetainedSubview::held(label.build()),
            icon: icon.map(|slot| RetainedSubview::held(slot.build())),
            content: RetainedSubview::held(content.build()),
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
    snapshot: Rc<RefCell<AnyViewsSnapshot<TabItem<HeldChild<E>>>>>,
    /// A relocation [`reconcile`] decided but has not published: callers run
    /// it under a `borrow_mut` of the whole state, and writing `selection`
    /// there would let a user watcher re-enter the borrowed state. The id
    /// resolves reads immediately; the binding write is flushed after the
    /// borrow drops.
    pending_selection: Cell<Option<Id>>,
    tabs_dirty: Rc<Cell<bool>>,
    /// The look the theme gave for the container's spec, held while the
    /// spec stays the same.
    look: Option<(TabsSpec, TabsLook)>,
    /// The parts recorded from the look, held while the look, the frames
    /// and the tabs stay the same.
    parts: Option<LiveTabsParts>,
    _watch: BoxWatcherGuard,
}

impl<E: crate::text::TextEngine> TabsRenderState<E> {
    pub(crate) fn from_tabs(
        tabs: TabsLayout<HeldChild<E>>,
        renderer: &crate::renderer::SemanticCore<E>,
    ) -> Self {
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
            look: None,
            parts: None,
            _watch: watch,
        };
        state.reconcile();
        state.flush_pending_selection();
        state
    }

    /// The container's spec.
    const fn spec(&self) -> TabsSpec {
        TabsSpec::new(self.style)
    }

    /// The look for the container's spec in `env`: the one already held
    /// when the spec is unchanged, otherwise requested again, which drops
    /// the parts recorded from the old one.
    pub(crate) fn look(&mut self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> TabsLook {
        let spec = self.spec();
        if self.held(spec).is_none() {
            self.parts = None;
            self.look = Some((spec, theme.tabs(&spec, env)));
        }
        self.held(spec).expect("the look was just ensured").clone()
    }

    fn held(&self, spec: TabsSpec) -> Option<&TabsLook> {
        self.look
            .as_ref()
            .filter(|(held, _)| *held == spec)
            .map(|(_, look)| look)
    }

    /// The look for the container's spec in `env` without holding it: the
    /// held one, or a fresh one for a measurement ahead of the render that
    /// holds it.
    fn peek(&self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> TabsLook {
        let spec = self.spec();
        self.held(spec)
            .cloned()
            .unwrap_or_else(|| theme.tabs(&spec, env))
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
            let Some(tag) = snapshot.get_id(index) else {
                contract_violation!(
                    "hydrolysis Tabs: tab {index} has no id although it is in the snapshot's range; the tab is skipped"
                );
                continue;
            };
            let tab = if let Some(tab) = old.remove(&tag) {
                tab
            } else {
                let Some(view) = snapshot.get_view(index) else {
                    contract_violation!(
                        "hydrolysis Tabs: tab {index} failed to materialize although it is in the snapshot's range; the tab is skipped"
                    );
                    continue;
                };
                TabRenderState::from_tab(tag, view)
            };
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
    pub(crate) fn prepare_tabs(&mut self, renderer: &mut HydrolysisRenderer<E>, env: &Environment) {
        self.reconcile();
        let theme = renderer.theme();
        let look = self.look(&theme, env);
        self.prebuild_labels_for_look(renderer, env, Some(&look));
    }

    /// Eagerly build each tab's content sub-view during node construction.
    /// Labels and icons depend on `TabsLook` foreground overrides and wait for
    /// [`Self::prepare_tabs`] (or [`Self::prebuild_labels`] in semantic-only
    /// mode).
    pub(crate) fn prebuild_contents(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        self.reconcile();
        for tab in &mut self.tabs {
            tab.content.ensure_built(renderer, env);
        }
    }

    /// Eagerly build the tab-label and icon sub-views (the measure path has
    /// no renderer to build on). The label builds title-only: the bar places
    /// the icon itself, so the label must not draw it a second time.
    pub(crate) fn prebuild_labels(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        self.reconcile();
        let look = self.held(self.spec()).cloned();
        self.prebuild_labels_for_look(renderer, env, look.as_ref());
    }

    pub(crate) fn prebuild_labels_for_look(
        &mut self,
        renderer: &mut SemanticCore<E>,
        env: &Environment,
        look: Option<&TabsLook>,
    ) {
        self.reconcile();
        for tab in &mut self.tabs {
            let item_env = tab_reactive_env(tab.tag, &self.selection, look, env);
            let label_env = tab_label_env(&item_env);
            tab.label.ensure_built(renderer, &label_env);
            if let Some(icon) = &mut tab.icon {
                icon.ensure_built(renderer, &item_env);
            }
            tab.content.ensure_built(renderer, env);
        }
    }

    /// The selected tab's index, or `None` when the set is empty. A
    /// selection naming no tab of a non-empty set is a contract violation by
    /// the app, reported; no tab is then selected (`None`).
    fn selected_index(&self, selected_id: Id) -> Option<usize> {
        if self.tabs.is_empty() {
            return None;
        }
        let selected_id = self.pending_selection.get().unwrap_or(selected_id);
        let index = self.tabs.iter().position(|tab| tab.tag == selected_id);
        if index.is_none() {
            contract_violation!(
                "hydrolysis Tabs selection is not present in tabs; no tab is selected"
            );
        }
        index
    }

    /// Each tab's live accessible badge text, read through `renderer` so a
    /// change schedules a frame. Hidden and dot badges report `None`.
    #[cfg(feature = "accessibility")]
    fn badge_values(
        &self,
        renderer: &mut crate::renderer::SemanticCore<E>,
        env: &Environment,
    ) -> Vec<Option<String>> {
        self.tabs
            .iter()
            .map(|tab| {
                tab.badge.as_ref().and_then(|badge| {
                    let value = renderer.read_signal(badge);
                    crate::widgets::layout::badge::badge_accessibility_text(&value, renderer, env)
                })
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

/// One tab's accessibility inputs: `(tag, interaction_key, default_label,
/// is_selected, badge_text)`.
#[cfg(feature = "accessibility")]
type TabAccessibilityEntry = (
    Id,
    crate::renderer::InteractionKey,
    Option<String>,
    bool,
    Option<String>,
);

/// Emits a tab list's accessibility tree from per-tab [`TabAccessibilityEntry`]
/// tuples. Shared by the dispatch path and the
/// retained `Widget`-node path (which extracts each default label from its
/// tab's [`RetainedSubview`]). `bar_rect` carries the bar's bounds-space
/// rect — the docked band [`WidgetRenderContext::chrome_splits`] carves out;
/// the semantic walk has no bounds and passes `None`.
#[cfg(feature = "accessibility")]
pub fn tabs_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    bar_rects: Option<(Rect, Rect)>,
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
        if let Some(badge) = badge {
            tab_node.set_value(badge.clone());
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
        let tab_node_id = match bar_rects {
            Some((_, items_rect)) => renderer.register_accessibility_child_node_with_key(
                key,
                tab_node,
                tabs_button_rect(items_rect, labels.len(), index, style),
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
    match bar_rects {
        Some((bar_rect, _)) => {
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
    // and the drawn bar answer the same layout (see `tabs_decide_placement`).
    let look = state.peek(theme, env);
    let margin = bottom_floating_margin(&look, state.style);
    let (layout, metrics, compact_width) = tabs_decide_placement(
        &look,
        state.style,
        proposal.width.map(f64::from),
        &item_sizes,
    );
    let content_proposal =
        tabs_content_proposal(proposal, state.style, metrics.bar_height + margin);
    let mut max_content_width: f64 = 0.0;
    let mut max_content_height: f64 = 0.0;
    let mut natural_bar_width = 0.0;
    for (tab, (label_size, icon_size)) in state.tabs.iter().zip(item_sizes.iter()) {
        natural_bar_width += tabs_item_natural_width(*label_size, *icon_size, &metrics, layout);

        let content_size =
            tab.content
                .measure_built_with_proposal(hydro, env, theme, content_proposal);
        max_content_width = max_content_width.max(f64::from(content_size.width));
        max_content_height = max_content_height.max(f64::from(content_size.height));
    }
    let bar_width = margin.mul_add(2.0, compact_width.unwrap_or(natural_bar_width));
    let (width, height) = match state.style {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => (
            max_content_width.max(bar_width),
            max_content_height + metrics.bar_height + margin,
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
    let theme = ctx.theme();
    let look = state.borrow_mut().look(&theme, env);
    let (layout, theme_metrics, compact_width, margin) =
        resolve_tabs_layout(ctx, state, env, &theme, &look, style);
    let chrome = ctx.chrome_splits([(dock_edge, theme_metrics.bar_height + margin)]);
    let bar_geometry = tabs_bar_geometry(
        &chrome.bars[0],
        &look,
        style,
        dock_edge,
        &theme_metrics,
        compact_width,
    );
    #[cfg(feature = "accessibility")]
    {
        let hidden = env
            .get::<waterui::accessibility::AccessibilityHidden>()
            .is_some_and(waterui::accessibility::AccessibilityHidden::is_hidden);
        if !hidden {
            let selected_id = ctx.renderer_mut().read_signal(&state.borrow().selection);
            let (selection, labels, relocation) = {
                let mut st = state.borrow_mut();
                st.reconcile();
                st.prebuild_labels_for_look(ctx.renderer_mut(), env, Some(&look));
                let selected_index = st.selected_index(selected_id);
                let badges = st.badge_values(ctx.renderer_mut(), env);
                let renderer = ctx.renderer_mut();
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
                            tab.label.accessibility_label(renderer),
                            Some(index) == selected_index,
                            badges[index].clone(),
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
                Some((bar_geometry.bar_rect, bar_geometry.items_rect)),
                &selection,
                style,
                &labels,
                env,
            );
        }
    }
    render_tabs_parts(
        ctx,
        state,
        env,
        TabsPlacement {
            chrome,
            bar_geometry,
            look,
            layout,
            metrics: theme_metrics,
        },
    );
}

/// Concrete placement rectangles derived for the tab bar chrome split.
#[derive(Clone, Copy)]
struct TabsBarGeometry {
    bar_rect: Rect,
    items_rect: Rect,
    bar_surface: Rect,
    divider: Rect,
}

/// How a render places a tab container: the chrome split its bar was carved
/// from, its bar geometry, its look, and the item layout and measures decided
/// with that split.
struct TabsPlacement {
    chrome: crate::renderer::ChromeGroup<1>,
    bar_geometry: TabsBarGeometry,
    look: TabsLook,
    layout: TabItemLayout,
    metrics: TabsMetrics,
}

/// One tab item as a render places it: its button, where its icon and label
/// lie and the geometry its parts follow.
struct ItemPlan {
    button: kurbo::Rect,
    icon: Option<kurbo::Rect>,
    label: kurbo::Rect,
    geometry: TabItemGeometry,
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
fn render_tabs_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<TabsRenderState<E>>>,
    env: &Environment,
    placement: TabsPlacement,
) {
    let TabsPlacement {
        chrome,
        bar_geometry,
        look,
        layout,
        metrics: theme_metrics,
    } = placement;
    let (selection, style, tab_count, relocation) = {
        let mut st = state.borrow_mut();
        st.reconcile();
        st.prebuild_labels_for_look(ctx.renderer_mut(), env, Some(&look));
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
    // with `layout`, `theme_metrics`, and `bar_geometry`.
    let [bar] = &chrome.bars;
    let content_rect = chrome.content;
    let measure_label_env = tab_label_env(env);
    let (bar_surface, divider, items_rect) = (
        bar_geometry.bar_surface,
        bar_geometry.divider,
        bar_geometry.items_rect,
    );

    // Each item's rects and geometry first: the look's parts are realized
    // from all of them together, and recorded once while they hold.
    let plans: Vec<ItemPlan> = (0..tab_count)
        .map(|index| {
            let button = tabs_button_rect(items_rect, tab_count, index, style);
            // The label and icon sub-views are prebuilt,
            // so measure them directly for placement.
            let (label_size, icon_size) = {
                let cell = state.borrow();
                let theme = ctx.theme();
                let label_size = cell.tabs[index].label.measure_built(
                    ctx.state_mut(),
                    &measure_label_env,
                    &theme,
                );
                let icon_size = cell.tabs[index]
                    .icon
                    .as_ref()
                    .map(|icon| icon.measure_built(ctx.state_mut(), env, &theme));
                (label_size, icon_size)
            };
            let (icon, label) =
                tabs_item_content_rects(button, icon_size, label_size, &theme_metrics, layout);
            // A horizontal item's indicator and state layer hug the icon+label
            // content grown by the button inset, not the whole button share;
            // vertical items keep the button-wide bounds. The hit target stays
            // the full button rect either way.
            let bounds = match layout {
                TabItemLayout::Horizontal => {
                    let content_x0 = icon.map_or(label.x0, |rect| rect.x0.min(label.x0));
                    let content_x1 = icon.map_or(label.x1, |rect| rect.x1.max(label.x1));
                    let inset = theme_metrics.button_horizontal_inset;
                    kurbo::Rect::new(
                        (content_x0 - inset).max(button.x0),
                        button.y0,
                        (content_x1 + inset).min(button.x1),
                        button.y1,
                    )
                }
                TabItemLayout::Vertical => button,
            };
            ItemPlan {
                button,
                icon,
                label,
                geometry: TabItemGeometry {
                    bounds,
                    icon,
                    title: label,
                    layout,
                    placement: style,
                },
            }
        })
        .collect();
    let tags: Vec<Id> = state.borrow().tabs.iter().map(|tab| tab.tag).collect();
    let frames = TabsFrames {
        surface: bar_surface,
        divider,
        items: plans.iter().map(|plan| plan.geometry).collect(),
    };
    let parts = LiveTabsParts::ensure(
        &mut state.borrow_mut().parts,
        &look,
        frames,
        &tags,
        selected_index,
        &selection,
        env,
    )
    .clone();
    let live = Rc::clone(parts.tabs());
    // Each item binds its own interaction on its own touch target, and its
    // chrome follows what the handles report.
    let targets: Vec<_> = plans
        .iter()
        .zip(&tags)
        .enumerate()
        .map(|(index, (plan, tag))| {
            let key = crate::renderer::InteractionKey::for_rc(
                state,
                crate::num_cast::i32_as_u32(i32::from(*tag)) as usize,
            );
            let item_chrome = live.chrome(index);
            let target = item_chrome
                .get_hit()
                .target(plan.button, item_chrome.frame(plan.geometry.bounds));
            let (_, press_slot, handles) = ctx.renderer_mut().bind_live_chrome_target(
                key.clone(),
                target,
                env,
                false,
                item_chrome,
            );
            let flags = ctx.renderer_mut().reported_interaction_state(&key);
            live.sync(index, flags);
            handles.attach_live_tab(Rc::clone(&live), index);
            (target, press_slot)
        })
        .collect();
    let render_ctx = ctx.render_context();
    parts.push(ctx.renderer_mut(), render_ctx);

    for (index, (plan, (target, press_slot))) in plans.iter().zip(targets).enumerate() {
        let (button_rect, icon_rect, label_rect) = (plan.button, plan.icon, plan.label);
        let tab_id = tags[index];
        {
            let selection_binding = selection.clone();
            let enabled = {
                let st = state.borrow();
                ctx.renderer_mut().read_signal(&st.tabs[index].enabled)
            };
            if enabled {
                if live.drags() {
                    let slop = press_slot.slop;
                    let track = tab_track(
                        Rc::clone(&live),
                        index,
                        tab_id,
                        style,
                        selection_binding.clone(),
                        slop,
                    );
                    ctx.renderer_mut().register_tracked_pointer_target(
                        target,
                        press_slot,
                        track,
                        move |_renderer, _point, _env| {
                            if selection_binding.snapshot() != tab_id {
                                selection_binding.set(tab_id);
                            }
                            true
                        },
                    );
                } else {
                    ctx.renderer_mut().register_interactive_pointer_target(
                        target,
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
        }
        let has_label = label_rect.width() > 0.0 && label_rect.height() > 0.0;
        let badge = {
            let badge = state.borrow().tabs[index].badge.clone();
            badge.map_or(BadgeValue::Hidden, |badge| {
                ctx.renderer_mut().read_signal(&badge)
            })
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
                    let item_env = tab_item_env(env, &look, Some(index) == selected_index);
                    let item_label_env = tab_label_env(&item_env);
                    let mut st = state.borrow_mut();
                    // The icon draws whether or not the label has text to show.
                    if let (Some(icon), Some(icon_rect)) = (&mut st.tabs[index].icon, icon_rect) {
                        let icon_area = bar.area_for(icon_rect);
                        icon.place(
                            ctx.renderer_mut(),
                            render_ctx,
                            &item_env,
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
                            &item_label_env,
                            ProposalSize::UNSPECIFIED,
                            label_rect,
                            label_area,
                        );
                    }
                    drop(st);
                    // The indicator anchors on the icon, on the title without one.
                    if !badge.is_hidden() {
                        let anchor = icon_rect.unwrap_or(label_rect);
                        crate::widgets::layout::badge::draw_badge_indicator(
                            ctx, anchor, &badge, env,
                        );
                    }
                },
            );
            #[cfg(feature = "accessibility")]
            ctx.renderer_mut().pop_accessibility_suppression();
        }
    }
    parts.push_over_labels(ctx.renderer_mut(), render_ctx);

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

/// The handler of a draggable tab's press: a press on the selected tab that
/// moves farther than `slop` along the bar's axis drags the selection
/// indicator and settles on the nearest tab on release; any other tap within
/// reach selects the tab.
fn tab_track(
    tabs: Rc<LiveTabs>,
    index: usize,
    tag: Id,
    style: NativeTabStyle,
    selection: Binding<Id>,
    slop: f64,
) -> TrackAction {
    let start = Cell::new(None::<Point>);
    let dragging = Cell::new(false);
    let vertical = matches!(style, NativeTabStyle::Sidebar);
    Rc::new(RefCell::new(move |step: Track| match step {
        Track::Begin(point) => {
            start.set(Some(point));
            dragging.set(false);
        }
        Track::Move(point, at) => {
            if !tabs.drags() || tabs.selected() != Some(index) {
                return;
            }
            let Some(from) = start.get() else {
                return;
            };
            let offset = Vec2::new(point.x - from.x, point.y - from.y);
            let along = if vertical { offset.y } else { offset.x };
            if !dragging.get() && along.abs() > slop {
                dragging.set(true);
                tabs.begin_drag(at);
            }
            if dragging.get() {
                tabs.drag_to(offset, at);
            }
        }
        Track::Release { within_reach, .. } => {
            start.set(None);
            if dragging.replace(false) {
                tabs.end_drag(&selection);
            } else if within_reach && selection.snapshot() != tag {
                selection.set(tag);
            }
        }
        Track::Cancel => {
            start.set(None);
            if dragging.replace(false) {
                tabs.cancel_drag();
            }
        }
    }))
}

/// The environment a tab's label builds under: the bar owns the icon's
/// placement, so the label always renders its title alone.
fn tab_label_env(env: &Environment) -> Environment {
    env.extending(LabelDisplayMode::TitleOnly)
}

fn tab_foreground_env(env: &Environment, color: Color) -> Environment {
    let mut scoped = env.clone();
    Plugin::install(ForegroundOverride::new(color), &mut scoped);
    scoped
}

/// Environment for a tab item's icon and label during placement, overriding
/// `Foreground` when `look` supplies a selected or unselected colour.
fn tab_item_env(env: &Environment, look: &TabsLook, selected: bool) -> Environment {
    let fg = if selected {
        look.get_selected_foreground()
    } else {
        look.get_unselected_foreground()
    };
    fg.map_or_else(
        || env.clone(),
        |color| tab_foreground_env(env, color.clone()),
    )
}

fn resolve_tabs_layout<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<TabsRenderState<E>>>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
    look: &TabsLook,
    style: NativeTabStyle,
) -> (TabItemLayout, TabsMetrics, Option<f64>, f64) {
    let margin = bottom_floating_margin(look, style);
    if style != NativeTabStyle::Sidebar
        && matches!(look.get_bar_width(), TabBarWidth::Compact { .. })
    {
        let measure_label_env = tab_label_env(env);
        let item_sizes: Vec<(LayoutSize, Option<LayoutSize>)> = {
            let mut st = state.borrow_mut();
            st.prebuild_labels_for_look(ctx.renderer_mut(), env, Some(look));
            let renderer = ctx.renderer_mut();
            st.tabs
                .iter()
                .map(|tab| {
                    let label_size =
                        tab.label
                            .measure_built(renderer.state_mut(), &measure_label_env, theme);
                    let icon_size = tab
                        .icon
                        .as_ref()
                        .map(|icon| icon.measure_built(renderer.state_mut(), env, theme));
                    (label_size, icon_size)
                })
                .collect()
        };
        let (layout, metrics, compact_width) =
            tabs_decide_placement(look, style, Some(ctx.bounds.width()), &item_sizes);
        return (layout, metrics, compact_width, margin);
    }
    let base_extent = tabs_bar_item_extent(ctx.bounds.width(), style, look);
    let extent = if style == NativeTabStyle::Sidebar {
        base_extent
    } else {
        margin.mul_add(-2.0, base_extent).max(0.0)
    };
    let layout = look.item_layout(extent);
    (layout, look.metrics(layout), None, margin)
}

/// Reactive `Foreground` override installed when building a tab item's icon
/// and label so views that capture `Foreground` at build time follow `selection`.
fn tab_reactive_env(
    tag: Id,
    selection: &Binding<Id>,
    look: Option<&TabsLook>,
    env: &Environment,
) -> Environment {
    let Some(look) = look else {
        return env.clone();
    };
    let selected_fg = look.get_selected_foreground().cloned();
    let unselected_fg = look.get_unselected_foreground().cloned();
    if selected_fg.is_none() && unselected_fg.is_none() {
        return env.clone();
    }
    let selected_color = selected_fg.unwrap_or_else(|| Color::new(Foreground));
    let unselected_color = unselected_fg.unwrap_or_else(|| Color::new(Foreground));
    let is_selected = selection.clone().map(move |id| id == tag).distinct();
    let combined = is_selected.map(move |selected| {
        if selected {
            selected_color.clone()
        } else {
            unselected_color.clone()
        }
    });
    tab_foreground_env(env, signal_color(combined))
}

const fn bottom_floating_margin(look: &TabsLook, style: NativeTabStyle) -> f64 {
    match style {
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => {
            let margin = look.get_floating_margin();
            if margin.is_finite() && margin > 0.0 {
                margin
            } else {
                0.0
            }
        }
        NativeTabStyle::Sidebar => 0.0,
    }
}

fn compact_bar_width(
    item_sizes: &[(LayoutSize, Option<LayoutSize>)],
    metrics: &TabsMetrics,
    layout: TabItemLayout,
    item_min_width: f64,
    max_width: Option<f64>,
    horizontal_padding: f64,
) -> f64 {
    let min_slot = if item_min_width.is_finite() && item_min_width > 0.0 {
        item_min_width
    } else {
        0.0
    };
    let pad = if horizontal_padding.is_finite() && horizontal_padding > 0.0 {
        horizontal_padding
    } else {
        0.0
    };
    let slot = item_sizes
        .iter()
        .map(|(label_size, icon_size)| {
            tabs_item_natural_width(*label_size, *icon_size, metrics, layout)
        })
        .fold(min_slot, f64::max);
    let desired = slot.mul_add(usize_as_f64(item_sizes.len()), pad * 2.0);
    match max_width {
        Some(max) if max.is_finite() && max >= 0.0 => desired.min(max),
        _ => desired,
    }
}

fn tabs_decide_placement(
    look: &TabsLook,
    style: NativeTabStyle,
    proposed_width: Option<f64>,
    item_sizes: &[(LayoutSize, Option<LayoutSize>)],
) -> (TabItemLayout, TabsMetrics, Option<f64>) {
    if style == NativeTabStyle::Sidebar {
        let (layout, metrics) = tabs_decide_layout(look, style, proposed_width, item_sizes);
        return (layout, metrics, None);
    }
    let margin = bottom_floating_margin(look, style);
    let available = proposed_width.map(|width| margin.mul_add(-2.0, width).max(0.0));
    match look.get_bar_width() {
        TabBarWidth::Full => {
            let (layout, metrics) = tabs_decide_layout(look, style, available, item_sizes);
            (layout, metrics, None)
        }
        TabBarWidth::Compact {
            item_min_width,
            max_width,
            horizontal_padding,
        } => {
            let vertical_metrics = look.metrics(TabItemLayout::Vertical);
            let mut vertical_width = compact_bar_width(
                item_sizes,
                &vertical_metrics,
                TabItemLayout::Vertical,
                item_min_width,
                max_width,
                horizontal_padding,
            );
            if let Some(available) = available {
                vertical_width = vertical_width.min(available);
            }
            let layout = look.item_layout(vertical_width);
            let metrics = look.metrics(layout);
            let mut pill_width = match layout {
                TabItemLayout::Vertical => vertical_width,
                TabItemLayout::Horizontal => compact_bar_width(
                    item_sizes,
                    &metrics,
                    TabItemLayout::Horizontal,
                    item_min_width,
                    max_width,
                    horizontal_padding,
                ),
            };
            if let Some(available) = available {
                pill_width = pill_width.min(available);
            }
            (layout, metrics, Some(pill_width))
        }
    }
}

fn tabs_bar_geometry(
    bar: &ChromeBar,
    look: &TabsLook,
    style: NativeTabStyle,
    dock_edge: Edge,
    metrics: &TabsMetrics,
    compact_width: Option<f64>,
) -> TabsBarGeometry {
    let bar_band = bar.band;
    if style == NativeTabStyle::Sidebar {
        let bar_surface = bar.surface(dock_edge);
        let inner = dock_edge.inner_edge(bar_band);
        let divider = match dock_edge {
            Edge::Leading => Rect::new(
                (inner - 1.0).max(bar_band.x0),
                bar_surface.y0,
                inner,
                bar_surface.y1,
            ),
            _ => Rect::new(
                inner,
                bar_surface.y0,
                (inner + 1.0).min(bar_band.x1),
                bar_surface.y1,
            ),
        };
        return TabsBarGeometry {
            bar_rect: bar_band,
            items_rect: bar_band,
            bar_surface,
            divider,
        };
    }
    let margin = bottom_floating_margin(look, style);
    let height = metrics.bar_height.min(bar_band.height());
    let y1 = (bar_band.y1 - margin).max(bar_band.y0);
    let y0 = (y1 - height).max(bar_band.y0);
    let available = margin.mul_add(-2.0, bar_band.width()).max(0.0);
    let bar_width = compact_width.map_or(available, |width| width.min(available));
    let x0 = (bar_band.width() - bar_width).mul_add(0.5, bar_band.x0);
    let bar_rect = Rect::new(x0, y0, x0 + bar_width, y1);
    let horizontal_padding = match look.get_bar_width() {
        TabBarWidth::Compact {
            horizontal_padding, ..
        } if horizontal_padding.is_finite() && horizontal_padding > 0.0 => {
            horizontal_padding.min(bar_rect.width() * 0.5)
        }
        _ => 0.0,
    };
    let items_rect = Rect::new(
        bar_rect.x0 + horizontal_padding,
        bar_rect.y0,
        bar_rect.x1 - horizontal_padding,
        bar_rect.y1,
    );
    let bar_surface = if margin > 0.0 || compact_width.is_some() {
        bar_rect
    } else {
        bar.surface(dock_edge)
    };
    let divider = Rect::new(
        bar_surface.x0,
        bar_rect.y0,
        bar_surface.x1,
        (bar_rect.y0 + 1.0).min(bar_rect.y1),
    );
    TabsBarGeometry {
        bar_rect,
        items_rect,
        bar_surface,
        divider,
    }
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

/// The extent a style's tab bar reports to [`TabsLook::item_layout`] when
/// the container is `container_width` wide: a bottom bar's extent is the
/// container's width; a sidebar strip reports its own thickness — the
/// vertical metrics' `bar_height` — which never widens into the pane.
pub const fn tabs_bar_item_extent(
    container_width: f64,
    style: NativeTabStyle,
    look: &TabsLook,
) -> f64 {
    match style {
        NativeTabStyle::Sidebar => look.metrics(TabItemLayout::Vertical).bar_height,
        NativeTabStyle::Automatic | NativeTabStyle::TabBar => container_width,
    }
}

/// Decides the bar's item layout and metrics once for a measure pass: the
/// look answers [`TabsLook::item_layout`] from the bar's own extent (see
/// [`tabs_bar_item_extent`]) computed from `item_sizes`, so the bar this pass
/// measures and the one the render pass draws under the same bounds agree.
#[expect(
    clippy::option_if_let_else,
    reason = "the if-let/else mirrors the control flow more clearly than the combinator chain here"
)]
pub fn tabs_decide_layout(
    look: &TabsLook,
    style: NativeTabStyle,
    proposed_width: Option<f64>,
    item_sizes: &[(LayoutSize, Option<LayoutSize>)],
) -> (TabItemLayout, crate::theme::TabsMetrics) {
    let extent = match proposed_width {
        Some(width) => tabs_bar_item_extent(width, style, look),
        None => {
            if style == NativeTabStyle::Sidebar {
                tabs_bar_item_extent(0.0, style, look)
            } else {
                let vertical_metrics = look.metrics(TabItemLayout::Vertical);
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
    let layout = look.item_layout(extent);
    (layout, look.metrics(layout))
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
#[cfg(hydrolysis_cherenkov)]
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
        let badges = state.badge_values(renderer, env);
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
                    tab.label.accessibility_label(renderer),
                    Some(index) == selected_index,
                    badges[index].clone(),
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
