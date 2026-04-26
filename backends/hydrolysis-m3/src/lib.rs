//! Material-style widget metrics and drawing primitives for Hydrolysis.
//!
//! This crate keeps the visual chrome for Hydrolysis controls in one place so
//! component renderers can share sizing, colors, and shape treatment without
//! duplicating backend-specific drawing code.

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
/// Paint source used by Material widget chrome.
pub enum Brush {
    /// A solid peniko color.
    Solid(vello::peniko::Color),
}

impl From<vello::peniko::Color> for Brush {
    fn from(value: vello::peniko::Color) -> Self {
        Self::Solid(value)
    }
}

/// Drawing operations required by Material widget chrome.
pub trait DrawContext {
    /// Fill an axis-aligned rectangle.
    fn fill_rect(&mut self, rect: Rect, brush: &Brush);
    /// Fill a rounded rectangle.
    fn fill_rounded_rect(&mut self, rect: Rect, radii: RoundedRectRadii, brush: &Brush);
    /// Stroke an axis-aligned rectangle.
    fn stroke_rect(&mut self, rect: Rect, brush: &Brush, width: f64);
    /// Stroke a rounded rectangle.
    fn stroke_rounded_rect(
        &mut self,
        rect: Rect,
        radii: RoundedRectRadii,
        brush: &Brush,
        width: f64,
    );
    /// Stroke a straight line segment.
    fn stroke_line(&mut self, from: Point, to: Point, brush: &Brush, width: f64);
    /// Stroke a circle.
    fn stroke_circle(&mut self, center: Point, radius: f64, brush: &Brush, width: f64);
    /// Fill a circle.
    fn fill_circle(&mut self, center: Point, radius: f64, brush: &Brush);
    /// Fill a Bezier path.
    fn fill_path(&mut self, path: &BezPath, brush: &Brush);
    /// Stroke a Bezier path.
    fn stroke_path(&mut self, path: &BezPath, brush: &Brush, width: f64);
    /// Push a temporary drawing layer.
    fn push_layer(&mut self, alpha: f32, clip: Option<&Rect>);
    /// Pop the current drawing layer.
    fn pop_layer(&mut self);
    /// Push a transform onto the drawing stack.
    fn push_transform(&mut self, affine: Affine);
    /// Pop the current transform.
    fn pop_transform(&mut self);
}

#[derive(Debug, Clone, Copy)]
/// Button layout metrics.
pub struct ButtonMetrics {
    /// Horizontal content padding.
    pub padding_x: f64,
    /// Vertical content padding.
    pub padding_y: f64,
    /// Minimum button width.
    pub min_width: f64,
    /// Minimum button height.
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
/// Toggle layout metrics.
pub struct ToggleMetrics {
    /// Toggle control width.
    pub width: f64,
    /// Toggle control height.
    pub height: f64,
}

impl ToggleMetrics {
    const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy)]
/// Stepper layout metrics.
pub struct StepperMetrics {
    /// Minimum size for each stepper button.
    pub button_min_size: f64,
    /// Maximum size for each stepper button.
    pub button_max_size: f64,
    /// Preferred size for each stepper button.
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
/// Text input layout metrics.
pub struct InputFieldMetrics {
    /// Height reserved for the input label.
    pub label_height: f64,
    /// Minimum input field width.
    pub min_width: f64,
    /// Minimum input field height.
    pub min_height: f64,
    /// Horizontal text inset.
    pub horizontal_inset: f64,
    /// Vertical text inset.
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
/// Picker layout metrics.
pub struct PickerMetrics {
    /// Minimum picker width.
    pub min_width: f64,
    /// Minimum picker height.
    pub min_height: f64,
    /// Horizontal picker inset.
    pub horizontal_inset: f64,
    /// Vertical picker inset.
    pub vertical_inset: f64,
    /// Space reserved for picker indicators.
    pub indicator_space: f64,
    /// Radio indicator diameter.
    pub radio_indicator_size: f64,
    /// Gap between a radio indicator and its label.
    pub radio_label_spacing: f64,
    /// Gap between radio picker rows.
    pub radio_row_spacing: f64,
    /// Vertical gap above popup menu rows.
    pub popup_top_spacing: f64,
    /// Popup menu corner radius.
    pub popup_corner_radius: f64,
}

#[derive(Debug, Clone, Copy)]
/// Slider layout metrics.
pub struct SliderMetrics {
    /// Horizontal slider inset.
    pub horizontal_inset: f64,
    /// Gap between slider labels and track.
    pub horizontal_spacing: f64,
    /// Vertical spacing around the track.
    pub vertical_spacing: f64,
    /// Minimum track width.
    pub min_track_width: f64,
    /// Track height.
    pub track_height: f64,
    /// Thumb radius.
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
/// Progress indicator layout metrics.
pub struct ProgressMetrics {
    /// Height reserved for labels.
    pub label_height: f64,
    /// Offset from label baseline to the linear bar.
    pub bar_top_offset: f64,
    /// Linear progress bar height.
    pub bar_height: f64,
    /// Horizontal inset for the linear bar.
    pub bar_horizontal_inset: f64,
    /// Gap above the value label.
    pub value_label_top_spacing: f64,
    /// Minimum linear track width.
    pub min_track_width: f64,
    /// Circular progress diameter.
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

/// Theme contract for Hydrolysis Material-style widgets.
pub trait WidgetTheme {
    /// Return metrics for a button style.
    fn button_metrics(&self, style: ButtonStyle) -> ButtonMetrics;
    /// Draw button chrome for a style.
    fn draw_button_chrome(&self, draw: &mut dyn DrawContext, bounds: Rect, style: ButtonStyle);

