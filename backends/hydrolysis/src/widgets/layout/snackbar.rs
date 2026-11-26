//! Hydrolysis realization of [`SnackbarHostConfig`]: self-drawn snackbar
//! banners styled by [`SnackbarLook`] on [`WidgetTheme`](crate::theme::WidgetTheme),
//! with content-hugging width bounds, per-placement stacking reflow, entrance
//! and exit animations, and semantic accessibility support.

use core::cell::RefCell;
use core::time::Duration;

use executor_core::spawn_local;
use nami::{Binding, Signal as _, SignalExt as _};
use waterui::ViewExt as _;
use waterui::shape::{FilledShape, Path, RoundedRectangle, ShapeExt as _};
use waterui::snackbar::{SnackbarHostConfig, SnackbarPosition, SnackbarPresentation};
use waterui::style::Shadow;
use waterui_backend_core::time::Instant;
use waterui_backend_core::{Child, NativeBackendFamily, SlotView};
use waterui_controls::slot::SlotBuilder;
use waterui_controls::{Button, ButtonStyle, Label, button, label};
use waterui_core::{Environment, OneOf2, View};
use waterui_graphics::color::Color;
use waterui_layout::container::FixedContainer;
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::{Alignment, hstack};
use waterui_layout::{
    Layout, LayoutError, PlacementInput, ProposalSize, Rect, Size, StretchAxis, SubView,
    SubviewPlacement, UnitPoint,
};

#[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
use crate::renderer::SemanticCore;
use crate::renderer::{
    HeldChild, HydrolysisRenderer, RenderContext, RetainedSubview, bounded_proposal,
};
use crate::text::TextEngine;
use crate::theme::SnackbarLook;

/// The grid the close-icon geometry below is authored on (24dp viewport).
const CLOSE_ICON_GRID: f32 = 24.0;

/// Outline of the filled `close` icon on [`CLOSE_ICON_GRID`], as the twelve
/// corners of the two crossed bars.
const CLOSE_ICON_OUTLINE: [(f32, f32); 12] = [
    (19.0, 6.41),
    (17.59, 5.0),
    (12.0, 10.59),
    (6.41, 5.0),
    (5.0, 6.41),
    (10.59, 12.0),
    (5.0, 17.59),
    (6.41, 19.0),
    (12.0, 13.41),
    (17.59, 19.0),
    (19.0, 17.59),
    (13.41, 12.0),
];

fn close_icon_path() -> Path {
    let mut corners = CLOSE_ICON_OUTLINE
        .into_iter()
        .map(|(x, y)| (x / CLOSE_ICON_GRID, y / CLOSE_ICON_GRID));
    let (start_x, start_y) = corners
        .next()
        .expect("close icon outline is a non-empty constant");
    corners
        .fold(Path::new().move_to(start_x, start_y), |path, (x, y)| {
            path.line_to(x, y)
        })
        .close()
}

/// The close affordance visual: the icon drawn at `icon_size`, centered in the
/// `box_size` square that carries the button's hit area.
#[derive(Debug, Clone)]
struct CloseIcon {
    color: Color,
    icon_size: f32,
    box_size: f32,
}

impl View for CloseIcon {
    fn body(self, _env: &Environment) -> impl View {
        FilledShape::new(close_icon_path(), self.color)
            .size(self.icon_size, self.icon_size)
            .size(self.box_size, self.box_size)
    }
}

/// Sizes a snackbar banner to its content, floored and capped by the look's
/// width bounds (`[min, max]`).
///
/// The child row is measured unconstrained so an internal spacer never forces
/// the banner to expand to `max`; during placement the row fills the clamped
/// width so trailing controls stay pinned to the trailing edge whenever `min`
/// leaves slack.
#[derive(Debug, Clone, Copy)]
struct HugWidth {
    min: f32,
    max: f32,
}

impl Layout for HugWidth {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let child = children.first().map_or_else(Size::zero, |child| {
            child.measure(ProposalSize::new(None, proposal.height)).size
        });
        Size::new(child.width.clamp(self.min, self.max), child.height)
    }

    fn resolve_extent(
        &self,
        input: PlacementInput,
        _children: &[&dyn SubView],
    ) -> Result<Rect, LayoutError> {
        Ok(input.bounds.align(input.extent()?, UnitPoint::TOP_LEADING))
    }

    fn place(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        let child_proposal = ProposalSize::new(Some(bounds.width()), proposal.height);
        children
            .iter()
            .map(|_| SubviewPlacement::new(bounds, child_proposal))
            .collect()
    }

    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        match children.first() {
            Some(StretchAxis::Vertical | StretchAxis::Both) => StretchAxis::Vertical,
            _ => StretchAxis::None,
        }
    }
}

