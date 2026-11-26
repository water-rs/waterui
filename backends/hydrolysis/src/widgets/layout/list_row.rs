//! Hydrolysis realization of [`ListRowConfig`].
//!
//! Draws only the label and value arranged according to [`RowLayout`] with
//! theme-supplied internal spacing and zero extra outer padding, because the
//! enclosing [`List`](waterui::component::list::List) owns the row insets.

use core::cell::RefCell;
use std::rc::Rc;

use kurbo::Rect;
use nami::Signal as _;
use waterui::ViewExt as _;
use waterui::component::list::{ListRowConfig, RowLayout};
use waterui_core::Environment;
use waterui_core::contract_violation;
use waterui_core::layout::{LayoutDirection, ProposalSize, Size, ViewDimensions, layout_direction};

use crate::num_cast::f64_as_f32;
use crate::renderer::{HeldChild, HydroState, RetainedSubview, SemanticCore, WidgetRenderContext};
use crate::text::TextEngine;
use crate::theme::WidgetTheme;
use crate::widgets::util::mirrored_in;

/// Retained render state for a key/value [`ListRowConfig`].
pub struct ListRowRenderState<E: TextEngine> {
    label: RetainedSubview<E>,
    value: RetainedSubview<E>,
    layout: RowLayout,
}

impl<E: TextEngine> ListRowRenderState<E> {
    pub(crate) fn from_config(config: ListRowConfig<HeldChild<E>>) -> Self {
        let (label, value, layout, value_color) = config.into_parts();
        let value = match value_color {
            Some(color) => RetainedSubview::new(value.foreground(color)),
            None => RetainedSubview::new(value),
        };
        Self {
            label: RetainedSubview::held(label.build()),
            value,
            layout,
        }
    }

    pub(crate) fn prebuild(&mut self, renderer: &mut SemanticCore<E>, env: &Environment) {
        self.label.ensure_built(renderer, env);
        self.value.ensure_built(renderer, env);
    }

    pub(crate) fn take_layout_dirty(&mut self) -> bool {
        self.label.take_layout_dirty_if_built() | self.value.take_layout_dirty_if_built()
    }

    /// The label this row contributes when an enclosing `ListItem` names itself
    /// from its content: non-empty trimmed label and value strings joined by a
    /// space, matching [`RenderNode::Container`].
    #[cfg(feature = "accessibility")]
    pub(crate) fn contributed_label(
        &self,
        renderer: &mut SemanticCore<E>,
        _env: &Environment,
    ) -> Option<String> {
        let labels = [&self.label, &self.value]
            .into_iter()
            .filter(|subview| subview.is_built())
            .filter_map(|subview| subview.accessibility_label(renderer))
            .map(|label| label.trim().to_owned())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();
        (!labels.is_empty()).then(|| labels.join(" "))
    }
}

fn inline_proposals(
    available: f64,
    spacing: f64,
    label_natural: f64,
    value_natural: f64,
) -> (ProposalSize, ProposalSize) {
    let remaining = (available - spacing).max(0.0);
    if label_natural + value_natural <= remaining {
        return (ProposalSize::UNSPECIFIED, ProposalSize::UNSPECIFIED);
    }
    let half = remaining * 0.5;
    let (label_max, value_max) = if label_natural <= half {
        (label_natural, (remaining - label_natural).max(0.0))
    } else if value_natural <= half {
        ((remaining - value_natural).max(0.0), value_natural)
    } else {
        (half, half)
    };
    (
        ProposalSize::new(Some(f64_as_f32(label_max)), None),
        ProposalSize::new(Some(f64_as_f32(value_max)), None),
    )
}

