//! Material Design 3 widget metrics and drawing primitives.
//!
//! This crate is a theme package. It implements the backend-neutral widget
//! chrome contract from `waterui-backend-core` and does not depend on the
//! Hydrolysis renderer crate.

pub mod chip;
pub mod color;
pub mod dialog;
pub mod fab;
pub mod icon_button;
pub mod material_badge;
pub mod material_card;
pub mod material_divider;
pub mod material_list;
pub mod material_snackbar;
pub mod material_tabs;
pub mod navigation_bar;
pub mod navigation_drawer;
pub mod segmented_button;
pub mod tooltip;

mod controls;
mod icons;
mod layout;
mod navigation;
mod semantics;
mod theme;

pub(crate) use controls::{button, input, picker, progress, slider, stepper, toggle};
pub(crate) use layout::{badge, card, divider, list, menu, scroll, snackbar, table};
pub(crate) use navigation::{navigation as navigation_chrome, tabs};
pub(crate) use theme::dimensions;

use vello::kurbo::{BezPath, Point, Rect};
use waterui::Plugin as _;
use waterui::text::font::Font;
use waterui::theme::{ColorScheme, ColorSettings, Theme};
pub use waterui_backend_core::widget::{
    BadgeMetrics, Brush, ButtonMetrics, DividerMetrics, DrawContext, InputFieldMetrics,
    InteractionMotion, ListMetrics, NavigationMetrics, NavigationMotion, PickerMetrics,
    ProgressIndicatorStyle, ProgressMetrics, ProgressMotion, SliderMetrics, StepperMetrics,
    TableMetrics, TabsMetrics, TextCaretMotion, TextContextMenuMetrics, ToggleMetrics,
    WidgetInteractionState, WidgetTheme,
};
use waterui_controls::button::ButtonStyle;
use waterui_controls::toggle::ToggleStyle;
use waterui_core::Environment;
use waterui_form::picker::PickerStyle;
use waterui_graphics::color::Color;

pub use chip::{
    AssistChip, FilterChip, InputChip, OutlinedChip, SuggestionChip, assist_chip, filter_chip,
    input_chip, suggestion_chip,
};
pub use dialog::{Dialog, DialogAction, dialog, dialog_action};
pub use fab::{
    ExtendedFab, Fab, FabVariantTokens, PrimaryFab, SecondaryFab, SurfaceFab, TertiaryFab,
    extended_fab, fab,
};
pub use icon_button::{
    FilledIconButton, FilledTonalIconButton, IconButton, IconButtonVariantTokens,
    OutlinedIconButton, SelectedOutlinedIconButton, SelectedStandardIconButton, StandardIconButton,
    filled_icon_button, filled_tonal_icon_button, icon_button, outlined_icon_button,
};
pub use material_badge::{MaterialBadge, material_badge};
pub use material_card::{MaterialCard, material_card};
pub use material_divider::{MaterialDivider, material_divider};
pub use material_color_utils::dynamic::{
    color_spec::{Platform as MaterialColorPlatform, SpecVersion as MaterialColorSpecVersion},
    variant::Variant as MaterialColorVariant,
};
pub use material_list::{MaterialList, MaterialListItem, material_list, material_list_item};
pub use material_snackbar::{MaterialSnackbar, material_snackbar};
pub use material_tabs::{MaterialTab, MaterialTabs, material_tab, material_tabs};
pub use navigation_bar::{NavigationBar, NavigationTab, navigation_bar, navigation_tab};
pub use navigation_drawer::{
    NavigationDrawer, NavigationDrawerItem, navigation_drawer, navigation_drawer_item,
};
pub use segmented_button::{
    OutlinedSegmentedButton, OutlinedSegmentedButtonSet, SegmentedButtonShape,
    outlined_segmented_button, outlined_segmented_button_set,
};
pub use theme::colors::{
    MaterialColorMode, MaterialColorScheme, MaterialColorSchemes, MaterialColorSource,
    MaterialContrastLevel, MaterialRoleColor,
};
pub use tooltip::{PlainTooltip, RichTooltip, plain_tooltip, rich_tooltip};

