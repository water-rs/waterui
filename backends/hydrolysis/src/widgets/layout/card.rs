//! Hydrolysis realization of [`Card`](waterui::widget::Card): a theme-driven
//! container [`Surface`] framing optional title, subtitle, and body content.

use core::cell::RefCell;
use core::time::Duration;
use std::rc::Rc;

use kurbo::Rect;
use nami::{Binding, Signal as _};
use waterui::animation::Animation;
use waterui::graphics::color::Color;
use waterui::text::Text;
use waterui::text::font::Body;
use waterui::theme::{ForegroundOverride, install_font_signal};
use waterui::widget::{CardConfig, CardStyle};
use waterui_core::Environment;
use waterui_core::layout::{
    LayoutDirection, ProposalSize, Size as LayoutSize, StretchAxis, ViewDimensions,
    layout_direction,
};
use waterui_core::plugin::Plugin;
use waterui_interaction::interaction::InteractionState;

use crate::num_cast::f64_as_f32;
use crate::renderer::chrome::LiveChromeParts;
use crate::renderer::{
    HeldChild, HydroState, HydrolysisRenderer, RetainedSubview, SafeAreaLayout, SemanticCore,
    WidgetRenderContext, bounded_proposal,
};
use crate::text::TextEngine;
use crate::theme::{
    CardLayout, CardLook, Chrome, FocusIndicator, Motion, PressFeedback, StateLayer, Surface,
    WidgetTheme,
};

/// Retained render state for a [`Card`](waterui::widget::Card).
pub struct CardRenderState<E: TextEngine> {
    style: CardStyle,
    title_text: Option<Text>,
    subtitle_text: Option<Text>,
    title: Option<RetainedSubview<E>>,
    subtitle: Option<RetainedSubview<E>>,
    content: Option<RetainedSubview<E>>,
    look: Option<CardLook>,
    chrome: Option<Chrome>,
    chrome_parts: Option<LiveChromeParts>,
    interaction_state: Binding<InteractionState>,
}

impl<E: TextEngine> CardRenderState<E> {
    pub(crate) fn from_config(config: CardConfig<HeldChild<E>>) -> Self {
        let CardConfig {
            title,
            subtitle,
            content,
            style,
        } = config;
        Self {
            style,
            title_text: title,
            subtitle_text: subtitle,
            title: None,
            subtitle: None,
            content: Some(RetainedSubview::held(content)),
            look: None,
            chrome: None,
            chrome_parts: None,
            interaction_state: Binding::new(InteractionState::empty()),
        }
    }

    /// Eagerly builds the body content sub-view during tree build.
    pub(crate) fn prebuild(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        if let Some(content) = &mut self.content {
            content.ensure_built(renderer, env);
        }
    }

    /// Stretch axis of the realized card, inherited from its prebuilt content.
    pub(crate) fn stretch_axis(&self) -> StretchAxis {
        self.content
            .as_ref()
            .map_or(StretchAxis::None, RetainedSubview::stretch_axis)
    }

    /// Builds theme-styled title and subtitle sub-views ahead of measurement.
    pub(crate) fn prepare(&mut self, renderer: &mut HydrolysisRenderer<E>, env: &Environment) {
        let theme = renderer.theme();
        let look = self.ensure_look(&theme, env).clone();
        self.ensure_header_views_with_look(renderer, env, &look);
    }

