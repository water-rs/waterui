#[cfg(feature = "list")]
use crate::theme::ListMetrics;
#[cfg(any(feature = "text-field", feature = "secure-field"))]
use crate::theme::TextFieldLook;
#[cfg(feature = "navigation")]
use crate::theme::{NavigationBarLook, NavigationMetrics, NavigationSpec};
#[cfg(any(feature = "text-field", feature = "secure-field"))]
use crate::widgets::controls::field_look::placeholder_text;
#[cfg(any(feature = "text-field", feature = "secure-field"))]
use crate::widgets::controls::field_look::value_text;
#[cfg(feature = "picker")]
use crate::widgets::controls::picker::segmented_intrinsic;
#[cfg(feature = "list")]
use waterui_core::backend::ChildSlot;
#[cfg(feature = "list")]
use waterui_core::views::AnyViewsSnapshot;
#[cfg(any(feature = "text-field", feature = "secure-field"))]
use waterui_interaction::interaction::InteractionState;
#[cfg(feature = "list")]
use waterui_layout::padding::EdgeInsets;
// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::text::TextLayout as _;
#[cfg(any(feature = "list", feature = "navigation", feature = "picker"))]
use crate::theme::WidgetTheme;

use std::rc::Rc;
#[cfg(any(feature = "list", feature = "picker"))]
use waterui_core::contract_violation;
#[cfg(feature = "navigation")]
use waterui_core::layout::LayoutDirection;
#[cfg(any(feature = "list", feature = "navigation"))]
use waterui_core::layout::layout_direction;
#[cfg(feature = "picker")]
use waterui_form::picker::PickerStyle;

#[cfg(feature = "table")]
pub struct MeasuredTableMetrics {
    pub(crate) column_widths: Vec<f64>,
    pub(crate) table_width: f64,
    pub table_height: f64,
}

#[cfg(feature = "table")]
pub fn table_header_cell_rect(
    origin_x: f64,
    origin_y: f64,
    x_offset: f64,
    width: f64,
    metrics: crate::theme::TableMetrics,
) -> kurbo::Rect {
    kurbo::Rect::new(
        origin_x + x_offset,
        origin_y,
        origin_x + x_offset + width,
        origin_y + f64::from(metrics.header_height),
    )
}

#[cfg(feature = "table")]
pub fn table_data_cell_rect(
    origin_x: f64,
    origin_y: f64,
    x_offset: f64,
    width: f64,
    row_index: usize,
    metrics: crate::theme::TableMetrics,
) -> kurbo::Rect {
    let row_height = f64::from(metrics.row_height);
    let y0 = row_height.mul_add(
        crate::num_cast::usize_as_f64(row_index),
        origin_y + f64::from(metrics.header_height),
    );
    kurbo::Rect::new(
        origin_x + x_offset,
        y0,
        origin_x + x_offset + width,
        y0 + row_height,
    )
}

/// The navigation chrome's look for the spec of `env`.
#[cfg(feature = "navigation")]
pub fn navigation_look(theme: &Rc<dyn WidgetTheme>, env: &Environment) -> NavigationBarLook {
    theme.navigation_bar(&NavigationSpec::new(layout_direction(env).snapshot()), env)
}

/// The height of a bar showing its title in `display_mode`, under `metrics`.
#[cfg(feature = "navigation")]
pub fn navigation_base_bar_height(
    display_mode: waterui_backend_core::navigation::NavigationTitleDisplayMode,
    metrics: NavigationMetrics,
) -> f64 {
    f64::from(match display_mode {
        waterui_backend_core::navigation::NavigationTitleDisplayMode::Automatic => {
            metrics.automatic_bar_height
        }
        waterui_backend_core::navigation::NavigationTitleDisplayMode::Inline => {
            metrics.inline_bar_height
        }
        waterui_backend_core::navigation::NavigationTitleDisplayMode::Medium => {
            metrics.medium_bar_height
        }
        waterui_backend_core::navigation::NavigationTitleDisplayMode::Large => {
            metrics.large_bar_height
        }
    })
}

#[cfg(feature = "navigation")]
pub fn split_compact_threshold(sidebar_width: f64) -> f64 {
    sidebar_width + 360.0
}

impl<E: crate::text::TextEngine> HydrolysisRenderer<E> {
    pub(crate) fn render_styled_text(
        &mut self,
        ctx: RenderContext,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
    ) {
        self.render_styled_text_limited(
            ctx,
            ctx.bounds_width_f32(),
            styled,
            alignment,
            env,
            TailMark::None,
            None,
        );
    }

