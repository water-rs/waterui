mod button;
mod colors;
mod dimensions;
mod input;
mod list;
mod navigation;
mod picker;
mod progress;
mod scroll;
mod slider;
mod stepper;
mod table;
mod tabs;
mod toggle;

use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};
use waterui::component::progress::ProgressStyle;
use waterui_controls::button::ButtonStyle;
use waterui_controls::toggle::ToggleStyle;
use waterui_form::picker::PickerStyle;
use waterui_graphics::color::Color;

#[derive(Debug, Clone)]
pub enum Brush {
    Solid(vello::peniko::Color),
}

impl From<vello::peniko::Color> for Brush {
    fn from(value: vello::peniko::Color) -> Self {
        Self::Solid(value)
    }
}

pub trait DrawContext {
    fn fill_rect(&mut self, rect: Rect, brush: &Brush);
    fn fill_rounded_rect(&mut self, rect: Rect, radii: RoundedRectRadii, brush: &Brush);
    fn stroke_rect(&mut self, rect: Rect, brush: &Brush, width: f64);
    fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radii: RoundedRectRadii,
        brush: &Brush,
        width: f64,
    );
    fn stroke_line(&mut self, from: Point, to: Point, brush: &Brush, width: f64);
    fn stroke_circle(&mut self, center: Point, radius: f64, brush: &Brush, width: f64);
    fn fill_circle(&mut self, center: Point, radius: f64, brush: &Brush);
    fn fill_path(&mut self, path: &BezPath, brush: &Brush);
    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, width: f64);
    fn push_layer(&mut self, alpha: f32, clip: Option<&Rect>);
    fn pop_layer(&mut self);
    fn push_transform(&mut self, affine: Affine);
    fn pop_transform(&mut self);
}

#[derive(Debug, Clone, Copy)]
pub struct ButtonMetrics {
    pub padding_x: f64,
    pub padding_y: f64,
    pub min_width: f64,
    pub min_height: f64,
}