    /// Consumes layout-dirty flags from title, subtitle, and content sub-views.
    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.title
            .as_mut()
            .is_some_and(RetainedSubview::take_layout_dirty_if_built)
            | self
                .subtitle
                .as_mut()
                .is_some_and(RetainedSubview::take_layout_dirty_if_built)
            | self
                .content
                .as_mut()
                .is_some_and(RetainedSubview::take_layout_dirty_if_built)
    }

    fn ensure_look(&mut self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> &CardLook {
        if self.look.is_none() {
            self.look = Some(theme.card(self.style, env));
        }
        self.look.as_ref().expect("card look was just ensured")
    }

    fn peek_look(&self, theme: &Rc<dyn WidgetTheme>, env: &Environment) -> CardLook {
        self.look
            .clone()
            .unwrap_or_else(|| theme.card(self.style, env))
    }

    fn ensure_chrome(&mut self, look: &CardLook) -> &Chrome {
        if self.chrome.is_none() {
            self.chrome = Some(card_chrome(look.surface().clone()));
        }
        self.chrome.as_ref().expect("card chrome was just ensured")
    }

    fn ensure_header_views_with_look(
        &mut self,
        renderer: &mut SemanticCore<E>,
        env: &Environment,
        look: &CardLook,
    ) {
        if self.title.is_none()
            && let Some(title) = self.title_text.clone()
        {
            let scoped = title_env(env, look);
            let mut subview = RetainedSubview::new(title);
            subview.ensure_built(renderer, &scoped);
            self.title = Some(subview);
        }
        if self.subtitle.is_none()
            && let Some(subtitle) = self.subtitle_text.clone()
        {
            let scoped = subtitle_env(env, look);
            let mut subview = RetainedSubview::new(subtitle);
            subview.ensure_built(renderer, &scoped);
            self.subtitle = Some(subview);
        }
    }

    #[cfg(feature = "accessibility")]
    #[cfg(hydrolysis_cherenkov)]
    fn ensure_header_views_semantic(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        if self.title.is_none()
            && let Some(title) = self.title_text.clone()
        {
            let mut subview = RetainedSubview::new(title);
            subview.ensure_built(renderer, env);
            self.title = Some(subview);
        }
        if self.subtitle.is_none()
            && let Some(subtitle) = self.subtitle_text.clone()
        {
            let mut subview = RetainedSubview::new(subtitle);
            subview.ensure_built(renderer, env);
            self.subtitle = Some(subview);
        }
    }
}

fn card_chrome(surface: Surface) -> Chrome {
    Chrome::new(
        surface,
        StateLayer::new(Color::transparent()),
        PressFeedback::new(),
        FocusIndicator::Tint,
        Motion::uniform(Animation::linear(Duration::ZERO)),
    )
}

fn title_env(env: &Environment, look: &CardLook) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, look.title_font_ref().resolve(env));
    scoped
}

fn subtitle_env(env: &Environment, look: &CardLook) -> Environment {
    let mut scoped = env.clone();
    install_font_signal::<Body>(&mut scoped, look.subtitle_font_ref().resolve(env));
    Plugin::install(
        ForegroundOverride::new(look.subtitle_color_ref().clone()),
        &mut scoped,
    );
    scoped
}