    /// Records `styled` broken at `max_width` with its `tail` treatment, its
    /// default-coloured runs drawn in `paint`, live, when given.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    #[expect(
        clippy::too_many_arguments,
        reason = "the text, its placement, its environment and its paint are independent inputs"
    )]
    pub(crate) fn render_styled_text_limited(
        &mut self,
        ctx: RenderContext,
        max_width: f32,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        tail: TailMark,
        paint: Option<&nami::Computed<cherenkov_record::Paint>>,
    ) {
        let input = resolve_text_layout_input(&styled, alignment, env);
        self.record_text_input(ctx, max_width, &input, tail, paint);
    }

    /// [`Self::render_styled_text_limited`], also returning the layout the
    /// recording was made from — the handle `record` shaped under its key,
    /// re-fetched from the shared cache rather than shaped twice. The text
    /// leaf uses it for per-span geometry (tap targets and `Link`
    /// accessibility children); paint-only callers skip the lookup.
    #[expect(
        clippy::too_many_arguments,
        reason = "the text, its placement, its environment and its paint are independent inputs"
    )]
    pub(crate) fn render_styled_text_with_layout(
        &mut self,
        ctx: RenderContext,
        max_width: f32,
        styled: &StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        tail: TailMark,
        paint: Option<&nami::Computed<cherenkov_record::Paint>>,
    ) -> E::Layout {
        let input = resolve_text_layout_input(styled, alignment, env);
        self.record_text_input(ctx, max_width, &input, tail, paint);
        let (max_lines, tail_ellipsis) = tail.parts();
        if tail_ellipsis {
            self.state
                .text
                .shape_limited(&input, Some(max_width), max_lines)
        } else {
            self.state.text.shape(&input, Some(max_width))
        }
    }

    /// Records `input` broken at `max_width` and positioned at `ctx`'s
    /// origin, its default-coloured runs in `paint` when given, and pushes
    /// the recording as a live source.
    fn record_text_input(
        &mut self,
        ctx: RenderContext,
        max_width: f32,
        input: &crate::text::ResolvedTextLayoutInput,
        tail: TailMark,
        paint: Option<&nami::Computed<cherenkov_record::Paint>>,
    ) {
        let state = &mut self.state;
        let max_width = Some(max_width);
        let fragment = match paint {
            Some(paint) => {
                state
                    .text
                    .record_painted(input, max_width, tail, paint, &mut state.counters)
            }
            None => state.text.record(
                input,
                max_width,
                tail,
                &mut state.counters,
                &mut state.text_scenes,
            ),
        };
        let source: Rc<dyn crate::renderer::chrome::LiveSource> = fragment;
        self.push_live(
            {
                let mut literal = ctx;
                literal.set_local(
                    ctx.local() * kurbo::Affine::translate((ctx.bounds().x0, ctx.bounds().y0)),
                );
                literal
            },
            &source,
        );
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn render_styled_text_single_line_centered(
        &mut self,
        ctx: RenderContext,
        styled: StyledStr,
        env: &Environment,
        scale: Option<(&Rc<crate::renderer::chrome::LiveChrome>, kurbo::Affine)>,
        paint: Option<&nami::Computed<cherenkov_record::Paint>>,
    ) {
        let state = &mut self.state;
        let input = resolve_text_layout_input(&styled, HorizontalAlignment::Leading, env);
        let layout = state.measuring_text().shape(&input, None);
        if layout.line_count() == 0 {
            return;
        }
        let metrics = layout.line_metrics(0);
        // Center the measured frame — advance widened to cover overhanging
        // ink — matching what `dimensions` reports for it.
        let width = state.text.ink_extent(&layout, Some(1)).map_or_else(
            || f64::from(metrics.advance),
            |(ink_min, ink_max)| f64::from(metrics.advance.max(ink_max) - ink_min.min(0.0)),
        );
        let height = f64::from(metrics.line_height);
        let x = ((ctx.bounds().width() - width) * 0.5).max(0.0);
        let y = ((ctx.bounds().height() - height) * 0.5).max(0.0);
        let fragment = match paint {
            Some(paint) => state.text.record_painted(
                &input,
                None,
                TailMark::Clip(1),
                paint,
                &mut state.counters,
            ),
            None => state.text.record(
                &input,
                None,
                TailMark::Clip(1),
                &mut state.counters,
                &mut state.text_scenes,
            ),
        };
        let mut literal = ctx;
        literal.set_local(ctx.local() * kurbo::Affine::translate((x, y)));
        let source: Rc<dyn crate::renderer::chrome::LiveSource> = match scale {
            Some((chrome, widget)) => Rc::new(crate::renderer::chrome::ScaledContent::new(
                fragment,
                Rc::clone(chrome),
                widget.inverse() * literal.local(),
            )),
            None => fragment,
        };
        self.push_live(literal, &source);
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    pub(crate) fn build_text_layout(
        state: &mut HydroState<E>,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
    ) -> E::Layout {
        let input = resolve_text_layout_input(&styled, alignment, env);
        state.measuring_text().shape(&input, max_width)
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn measure_text_dimensions(
        state: &mut HydroState<E>,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
        max_lines: Option<usize>,
    ) -> ViewDimensions {
        let input = resolve_text_layout_input(&styled, alignment, env);
        let text = state.measuring_text();
        let layout = text.shape_limited(&input, max_width, max_lines);
        text.dimensions(&layout, max_lines)
    }

    pub(crate) fn measure_text_intrinsic_size(
        state: &mut HydroState<E>,
        styled: StyledStr,
        env: &Environment,
    ) -> LayoutSize {
        Self::measure_text_dimensions(state, styled, HorizontalAlignment::Leading, env, None, None)
            .size
    }

    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    pub(crate) fn measure_text_intrinsic_size_with_line_limit(
        state: &mut HydroState<E>,
        styled: StyledStr,
        env: &Environment,
        max_lines: Option<usize>,
    ) -> LayoutSize {
        Self::measure_text_dimensions(
            state,
            styled,
            HorizontalAlignment::Leading,
            env,
            None,
            max_lines,
        )
        .size
    }

    /// The size of an editable field's content, at most `max_lines` lines.
    /// Empty text still measures the one line a caret occupies, so a field
    /// keeps its height as text is entered.
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    pub(crate) fn measure_editable_text_size(
        state: &HydroState<E>,
        styled: &StyledStr,
        env: &Environment,
        max_lines: Option<usize>,
    ) -> LayoutSize {
        let input = resolve_text_layout_input(styled, HorizontalAlignment::Leading, env);
        let layout = state.text.shape_editable(&input, None);
        state.text.dimensions(&layout, max_lines).size
    }
}

/// The proposal a tab's content measures at: the pane minus the tab bar — a
/// bottom strip for `Automatic`/`TabBar`, a leading strip for `Sidebar`.
/// This is a *measurement* proposal, not the rendered content rect: the §7.1
/// chrome split at render time carves the band out of the laid-out frame,
/// and the rendered content rect keeps the whole frame once the keyboard
/// covers the band outright (keyboard deeper than container inset plus bar
/// height). Bounded axes echo the offer; an axis the container left open
/// stays open.
#[cfg(feature = "navigation")]
pub fn tabs_content_proposal(
    proposal: ProposalSize,
    style: TabStyle,
    bar_extent: f64,
) -> ProposalSize {
    if matches!(style, TabStyle::Sidebar) {
        ProposalSize::new(
            proposal
                .width
                .map(|width| crate::num_cast::f64_as_f32((f64::from(width) - bar_extent).max(0.0))),
            proposal.height,
        )
    } else {
        ProposalSize::new(
            proposal.width,
            proposal.height.map(|height| {
                crate::num_cast::f64_as_f32((f64::from(height) - bar_extent).max(0.0))
            }),
        )
    }
}

#[cfg(feature = "navigation")]
pub fn tabs_button_rect(
    bar_rect: kurbo::Rect,
    tab_count: usize,
    index: usize,
    style: TabStyle,
) -> kurbo::Rect {
    if matches!(style, TabStyle::Sidebar) {
        let button_height = bar_rect.height() / crate::num_cast::usize_as_f64(tab_count);
        let y0 = button_height.mul_add(crate::num_cast::usize_as_f64(index), bar_rect.y0);
        kurbo::Rect::new(bar_rect.x0, y0, bar_rect.x1, y0 + button_height)
    } else {
        let button_width = bar_rect.width() / crate::num_cast::usize_as_f64(tab_count);
        let x0 = button_width.mul_add(crate::num_cast::usize_as_f64(index), bar_rect.x0);
        kurbo::Rect::new(x0, bar_rect.y0, x0 + button_width, bar_rect.y1)
    }
}

#[cfg(feature = "navigation")]
pub fn navigation_back_button_rect(
    bounds: kurbo::Rect,
    metrics: NavigationMetrics,
    direction: LayoutDirection,
) -> kurbo::Rect {
    let leading_inset = f64::from(metrics.back_button_leading_inset);
    let top_inset = f64::from(metrics.back_button_top_inset);
    let button_size = f64::from(metrics.back_button_size);
    let (x0, x1) = if direction.is_right_to_left() {
        let x1 = bounds.x1 - leading_inset;
        (x1 - button_size, x1)
    } else {
        let x0 = bounds.x0 + leading_inset;
        (x0, x0 + button_size)
    };
    kurbo::Rect::new(
        x0,
        bounds.y0 + top_inset,
        x1,
        bounds.y0 + top_inset + button_size,
    )
}

/// The `(left, right)` insets a list row opens its content by: the row's
/// `insets` replace the theme's row insets edge for edge, and unset the theme's
/// symmetric horizontal inset applies. `EdgeInsets` is logical, so leading is
/// the side the layout direction starts from.
#[cfg(feature = "list")]
pub fn list_row_horizontal_insets(
    insets: Option<&EdgeInsets>,
    metrics: ListMetrics,
    env: &Environment,
) -> (f64, f64) {
    let default_inset = f64::from(metrics.horizontal_inset);
    let (leading, trailing) = insets.map_or((default_inset, default_inset), |insets| {
        (f64::from(insets.leading()), f64::from(insets.trailing()))
    });
    if layout_direction(env).snapshot().is_right_to_left() {
        (trailing, leading)
    } else {
        (leading, trailing)
    }
}

/// How a list draws its backdrop, row containers, and selection fill.
#[cfg(feature = "list")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSurfaceKind {
    /// Standard theme rows with `SelectionContainer` selection fill.
    Plain,
    /// Full-width grouped bands over the list's `Background` token.
    Grouped,
    /// Horizontally inset rounded cards per section over `Background`.
    InsetGrouped,
    /// Transparent sidebar rows with an unemphasised rounded pill selection.
    Sidebar,
}

/// Which rows draw a bottom separator hairline.
#[cfg(feature = "list")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSeparatorMode {
    /// Draw separators under every row.
    All,
    /// Draw separators between rows of the same section, omitting the section's final row.
    WithinSection,
    /// Draw no row separators.
    None,
}

