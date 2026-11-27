//! Hydrolysis realization of `Code` ([`CodeConfig`]): a theme-driven
//! container [`Surface`] framing a language caption header, an accessible copy
//! button, and syntax-highlighted monospaced source text that tracks the
//! installed `Body` font slot.

use core::cell::RefCell;
use core::time::Duration;
use std::rc::Rc;

use kurbo::Rect;
use nami::{Binding, Signal as _};
use waterui_controls::button::{ButtonStyle, button};
use waterui_controls::theme::{ForegroundOverride, install_font_signal};
use waterui_core::Environment;
use waterui_core::animation::Animation;
use waterui_core::layout::{
    LayoutDirection, ProposalSize, Size as LayoutSize, StretchAxis, ViewDimensions,
    layout_direction,
};
use waterui_core::plugin::Plugin;
use waterui_graphics::color::Color;
use waterui_interaction::interaction::InteractionState;
use waterui_text::code::CodeConfig;
use waterui_text::font::Body;
use waterui_text::text;

use crate::num_cast::f64_as_f32;
use crate::renderer::chrome::LiveChromeParts;
use crate::renderer::{
    HydroState, HydrolysisRenderer, RetainedSubview, SafeAreaLayout, SemanticCore,
    WidgetRenderContext, bounded_proposal,
};
use crate::text::TextEngine;
use crate::theme::{
    Chrome, CodeLook, CodeMetrics, FocusIndicator, Motion, PressFeedback, StateLayer, Surface,
    WidgetTheme,
};

/// Minimum horizontal gap between the language label and the copy button in logical points.
const HEADER_ITEM_GAP: f32 = 8.0;

/// Retained render state for a `Code` block.
pub struct CodeRenderState<E: TextEngine> {
    config: CodeConfig,
    header_label: String,
    header: Option<RetainedSubview<E>>,
    copy_button: Option<RetainedSubview<E>>,
    body: Option<RetainedSubview<E>>,
    look: Option<CodeLook>,
    chrome: Option<Chrome>,
    chrome_parts: Option<LiveChromeParts>,
    interaction_state: Binding<InteractionState>,
}

impl<E: TextEngine> CodeRenderState<E> {
    pub(crate) fn from_config(config: CodeConfig) -> Self {
        let header_label = config.header_label();
        Self {
            config,
            header_label,
            header: None,
            copy_button: None,
            body: None,
            look: None,
            chrome: None,
            chrome_parts: None,
            interaction_state: Binding::new(InteractionState::empty()),
        }
    }

    /// Stretch axis of a code block: stretches horizontally to fill its container.
    #[allow(clippy::unused_self, reason = "matches sibling widget state API")]
    pub(crate) const fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    /// Builds theme-styled header, copy button, and body sub-views ahead of measurement.
    pub(crate) fn prepare(&mut self, renderer: &mut HydrolysisRenderer<E>, env: &Environment) {
        let theme = renderer.theme();
        let look = self.ensure_look(&theme, env).clone();
        self.ensure_subviews_with_look(renderer, env, &look);
    }

