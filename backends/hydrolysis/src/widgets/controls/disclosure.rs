//! Hydrolysis realization of [`Disclosure`](waterui::widget::disclosure::Disclosure):
//! a keyboard-focusable header row carrying an animated [`ToggleControl`] indicator
//! and a lazily realized body when expanded.

use core::cell::RefCell;
use core::mem::take;
use std::rc::Rc;

#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, Role as AccessibilityNodeRole,
};
use kurbo::Rect;
use nami::{Binding, Signal as _};
use waterui::widget::disclosure::DisclosureConfig;
use waterui_core::Environment;
use waterui_core::layout::{
    LayoutDirection, ProposalSize, Size as LayoutSize, ViewDimensions, layout_direction,
};
use waterui_interaction::interaction::InteractionState;

use crate::num_cast::f64_as_f32;
use crate::renderer::chrome::{ChromeContent, LiveChromeParts};
use crate::renderer::live_toggle::LiveToggleParts;
use crate::renderer::mount::ScopeKey;
#[cfg(feature = "accessibility")]
use crate::renderer::{AccessibilityActionTarget, RenderContext};
use crate::renderer::{
    GroupOpacity, HeldChild, HydroState, HydrolysisRenderer, InteractionKey, PressSlot,
    RetainedSubview, SemanticCore, WidgetRenderContext, bounded_proposal,
};
use crate::text::TextEngine;
use crate::theme::{
    DISABLED_FOREGROUND_ALPHA, DisclosureLook, DisclosureSpec, SpecTint, WidgetTheme,
};
use crate::widgets::util::{mirrored_in, widget_disabled};

/// Retained render state for a [`Disclosure`](waterui::widget::disclosure::Disclosure).
///
/// The header label stays retained for its lifetime so measurements match renders,
/// while the expandable content sub-view is created only when `expanded` is `true`
/// and dropped on collapse.
pub struct DisclosureRenderState<E: TextEngine> {
    config: DisclosureConfig<HeldChild<E>>,
    header_view: RetainedSubview<E>,
    content_view: Option<RetainedSubview<E>>,
    look: Option<(DisclosureSpec, DisclosureLook)>,
    chrome_parts: Option<LiveChromeParts>,
    indicator_parts: Option<LiveToggleParts>,
    header_state: Binding<InteractionState>,
    content_dirty: bool,
}

impl<E: TextEngine> DisclosureRenderState<E> {
    /// The label this control contributes to a node naming itself from its
    /// content: its header label.
    #[cfg(feature = "accessibility")]
    pub(crate) fn contributed_label(
        &self,
        renderer: &mut SemanticCore<E>,
        env: &Environment,
    ) -> Option<String> {
        renderer.accessibility_label_from_label(self.config.header(), env)
    }

    pub(crate) fn from_config(config: DisclosureConfig<HeldChild<E>>) -> Self {
        Self {
            header_view: RetainedSubview::held(config.header().build()),
            config,
            content_view: None,
            look: None,
            chrome_parts: None,
            indicator_parts: None,
            header_state: Binding::new(InteractionState::empty()),
            content_dirty: false,
        }
    }

    /// Eagerly builds the header sub-view and realizes content if initially expanded.
    pub(crate) fn prebuild(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        self.header_view.ensure_built(renderer, env);
        let _ = self.sync_content(renderer, env);
    }

    /// Synchronizes the lazy content subtree during layout preparation so
    /// measurement reads the expanded body only when active.
    pub(crate) fn prepare(&mut self, renderer: &mut HydrolysisRenderer<E>, env: &Environment) {
        let _ = self.sync_content(renderer, env);
    }