fn item_horizontal_bounds(
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

/// Measures a retained card leaf from its layout padding, optional title and
/// subtitle, and body content.
pub fn measure_card_node<E: TextEngine>(
    render_state: &CardRenderState<E>,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    let look = render_state.peek_look(theme, env);
    let layout = look.layout();
    let h_pad = layout.horizontal_padding;
    let v_pad = layout.vertical_padding;

    let inner_width = proposal
        .width
        .filter(|w| w.is_finite())
        .map(|w| f64_as_f32(2.0_f64.mul_add(-h_pad, f64::from(w)).max(0.0)));
    let inner_height = proposal
        .height
        .filter(|h| h.is_finite())
        .map(|h| 2.0_f64.mul_add(-v_pad, f64::from(h)).max(0.0));

    let header_proposal = ProposalSize::new(inner_width, None);
    let title_scoped = render_state.title.as_ref().map(|_| title_env(env, &look));
    let title_size = match (&render_state.title, &title_scoped) {
        (Some(title), Some(scoped)) if title.is_built() => {
            Some(title.measure_built_with_proposal(state, scoped, theme, header_proposal))
        }
        _ => None,
    };
    let subtitle_scoped = render_state
        .subtitle
        .as_ref()
        .map(|_| subtitle_env(env, &look));
    let subtitle_size = match (&render_state.subtitle, &subtitle_scoped) {
        (Some(subtitle), Some(scoped)) if subtitle.is_built() => {
            Some(subtitle.measure_built_with_proposal(state, scoped, theme, header_proposal))
        }
        _ => None,
    };

    let header_count = usize::from(title_size.is_some()) + usize::from(subtitle_size.is_some());
    let total_gaps = f64::from(u32::try_from(header_count).unwrap_or(0)) * layout.content_spacing;
    let header_height = title_size.map_or(0.0, |s| f64::from(s.height))
        + subtitle_size.map_or(0.0, |s| f64::from(s.height))
        + total_gaps;

    let content_proposal = ProposalSize::new(
        inner_width,
        inner_height.map(|h| f64_as_f32((h - header_height).max(0.0))),
    );
    let content_size = render_state
        .content
        .as_ref()
        .map_or(LayoutSize::new(0.0, 0.0), |c| {
            c.measure_built_with_proposal(state, env, theme, content_proposal)
        });

    let inner_w = f64::from(content_size.width)
        .max(title_size.map_or(0.0, |s| f64::from(s.width)))
        .max(subtitle_size.map_or(0.0, |s| f64::from(s.width)));
    let inner_h = header_height + f64::from(content_size.height);

    ViewDimensions::new(LayoutSize::new(
        f64_as_f32(2.0_f64.mul_add(h_pad, inner_w)),
        f64_as_f32(2.0_f64.mul_add(v_pad, inner_h)),
    ))
}

struct PlacedCardSlot {
    bounds: Rect,
    proposal: ProposalSize,
    safe_area: Option<SafeAreaLayout>,
}

struct PlacedCardSlots {
    title: Option<(PlacedCardSlot, Environment)>,
    subtitle: Option<(PlacedCardSlot, Environment)>,
    content: Option<PlacedCardSlot>,
}

fn compute_card_slots<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<CardRenderState<E>>>,
    env: &Environment,
    look: &CardLook,
    layout: CardLayout,
) -> PlacedCardSlots {
    let bounds = ctx.bounds;
    let direction = layout_direction(env).snapshot();
    let content_left = bounds.x0 + layout.horizontal_padding;
    let content_right = (bounds.x1 - layout.horizontal_padding).max(content_left);
    let content_top = bounds.y0 + layout.vertical_padding;
    let content_bottom = (bounds.y1 - layout.vertical_padding).max(content_top);
    let content_width = (content_right - content_left).max(0.0);
    let content_height = (content_bottom - content_top).max(0.0);
    let header_proposal = ProposalSize::new(Some(f64_as_f32(content_width)), None);

    let mut cursor_y = content_top;
    let mut title_slot = None;
    let mut subtitle_slot = None;

    {
        let mut state_ref = state.borrow_mut();
        if let Some(title) = &mut state_ref.title {
            let scoped = title_env(env, look);
            let size = title
                .patch_and_measure(ctx.renderer_mut(), &scoped, header_proposal)
                .0;
            let (left, right) = item_horizontal_bounds(
                content_left,
                content_right,
                f64::from(size.width),
                direction,
            );
            let top = cursor_y;
            let bottom = (top + f64::from(size.height)).min(content_bottom);
            let rect = Rect::new(left, top, right, bottom);
            cursor_y = (bottom + layout.content_spacing).min(content_bottom);
            title_slot = Some((
                PlacedCardSlot {
                    bounds: rect,
                    proposal: bounded_proposal(rect),
                    safe_area: ctx.safe_area_for(rect),
                },
                scoped,
            ));
        }

        if let Some(subtitle) = &mut state_ref.subtitle {
            let scoped = subtitle_env(env, look);
            let size = subtitle
                .patch_and_measure(ctx.renderer_mut(), &scoped, header_proposal)
                .0;
            let (left, right) = item_horizontal_bounds(
                content_left,
                content_right,
                f64::from(size.width),
                direction,
            );
            let top = cursor_y;
            let bottom = (top + f64::from(size.height)).min(content_bottom);
            let rect = Rect::new(left, top, right, bottom);
            cursor_y = (bottom + layout.content_spacing).min(content_bottom);
            subtitle_slot = Some((
                PlacedCardSlot {
                    bounds: rect,
                    proposal: bounded_proposal(rect),
                    safe_area: ctx.safe_area_for(rect),
                },
                scoped,
            ));
        }
    }

    let rem_h = (content_bottom - cursor_y).max(0.0).min(content_height);
    let content_measure_proposal =
        ProposalSize::new(Some(f64_as_f32(content_width)), Some(f64_as_f32(rem_h)));
    let content_size = {
        let mut state_ref = state.borrow_mut();
        state_ref.content.as_mut().map(|content| {
            content
                .patch_and_measure(ctx.renderer_mut(), env, content_measure_proposal)
                .0
        })
    };

    let content_slot = content_size.map(|size| {
        let (left, right) = item_horizontal_bounds(
            content_left,
            content_right,
            f64::from(size.width),
            direction,
        );
        let top = cursor_y;
        let bottom = (top + f64::from(size.height)).min(content_bottom);
        let rect = Rect::new(left, top, right, bottom);
        PlacedCardSlot {
            bounds: rect,
            proposal: bounded_proposal(rect),
            safe_area: ctx.safe_area_for(rect),
        }
    });

    PlacedCardSlots {
        title: title_slot,
        subtitle: subtitle_slot,
        content: content_slot,
    }
}