    /// Consumes layout-dirty flags from header, copy button, and body sub-views.
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.header
            .as_mut()
            .is_some_and(RetainedSubview::take_layout_dirty_if_built)
            | self
                .copy_button
                .as_mut()
                .is_some_and(RetainedSubview::take_layout_dirty_if_built)
            | self
                .body
                .as_mut()
                .is_some_and(RetainedSubview::take_layout_dirty_if_built)
    }

    /// The label this code block contributes when an enclosing container names
    /// itself from its content.
    #[cfg(feature = "accessibility")]
    pub(crate) fn contributed_label(
        &self,
        renderer: &mut SemanticCore<E>,
        _env: &Environment,
    ) -> Option<String> {
        let content = renderer.read_signal(self.config.content());
        let labels = [self.header_label.as_str(), content.as_str()]
            .into_iter()
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();
        (!labels.is_empty()).then(|| labels.join(" "))
    }

    fn ensure_look(&mut self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> &CodeLook {
        if self.look.is_none() {
            self.look = Some(theme.code(env));
        }
        self.look.as_ref().expect("code look was just ensured")
    }

    fn peek_look(&self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> CodeLook {
        self.look.clone().unwrap_or_else(|| theme.code(env))
    }

    fn ensure_chrome(&mut self, look: &CodeLook) -> &Chrome {
        if self.chrome.is_none() {
            self.chrome = Some(code_chrome(look.surface().clone()));
        }
        self.chrome.as_ref().expect("code chrome was just ensured")
    }

    fn build_copy_button(&self) -> impl waterui_core::View {
        let config = self.config.clone();
        button("Copy")
            .style(ButtonStyle::Borderless)
            .action(move |env: Environment| {
                config.copy_to_clipboard(&env);
            })
    }

    fn ensure_subviews_with_look(
        &mut self,
        renderer: &mut SemanticCore<E>,
        env: &Environment,
        look: &CodeLook,
    ) {
        if self.header.is_none() {
            let scoped = header_env(env, look);
            let mut subview = RetainedSubview::new(text(self.header_label.clone()).bold(true));
            subview.ensure_built(renderer, &scoped);
            self.header = Some(subview);
        }
        if self.copy_button.is_none() {
            let scoped = copy_env(env, look);
            let mut subview = RetainedSubview::new(self.build_copy_button());
            subview.ensure_built(renderer, &scoped);
            self.copy_button = Some(subview);
        }
        if self.body.is_none() {
            let scoped = body_env(env, look);
            let highlighted = self.config.highlighted(env);
            let mut subview = RetainedSubview::new(text(highlighted));
            subview.ensure_built(renderer, &scoped);
            self.body = Some(subview);
        }
    }

    #[cfg(feature = "accessibility")]
    #[cfg(hydrolysis_cherenkov)]
    fn ensure_subviews_semantic(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        if self.header.is_none() {
            let mut subview = RetainedSubview::new(text(self.header_label.clone()).bold(true));
            subview.ensure_built(renderer, env);
            self.header = Some(subview);
        }
        if self.copy_button.is_none() {
            let mut subview = RetainedSubview::new(self.build_copy_button());
            subview.ensure_built(renderer, env);
            self.copy_button = Some(subview);
        }
        if self.body.is_none() {
            let highlighted = self.config.highlighted(env);
            let mut subview = RetainedSubview::new(text(highlighted));
            subview.ensure_built(renderer, env);
            self.body = Some(subview);
        }
    }
}

fn code_chrome(surface: Surface) -> Chrome {
    Chrome::new(
        surface,
        StateLayer::new(Color::transparent()),
        PressFeedback::new(),
        FocusIndicator::Tint,
        Motion::uniform(Animation::linear(Duration::ZERO)),
    )
}

fn header_env(env: &Environment, look: &CodeLook) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, look.header_font_ref().resolve(env));
    Plugin::install(
        ForegroundOverride::new(look.header_color_ref().clone()),
        &mut scoped,
    );
    scoped
}

fn copy_env(env: &Environment, look: &CodeLook) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, look.header_font_ref().resolve(env));
    Plugin::install(
        ForegroundOverride::new(look.copy_color_ref().clone()),
        &mut scoped,
    );
    scoped
}

fn body_env(env: &Environment, look: &CodeLook) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, look.body_font_ref().resolve(env));
    scoped
}

fn leading_horizontal_bounds(
    inner_x0: f64,
    inner_x1: f64,
    item_width: f64,
    direction: LayoutDirection,
) -> (f64, f64) {
    let avail = (inner_x1 - inner_x0).max(0.0);
    let width = item_width.min(avail);
    match direction {
        LayoutDirection::LeftToRight => (inner_x0, inner_x0 + width),
        LayoutDirection::RightToLeft => (inner_x1 - width, inner_x1),
    }
}