    /// Consumes layout-dirty flags from child subviews or expand/collapse transitions.
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        take(&mut self.content_dirty)
            | self.header_view.take_layout_dirty_if_built()
            | self
                .content_view
                .as_mut()
                .is_some_and(RetainedSubview::take_layout_dirty_if_built)
    }

    fn sync_content(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) -> bool {
        let expanded = renderer.read_signal(self.config.expanded());
        if expanded {
            let built = self
                .content_view
                .get_or_insert_with(|| RetainedSubview::held(self.config.content().build()));
            if !built.is_built() {
                built.ensure_built(renderer, env);
                self.content_dirty = true;
            }
        } else if self.content_view.take().is_some() {
            self.content_dirty = true;
        }
        expanded
    }

    fn look(
        &mut self,
        theme: &Rc<dyn WidgetTheme>,
        spec: DisclosureSpec,
        env: &Environment,
    ) -> &DisclosureLook {
        if self.held(&spec).is_none() {
            let look = theme.disclosure(&spec, env);
            self.look = Some((spec, look));
        }
        &self
            .look
            .as_ref()
            .expect("disclosure look was just ensured")
            .1
    }

    fn held(&self, spec: &DisclosureSpec) -> Option<&DisclosureLook> {
        self.look
            .as_ref()
            .filter(|(held, _)| held == spec)
            .map(|(_, look)| look)
    }

    fn peek(
        &self,
        theme: &Rc<dyn WidgetTheme>,
        spec: &DisclosureSpec,
        env: &Environment,
    ) -> DisclosureLook {
        self.held(spec)
            .cloned()
            .unwrap_or_else(|| theme.disclosure(spec, env))
    }
}

fn disclosure_spec(env: &Environment) -> DisclosureSpec {
    DisclosureSpec::new(layout_direction(env).snapshot(), SpecTint::resolve(env))
}

fn header_height_for(label_size: LayoutSize, look: &DisclosureLook) -> f64 {
    let layout = look.layout();
    let indicator_h = look.control().size().height;
    let inner_h = f64::from(label_size.height).max(indicator_h);
    f64::from(layout.vertical_inset)
        .mul_add(2.0, inner_h)
        .max(f64::from(layout.min_header_height))
}

fn header_intrinsic_width(label_size: LayoutSize, look: &DisclosureLook) -> f64 {
    let layout = look.layout();
    let indicator = look.control();
    let label_w = f64::from(label_size.width);
    let label_part = if label_w > 0.0 {
        f64::from(indicator.get_label_spacing()) + label_w
    } else {
        0.0
    };
    f64::from(layout.horizontal_inset).mul_add(2.0, indicator.size().width) + label_part
}

fn header_label_proposal(header_width: f64, look: &DisclosureLook) -> ProposalSize {
    let layout = look.layout();
    let indicator = look.control();
    let reserved = f64::from(layout.horizontal_inset).mul_add(2.0, indicator.size().width)
        + f64::from(indicator.get_label_spacing());
    ProposalSize::new(Some(f64_as_f32((header_width - reserved).max(0.0))), None)
}

fn header_part_bounds(
    header: Rect,
    direction: LayoutDirection,
    look: &DisclosureLook,
    label_size: LayoutSize,
) -> (Rect, Rect) {
    let layout = look.layout();
    let indicator = look.control();
    let indicator_size = indicator.size();
    let horizontal_inset = f64::from(layout.horizontal_inset);
    let inner_x0 = header.x0 + horizontal_inset;
    let inner_x1 = (header.x1 - horizontal_inset).max(inner_x0);
    let ind_y0 = header.y0 + ((header.height() - indicator_size.height) * 0.5).max(0.0);
    let indicator_ltr = Rect::new(
        inner_x0,
        ind_y0,
        (inner_x0 + indicator_size.width).min(header.x1),
        (ind_y0 + indicator_size.height).min(header.y1),
    );
    let label_left = if label_size.width > 0.0 {
        (indicator_ltr.x1 + f64::from(indicator.get_label_spacing())).min(inner_x1)
    } else {
        indicator_ltr.x1
    };
    let label_w = f64::from(label_size.width).min((inner_x1 - label_left).max(0.0));
    let label_h = f64::from(label_size.height).min(header.height());
    let label_top = header.y0 + ((header.height() - label_h) * 0.5).max(0.0);
    let label_ltr = Rect::new(
        label_left,
        label_top,
        label_left + label_w,
        label_top + label_h,
    );
    match direction {
        LayoutDirection::LeftToRight => (indicator_ltr, label_ltr),
        LayoutDirection::RightToLeft => (
            mirrored_in(indicator_ltr, header),
            mirrored_in(label_ltr, header),
        ),
    }
}