/// Measures a key/value list row without adding outer padding.
pub fn measure_list_row_node<E: TextEngine>(
    state: &ListRowRenderState<E>,
    proposal: ProposalSize,
    hydro: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> ViewDimensions {
    let metrics = theme.list_row_metrics(env);
    let proposed_width = proposal.width.filter(|w| w.is_finite()).map(f64::from);

    match state.layout {
        RowLayout::Inline => {
            let label_nat = state.label.measure_built_with_proposal(
                hydro,
                env,
                theme,
                ProposalSize::UNSPECIFIED,
            );
            let value_nat = state.value.measure_built_with_proposal(
                hydro,
                env,
                theme,
                ProposalSize::UNSPECIFIED,
            );
            let spacing = if label_nat.width > 0.0 && value_nat.width > 0.0 {
                f64::from(metrics.inline_spacing)
            } else {
                0.0
            };
            let Some(avail) = proposed_width else {
                let width = f64::from(label_nat.width) + spacing + f64::from(value_nat.width);
                let height = f64::from(label_nat.height).max(f64::from(value_nat.height));
                return ViewDimensions::new(Size::new(f64_as_f32(width), f64_as_f32(height)));
            };
            let (label_prop, value_prop) = inline_proposals(
                avail,
                spacing,
                f64::from(label_nat.width),
                f64::from(value_nat.width),
            );
            let label_size = if label_prop == ProposalSize::UNSPECIFIED {
                label_nat
            } else {
                state
                    .label
                    .measure_built_with_proposal(hydro, env, theme, label_prop)
            };
            let value_size = if value_prop == ProposalSize::UNSPECIFIED {
                value_nat
            } else {
                state
                    .value
                    .measure_built_with_proposal(hydro, env, theme, value_prop)
            };
            let intrinsic_w = f64::from(label_size.width) + spacing + f64::from(value_size.width);
            let width = avail.max(intrinsic_w);
            let height = f64::from(label_size.height).max(f64::from(value_size.height));
            ViewDimensions::new(Size::new(f64_as_f32(width), f64_as_f32(height)))
        }
        RowLayout::Detail => {
            let child_prop = proposed_width.map_or(ProposalSize::UNSPECIFIED, |w| {
                ProposalSize::new(Some(f64_as_f32(w)), None)
            });
            let label_size = state
                .label
                .measure_built_with_proposal(hydro, env, theme, child_prop);
            let value_size = state
                .value
                .measure_built_with_proposal(hydro, env, theme, child_prop);
            let intrinsic_w = f64::from(label_size.width).max(f64::from(value_size.width));
            let width = proposed_width.map_or(intrinsic_w, |w| w.max(intrinsic_w));
            let spacing = if label_size.height > 0.0 && value_size.height > 0.0 {
                f64::from(metrics.detail_spacing)
            } else {
                0.0
            };
            let height = f64::from(label_size.height) + spacing + f64::from(value_size.height);
            ViewDimensions::new(Size::new(f64_as_f32(width), f64_as_f32(height)))
        }
        _ => {
            contract_violation!("hydrolysis: unsupported RowLayout variant in ListRowConfig");
            ViewDimensions::new(Size::zero())
        }
    }
}

struct RowPlacement {
    label_proposal: ProposalSize,
    value_proposal: ProposalSize,
    label_rect: Rect,
    value_rect: Rect,
}

fn measure_inline_placement<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRowRenderState<E>>>,
    env: &Environment,
    bounds: Rect,
    inline_spacing: f64,
) -> RowPlacement {
    let (label_nat, value_nat) = {
        let mut state_ref = state.borrow_mut();
        let renderer = ctx.renderer_mut();
        let label_nat = state_ref
            .label
            .patch_and_measure(renderer, env, ProposalSize::UNSPECIFIED)
            .0;
        let value_nat = state_ref
            .value
            .patch_and_measure(renderer, env, ProposalSize::UNSPECIFIED)
            .0;
        (label_nat, value_nat)
    };
    let spacing = if label_nat.width > 0.0 && value_nat.width > 0.0 {
        inline_spacing
    } else {
        0.0
    };
    let (label_proposal, value_proposal) = inline_proposals(
        bounds.width(),
        spacing,
        f64::from(label_nat.width),
        f64::from(value_nat.width),
    );
    let (label_size, value_size) = {
        let mut state_ref = state.borrow_mut();
        let renderer = ctx.renderer_mut();
        let label_size = if label_proposal == ProposalSize::UNSPECIFIED {
            label_nat
        } else {
            state_ref
                .label
                .patch_and_measure(renderer, env, label_proposal)
                .0
        };
        let value_size = if value_proposal == ProposalSize::UNSPECIFIED {
            value_nat
        } else {
            state_ref
                .value
                .patch_and_measure(renderer, env, value_proposal)
                .0
        };
        (label_size, value_size)
    };

    let label_w = f64::from(label_size.width).min(bounds.width());
    let label_h = f64::from(label_size.height).min(bounds.height());
    let label_top = bounds.y0 + ((bounds.height() - label_h) * 0.5).max(0.0);
    let label_rect = Rect::new(
        bounds.x0,
        label_top,
        bounds.x0 + label_w,
        (label_top + label_h).min(bounds.y1),
    );

    let max_value_w = (bounds.width() - label_w - spacing).max(0.0);
    let value_w = f64::from(value_size.width).min(max_value_w);
    let value_h = f64::from(value_size.height).min(bounds.height());
    let value_top = bounds.y0 + ((bounds.height() - value_h) * 0.5).max(0.0);
    let value_left = (bounds.x1 - value_w)
        .max(label_rect.x1 + spacing)
        .min(bounds.x1);
    let value_rect = Rect::new(
        value_left,
        value_top,
        bounds.x1,
        (value_top + value_h).min(bounds.y1),
    );

    RowPlacement {
        label_proposal,
        value_proposal,
        label_rect,
        value_rect,
    }
}