fn trailing_horizontal_bounds(
    inner_x0: f64,
    inner_x1: f64,
    item_width: f64,
    direction: LayoutDirection,
) -> (f64, f64) {
    let avail = (inner_x1 - inner_x0).max(0.0);
    let width = item_width.min(avail);
    match direction {
        LayoutDirection::LeftToRight => (inner_x1 - width, inner_x1),
        LayoutDirection::RightToLeft => (inner_x0, inner_x0 + width),
    }
}

/// Measures a retained code block leaf from its layout padding, header row
/// (language caption + copy action), and monospaced source body.
pub fn measure_code_node<E: TextEngine>(
    render_state: &CodeRenderState<E>,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    let look = render_state.peek_look(theme, env);
    let metrics = look.metrics();
    let h_pad = metrics.horizontal_padding;
    let v_pad = metrics.vertical_padding;

    let inner_width = proposal
        .width
        .filter(|w| w.is_finite())
        .map(|w| 2.0_f32.mul_add(-h_pad, w).max(0.0));
    let inner_height = proposal
        .height
        .filter(|h| h.is_finite())
        .map(|h| 2.0_f32.mul_add(-v_pad, h).max(0.0));

    let copy_scoped = render_state
        .copy_button
        .as_ref()
        .map(|_| copy_env(env, &look));
    let copy_size = match (&render_state.copy_button, &copy_scoped) {
        (Some(copy_btn), Some(scoped)) if copy_btn.is_built() => copy_btn
            .measure_built_with_proposal(
                state,
                scoped,
                theme,
                ProposalSize::new(inner_width, None),
            ),
        _ => LayoutSize::new(0.0, 0.0),
    };

    let header_max_w = inner_width.map(|w| (w - copy_size.width - HEADER_ITEM_GAP).max(0.0));
    let header_scoped = render_state.header.as_ref().map(|_| header_env(env, &look));
    let header_size = match (&render_state.header, &header_scoped) {
        (Some(header), Some(scoped)) if header.is_built() => header.measure_built_with_proposal(
            state,
            scoped,
            theme,
            ProposalSize::new(header_max_w, None),
        ),
        _ => LayoutSize::new(0.0, 0.0),
    };

    let header_row_w = header_size.width + HEADER_ITEM_GAP + copy_size.width;
    let header_row_h = header_size.height.max(copy_size.height);

    let body_proposal = ProposalSize::new(
        inner_width,
        inner_height.map(|h| (h - header_row_h - metrics.header_spacing).max(0.0)),
    );
    let body_scoped = render_state.body.as_ref().map(|_| body_env(env, &look));
    let body_size = match (&render_state.body, &body_scoped) {
        (Some(body), Some(scoped)) if body.is_built() => {
            body.measure_built_with_proposal(state, scoped, theme, body_proposal)
        }
        _ => LayoutSize::new(0.0, 0.0),
    };

    let natural_inner_w = header_row_w.max(body_size.width);
    let inner_w = inner_width.map_or(natural_inner_w, |proposed| proposed.max(natural_inner_w));
    let inner_h = header_row_h + metrics.header_spacing + body_size.height;

    ViewDimensions::new(LayoutSize::new(
        2.0_f32.mul_add(h_pad, inner_w),
        2.0_f32.mul_add(v_pad, inner_h),
    ))
}

struct PlacedCodeSlot {
    bounds: Rect,
    proposal: ProposalSize,
    safe_area: Option<SafeAreaLayout>,
    env: Environment,
}

struct PlacedCodeSlots {
    header: Option<PlacedCodeSlot>,
    copy_button: Option<PlacedCodeSlot>,
    body: Option<PlacedCodeSlot>,
}

fn place_code_slot<E: TextEngine>(
    ctx: &WidgetRenderContext<'_, E>,
    rect: Rect,
    env: Environment,
) -> PlacedCodeSlot {
    PlacedCodeSlot {
        bounds: rect,
        proposal: bounded_proposal(rect),
        safe_area: ctx.safe_area_for(rect),
        env,
    }
}