#[cfg(feature = "accessibility")]
fn disclosure_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    ctx: Option<RenderContext>,
    config: &DisclosureConfig<HeldChild<E>>,
    env: &Environment,
    focus_key: &InteractionKey,
) {
    let mut node = AccessibilityNode::new(SemanticCore::<E>::resolve_accessibility_role(
        env,
        AccessibilityNodeRole::Button,
    ));
    let default_label = renderer.accessibility_label_from_label(config.header(), env);
    if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
        node.set_label(label);
    }
    if let Some(value) = renderer.resolve_accessibility_value(env, None) {
        node.set_value(value);
    }
    let expanded = renderer.read_signal(config.expanded());
    node.set_expanded(expanded);
    node.add_action(AccessibilityAction::Focus);
    let disabled = renderer.read_signal(&widget_disabled(env));
    let action_target = if disabled {
        node.set_disabled();
        None
    } else {
        node.add_action(AccessibilityAction::Click);
        node.add_action(AccessibilityAction::Expand);
        node.add_action(AccessibilityAction::Collapse);
        Some(AccessibilityActionTarget::Toggle {
            binding: config.expanded().clone(),
        })
    };
    if let Some(node_id) = renderer.register_accessibility_leaf(ctx, node, env, action_target) {
        renderer.register_accessibility_focus_link(focus_key, node_id);
    }
}

/// Emits accessibility nodes for a [`Disclosure`] during semantic-only tree walks.
#[cfg(feature = "accessibility")]
#[cfg(hydrolysis_cherenkov)]
pub fn emit_disclosure_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    state: &Rc<RefCell<DisclosureRenderState<E>>>,
    env: &Environment,
) {
    if renderer.is_accessibility_hidden(env) {
        return;
    }
    let expanded = state.borrow_mut().sync_content(renderer, env);
    let focus_key = InteractionKey::for_rc(state, 0);
    {
        let config = &state.borrow().config;
        disclosure_accessibility(renderer, None, config, env, &focus_key);
    }
    if expanded {
        let mut content_view = state.borrow_mut().content_view.take();
        if let Some(view) = &mut content_view {
            view.emit_accessibility(renderer, env);
        }
        let mut state_ref = state.borrow_mut();
        if state_ref.content_view.is_none() {
            state_ref.content_view = content_view;
        }
    }
}

/// Measures a retained disclosure leaf from its header and, when expanded, its
/// lazily built body.
pub fn measure_disclosure_node<E: TextEngine>(
    render_state: &DisclosureRenderState<E>,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    let spec = disclosure_spec(env);
    let look = render_state.peek(theme, &spec, env);
    let header_proposal = proposal
        .width
        .filter(|w| w.is_finite())
        .map_or(ProposalSize::UNSPECIFIED, |w| {
            header_label_proposal(f64::from(w), &look)
        });
    let label_size =
        render_state
            .header_view
            .measure_built_with_proposal(state, env, theme, header_proposal);
    let intrinsic_header_w = header_intrinsic_width(label_size, &look);
    let total_w = proposal
        .width
        .filter(|w| w.is_finite())
        .map_or(intrinsic_header_w, |w| f64::from(w).max(intrinsic_header_w));
    let header_h = header_height_for(label_size, &look);
    let expanded = render_state.config.expanded().snapshot();
    if expanded && let Some(content) = &render_state.content_view {
        let layout = look.layout();
        let content_indent = f64::from(layout.content_indent);
        let content_spacing = f64::from(layout.content_spacing);
        let content_width = (total_w - content_indent).max(0.0);
        let content_proposal = ProposalSize::new(
            Some(f64_as_f32(content_width)),
            proposal
                .height
                .filter(|h| h.is_finite())
                .map(|h| (f64::from(h) - header_h - content_spacing).max(0.0))
                .map(f64_as_f32),
        );
        let content_size = content.measure_built_with_proposal(state, env, theme, content_proposal);
        let width = total_w.max(content_indent + f64::from(content_size.width));
        let height = header_h + content_spacing + f64::from(content_size.height);
        return ViewDimensions::new(LayoutSize::new(f64_as_f32(width), f64_as_f32(height)));
    }
    ViewDimensions::new(LayoutSize::new(f64_as_f32(total_w), f64_as_f32(header_h)))
}