/// Visual and layout chrome derived from a resolved [`ListStyle`].
#[cfg(feature = "list")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListStyleGeometry {
    /// Horizontal outer inset applied to each row's visual rect.
    pub outer_horizontal_inset: f64,
    /// Corner radius of a row, card, or selection pill.
    pub corner_radius: f64,
    /// Vertical gap inserted above non-first sections.
    pub inter_section_gap: f64,
    /// Vertical gap left below each row (e.g., compact sidebar pills).
    pub row_gap: f64,
    /// Default minimum row height when `ListMinRowHeight` is unset.
    pub default_min_row_height: Option<f32>,
    /// Backdrop and row container treatment.
    pub surface: ListSurfaceKind,
    /// Separator visibility rule.
    pub separators: ListSeparatorMode,
}

/// Resolves the geometry and chrome flags for `style`.
#[cfg(feature = "list")]
#[must_use]
pub const fn list_style_geometry(style: waterui_controls::list::ListStyle) -> ListStyleGeometry {
    use waterui_controls::list::ListStyle;
    match style {
        ListStyle::Inset => ListStyleGeometry {
            outer_horizontal_inset: 16.0,
            corner_radius: 6.0,
            inter_section_gap: 0.0,
            row_gap: 0.0,
            default_min_row_height: None,
            surface: ListSurfaceKind::Plain,
            separators: ListSeparatorMode::WithinSection,
        },
        ListStyle::Grouped => ListStyleGeometry {
            outer_horizontal_inset: 0.0,
            corner_radius: 0.0,
            inter_section_gap: 12.0,
            row_gap: 0.0,
            default_min_row_height: None,
            surface: ListSurfaceKind::Grouped,
            separators: ListSeparatorMode::WithinSection,
        },
        ListStyle::InsetGrouped => ListStyleGeometry {
            outer_horizontal_inset: 16.0,
            corner_radius: 12.0,
            inter_section_gap: 12.0,
            row_gap: 0.0,
            default_min_row_height: None,
            surface: ListSurfaceKind::InsetGrouped,
            separators: ListSeparatorMode::WithinSection,
        },
        ListStyle::Sidebar => ListStyleGeometry {
            outer_horizontal_inset: 8.0,
            corner_radius: 8.0,
            inter_section_gap: 8.0,
            row_gap: 2.0,
            default_min_row_height: Some(36.0),
            surface: ListSurfaceKind::Sidebar,
            separators: ListSeparatorMode::None,
        },
        // `ListStyle::Automatic`, `ListStyle::Plain`, and any future variants default to plain chrome.
        _ => ListStyleGeometry {
            outer_horizontal_inset: 0.0,
            corner_radius: 0.0,
            inter_section_gap: 0.0,
            row_gap: 0.0,
            default_min_row_height: None,
            surface: ListSurfaceKind::Plain,
            separators: ListSeparatorMode::All,
        },
    }
}