    /// Return metrics for a toggle style.
    fn toggle_metrics(&self, style: ToggleStyle) -> ToggleMetrics;
    /// Draw switch-style toggle chrome.
    fn draw_toggle_switch(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32);
    /// Draw checkbox-style toggle chrome.
    fn draw_toggle_checkbox(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32);

    /// Return stepper metrics.
    fn stepper_metrics(&self) -> StepperMetrics;
    /// Draw one stepper button.
    fn draw_stepper_button(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Return text input metrics.
    fn input_field_metrics(&self) -> InputFieldMetrics;
    /// Return the placeholder text color.
    fn input_placeholder_color(&self) -> Color;
    /// Draw text input chrome.
    fn draw_input_field(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Return picker metrics for a style.
    fn picker_metrics(&self, style: PickerStyle) -> PickerMetrics;
    /// Draw the picker indicator.
    fn draw_picker_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw the picker popup container.
    fn draw_picker_popup(&self, draw: &mut dyn DrawContext, popup_rect: Rect);
    /// Draw one picker popup row background.
    fn draw_picker_popup_row_background(
        &self,
        draw: &mut dyn DrawContext,
        row_rect: Rect,
        selected: bool,
    );
    /// Draw a picker separator.
    fn draw_picker_separator(&self, draw: &mut dyn DrawContext, separator: Rect);
    /// Draw a radio picker indicator.
    fn draw_radio_indicator(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        selected: bool,
    );

    /// Return slider metrics.
    fn slider_metrics(&self) -> SliderMetrics;
    /// Draw slider track chrome.
    fn draw_slider_track(&self, draw: &mut dyn DrawContext, track_rect: Rect, fill_rect: Rect);
    /// Draw slider thumb chrome.
    fn draw_slider_thumb(&self, draw: &mut dyn DrawContext, center: Point, radius: f64);

    /// Return progress indicator metrics.
    fn progress_metrics(&self, style: ProgressStyle) -> ProgressMetrics;
    /// Draw the linear progress track.
    fn draw_progress_linear_track(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw the linear progress fill.
    fn draw_progress_linear_fill(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw the circular progress track.
    fn draw_progress_circular_track(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        width: f64,
    );
    /// Draw the circular progress fill path.
    fn draw_progress_circular_fill(&self, draw: &mut dyn DrawContext, path: &BezPath, width: f64);

    /// Draw a navigation bar background.
    fn draw_navigation_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, background: &Brush);
    /// Draw a navigation bar separator.
    fn draw_navigation_bar_separator(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a navigation back button.
    fn draw_navigation_back_button(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Draw a tabs bar.
    fn draw_tabs_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, top_edge: bool);
    /// Draw the selected tab highlight.
    fn draw_tabs_highlight(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Draw a scroll indicator.
    fn draw_scroll_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Draw a list row background.
    fn draw_list_row_background(&self, draw: &mut dyn DrawContext, bounds: Rect, alternate: bool);
    /// Draw a list move affordance.
    fn draw_list_move_control(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a list delete affordance.
    fn draw_list_delete_control(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a list separator.
    fn draw_list_separator(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Draw a table header background.
    fn draw_table_header_background(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a table cell border.
    fn draw_table_cell_border(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a table column separator.
    fn draw_table_column_separator(&self, draw: &mut dyn DrawContext, from: Point, to: Point);
}

#[derive(Debug, Default, Clone, Copy)]
/// Default Material-style widget theme.
pub struct MaterialTheme;

impl MaterialTheme {
    /// Create a Material-style widget theme.
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