const fn position_alignment(position: SnackbarPosition) -> Alignment {
    match position {
        SnackbarPosition::TopCenter => Alignment::Top,
        SnackbarPosition::BottomLeading => Alignment::BottomLeading,
        SnackbarPosition::BottomTrailing => Alignment::BottomTrailing,
        SnackbarPosition::BottomCenter | _ => Alignment::Bottom,
    }
}

const fn initial_offset_y(position: SnackbarPosition, travel: f32) -> f32 {
    if position.is_top() { -travel } else { travel }
}

const fn stack_direction(position: SnackbarPosition) -> f32 {
    if position.is_top() { 1.0 } else { -1.0 }
}

struct RetainedSnackbarRow<E: TextEngine> {
    id: u64,
    presentation: SnackbarPresentation<HeldChild<E>>,
    opacity: Binding<f32>,
    entrance_offset: Binding<f32>,
    stack_offset: Binding<f32>,
    exit_started_at: Option<Instant>,
    rendered: RetainedSubview<E>,
}

#[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
struct SemanticSnackbarRow<E: TextEngine> {
    id: u64,
    subview: RetainedSubview<E>,
}

/// Per-node state carried by [`WrapperEffect::SnackbarHost`](crate::renderer::tree::nodes::WrapperEffect).
pub struct SnackbarHostEffect<E: TextEngine> {
    config: SnackbarHostConfig<HeldChild<E>>,
    rows: RefCell<Vec<RetainedSnackbarRow<E>>>,
    #[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
    semantic_rows: RefCell<Vec<SemanticSnackbarRow<E>>>,
}

impl<E: TextEngine> SnackbarHostEffect<E> {
    pub const fn new(config: SnackbarHostConfig<HeldChild<E>>) -> Self {
        Self {
            config,
            rows: RefCell::new(Vec::new()),
            #[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
            semantic_rows: RefCell::new(Vec::new()),
        }
    }
}

struct BuiltSlotView<F: NativeBackendFamily>(SlotBuilder<Child<F>>);

impl<F: NativeBackendFamily> Clone for BuiltSlotView<F> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<F: NativeBackendFamily> View for BuiltSlotView<F> {
    fn body(self, _env: &Environment) -> impl View {
        SlotView::new(self.0.build())
    }
}

fn build_row_content<E: TextEngine>(
    presentation: &SnackbarPresentation<HeldChild<E>>,
    look: &SnackbarLook,
) -> impl View + use<E> {
    let has_trailing_controls = presentation.action().is_some() || presentation.closeable();

    let label_view = presentation
        .icon()
        .map_or_else(
            || {
                OneOf2::B(
                    label(presentation.message().clone()).font(look.supporting_text_font.clone()),
                )
            },
            |icon| {
                OneOf2::A(
                    label(presentation.message().clone())
                        .icon(BuiltSlotView(icon.clone()))
                        .font(look.supporting_text_font.clone()),
                )
            },
        )
        .foreground(look.supporting_text_color.clone());

    let action_button = presentation.action().map(|action| {
        let presentation_for_action = presentation.clone();
        let action_label = action.label().clone();
        let semantic_text = action_label.semantic_text().clone();
        let action_font = look.action_label_font.clone();
        let action_color = look.action_label_color.clone();
        button(Label::new(semantic_text, move || {
            action_label
                .clone()
                .font(action_font.clone())
                .foreground(action_color.clone())
        }))
        .style(ButtonStyle::Borderless)
        .action(move |env: Environment| {
            presentation_for_action.trigger_action(&env);
        })
    });

    let close_button = presentation.closeable().then(|| {
        let presentation_for_close = presentation.clone();
        Button::new(
            label("Close")
                .icon(CloseIcon {
                    color: look.supporting_text_color.clone(),
                    icon_size: look.close_icon_size,
                    box_size: look.close_state_layer_size,
                })
                .icon_only(),
        )
        .style(ButtonStyle::Borderless)
        .action(move |_env: Environment| {
            presentation_for_close.close();
        })
    });

    if !has_trailing_controls {
        return OneOf2::A(hstack((label_view,)).spacing(look.content_spacing));
    }
    OneOf2::B(
        hstack((label_view, spacer(), action_button, close_button)).spacing(look.content_spacing),
    )
}