/// The spec of a list in `env` with presentation `style`.
#[must_use]
#[cfg(feature = "list")]
pub fn list_spec(
    env: &Environment,
    style: waterui_controls::list::ListStyle,
) -> crate::theme::ListSpec {
    crate::theme::ListSpec::new(layout_direction(env).snapshot()).with_style(style)
}

/// The list's own size from its first row's measured content, which every
/// row is estimated at: `insets` are that row's [`ViewExt::list_row_insets`].
#[cfg(feature = "list")]
pub fn list_intrinsic_for_first_row<E: crate::text::TextEngine, C>(
    list: &ListConfig<C>,
    contents: &AnyViewsSnapshot<C>,
    content_size: LayoutSize,
    insets: Option<&EdgeInsets>,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let row_count = contents.len();
    if row_count == 0 {
        return LayoutSize::zero();
    }
    let editing = state.measure_signal(list.editing());
    let style = list.style();
    let style_geom = list_style_geometry(style);
    let metrics = theme.list(&list_spec(env, style), env).metrics();
    let min_height = list.min_row_height().or(style_geom.default_min_row_height);
    let row_height =
        list_row_height_for_content(f64::from(content_size.height), insets, min_height, metrics)
            + style_geom.row_gap;

    let horizontal_inset = f64::from(metrics.horizontal_inset);
    let outer_inset = style_geom.outer_horizontal_inset * 2.0;
    let mut row_width = horizontal_inset.mul_add(2.0, f64::from(content_size.width)) + outer_inset;
    if editing && list.edits().has_move() {
        row_width += f64::from(metrics.move_control_width + metrics.trailing_control_spacing);
    }
    if editing && list.edits().has_delete() {
        row_width += f64::from(metrics.delete_control_width + metrics.trailing_control_spacing);
    }
    // Section chrome is part of the list's own height: each section holding
    // rows draws its header and footer once. The sections come from row ids,
    // so no row is materialized for them.
    let mut section_height = 0.0;
    for (section_index, span) in list.sections().spans(contents).into_iter().enumerate() {
        if section_index > 0 {
            section_height += style_geom.inter_section_gap;
        }
        let Some(section) = span.section else {
            continue;
        };
        if section.label().is_some() {
            section_height += f64::from(metrics.section_header_height);
        }
        if section.footer().is_some() {
            section_height += f64::from(metrics.section_footer_height);
        }
    }

    let total_height = row_height.mul_add(crate::num_cast::usize_as_f64(row_count), section_height);
    let max_width = row_width.max(horizontal_inset.mul_add(2.0, outer_inset));

    LayoutSize::new(
        crate::num_cast::f64_as_f32(max_width),
        crate::num_cast::f64_as_f32(total_height),
    )
}