struct DisclosureFrame {
    expanded: bool,
    expanded_binding: Binding<bool>,
    look: DisclosureLook,
    header_bounds: Rect,
    label_bounds: Rect,
    chrome_parts: LiveChromeParts,
    indicator_parts: LiveToggleParts,
    press_slot: PressSlot,
}

fn prepare_disclosure_frame<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<DisclosureRenderState<E>>>,
    env: &Environment,
    key: &InteractionKey,
    direction: LayoutDirection,
    disabled: bool,
) -> DisclosureFrame {
    let theme = ctx.theme();
    let spec = disclosure_spec(env);
    let mut state_ref = state.borrow_mut();
    let expanded = state_ref.sync_content(ctx.renderer_mut(), env);
    let expanded_binding = state_ref.config.expanded().clone();
    let look = state_ref.look(&theme, spec, env).clone();
    let header_proposal = header_label_proposal(ctx.bounds.width(), &look);
    let label_size = state_ref
        .header_view
        .patch_and_measure(ctx.renderer_mut(), env, header_proposal)
        .0;
    let header_h = header_height_for(label_size, &look);
    let header_bounds = Rect::new(
        ctx.bounds.x0,
        ctx.bounds.y0,
        ctx.bounds.x1,
        (ctx.bounds.y0 + header_h).min(ctx.bounds.y1),
    );
    let (indicator_bounds, label_bounds) =
        header_part_bounds(header_bounds, direction, &look, label_size);

    let chrome = look.chrome().clone();
    let (_, press_slot, handles) = ctx.renderer_mut().bind_live_chrome_target(
        key.clone(),
        header_bounds,
        env,
        disabled,
        &chrome,
    );
    let flags = ctx.renderer_mut().reported_interaction_state(key);
    if state_ref.header_state.snapshot() != flags {
        state_ref.header_state.set(flags);
    }
    let header_state = state_ref.header_state.clone();
    let chrome_parts = LiveChromeParts::ensure(
        &mut state_ref.chrome_parts,
        &chrome,
        &ChromeContent::default(),
        header_bounds,
        flags,
        env,
        &header_state,
    )
    .clone();
    handles.attach_live(Rc::clone(chrome_parts.chrome()));
    let indicator_parts = LiveToggleParts::ensure(
        &mut state_ref.indicator_parts,
        look.control(),
        indicator_bounds,
        flags,
        &expanded_binding,
    )
    .clone();

    DisclosureFrame {
        expanded,
        expanded_binding,
        look,
        header_bounds,
        label_bounds,
        chrome_parts,
        indicator_parts,
        press_slot,
    }
}