impl ButtonMetrics {
    const fn new(padding_x: f64, padding_y: f64, min_width: f64, min_height: f64) -> Self {
        Self {
            padding_x,
            padding_y,
            min_width,
            min_height,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ToggleMetrics {
    pub width: f64,
    pub height: f64,
}

impl ToggleMetrics {
    const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StepperMetrics {
    pub button_min_size: f64,
    pub button_max_size: f64,
    pub button_intrinsic_size: f64,
}

impl StepperMetrics {
    const fn new(button_min_size: f64, button_max_size: f64, button_intrinsic_size: f64) -> Self {
        Self {
            button_min_size,
            button_max_size,
            button_intrinsic_size,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InputFieldMetrics {
    pub label_height: f64,
    pub min_width: f64,
    pub min_height: f64,
    pub horizontal_inset: f64,
    pub vertical_inset: f64,
}

impl InputFieldMetrics {
    const fn new(
        label_height: f64,
        min_width: f64,
        min_height: f64,
        horizontal_inset: f64,
        vertical_inset: f64,
    ) -> Self {
        Self {
            label_height,
            min_width,
            min_height,
            horizontal_inset,
            vertical_inset,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PickerMetrics {
    pub min_width: f64,
    pub min_height: f64,
    pub horizontal_inset: f64,
    pub vertical_inset: f64,
    pub indicator_space: f64,
    pub radio_indicator_size: f64,
    pub radio_label_spacing: f64,
    pub radio_row_spacing: f64,
    pub popup_top_spacing: f64,
    pub popup_corner_radius: f64,
}

impl PickerMetrics {
    const fn new(
        min_width: f64,
        min_height: f64,
        horizontal_inset: f64,
        vertical_inset: f64,
        indicator_space: f64,
        radio_indicator_size: f64,
        radio_label_spacing: f64,
        radio_row_spacing: f64,
        popup_top_spacing: f64,
        popup_corner_radius: f64,
    ) -> Self {
        Self {
            min_width,
            min_height,
            horizontal_inset,
            vertical_inset,
            indicator_space,
            radio_indicator_size,
            radio_label_spacing,
            radio_row_spacing,
            popup_top_spacing,
            popup_corner_radius,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SliderMetrics {
    pub horizontal_inset: f64,
    pub horizontal_spacing: f64,
    pub vertical_spacing: f64,
    pub min_track_width: f64,
    pub track_height: f64,
    pub thumb_radius: f64,
}

impl SliderMetrics {
    const fn new(
        horizontal_inset: f64,
        horizontal_spacing: f64,
        vertical_spacing: f64,
        min_track_width: f64,
        track_height: f64,
        thumb_radius: f64,
    ) -> Self {
        Self {
            horizontal_inset,
            horizontal_spacing,
            vertical_spacing,
            min_track_width,
            track_height,
            thumb_radius,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ProgressMetrics {
    pub label_height: f64,
    pub bar_top_offset: f64,
    pub bar_height: f64,
    pub bar_horizontal_inset: f64,
    pub value_label_top_spacing: f64,
    pub min_track_width: f64,
    pub circular_diameter: f64,
}

impl ProgressMetrics {
    const fn linear(
        label_height: f64,
        bar_top_offset: f64,
        bar_height: f64,
        bar_horizontal_inset: f64,
        value_label_top_spacing: f64,
        min_track_width: f64,
    ) -> Self {
        Self {
            label_height,
            bar_top_offset,
            bar_height,
            bar_horizontal_inset,
            value_label_top_spacing,
            min_track_width,
            circular_diameter: 0.0,
        }
    }

    const fn circular(circular_diameter: f64) -> Self {
        Self {
            label_height: 0.0,
            bar_top_offset: 0.0,
            bar_height: 0.0,
            bar_horizontal_inset: 0.0,
            value_label_top_spacing: 0.0,
            min_track_width: 0.0,
            circular_diameter,
        }
    }
}

pub trait WidgetTheme {
    fn button_metrics(&self, style: ButtonStyle) -> ButtonMetrics;
    fn draw_button_chrome(&self, draw: &mut dyn DrawContext, bounds: Rect, style: ButtonStyle);

    fn toggle_metrics(&self, style: ToggleStyle) -> ToggleMetrics;
    fn draw_toggle_switch(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32);
    fn draw_toggle_checkbox(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32);

    fn stepper_metrics(&self) -> StepperMetrics;
    fn draw_stepper_button(&self, draw: &mut dyn DrawContext, bounds: Rect);

    fn input_field_metrics(&self) -> InputFieldMetrics;
    fn input_placeholder_color(&self) -> Color;
    fn draw_input_field(&self, draw: &mut dyn DrawContext, bounds: Rect);

    fn picker_metrics(&self, style: PickerStyle) -> PickerMetrics;
    fn draw_picker_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect);
    fn draw_picker_popup(&self, draw: &mut dyn DrawContext, popup_rect: Rect);
    fn draw_picker_popup_row_background(
        &self,
        draw: &mut dyn DrawContext,
        row_rect: Rect,
        selected: bool,
    );
    fn draw_picker_separator(&self, draw: &mut dyn DrawContext, separator: Rect);
    fn draw_radio_indicator(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        selected: bool,
    );

    fn slider_metrics(&self) -> SliderMetrics;
    fn draw_slider_track(&self, draw: &mut dyn DrawContext, track_rect: Rect, fill_rect: Rect);
    fn draw_slider_thumb(&self, draw: &mut dyn DrawContext, center: Point, radius: f64);

    fn progress_metrics(&self, style: ProgressStyle) -> ProgressMetrics;
    fn draw_progress_linear_track(&self, draw: &mut dyn DrawContext, bounds: Rect);
    fn draw_progress_linear_fill(&self, draw: &mut dyn DrawContext, bounds: Rect);
    fn draw_progress_circular_track(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        width: f64,
    );
    fn draw_progress_circular_fill(&self, draw: &mut dyn DrawContext, path: &BezPath, width: f64);

    fn draw_navigation_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, background: &Brush);
    fn draw_navigation_bar_separator(&self, draw: &mut dyn DrawContext, bounds: Rect);
    fn draw_navigation_back_button(&self, draw: &mut dyn DrawContext, bounds: Rect);

    fn draw_tabs_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, top_edge: bool);
    fn draw_tabs_highlight(&self, draw: &mut dyn DrawContext, bounds: Rect);

    fn draw_scroll_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect);

    fn draw_list_row_background(&self, draw: &mut dyn DrawContext, bounds: Rect, alternate: bool);
    fn draw_list_move_control(&self, draw: &mut dyn DrawContext, bounds: Rect);
    fn draw_list_delete_control(&self, draw: &mut dyn DrawContext, bounds: Rect);
    fn draw_list_separator(&self, draw: &mut dyn DrawContext, bounds: Rect);

    fn draw_table_header_background(&self, draw: &mut dyn DrawContext, bounds: Rect);
    fn draw_table_cell_border(&self, draw: &mut dyn DrawContext, bounds: Rect);
    fn draw_table_column_separator(&self, draw: &mut dyn DrawContext, from: Point, to: Point);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MaterialTheme;

impl MaterialTheme {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl WidgetTheme for MaterialTheme {
    fn button_metrics(&self, style: ButtonStyle) -> ButtonMetrics {
        button::metrics(style)
    }

    fn draw_button_chrome(&self, draw: &mut dyn DrawContext, bounds: Rect, style: ButtonStyle) {
        button::draw_chrome(draw, bounds, style);
    }

    fn toggle_metrics(&self, style: ToggleStyle) -> ToggleMetrics {
        toggle::metrics(style)
    }

    fn draw_toggle_switch(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32) {
        toggle::draw_switch(draw, bounds, progress);
    }

    fn draw_toggle_checkbox(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32) {
        toggle::draw_checkbox(draw, bounds, progress);
    }

    fn stepper_metrics(&self) -> StepperMetrics {
        stepper::metrics()
    }

    fn draw_stepper_button(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        stepper::draw_button(draw, bounds);
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

    fn picker_metrics(&self, style: PickerStyle) -> PickerMetrics {
        picker::metrics(style)
    }

    fn draw_picker_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        picker::draw_indicator(draw, bounds);
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

    fn slider_metrics(&self) -> SliderMetrics {
        slider::metrics()
    }

    fn draw_slider_track(&self, draw: &mut dyn DrawContext, track_rect: Rect, fill_rect: Rect) {
        slider::draw_track(draw, track_rect, fill_rect);
    }

    fn draw_slider_thumb(&self, draw: &mut dyn DrawContext, center: Point, radius: f64) {
        slider::draw_thumb(draw, center, radius);
    }

    fn progress_metrics(&self, style: ProgressStyle) -> ProgressMetrics {
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
        navigation::draw_bar(draw, bounds, background);
    }

    fn draw_navigation_bar_separator(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        navigation::draw_bar_separator(draw, bounds);
    }

    fn draw_navigation_back_button(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        navigation::draw_back_button(draw, bounds);
    }

    fn draw_tabs_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, top_edge: bool) {
        tabs::draw_bar(draw, bounds, top_edge);
    }

    fn draw_tabs_highlight(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        tabs::draw_highlight(draw, bounds);
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

    fn draw_list_delete_control(&self, draw: &mut dyn DrawContext, bounds: Rect) {
        list::draw_delete_control(draw, bounds);
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
    start + (end - start) * t
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
    start + (end - start) * f64::from(t.clamp(0.0, 1.0))
}