/// Materializes the row at `index` from `contents` — an immutable
/// snapshot of the row collection, so the caller's whole pass (section walk,
/// accessibility emit, or draw loop) reads one coherent membership.
#[cfg(feature = "list")]
pub fn materialize_list_item<E: crate::text::TextEngine>(
    contents: &AnyViewsSnapshot<HeldChild<E>>,
    index: usize,
) -> HeldChild<E> {
    contents.get_view(index).unwrap_or_else(|| {
        contract_violation!(
            "ListConfig failed to materialize item at index {index}; the row is empty"
        );
        HeldChild::from_view(())
    })
}

/// A list row's extent from its content's measured height: the content plus
/// the row's vertical insets, floored at the configured minimum. `insets` comes
/// from `ListRowInsets` — `None` uses the theme's `vertical_inset` — and
/// `min_height` is `ListConfig::min_row_height` — `None` uses the theme's
/// `one_line_row_height`, so both unset reproduces the theme metrics exactly.
/// The caller measures the content (the realized row's node) and the
/// section chrome height is added on top of what this returns.
#[cfg(feature = "list")]
pub fn list_row_height_for_content(
    content_height: f64,
    insets: Option<&EdgeInsets>,
    min_height: Option<f32>,
    metrics: crate::theme::ListMetrics,
) -> f64 {
    let vertical_insets = insets.map_or_else(
        || f64::from(metrics.vertical_inset) * 2.0,
        |insets| f64::from(insets.top() + insets.bottom()),
    );
    let floor = f64::from(min_height.unwrap_or(metrics.one_line_row_height));
    (content_height + vertical_insets).max(floor)
}

/// Measures a text field's size under a concrete proposal, from the label
/// size measured from its built `RetainedSubview` and the field's look, so
/// layout and the floating-label render agree on the label height.
///
/// The field is a `Horizontal` leaf: it answers the proposal width floored at
/// the label's extent, so the reported width never depends on the field's
/// text — text scrolls inside the dealt width. `None` answers the ideal, the
/// look's `min_width` floored at the label. Height is the intrinsic height,
/// whose text line an empty field measures too.
#[cfg(feature = "text-field")]
pub fn measure_text_field_size_with_label_size<E: crate::text::TextEngine, S>(
    text_field: &TextFieldConfig<S>,
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
    look: &TextFieldLook,
    proposal: ProposalSize,
) -> LayoutSize {
    let metrics = look.metrics();
    let line_limit = state
        .measure_signal(text_field.line_limit())
        .map(NonZeroUsize::get);
    let prompt = placeholder_text(
        look,
        state
            .measure_signal(&text_field.prompt().content())
            .to_plain(),
        kurbo::Rect::ZERO,
        InteractionState::empty(),
    );
    let value = value_text(
        look,
        state.measure_signal(text_field.value()),
        kurbo::Rect::ZERO,
        InteractionState::empty(),
    );
    let prompt_size = HydrolysisRenderer::measure_text_intrinsic_size_with_line_limit(
        state, prompt, env, line_limit,
    );
    let value_size = HydrolysisRenderer::measure_editable_text_size(state, &value, env, line_limit);
    let label_height = measured_input_label_height(label_size, metrics.label_height);
    let text_height = prompt_size.height.max(value_size.height);
    let label_width = metrics.horizontal_inset.mul_add(2.0, label_size.width);

    let field_height = measured_input_field_height(text_height, label_height, metrics);
    let width = input_field_width(
        proposal.width,
        label_width.max(metrics.min_width),
        label_width,
    );
    LayoutSize::new(width, field_height)
}

/// Resolves an input field's width from a proposal: every `Some` proposal is
/// answered floored at `minimum` — the `0` probe included — so an unbounded
/// probe answers `INFINITY`; `None` answers the `ideal` (theme `min_width`
/// floor applied). The field's text is not an input to the width.
#[cfg(any(feature = "text-field", feature = "secure-field"))]
fn input_field_width(proposal: Option<f32>, ideal: f32, minimum: f32) -> f32 {
    proposal.map_or(ideal, |width| width.max(minimum))
}