fn measure_detail_placement<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRowRenderState<E>>>,
    env: &Environment,
    bounds: Rect,
    detail_spacing: f64,
) -> RowPlacement {
    let child_prop = ProposalSize::new(Some(f64_as_f32(bounds.width())), None);
    let (label_size, value_size) = {
        let mut state_ref = state.borrow_mut();
        let renderer = ctx.renderer_mut();
        let label_size = state_ref
            .label
            .patch_and_measure(renderer, env, child_prop)
            .0;
        let value_size = state_ref
            .value
            .patch_and_measure(renderer, env, child_prop)
            .0;
        (label_size, value_size)
    };

    let label_w = f64::from(label_size.width).min(bounds.width());
    let label_h = f64::from(label_size.height).min(bounds.height());
    let label_rect = Rect::new(
        bounds.x0,
        bounds.y0,
        bounds.x0 + label_w,
        bounds.y0 + label_h,
    );

    let spacing = if label_size.height > 0.0 && value_size.height > 0.0 {
        detail_spacing
    } else {
        0.0
    };
    let value_w = f64::from(value_size.width).min(bounds.width());
    let value_top = (label_rect.y1 + spacing).min(bounds.y1);
    let value_h = f64::from(value_size.height).min((bounds.y1 - value_top).max(0.0));
    let value_rect = Rect::new(
        bounds.x0,
        value_top,
        bounds.x0 + value_w,
        value_top + value_h,
    );

    RowPlacement {
        label_proposal: child_prop,
        value_proposal: child_prop,
        label_rect,
        value_rect,
    }
}

/// Renders a key/value list row into `ctx.bounds` without extra outer padding.
pub fn render_list_row_node<E: TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRowRenderState<E>>>,
    env: &Environment,
) {
    let bounds = ctx.bounds;
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return;
    }

    let direction = layout_direction(env).snapshot();
    let metrics = ctx.theme().list_row_metrics(env);
    let label_ctx = ctx.render_context();
    let value_ctx = ctx.render_context();
    let layout = state.borrow().layout;

    let RowPlacement {
        label_proposal: label_prop,
        value_proposal: value_prop,
        label_rect: label_ltr,
        value_rect: value_ltr,
    } = match layout {
        RowLayout::Inline => {
            measure_inline_placement(ctx, state, env, bounds, f64::from(metrics.inline_spacing))
        }
        RowLayout::Detail => {
            measure_detail_placement(ctx, state, env, bounds, f64::from(metrics.detail_spacing))
        }
        _ => {
            contract_violation!("hydrolysis: unsupported RowLayout variant in ListRowConfig");
            return;
        }
    };

    let (label_rect, value_rect) = match direction {
        LayoutDirection::LeftToRight => (label_ltr, value_ltr),
        LayoutDirection::RightToLeft => (
            mirrored_in(label_ltr, bounds),
            mirrored_in(value_ltr, bounds),
        ),
    };
    let label_area = ctx.safe_area_for(label_rect);
    let value_area = ctx.safe_area_for(value_rect);

    let mut state_ref = state.borrow_mut();
    let renderer = ctx.renderer_mut();
    if label_rect.width() > 0.0 && label_rect.height() > 0.0 {
        state_ref
            .label
            .place(renderer, label_ctx, env, label_prop, label_rect, label_area);
    }
    if value_rect.width() > 0.0 && value_rect.height() > 0.0 {
        state_ref
            .value
            .place(renderer, value_ctx, env, value_prop, value_rect, value_area);
    }
}

/// Emits accessibility nodes for both the label and value subviews.
#[cfg(feature = "accessibility")]
#[cfg(hydrolysis_cherenkov)]
pub fn emit_list_row_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    state: &Rc<RefCell<ListRowRenderState<E>>>,
    env: &Environment,
) {
    let mut state = state.borrow_mut();
    state.label.emit_accessibility(renderer, env);
    state.value.emit_accessibility(renderer, env);
}