fn measure_code_header_row<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<CodeRenderState<E>>>,
    header_scoped: &Environment,
    copy_scoped: &Environment,
    content_width: f64,
    gap: f64,
) -> (Option<LayoutSize>, Option<LayoutSize>) {
    let mut state_ref = state.borrow_mut();
    let copy_size = state_ref.copy_button.as_mut().map(|copy_btn| {
        copy_btn
            .patch_and_measure(
                ctx.renderer_mut(),
                copy_scoped,
                ProposalSize::new(Some(f64_as_f32(content_width)), None),
            )
            .0
    });
    let copy_w = copy_size.map_or(0.0, |s| f64::from(s.width));
    let header_avail_w = (content_width - copy_w - gap).max(0.0);
    let header_size = state_ref.header.as_mut().map(|header| {
        header
            .patch_and_measure(
                ctx.renderer_mut(),
                header_scoped,
                ProposalSize::new(Some(f64_as_f32(header_avail_w)), None),
            )
            .0
    });
    (header_size, copy_size)
}

fn compute_code_slots<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<CodeRenderState<E>>>,
    env: &Environment,
    look: &CodeLook,
    metrics: CodeMetrics,
) -> PlacedCodeSlots {
    let bounds = ctx.bounds;
    let direction = layout_direction(env).snapshot();
    let h_pad = f64::from(metrics.horizontal_padding);
    let v_pad = f64::from(metrics.vertical_padding);
    let spacing = f64::from(metrics.header_spacing);
    let gap = f64::from(HEADER_ITEM_GAP);
    let content_left = bounds.x0 + h_pad;
    let content_right = (bounds.x1 - h_pad).max(content_left);
    let content_top = bounds.y0 + v_pad;
    let content_bottom = (bounds.y1 - v_pad).max(content_top);
    let content_width = (content_right - content_left).max(0.0);
    let content_height = (content_bottom - content_top).max(0.0);

    let copy_scoped = copy_env(env, look);
    let header_scoped = header_env(env, look);
    let body_scoped = body_env(env, look);

    let (header_size, copy_size) =
        measure_code_header_row(ctx, state, &header_scoped, &copy_scoped, content_width, gap);

    let header_row_h = header_size
        .map_or(0.0, |s| f64::from(s.height))
        .max(copy_size.map_or(0.0, |s| f64::from(s.height)))
        .min(content_height);

    let copy_slot = copy_size.map(|size| {
        let h = f64::from(size.height).min(header_row_h);
        let top = (header_row_h - h).mul_add(0.5, content_top);
        let (left, right) = trailing_horizontal_bounds(
            content_left,
            content_right,
            f64::from(size.width),
            direction,
        );
        place_code_slot(
            ctx,
            Rect::new(left, top, right, (top + h).min(content_bottom)),
            copy_scoped,
        )
    });

    let header_slot = header_size.map(|size| {
        let h = f64::from(size.height).min(header_row_h);
        let top = (header_row_h - h).mul_add(0.5, content_top);
        let max_w = (content_width - copy_size.map_or(0.0, |s| f64::from(s.width)) - gap).max(0.0);
        let (left, right) = leading_horizontal_bounds(
            content_left,
            content_right,
            f64::from(size.width).min(max_w),
            direction,
        );
        place_code_slot(
            ctx,
            Rect::new(left, top, right, (top + h).min(content_bottom)),
            header_scoped,
        )
    });

    let body_top = (content_top + header_row_h + spacing).min(content_bottom);
    let rem_h = (content_bottom - body_top).max(0.0);
    let body_size = {
        let mut state_ref = state.borrow_mut();
        state_ref.body.as_mut().map(|body| {
            body.patch_and_measure(
                ctx.renderer_mut(),
                &body_scoped,
                ProposalSize::new(Some(f64_as_f32(content_width)), Some(f64_as_f32(rem_h))),
            )
            .0
        })
    };

    let body_slot = body_size.map(|size| {
        let (left, right) = leading_horizontal_bounds(
            content_left,
            content_right,
            f64::from(size.width),
            direction,
        );
        let rect = Rect::new(
            left,
            body_top,
            right,
            (body_top + f64::from(size.height)).min(content_bottom),
        );
        place_code_slot(ctx, rect, body_scoped)
    });

    PlacedCodeSlots {
        header: header_slot,
        copy_button: copy_slot,
        body: body_slot,
    }
}