/// Measures a secure field's size under a concrete proposal, from the label
/// size measured from its built `RetainedSubview` and the field's look, so
/// layout and the floating-label render agree on the label height. Same
/// `Horizontal`-leaf contract as the text field: the width answers the
/// proposal floored at the label's extent and never the masked text; `None`
/// answers the look's `min_width` ideal.
#[cfg(feature = "secure-field")]
pub fn measure_secure_field_size_with_label_size<E: crate::text::TextEngine, S>(
    secure_field: &SecureFieldConfig<S>,
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
    look: &TextFieldLook,
    proposal: ProposalSize,
) -> LayoutSize {
    let metrics = look.metrics();
    let secure_len = state
        .measure_signal(secure_field.value())
        .expose()
        .chars()
        .count();
    let prompt = placeholder_text(
        look,
        state
            .measure_signal(&secure_field.prompt().content())
            .to_plain(),
        kurbo::Rect::ZERO,
        InteractionState::empty(),
    );
    let masked = value_text(
        look,
        StyledStr::plain("*".repeat(secure_len)),
        kurbo::Rect::ZERO,
        InteractionState::empty(),
    );
    let prompt_size = HydrolysisRenderer::measure_text_intrinsic_size_with_line_limit(
        state,
        prompt,
        env,
        Some(1),
    );
    let value_size = HydrolysisRenderer::measure_editable_text_size(state, &masked, env, None);
    let label_height = measured_input_label_height(label_size, metrics.label_height);
    let text_height = prompt_size.height.max(value_size.height);
    let label_width = metrics.horizontal_inset.mul_add(2.0, label_size.width);
    let field_height = measured_input_field_height(text_height, label_height, metrics);
    let width = input_field_width(
        proposal.width,
        label_width.max(metrics.min_width),
        label_width,
    );
    LayoutSize::new(width, field_height)
}

/// The band a labeled field reserves for its floating label is exactly the
/// theme's `label_height`: the label scales into the band when it floats, so
/// the unscaled label measure only decides whether the band exists, never
/// how tall it is (water-rs/waterui#2387).
#[cfg(any(feature = "text-field", feature = "secure-field"))]
fn measured_input_label_height(label_size: LayoutSize, label_height: f32) -> f32 {
    if label_size.width > 0.0 || label_size.height > 0.0 {
        label_height
    } else {
        0.0
    }
}

#[cfg(any(feature = "text-field", feature = "secure-field"))]
fn measured_input_field_height(
    text_height: f32,
    label_height: f32,
    metrics: crate::theme::InputFieldMetrics,
) -> f32 {
    let measured_height = if label_height > 0.0 {
        label_height + metrics.vertical_inset + text_height + metrics.vertical_inset
    } else {
        metrics.vertical_inset.mul_add(2.0, text_height)
    };
    measured_height.max(metrics.min_height)
}

/// The cells of a table a frame shows: the rows and columns in view.
#[derive(Debug, Clone, Copy)]
#[cfg(feature = "table")]
pub struct VisibleCells {
    pub rows: VisibleIndexWindow,
    pub columns: VisibleColumnWindow,
}

#[cfg(feature = "picker")]
fn resolved_text_styled<E: crate::text::TextEngine>(
    text: &Text,
    env: &Environment,
    state: &mut HydroState<E>,
) -> StyledStr {
    let resolved = text.resolve(env);
    state.measure_signal(&resolved.content())
}

/// Measures a segmented picker of `items` under `look` beneath a heading of
/// `label_size`: each item's label at the larger of its selected and
/// unselected fonts, laid out by the look's layout.
#[cfg(feature = "picker")]
pub fn measure_segmented_intrinsic<E: crate::text::TextEngine>(
    look: &crate::theme::SegmentedLook,
    items: &[waterui_form::picker::PickerItem<waterui_core::id::Id>],
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
) -> LayoutSize {
    if items.is_empty() {
        contract_violation!(
            "hydrolysis picker requires at least one item; the picker measures as zero"
        );
        return LayoutSize::zero();
    }
    let count = items.len();
    let sizes: Vec<LayoutSize> = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let styled = resolved_text_styled(&item.content, env, state);
            crate::widgets::controls::picker::segmented_label_size(
                look,
                &styled,
                crate::theme::Segment { index, count },
                state,
                env,
            )
        })
        .collect();
    let effective_label = if look.shows_label() {
        label_size
    } else {
        LayoutSize::zero()
    };
    segmented_intrinsic(look.layout(), &sizes, effective_label)
}

