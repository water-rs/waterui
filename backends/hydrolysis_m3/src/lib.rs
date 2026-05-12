//! Material Design 3 widget metrics and drawing primitives.
//!
//! This crate is a theme package. It implements the backend-neutral widget
//! chrome contract from `waterui-backend-core` and does not depend on the
//! Hydrolysis renderer crate.

mod controls;
mod layout;
mod navigation;
mod theme;

pub(crate) use controls::{button, input, picker, progress, slider, stepper, toggle};
pub(crate) use layout::{list, scroll, table};
pub(crate) use navigation::{navigation as navigation_chrome, tabs};
pub(crate) use theme::{colors, dimensions};

use vello::kurbo::{BezPath, Point, Rect};
pub use waterui_backend_core::widget::{
    Brush, ButtonMetrics, DrawContext, InputFieldMetrics, PickerMetrics, ProgressIndicatorStyle,
    ProgressMetrics, SliderMetrics, StepperMetrics, ToggleMetrics, WidgetInteractionState,
    WidgetTheme,
};
use waterui_controls::button::ButtonStyle;
use waterui_controls::toggle::ToggleStyle;
use waterui_core::Environment;
use waterui_form::picker::PickerStyle;
use waterui_graphics::color::Color;

#[derive(Debug, Default, Clone, Copy)]
/// Material Design 3 widget theme.
pub struct MaterialTheme;

impl MaterialTheme {
    /// Create a Material Design 3 widget theme.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Install the Material Design 3 widget theme into an environment.
pub fn install(env: &mut Environment) {
    env.insert(Box::new(MaterialTheme::new()) as Box<dyn WidgetTheme>);
}

impl WidgetTheme for MaterialTheme {
    fn button_metrics(&self, style: ButtonStyle) -> ButtonMetrics {
        button::metrics(style)
    }

    fn draw_button_chrome(&self, draw: &mut dyn DrawContext, bounds: Rect, style: ButtonStyle) {
        button::draw_chrome(draw, bounds, style);
    }

    fn draw_button_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        style: ButtonStyle,
        state: WidgetInteractionState,
    ) {
        button::draw_state_layer(draw, bounds, style, state);
    }

    fn toggle_metrics(&self, style: ToggleStyle) -> ToggleMetrics {
        toggle::metrics(style)
    }

    fn draw_toggle_switch(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32) {
        toggle::draw_switch(draw, bounds, progress);
    }