/// Renders a retained card leaf: theme container surface via live chrome parts,
/// followed by stacked title, subtitle, and body content sub-views.
pub fn render_card_node<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<CardRenderState<E>>>,
    env: &Environment,
) {
    let theme = ctx.theme();
    let (look, parts) = {
        let mut state_ref = state.borrow_mut();
        let look = state_ref.ensure_look(&theme, env).clone();
        state_ref.ensure_header_views_with_look(ctx.renderer_mut(), env, &look);
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

    let PlacedCardSlots {
        title: title_slot,
        subtitle: subtitle_slot,
        content: content_slot,
    } = compute_card_slots(ctx, state, env, &look, look.layout());

    let render_ctx = ctx.render_context();
    parts.push_below(ctx.renderer_mut(), render_ctx);
    parts.wrap_content(ctx.renderer_mut(), render_ctx, |renderer| {
        if let Some((slot, scoped)) = title_slot {
            let mut title = state.borrow_mut().title.take();
            if let Some(subview) = &mut title {
                subview.place(
                    renderer,
                    render_ctx,
                    &scoped,
                    slot.proposal,
                    slot.bounds,
                    slot.safe_area,
                );
            }
            let mut state_ref = state.borrow_mut();
            if state_ref.title.is_none() {
                state_ref.title = title;
            }
        }
        if let Some((slot, scoped)) = subtitle_slot {
            let mut subtitle = state.borrow_mut().subtitle.take();
            if let Some(subview) = &mut subtitle {
                subview.place(
                    renderer,
                    render_ctx,
                    &scoped,
                    slot.proposal,
                    slot.bounds,
                    slot.safe_area,
                );
            }
            let mut state_ref = state.borrow_mut();
            if state_ref.subtitle.is_none() {
                state_ref.subtitle = subtitle;
            }
        }
        if let Some(slot) = content_slot {
            let mut content = state.borrow_mut().content.take();
            if let Some(subview) = &mut content {
                subview.place(
                    renderer,
                    render_ctx,
                    env,
                    slot.proposal,
                    slot.bounds,
                    slot.safe_area,
                );
            }
            let mut state_ref = state.borrow_mut();
            if state_ref.content.is_none() {
                state_ref.content = content;
            }
        }
    });
}

/// Emits accessibility nodes for a [`Card`] during semantic-only tree walks.
#[cfg(feature = "accessibility")]
#[cfg(hydrolysis_cherenkov)]
pub fn emit_card_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    state: &Rc<RefCell<CardRenderState<E>>>,
    env: &Environment,
) {
    state
        .borrow_mut()
        .ensure_header_views_semantic(renderer, env);
    let mut title = state.borrow_mut().title.take();
    if let Some(subview) = &mut title {
        subview.emit_accessibility(renderer, env);
    }
    {
        let mut state_ref = state.borrow_mut();
        if state_ref.title.is_none() {
            state_ref.title = title;
        }
    }

    let mut subtitle = state.borrow_mut().subtitle.take();
    if let Some(subview) = &mut subtitle {
        subview.emit_accessibility(renderer, env);
    }
    {
        let mut state_ref = state.borrow_mut();
        if state_ref.subtitle.is_none() {
            state_ref.subtitle = subtitle;
        }
    }

    let mut content = state.borrow_mut().content.take();
    if let Some(subview) = &mut content {
        subview.emit_accessibility(renderer, env);
    }
    {
        let mut state_ref = state.borrow_mut();
        if state_ref.content.is_none() {
            state_ref.content = content;
        }
    }
}