#[derive(Debug, Clone, Copy)]
/// Material Design 3 widget theme.
pub struct MaterialTheme {
    colors: MaterialColorScheme,
}

impl MaterialTheme {
    /// Create a Material Design 3 widget theme.
    #[must_use]
    pub fn new() -> Self {
        Self::with_colors(MaterialColorScheme::baseline_light())
    }

    /// Create a Material Design 3 widget theme from explicit Material color roles.
    #[must_use]
    pub const fn with_colors(colors: MaterialColorScheme) -> Self {
        Self { colors }
    }

    /// Return the Material color roles used by this theme.
    #[must_use]
    pub const fn colors(&self) -> &MaterialColorScheme {
        &self.colors
    }
}

/// Install the Material Design 3 widget theme into an environment.
pub fn install(env: &mut Environment) {
    install_with_colors(env, MaterialColorScheme::baseline_light());
}

/// Install the dark Material Design 3 baseline color scheme into an environment.
pub fn install_dark(env: &mut Environment) {
    install_with_colors(env, MaterialColorScheme::baseline_dark());
}

/// Install a Material Design 3 widget theme generated from a Material You source color.
pub fn install_with_seed(
    env: &mut Environment,
    seed: material_color_utils::utils::color_utils::Argb,
) {
    install_with_seed_mode(env, seed, MaterialColorMode::Light);
}

/// Install a Material Design 3 widget theme generated from a source color and mode.
pub fn install_with_seed_mode(
    env: &mut Environment,
    seed: material_color_utils::utils::color_utils::Argb,
    mode: MaterialColorMode,
) {
    install_with_source(env, MaterialColorSource::new(seed), mode);
}

/// Install a Material Design 3 widget theme generated from a complete Material You source.
pub fn install_with_source(
    env: &mut Environment,
    source: MaterialColorSource,
    mode: MaterialColorMode,
) {
    install_with_colors(env, source.scheme(mode));
}

/// Install one scheme from paired light and dark Material You color schemes.
pub fn install_with_color_schemes(
    env: &mut Environment,
    schemes: &MaterialColorSchemes,
    mode: MaterialColorMode,
) {
    install_with_colors(env, schemes.scheme(mode));
}

/// Install a Material Design 3 widget theme from an explicit color scheme.
pub fn install_with_colors(env: &mut Environment, colors: MaterialColorScheme) {
    let color_scheme = match colors.mode {
        MaterialColorMode::Light => ColorScheme::Light,
        MaterialColorMode::Dark => ColorScheme::Dark,
    };
    Theme::new()
        .color_scheme(color_scheme)
        .colors(
            ColorSettings::new()
                .background(colors.background.resolved())
                .surface(colors.surface.resolved())
                .surface_variant(colors.surface_variant.resolved())
                .border(colors.outline.resolved())
                .foreground(colors.on_surface.resolved())
                .muted_foreground(colors.on_surface_variant.resolved())
                .accent(colors.primary.resolved())
                .accent_foreground(colors.on_primary.resolved()),
        )
        .fonts(theme::typography::settings())
        .install(env);
    env.insert(colors);
    env.insert(card::theme(&colors));
    env.insert(snackbar::theme(&colors));
    env.insert(Box::new(MaterialTheme::with_colors(colors)) as Box<dyn WidgetTheme>);
}

impl Default for MaterialTheme {
    fn default() -> Self {
        Self::new()
    }
}

impl WidgetTheme for MaterialTheme {
    fn interaction_motion(&self) -> InteractionMotion {
        theme::motion::interaction()
    }

    fn progress_motion(&self) -> ProgressMotion {
        theme::motion::progress()
    }

    fn text_caret_motion(&self) -> TextCaretMotion {
        theme::motion::text_caret()
    }