    fn draw_toggle_switch_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        progress: f32,
        state: WidgetInteractionState,
    ) {
        toggle::draw_switch_state_layer(draw, bounds, progress, state);
    }

    fn draw_toggle_checkbox(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32) {
        toggle::draw_checkbox(draw, bounds, progress);
    }

    fn draw_toggle_checkbox_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        progress: f32,
        state: WidgetInteractionState,
    ) {
        toggle::draw_checkbox_state_layer(draw, bounds, progress, state);
    }

    fn stepper_metrics(&self) -> StepperMetrics {
        stepper::metrics()
    }

    fn draw_stepper_button(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        stepper::draw_button(draw, bounds);
    }

    fn draw_stepper_button_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        stepper::draw_button_state_layer(draw, bounds, state);
    }

    fn input_field_metrics(&self) -> InputFieldMetrics {
        input::metrics()
    }

    fn input_placeholder_color(&self) -> Color {
        input::placeholder_color()
    }

    fn draw_input_field(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        input::draw_field(draw, bounds);
    }

    fn draw_input_field_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        input::draw_state_layer(draw, bounds, state);
    }

    fn picker_metrics(&self, style: PickerStyle) -> PickerMetrics {
        picker::metrics(style)
    }

    fn draw_picker_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        picker::draw_indicator(draw, bounds);
    }

    fn draw_picker_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        picker::draw_state_layer(draw, bounds, state);
    }

    fn draw_picker_popup(&self, draw: &mut dyn DrawContext, popup_rect: Rect) {
        picker::draw_popup(draw, popup_rect);
    }

    fn draw_picker_popup_row_background(
        &self,
        draw: &mut dyn DrawContext,
        row_rect: Rect,
        selected: bool,
    ) {
        picker::draw_popup_row_background(draw, row_rect, selected);
    }

    fn draw_picker_popup_row_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        row_rect: Rect,
        selected: bool,
        state: WidgetInteractionState,
    ) {
        picker::draw_popup_row_state_layer(draw, row_rect, selected, state);
    }

    fn draw_picker_separator(&self, draw: &mut dyn DrawContext, separator: Rect) {
        picker::draw_separator(draw, separator);
    }

    fn draw_radio_indicator(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        selected: bool,
    ) {
        picker::draw_radio_indicator(draw, center, radius, selected);
    }

    fn draw_radio_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        selected: bool,
        state: WidgetInteractionState,
    ) {
        picker::draw_radio_state_layer(draw, center, radius, selected, state);
    }

    fn slider_metrics(&self) -> SliderMetrics {
        slider::metrics()
    }

    fn draw_slider_track(&self, draw: &mut dyn DrawContext, track_rect: Rect, fill_rect: Rect) {
        slider::draw_track(draw, track_rect, fill_rect);
    }

    fn draw_slider_thumb(&self, draw: &mut dyn DrawContext, center: Point, radius: f64) {
        slider::draw_thumb(draw, center, radius);
    }

    fn draw_slider_thumb_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        state: WidgetInteractionState,
    ) {
        slider::draw_thumb_state_layer(draw, center, radius, state);
    }

    fn progress_metrics(&self, style: ProgressIndicatorStyle) -> ProgressMetrics {
        progress::metrics(style)
    }

    fn draw_progress_linear_track(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        progress::draw_linear_track(draw, bounds);
    }

    fn draw_progress_linear_fill(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        progress::draw_linear_fill(draw, bounds);
    }

    fn draw_progress_circular_track(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        width: f64,
    ) {
        progress::draw_circular_track(draw, center, radius, width);
    }

    fn draw_progress_circular_fill(&self, draw: &mut dyn DrawContext, path: &BezPath, width: f64) {
        progress::draw_circular_fill(draw, path, width);
    }

    fn draw_navigation_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, background: &Brush) {
        navigation_chrome::draw_bar(draw, bounds, background);
    }

    fn draw_navigation_bar_separator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        navigation_chrome::draw_bar_separator(draw, bounds);
    }

    fn draw_navigation_back_button(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        navigation_chrome::draw_back_button(draw, bounds);
    }

    fn draw_tabs_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, top_edge: bool) {
        tabs::draw_bar(draw, bounds, top_edge);
    }

    fn draw_tabs_highlight(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        tabs::draw_highlight(draw, bounds);
    }

    fn draw_tabs_button_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        selected: bool,
        state: WidgetInteractionState,
    ) {
        tabs::draw_button_state_layer(draw, bounds, selected, state);
    }

    fn draw_scroll_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        scroll::draw_indicator(draw, bounds);
    }

    fn draw_list_row_background(&self, draw: &mut dyn DrawContext, bounds: Rect, alternate: bool) {
        list::draw_row_background(draw, bounds, alternate);
    }

    fn draw_list_move_control(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        list::draw_move_control(draw, bounds);
    }

    fn draw_list_move_control_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        list::draw_move_control_state_layer(draw, bounds, state);
    }

    fn draw_list_delete_control(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        list::draw_delete_control(draw, bounds);
    }

    fn draw_list_delete_control_state_layer(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    ) {
        list::draw_delete_control_state_layer(draw, bounds, state);
    }

    fn draw_list_separator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        list::draw_separator(draw, bounds);
    }

    fn draw_table_header_background(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        table::draw_header_background(draw, bounds);
    }

    fn draw_table_cell_border(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        table::draw_cell_border(draw, bounds);
    }

    fn draw_table_column_separator(&self, draw: &mut dyn DrawContext, from: Point, to: Point) {
        table::draw_column_separator(draw, from, to);
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