/// The intrinsic size of a menu picker's field: the widest item value, the
/// in-field label above it and the look's insets and indicator space.
#[cfg(feature = "picker")]
fn measure_menu_picker_intrinsic<E: crate::text::TextEngine>(
    look: &crate::theme::MenuPickerLook,
    items: &[waterui_form::picker::PickerItem<waterui_core::id::Id>],
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
) -> LayoutSize {
    let metrics = look.layout();
    let mut max_item_width: f64 = 0.0;
    let mut max_item_height: f64 = 0.0;
    for item in items {
        let styled = crate::widgets::controls::button::label_styled(
            look.get_value(),
            resolved_text_styled(&item.content, env, state),
        );
        let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
        max_item_width = max_item_width.max(f64::from(size.width));
        max_item_height = max_item_height.max(f64::from(size.height));
    }

    // The field label sits inside the field above the value: it adds its own
    // height plus the metrics' label spacing to the field height, and its
    // width competes with the items for the field's content width. A label
    // measuring zero height (a hidden one) adds nothing — the field keeps its
    // unlabelled size. The render path decides presence the same way, on
    // `label_size.height > 0`.
    let has_label = label_size.height > 0.0;
    let content_width = max_item_width.max(f64::from(label_size.width));
    let width = (f64::from(metrics.horizontal_inset).mul_add(2.0, content_width)
        + f64::from(metrics.indicator_space))
    .max(f64::from(metrics.min_width));
    let height = f64::from(metrics.vertical_inset)
        .mul_add(
            2.0,
            max_item_height
                + f64::from(label_size.height)
                + if has_label {
                    f64::from(metrics.label_spacing)
                } else {
                    0.0
                },
        )
        .max(f64::from(metrics.min_height));
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

/// The intrinsic size of a radio group: one row per item, stacked with the
/// look's row spacing, under the group label.
#[cfg(feature = "picker")]
fn measure_radio_picker_intrinsic<E: crate::text::TextEngine>(
    look: &crate::theme::RadioGroupLook,
    items: &[waterui_form::picker::PickerItem<waterui_core::id::Id>],
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
) -> LayoutSize {
    let metrics = look.layout();
    let indicator = look.indicator();
    let mut max_item_width: f64 = 0.0;
    let mut total_height = 0.0;
    for (index, item) in items.iter().enumerate() {
        let styled = crate::widgets::controls::button::label_styled(
            look.get_label(),
            resolved_text_styled(&item.content, env, state),
        );
        let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
        max_item_width = max_item_width.max(f64::from(size.width));
        total_height += f64::from(size.height).max(indicator.size().height);
        if index + 1 < items.len() {
            total_height += f64::from(metrics.row_spacing);
        }
    }
    // A visible group label heads the picker above the option rows: inside
    // the horizontal insets, its width competes with a row's content width,
    // and its height plus the metrics' label spacing stacks on top of the
    // rows. A zero-height (hidden) label adds nothing — same presence rule as
    // the menu field label.
    let content_width =
        (indicator.size().width + f64::from(indicator.get_label_spacing()) + max_item_width)
            .max(f64::from(label_size.width));
    let width = f64::from(metrics.horizontal_inset)
        .mul_add(2.0, content_width)
        .max(f64::from(metrics.min_width));
    // The minimum height floors the rows block alone: the heading adds on top
    // of it so the rows keep their unlabelled height.
    let rows_height = f64::from(metrics.vertical_inset)
        .mul_add(2.0, total_height)
        .max(f64::from(metrics.min_height));
    let height = rows_height
        + if label_size.height > 0.0 {
            f64::from(label_size.height) + f64::from(metrics.label_spacing)
        } else {
            0.0
        };
    LayoutSize::new(
        crate::num_cast::f64_as_f32(width),
        crate::num_cast::f64_as_f32(height),
    )
}

/// Measures a picker's intrinsic size from the label size measured from its
/// built [`crate::renderer::RetainedSubview`], so layout and the in-field
/// label render agree on the label height.
#[cfg(feature = "picker")]
pub fn measure_picker_intrinsic_with_label_size<E: crate::text::TextEngine, S>(
    picker: &PickerConfig<S>,
    label_size: LayoutSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> LayoutSize {
    let items = state.measure_signal(picker.items());
    if items.is_empty() {
        contract_violation!(
            "hydrolysis picker requires at least one item; the picker measures as zero"
        );
        return LayoutSize::zero();
    }
    let item_count = items.len();

    match picker.style() {
        PickerStyle::Automatic | PickerStyle::Menu => {
            let look = theme.menu_picker(
                &crate::widgets::controls::picker::picker_spec(picker, env),
                env,
            );
            measure_menu_picker_intrinsic(&look, &items, label_size, state, env)
        }
        PickerStyle::Radio => {
            let look = theme.radio_group(
                &crate::widgets::controls::picker::picker_spec(picker, env),
                env,
            );
            measure_radio_picker_intrinsic(&look, &items, label_size, state, env)
        }
        PickerStyle::Segmented => {
            let spec = crate::widgets::controls::picker::segmented_spec(picker, item_count, env);
            let look = theme.segmented_picker(&spec, env);
            measure_segmented_intrinsic(&look, &items, label_size, state, env)
        }
        other => {
            contract_violation!(
                "hydrolysis PickerStyle variant {other:?} is not implemented; the picker measures as zero"
            );
            LayoutSize::zero()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::measured_input_field_height;

    use crate::renderer::tests::{MinimalTestTheme, Realized, test_environment};
    use waterui_core::layout::ProposalSize;

    use crate::theme::InputFieldMetrics;
    use crate::theme::WidgetTheme;
    use std::rc::Rc;
    use waterui::{Binding, Computed};
    use waterui_controls::text_field::field;
    use waterui_controls::theme::install_font_signal;
    use waterui_core::{AnyView, Str};
    use waterui_form::secure::{Secure, secure};
    use waterui_text::font::{Body, FontWeight, ResolvedFont};

    #[test]
    fn labeled_input_field_height_reserves_space_for_tall_text() {
        let metrics = InputFieldMetrics::new(18.0, 72.0, 56.0, 16.0, 8.0);

        approx::assert_relative_eq!(measured_input_field_height(22.0, 18.0, metrics), 56.0);
        approx::assert_relative_eq!(measured_input_field_height(34.0, 18.0, metrics), 68.0);
    }

    /// A labeled field reserves exactly the theme's `label_height` for its
    /// floating label: the label's unscaled line must not grow the band
    /// (water-rs/waterui#2387). A 16 pt band, two 8 pt insets and a 24 pt
    /// body line compose 56 pt — the Material 3 filled field height — and
    /// the secure field composes identically. The theme's `min_height`
    /// stays below the composition so the assertion cannot be met by the
    /// floor.
    #[test]
    fn labeled_fields_reserve_the_theme_label_height() {
        let mut env = test_environment();
        install_font_signal::<Body>(
            &mut env,
            Computed::constant(
                ResolvedFont::new(16.0, FontWeight::Normal).with_typography_metrics(24.0, 0.0),
            ),
        );
        let theme: Rc<dyn WidgetTheme> = Rc::new(
            MinimalTestTheme::default()
                .with_input_metrics(InputFieldMetrics::new(16.0, 280.0, 40.0, 16.0, 8.0)),
        );
        let value = Binding::new(Str::from("x"));
        let view = AnyView::new(field("Label", &value));
        let size = Realized::with_theme(view, &env, &theme)
            .measure(ProposalSize::UNSPECIFIED)
            .size;
        approx::assert_relative_eq!(f64::from(size.height), 56.0);

        let secret = Binding::new(Secure::new(String::from("x")));
        let view = AnyView::new(secure("Label", &secret));
        let size = Realized::with_theme(view, &env, &theme)
            .measure(ProposalSize::UNSPECIFIED)
            .size;
        approx::assert_relative_eq!(f64::from(size.height), 56.0);
    }

    #[test]
    fn unlabeled_input_field_height_uses_minimum_until_text_needs_more() {
        let metrics = InputFieldMetrics::new(18.0, 72.0, 56.0, 16.0, 8.0);

        approx::assert_relative_eq!(measured_input_field_height(34.0, 0.0, metrics), 56.0);
        approx::assert_relative_eq!(measured_input_field_height(48.0, 0.0, metrics), 64.0);
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;
    use crate::renderer::tests::test_environment;
    use waterui_graphics::color::Color;
    use waterui_text::styled::{Style as TextStyle, StyledStr};

    const BACKGROUND: [u8; 4] = [0, 128, 0, 255];

    /// The premultiplied solid colours a recording draws — glyph brushes and
    /// background fills alike — read off the recorded ops.
    fn solid_fill_colours(scene: &Recording) -> Vec<u32> {
        scene.solid_fill_colours()
    }

    fn rendered_fill_colours(styled: &StyledStr, width: f64) -> Vec<u32> {
        let env = test_environment();
        let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
        let input = resolve_text_layout_input(styled, HorizontalAlignment::Leading, &env);
        let scene = state.text.record(
            &input,
            Some(crate::num_cast::f64_as_f32(width)),
            TailMark::None,
            &mut state.counters,
            &mut state.text_scenes,
        );
        solid_fill_colours(&scene)
    }

    /// A span's `TextStyle::background` must reach the encoded scene: one fill
    /// per contiguous backgrounded run on each line it covers.
    #[test]
    fn styled_backgrounds_paint_fills_under_their_runs() {
        let expected = u32::from_ne_bytes(BACKGROUND);

        let mut single = StyledStr::empty();
        single.push("before ", TextStyle::new());
        single.push(
            "spoiler",
            TextStyle::new().background(Color::srgb(0, 128, 0)),
        );
        single.push(" after", TextStyle::new());
        let colours = rendered_fill_colours(&single, 300.0);
        assert!(
            colours.contains(&expected),
            "a mid-line span paints its background fill"
        );

        let mut wrapped = StyledStr::empty();
        wrapped.push(
            "the hidden words run long enough to wrap the line ",
            TextStyle::new(),
        );
        wrapped.push(
            "across its own boundary here",
            TextStyle::new().background(Color::srgb(0, 128, 0)),
        );
        wrapped.push(" and out", TextStyle::new());
        let colours = rendered_fill_colours(&wrapped, 120.0);
        assert!(
            colours.iter().filter(|colour| **colour == expected).count() >= 2,
            "a span wrapped over two lines paints one fill per line"
        );
    }
}