    fn navigation_motion(&self) -> NavigationMotion {
        theme::motion::navigation()
    }

    fn button_metrics(&self, style: ButtonStyle) -> ButtonMetrics {
        button::metrics(style)
    }

    fn button_label_color(&self, style: ButtonStyle) -> Option<Color> {
        button::label_color(&self.colors, style)
    }

    fn button_label_font(&self, _style: ButtonStyle) -> Option<Font> {
        Some(theme::typography::label_large())
    }

    fn draw_button_chrome(&self, draw: &mut dyn DrawContext, bounds: Rect, style: ButtonStyle) {
        button::draw_chrome(&self.colors, draw, bounds, style);
    }

    fn draw_button_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        style: ButtonStyle,
        state: WidgetInteractionState,
    ) {
        button::draw_state_layer(&self.colors, draw, bounds, style, state);
    }

    fn toggle_metrics(&self, style: ToggleStyle) -> ToggleMetrics {
        toggle::metrics(style)
    }

    fn toggle_value_animation(&self) -> waterui::animation::Animation {
        theme::motion::toggle_value()
    }

    fn draw_toggle_switch(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        progress: f32,
        state: WidgetInteractionState,
    ) {
        toggle::draw_switch(&self.colors, draw, bounds, progress, state);
    }

    fn draw_toggle_switch_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        progress: f32,
        state: WidgetInteractionState,
    ) {
        toggle::draw_switch_state_layer(&self.colors, draw, bounds, progress, state);
    }

    fn draw_toggle_checkbox(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32) {
        toggle::draw_checkbox(&self.colors, draw, bounds, progress);
    }

    fn draw_toggle_checkbox_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        progress: f32,
        state: WidgetInteractionState,
    ) {
        toggle::draw_checkbox_state_layer(&self.colors, draw, bounds, progress, state);
    }

    fn stepper_metrics(&self) -> StepperMetrics {
        stepper::metrics()
    }

    fn draw_stepper_button(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        stepper::draw_button(&self.colors, draw, bounds);
    }

    fn draw_stepper_decrement_icon(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        stepper::draw_decrement_icon(&self.colors, draw, bounds);
    }

    fn draw_stepper_increment_icon(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        stepper::draw_increment_icon(&self.colors, draw, bounds);
    }

    fn draw_stepper_button_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        stepper::draw_button_state_layer(&self.colors, draw, bounds, state);
    }

    fn input_field_metrics(&self) -> InputFieldMetrics {
        input::metrics()
    }

    fn input_placeholder_color(&self) -> Color {
        input::placeholder_color(&self.colors)
    }

    fn input_selection_brush(&self) -> Brush {
        input::selection_brush(&self.colors)
    }

    fn input_caret_brush(&self, opacity: f32) -> Brush {
        input::caret_brush(&self.colors, opacity)
    }

    fn draw_input_field(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        input::draw_field(&self.colors, draw, bounds, state);
    }

    fn draw_input_field_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        input::draw_state_layer(&self.colors, draw, bounds, state);
    }

    fn text_context_menu_metrics(&self) -> TextContextMenuMetrics {
        menu::text_context_metrics()
    }

    fn draw_text_context_menu_panel(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        menu::draw_text_context_panel(&self.colors, draw, bounds);
    }

    fn draw_text_context_menu_separator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        menu::draw_text_context_separator(&self.colors, draw, bounds);
    }

    fn picker_metrics(&self, style: PickerStyle) -> PickerMetrics {
        picker::metrics(style)
    }

    fn draw_picker_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        picker::draw_indicator(&self.colors, draw, bounds);
    }

    fn draw_picker_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        picker::draw_state_layer(&self.colors, draw, bounds, state);
    }

    fn draw_picker_popup(&self, draw: &mut dyn DrawContext, popup_rect: Rect) {
        picker::draw_popup(&self.colors, draw, popup_rect);
    }

    fn draw_picker_popup_row_background(
        &self,
        draw: &mut dyn DrawContext,
        row_rect: Rect,
        selected: bool,
    ) {
        picker::draw_popup_row_background(&self.colors, draw, row_rect, selected);
    }

    fn draw_picker_popup_row_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        row_rect: Rect,
        selected: bool,
        state: WidgetInteractionState,
    ) {
        picker::draw_popup_row_state_layer(&self.colors, draw, row_rect, selected, state);
    }

    fn draw_picker_separator(&self, draw: &mut dyn DrawContext, separator: Rect) {
        picker::draw_separator(&self.colors, draw, separator);
    }

    fn draw_radio_indicator(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        selected: bool,
    ) {
        picker::draw_radio_indicator(&self.colors, draw, center, radius, selected);
    }

    fn draw_radio_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        selected: bool,
        state: WidgetInteractionState,
    ) {
        picker::draw_radio_state_layer(&self.colors, draw, center, radius, selected, state);
    }

    fn segmented_picker_label_color(&self, selected: bool) -> Option<Color> {
        Some(picker::segmented_label_color(&self.colors, selected))
    }

    fn draw_segmented_picker_container(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        segment_count: usize,
    ) {
        picker::draw_segmented_container(&self.colors, draw, bounds, segment_count);
    }

    fn draw_segmented_picker_segment(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        selected: bool,
        is_first: bool,
        is_last: bool,
    ) {
        picker::draw_segmented_segment(&self.colors, draw, bounds, selected, is_first, is_last);
    }

    fn draw_segmented_picker_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        selected: bool,
        state: WidgetInteractionState,
    ) {
        picker::draw_segmented_state_layer(&self.colors, draw, bounds, selected, state);
    }

    fn slider_metrics(&self) -> SliderMetrics {
        slider::metrics()
    }

    fn draw_slider_track(&self, draw: &mut dyn DrawContext, track_rect: Rect, fill_rect: Rect) {
        slider::draw_track(&self.colors, draw, track_rect, fill_rect);
    }

    fn draw_slider_thumb(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        state: WidgetInteractionState,
    ) {
        slider::draw_thumb(&self.colors, draw, center, radius, state);
    }

    fn draw_slider_thumb_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        state: WidgetInteractionState,
    ) {
        slider::draw_thumb_state_layer(&self.colors, draw, center, radius, state);
    }

    fn progress_metrics(&self, style: ProgressIndicatorStyle) -> ProgressMetrics {
        progress::metrics(style)
    }

    fn draw_progress_linear_track(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        progress::draw_linear_track(&self.colors, draw, bounds);
    }

    fn draw_progress_linear_fill(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        progress::draw_linear_fill(&self.colors, draw, bounds);
    }

    fn draw_progress_linear_indeterminate(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        elapsed: core::time::Duration,
        four_color: bool,
    ) {
        progress::draw_linear_indeterminate(&self.colors, draw, bounds, elapsed, four_color);
    }

    fn draw_progress_circular_track(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        width: f64,
    ) {
        progress::draw_circular_track(&self.colors, draw, center, radius, width);
    }

    fn draw_progress_circular_fill(&self, draw: &mut dyn DrawContext, path: &BezPath, width: f64) {
        progress::draw_circular_fill(&self.colors, draw, path, width);
    }

    fn draw_progress_circular_indeterminate(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        width: f64,
        elapsed: core::time::Duration,
        four_color: bool,
    ) {
        progress::draw_circular_indeterminate(
            &self.colors,
            draw,
            center,
            radius,
            width,
            elapsed,
            four_color,
        );
    }

    fn navigation_metrics(&self) -> NavigationMetrics {
        navigation_chrome::metrics()
    }

    fn draw_navigation_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, background: &Brush) {
        navigation_chrome::draw_bar(draw, bounds, background);
    }

    fn draw_navigation_bar_separator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        navigation_chrome::draw_bar_separator(&self.colors, draw, bounds);
    }

    fn draw_navigation_back_button(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        navigation_chrome::draw_back_button(&self.colors, draw, bounds);
    }

    fn tabs_metrics(&self) -> TabsMetrics {
        tabs::metrics()
    }

    fn draw_tabs_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, top_edge: bool) {
        tabs::draw_bar(&self.colors, draw, bounds, top_edge);
    }

    fn draw_tabs_highlight(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        tabs::draw_highlight(&self.colors, draw, bounds);
    }

    fn draw_tabs_button_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        selected: bool,
        state: WidgetInteractionState,
    ) {
        tabs::draw_button_state_layer(&self.colors, draw, bounds, selected, state);
    }

    fn draw_scroll_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        scroll::draw_indicator(&self.colors, draw, bounds);
    }

    fn divider_metrics(&self) -> DividerMetrics {
        divider::metrics()
    }

    fn draw_divider(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        divider::draw(&self.colors, draw, bounds);
    }

    fn badge_metrics(&self) -> BadgeMetrics {
        badge::metrics()
    }

    fn badge_label_color(&self) -> Color {
        badge::label_color(&self.colors)
    }

    fn badge_label_font(&self) -> Font {
        theme::typography::label_small()
    }

    fn draw_badge_small(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        badge::draw_small(&self.colors, draw, bounds);
    }

    fn draw_badge_large(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        badge::draw_large(&self.colors, draw, bounds);
    }

    fn list_metrics(&self) -> ListMetrics {
        list::metrics()
    }

    fn draw_list_row_background(&self, draw: &mut dyn DrawContext, bounds: Rect, alternate: bool) {
        list::draw_row_background(&self.colors, draw, bounds, alternate);
    }

    fn draw_list_move_control(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        list::draw_move_control(&self.colors, draw, bounds);
    }

    fn draw_list_move_control_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        list::draw_move_control_state_layer(&self.colors, draw, bounds, state);
    }

    fn draw_list_delete_control(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        list::draw_delete_control(&self.colors, draw, bounds);
    }

    fn draw_list_delete_control_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        list::draw_delete_control_state_layer(&self.colors, draw, bounds, state);
    }

    fn draw_list_separator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        list::draw_separator(&self.colors, draw, bounds);
    }

    fn table_metrics(&self) -> TableMetrics {
        table::metrics()
    }

    fn draw_table_background(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        table::draw_background(&self.colors, draw, bounds);
    }

    fn draw_table_header_background(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        table::draw_header_background(&self.colors, draw, bounds);
    }

    fn draw_table_cell_border(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        table::draw_cell_border(&self.colors, draw, bounds);
    }

    fn draw_table_column_separator(&self, draw: &mut dyn DrawContext, from: Point, to: Point) {
        table::draw_column_separator(&self.colors, draw, from, to);
    }
}

fn lerp_channel(start: f32, end: f32, t: f32) -> f32 {
    (end - start).mul_add(t, start)
}

fn lerp_color(
    start: vello::peniko::Color,
    end: vello::peniko::Color,
    t: f32,
) -> vello::peniko::Color {
    let t = t.clamp(0.0, 1.0);
    let start = start.to_rgba8();
    let end = end.to_rgba8();
    vello::peniko::Color::new([
        lerp_channel(f32::from(start.r) / 255.0, f32::from(end.r) / 255.0, t),
        lerp_channel(f32::from(start.g) / 255.0, f32::from(end.g) / 255.0, t),
        lerp_channel(f32::from(start.b) / 255.0, f32::from(end.b) / 255.0, t),
        lerp_channel(f32::from(start.a) / 255.0, f32::from(end.a) / 255.0, t),
    ])
}

fn lerp_f64(start: f64, end: f64, t: f32) -> f64 {
    (end - start).mul_add(f64::from(t.clamp(0.0, 1.0)), start)
}