fn build_rendered_snackbar<E: TextEngine>(
    presentation: &SnackbarPresentation<HeldChild<E>>,
    look: &SnackbarLook,
    opacity: &Binding<f32>,
    entrance_offset: &Binding<f32>,
    stack_offset: &Binding<f32>,
) -> RetainedSubview<E> {
    let position = presentation.position();
    let content_padding = if presentation.action().is_some() || presentation.closeable() {
        EdgeInsets::new(
            look.content_padding.top(),
            look.content_padding.leading(),
            look.content_padding.bottom(),
            look.action_trailing_padding,
        )
    } else {
        look.content_padding.clone()
    };

    let content = build_row_content(presentation, look);
    let enter_animation = look.enter_animation.clone();
    let shape = RoundedRectangle::new(look.corner_radius);
    let shadow = Shadow::splat(look.shadow_radius)
        .color(look.shadow_color.clone())
        .offset(0.0, look.shadow_offset_y)
        .silhouette(shape);
    let ambient_shadow = Shadow::splat(look.ambient_shadow_radius)
        .color(look.ambient_shadow_color.clone())
        .offset(0.0, look.ambient_shadow_offset_y)
        .silhouette(shape);

    let entrance_opacity = opacity.clone();
    let entrance_slide = entrance_offset.clone();

    let bar = FixedContainer::new(
        HugWidth {
            min: look.min_width,
            max: look.max_width,
        },
        (content
            .on_appear(move || {
                entrance_opacity.set(1.0);
                entrance_slide.set(0.0);
            })
            .padding_with(content_padding)
            .height(look.single_line_min_height),),
    );

    let surfaced = if let Some(glass) = &look.background_glass {
        OneOf2::A(bar.background(glass.clone()))
    } else {
        OneOf2::B(bar.background(shape.fill(look.container_color.clone())))
    };

    let view = Frame::new(
        surfaced
            .shadow(ambient_shadow)
            .shadow(shadow)
            .opacity(opacity.with(enter_animation.clone()))
            .offset(0.0, entrance_offset.with(enter_animation.clone()))
            .offset(0.0, stack_offset.with(enter_animation)),
    )
    .alignment(position_alignment(position))
    .padding_with(look.viewport_padding.clone());

    RetainedSubview::new(view)
}

#[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
fn build_semantic_snackbar<E: TextEngine>(
    presentation: &SnackbarPresentation<HeldChild<E>>,
) -> RetainedSubview<E> {
    let label_view = presentation.icon().map_or_else(
        || OneOf2::B(label(presentation.message().clone())),
        |icon| OneOf2::A(label(presentation.message().clone()).icon(BuiltSlotView(icon.clone()))),
    );

    let action_button = presentation.action().map(|action| {
        let presentation_for_action = presentation.clone();
        button(action.label().clone()).action(move |env: Environment| {
            presentation_for_action.trigger_action(&env);
        })
    });

    let close_button = presentation.closeable().then(|| {
        let presentation_for_close = presentation.clone();
        Button::new(label("Close").icon_only()).action(move |_env: Environment| {
            presentation_for_close.close();
        })
    });

    RetainedSubview::new(hstack((label_view, action_button, close_button)))
}