fn render_disclosure_header<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<DisclosureRenderState<E>>>,
    env: &Environment,
    frame: &DisclosureFrame,
    disabled: bool,
) {
    let reach = frame
        .chrome_parts
        .chrome()
        .visual_bounds()
        .union(frame.header_bounds)
        .union(frame.indicator_parts.toggle().visual_bounds());
    let group_key = ScopeKey {
        role: "disclosure-header",
        item: 0,
    };
    ctx.with_group_scope(
        group_key,
        GroupOpacity::Fixed(1.0),
        Option::default(),
        reach,
        |ctx| {
            let render_ctx = ctx.render_context();
            frame
                .chrome_parts
                .push_below(ctx.renderer_mut(), render_ctx);

            ctx.with_scope_if(
                disabled,
                ScopeKey {
                    role: "disclosure-dim",
                    item: 0,
                },
                DISABLED_FOREGROUND_ALPHA,
                frame.header_bounds,
                |ctx| {
                    let ind_render_ctx = ctx.render_context();
                    frame
                        .indicator_parts
                        .push(ctx.renderer_mut(), ind_render_ctx);
                    if frame.label_bounds.width() > 0.0 && frame.label_bounds.height() > 0.0 {
                        let label_render_ctx = ctx.render_context();
                        let label_area = ctx.safe_area_for(frame.label_bounds);
                        let mut state_ref = state.borrow_mut();
                        ctx.renderer_mut()
                            .with_suppressed_accessibility(|renderer| {
                                state_ref.header_view.place(
                                    renderer,
                                    label_render_ctx,
                                    env,
                                    ProposalSize::UNSPECIFIED,
                                    frame.label_bounds,
                                    label_area,
                                );
                            });
                    }
                },
            );

            let render_ctx = ctx.render_context();
            frame
                .chrome_parts
                .push_above(ctx.renderer_mut(), render_ctx);
        },
    );
}

/// Renders a retained disclosure leaf: header chrome, animated indicator, header
/// label, interactive focus/click target, and lazily placed body when expanded.
pub fn render_disclosure_node<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<DisclosureRenderState<E>>>,
    env: &Environment,
) {
    let key = InteractionKey::for_rc(state, 0);
    let direction = disclosure_spec(env).direction();
    let disabled = {
        let signal = widget_disabled(env);
        ctx.renderer_mut().read_signal(&signal)
    };
    let frame = prepare_disclosure_frame(ctx, state, env, &key, direction, disabled);

    #[cfg(feature = "accessibility")]
    {
        let hidden = ctx.renderer_mut().is_accessibility_hidden(env);
        if !hidden {
            let header_ctx = RenderContext::new(ctx.local, frame.header_bounds);
            let config = &state.borrow().config;
            disclosure_accessibility(ctx.renderer_mut(), Some(header_ctx), config, env, &key);
        }
    }

    render_disclosure_header(ctx, state, env, &frame, disabled);

    if !disabled {
        let toggle_binding = frame.expanded_binding;
        ctx.renderer_mut().register_interactive_pointer_target(
            frame.header_bounds,
            frame.press_slot,
            move |_renderer, _point, _env| {
                toggle_binding.toggle();
                true
            },
        );
    }

    if frame.expanded {
        let layout = frame.look.layout();
        let content_indent = f64::from(layout.content_indent);
        let content_spacing = f64::from(layout.content_spacing);
        let body_top = (frame.header_bounds.y1 + content_spacing).min(ctx.bounds.y1);
        let (body_left, body_right) = match direction {
            LayoutDirection::LeftToRight => (
                (ctx.bounds.x0 + content_indent).min(ctx.bounds.x1),
                ctx.bounds.x1,
            ),
            LayoutDirection::RightToLeft => (
                ctx.bounds.x0,
                (ctx.bounds.x1 - content_indent).max(ctx.bounds.x0),
            ),
        };
        let content_bounds = Rect::new(body_left, body_top, body_right, ctx.bounds.y1);
        if content_bounds.width() > 0.0 && content_bounds.height() > 0.0 {
            let content_proposal = bounded_proposal(content_bounds);
            let render_ctx = ctx.render_context();
            let content_area = ctx.safe_area_for(content_bounds);
            let mut content_view = state.borrow_mut().content_view.take();
            if let Some(view) = &mut content_view {
                view.place(
                    ctx.renderer_mut(),
                    render_ctx,
                    env,
                    content_proposal,
                    content_bounds,
                    content_area,
                );
            }
            let mut state_ref = state.borrow_mut();
            if state_ref.content_view.is_none() {
                state_ref.content_view = content_view;
            }
        }
    }
}