/// Renders a retained code block leaf: theme container surface via live chrome
/// parts, followed by the header row and syntax-highlighted body sub-views.
pub fn render_code_node<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<CodeRenderState<E>>>,
    env: &Environment,
) {
    let theme = ctx.theme();
    let (look, parts) = {
        let mut state_ref = state.borrow_mut();
        let look = state_ref.ensure_look(&theme, env).clone();
        state_ref.ensure_subviews_with_look(ctx.renderer_mut(), env, &look);
        let chrome = state_ref.ensure_chrome(&look).clone();
        let interaction = state_ref.interaction_state.clone();
        let parts = LiveChromeParts::ensure_following(
            &mut state_ref.chrome_parts,
            &chrome,
            ctx.bounds,
            InteractionState::empty(),
            env,
            &interaction,
        )
        .clone();
        (look, parts)
    };

    let PlacedCodeSlots {
        header: header_slot,
        copy_button: copy_slot,
        body: body_slot,
    } = compute_code_slots(ctx, state, env, &look, look.metrics());

    let render_ctx = ctx.render_context();
    parts.push_below(ctx.renderer_mut(), render_ctx);
    parts.wrap_content(ctx.renderer_mut(), render_ctx, |renderer| {
        if let Some(slot) = header_slot {
            let mut header = state.borrow_mut().header.take();
            if let Some(subview) = &mut header {
                subview.place(
                    renderer,
                    render_ctx,
                    &slot.env,
                    slot.proposal,
                    slot.bounds,
                    slot.safe_area,
                );
            }
            let mut state_ref = state.borrow_mut();
            if state_ref.header.is_none() {
                state_ref.header = header;
            }
        }
        if let Some(slot) = copy_slot {
            let mut copy_btn = state.borrow_mut().copy_button.take();
            if let Some(subview) = &mut copy_btn {
                subview.place(
                    renderer,
                    render_ctx,
                    &slot.env,
                    slot.proposal,
                    slot.bounds,
                    slot.safe_area,
                );
            }
            let mut state_ref = state.borrow_mut();
            if state_ref.copy_button.is_none() {
                state_ref.copy_button = copy_btn;
            }
        }
        if let Some(slot) = body_slot {
            let mut body = state.borrow_mut().body.take();
            if let Some(subview) = &mut body {
                subview.place(
                    renderer,
                    render_ctx,
                    &slot.env,
                    slot.proposal,
                    slot.bounds,
                    slot.safe_area,
                );
            }
            let mut state_ref = state.borrow_mut();
            if state_ref.body.is_none() {
                state_ref.body = body;
            }
        }
    });
}

/// Emits accessibility nodes for a `Code` block
/// during semantic-only tree walks.
#[cfg(feature = "accessibility")]
#[cfg(hydrolysis_cherenkov)]
pub fn emit_code_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    state: &Rc<RefCell<CodeRenderState<E>>>,
    env: &Environment,
) {
    state.borrow_mut().ensure_subviews_semantic(renderer, env);
    let mut header = state.borrow_mut().header.take();
    if let Some(subview) = &mut header {
        subview.emit_accessibility(renderer, env);
    }
    {
        let mut state_ref = state.borrow_mut();
        if state_ref.header.is_none() {
            state_ref.header = header;
        }
    }

    let mut copy_btn = state.borrow_mut().copy_button.take();
    if let Some(subview) = &mut copy_btn {
        subview.emit_accessibility(renderer, env);
    }
    {
        let mut state_ref = state.borrow_mut();
        if state_ref.copy_button.is_none() {
            state_ref.copy_button = copy_btn;
        }
    }

    let mut body = state.borrow_mut().body.take();
    if let Some(subview) = &mut body {
        subview.emit_accessibility(renderer, env);
    }
    {
        let mut state_ref = state.borrow_mut();
        if state_ref.body.is_none() {
            state_ref.body = body;
        }
    }
}