/// Flushes the active snackbar overlay stack above the host's wrapped content.
#[expect(
    clippy::cast_precision_loss,
    reason = "the number of simultaneously visible snackbars is exactly representable as f32"
)]
pub fn flush_snackbar_host<E: TextEngine>(
    renderer: &mut HydrolysisRenderer<E>,
    ctx: RenderContext,
    env: &Environment,
    effect: &SnackbarHostEffect<E>,
) {
    let look = renderer.theme().snackbar(env);
    let presentations = renderer.read_signal(effect.config.presentations());
    let now = renderer.frame_instant();
    let exit_duration = look.exit_animation.duration();

    let mut completed: Vec<SnackbarPresentation<HeldChild<E>>> = Vec::new();
    let mut newly_exiting: Vec<(SnackbarPresentation<HeldChild<E>>, Duration)> = Vec::new();

    {
        let mut rows = effect.rows.borrow_mut();

        // Remove rows whose presentation has left the active list.
        rows.retain(|row| presentations.iter().any(|p| p.id() == row.id));

        // Mount rows for newly arrived presentations, preserving existing ones.
        for presentation in &presentations {
            if rows.iter().any(|row| row.id == presentation.id()) {
                continue;
            }
            let opacity = Binding::new(0.0_f32);
            let entrance_offset = Binding::new(initial_offset_y(
                presentation.position(),
                look.motion_offset_y,
            ));
            let stack_offset = Binding::new(0.0_f32);
            let subview = build_rendered_snackbar(
                presentation,
                &look,
                &opacity,
                &entrance_offset,
                &stack_offset,
            );
            rows.push(RetainedSnackbarRow {
                id: presentation.id(),
                presentation: presentation.clone(),
                opacity,
                entrance_offset,
                stack_offset,
                exit_started_at: None,
                rendered: subview,
            });
        }

        // Observe dismissing flags and kick off exit animations.
        for row in rows.iter_mut() {
            let is_dismissing = renderer.read_signal(row.presentation.dismissing());
            if is_dismissing && row.exit_started_at.is_none() {
                row.exit_started_at = Some(now);
                row.opacity.set(0.0);
                row.entrance_offset.set(initial_offset_y(
                    row.presentation.position(),
                    look.motion_offset_y,
                ));
                if exit_duration.is_zero() {
                    completed.push(row.presentation.clone());
                } else {
                    newly_exiting.push((row.presentation.clone(), exit_duration));
                }
            }
        }

        // Reflow surviving (non-dismissing) rows per placement.
        let row_step = look.single_line_min_height + look.stack_gap;
        for index in 0..rows.len() {
            if rows[index].exit_started_at.is_some() {
                continue;
            }
            let position = rows[index].presentation.position();
            let earlier = rows[..index]
                .iter()
                .filter(|other| {
                    other.exit_started_at.is_none() && other.presentation.position() == position
                })
                .count();
            let target = stack_direction(position) * (earlier as f32) * row_step;
            if (rows[index].stack_offset.snapshot() - target).abs() > f32::EPSILON {
                rows[index].stack_offset.set(target);
            }
        }

        // Place each retained snackbar overlay over the host's bounds.
        let bounds = ctx.bounds();
        let proposal = bounded_proposal(bounds);
        for row in rows.iter_mut() {
            row.rendered
                .place(renderer, ctx, env, proposal, bounds, None);
            if let Some(started) = row.exit_started_at
                && !exit_duration.is_zero()
                && now.saturating_duration_since(started) >= exit_duration
            {
                completed.push(row.presentation.clone());
            }
        }
    }

    // Schedule async completion for real windowed runners whose frame loop may
    // go idle once visual motions finish; `complete_dismissal` is idempotent.
    for (presentation, duration) in newly_exiting {
        spawn_local(async move {
            native_executor::sleep(duration).await;
            presentation.complete_dismissal();
        })
        .detach();
    }

    // Complete any finished exits outside the `rows` borrow so re-entrant
    // `on_dismiss` callbacks may safely show or dismiss snackbars.
    for presentation in completed {
        presentation.complete_dismissal();
    }
}

/// Emits accessibility nodes for live snackbars during a semantic walk without
/// consulting [`WidgetTheme`](crate::theme::WidgetTheme).
#[cfg(all(feature = "accessibility", hydrolysis_cherenkov))]
pub fn emit_snackbar_host_accessibility<E: TextEngine>(
    renderer: &mut SemanticCore<E>,
    env: &Environment,
    effect: &SnackbarHostEffect<E>,
) {
    let presentations = renderer.read_signal(effect.config.presentations());
    let mut completed = Vec::new();
    let mut live_ids = Vec::new();

    for presentation in &presentations {
        if renderer.read_signal(presentation.dismissing()) {
            completed.push(presentation.clone());
        } else {
            live_ids.push(presentation.id());
        }
    }

    {
        let mut rows = effect.semantic_rows.borrow_mut();
        rows.retain(|row| live_ids.contains(&row.id));
        for presentation in &presentations {
            if !live_ids.contains(&presentation.id()) {
                continue;
            }
            if rows.iter().any(|row| row.id == presentation.id()) {
                continue;
            }
            rows.push(SemanticSnackbarRow {
                id: presentation.id(),
                subview: build_semantic_snackbar(presentation),
            });
        }
        for row in rows.iter_mut() {
            row.subview.emit_accessibility(renderer, env);
        }
    }

    for presentation in completed {
        presentation.complete_dismissal();
    }
}
