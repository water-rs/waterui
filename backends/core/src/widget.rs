//! Widget chrome contracts shared by rendering backends and theme packages.

use vello::kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};
use waterui_controls::button::ButtonStyle;
use waterui_controls::toggle::ToggleStyle;
use waterui_form::picker::PickerStyle;
use waterui_graphics::color::Color;

/// Paint source used by backend widget chrome.
#[derive(Debug, Clone)]
pub enum Brush {
    /// A solid peniko color.
    Solid(vello::peniko::Color),
}

impl From<vello::peniko::Color> for Brush {
    fn from(value: vello::peniko::Color) -> Self {
        Self::Solid(value)
    }
}

/// Drawing operations required by backend widget chrome.
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
    /// Push a temporary drawing layer clipped to a rounded rectangle.
    fn push_rounded_layer(&mut self, alpha: f32, clip: Rect, radii: RoundedRectRadii) {
        let _ = radii;
        self.push_layer(alpha, Some(&clip));
    }
    /// Pop the current drawing layer.
    fn pop_layer(&mut self);
    /// Push a transform onto the drawing stack.
    fn push_transform(&mut self, affine: Affine);
    /// Pop the current transform.
    fn pop_transform(&mut self);
}

/// Button layout metrics.
#[derive(Debug, Clone, Copy)]
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
    /// Create button layout metrics.
    #[must_use]
    pub const fn new(padding_x: f64, padding_y: f64, min_width: f64, min_height: f64) -> Self {
        Self {
            padding_x,
            padding_y,
            min_width,
            min_height,
        }
    }
}

/// Interactive state snapshot for widget chrome.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WidgetInteractionState {
    /// Pointer is inside the widget bounds.
    pub hovered: bool,
    /// Primary pointer is actively pressing the widget.
    pub pressed: bool,
    /// Keyboard focus is visible on the widget.
    pub focus_visible: bool,
    /// Animated state-layer opacity sampled by the renderer.
    pub state_layer_opacity: f32,
    /// Animated press-layer opacity sampled by the renderer.
    pub press_layer_opacity: f32,
    /// Absolute logical-unit origin of the active press interaction.
    pub press_origin: Option<Point>,
    /// Animated press grow progress in the 0.0..=1.0 range.
    pub press_progress: f32,
}

impl WidgetInteractionState {
    /// No active interaction state.
    pub const NONE: Self = Self {
        hovered: false,
        pressed: false,
        focus_visible: false,
        state_layer_opacity: 0.0,
        press_layer_opacity: 0.0,
        press_origin: None,
        press_progress: 0.0,
    };

    /// Material Design 3 hover state-layer opacity.
    pub const HOVER_STATE_LAYER_OPACITY: f32 = 0.08;
    /// Material Design 3 focus state-layer opacity.
    pub const FOCUS_STATE_LAYER_OPACITY: f32 = 0.10;
    /// Material Design 3 pressed state-layer opacity.
    pub const PRESSED_STATE_LAYER_OPACITY: f32 = 0.10;

    /// Return the dominant state-layer opacity for the current state.
    #[must_use]
    pub const fn state_layer_opacity(self) -> f32 {
        if self.state_layer_opacity > 0.0 {
            return self.state_layer_opacity;
        }
        if self.focus_visible {
            Self::FOCUS_STATE_LAYER_OPACITY
        } else if self.hovered {
            Self::HOVER_STATE_LAYER_OPACITY
        } else {
            0.0
        }
    }

    /// Return the animated pressed ripple opacity.
    #[must_use]
    pub const fn press_layer_opacity(self) -> f32 {
        if self.press_layer_opacity > 0.0 {
            return self.press_layer_opacity;
        }
        if self.pressed {
            Self::PRESSED_STATE_LAYER_OPACITY
        } else {
            0.0
        }
    }
}

/// Toggle layout metrics.
#[derive(Debug, Clone, Copy)]
pub struct ToggleMetrics {
    /// Toggle control width.
    pub width: f64,
    /// Toggle control height.
    pub height: f64,
}

impl ToggleMetrics {
    /// Create toggle layout metrics.
    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// Stepper layout metrics.
#[derive(Debug, Clone, Copy)]
pub struct StepperMetrics {
    /// Minimum size for each stepper button.
    pub button_min_size: f64,
    /// Maximum size for each stepper button.
    pub button_max_size: f64,
    /// Preferred size for each stepper button.
    pub button_intrinsic_size: f64,
}

impl StepperMetrics {
    /// Create stepper layout metrics.
    #[must_use]
    pub const fn new(
        button_min_size: f64,
        button_max_size: f64,
        button_intrinsic_size: f64,
    ) -> Self {
        Self {
            button_min_size,
            button_max_size,
            button_intrinsic_size,
        }
    }
}

/// Text input layout metrics.
#[derive(Debug, Clone, Copy)]
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
    /// Create text input layout metrics.
    #[must_use]
    pub const fn new(
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

/// Picker layout metrics.
#[derive(Debug, Clone, Copy)]
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

/// Slider layout metrics.
#[derive(Debug, Clone, Copy)]
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
    /// Create slider layout metrics.
    #[must_use]
    pub const fn new(
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

/// Progress indicator layout metrics.
#[derive(Debug, Clone, Copy)]
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
    /// Create linear progress layout metrics.
    #[must_use]
    pub const fn linear(
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

    /// Create circular progress layout metrics.
    #[must_use]
    pub const fn circular(circular_diameter: f64) -> Self {
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

/// Theme contract for backend-rendered widgets.
pub trait WidgetTheme {
    /// Return metrics for a button style.
    fn button_metrics(&self, style: ButtonStyle) -> ButtonMetrics;
    /// Draw button chrome for a style.
    fn draw_button_chrome(&self, draw: &mut dyn DrawContext, bounds: Rect, style: ButtonStyle);
    /// Draw the button state layer for a style.
    fn draw_button_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _style: ButtonStyle,
        _state: WidgetInteractionState,
    ) {
    }

    /// Return metrics for a toggle style.
    fn toggle_metrics(&self, style: ToggleStyle) -> ToggleMetrics;
    /// Draw switch-style toggle chrome.
    fn draw_toggle_switch(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32);
    /// Draw switch-style toggle state layer.
    fn draw_toggle_switch_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _progress: f32,
        _state: WidgetInteractionState,
    ) {
    }
    /// Draw checkbox-style toggle chrome.
    fn draw_toggle_checkbox(&self, draw: &mut dyn DrawContext, bounds: Rect, progress: f32);
    /// Draw checkbox-style toggle state layer.
    fn draw_toggle_checkbox_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _progress: f32,
        _state: WidgetInteractionState,
    ) {
    }

    /// Return stepper metrics.
    fn stepper_metrics(&self) -> StepperMetrics;
    /// Draw one stepper button.
    fn draw_stepper_button(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw one stepper button state layer.
    fn draw_stepper_button_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }

    /// Return text input metrics.
    fn input_field_metrics(&self) -> InputFieldMetrics;
    /// Return the placeholder text color.
    fn input_placeholder_color(&self) -> Color;
    /// Draw text input chrome.
    fn draw_input_field(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw text input state layer.
    fn draw_input_field_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }

    /// Return picker metrics for a style.
    fn picker_metrics(&self, style: PickerStyle) -> PickerMetrics;
    /// Draw the picker indicator.
    fn draw_picker_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw picker field state layer.
    fn draw_picker_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }
    /// Draw the picker popup container.
    fn draw_picker_popup(&self, draw: &mut dyn DrawContext, popup_rect: Rect);
    /// Draw one picker popup row background.
    fn draw_picker_popup_row_background(
        &self,
        draw: &mut dyn DrawContext,
        row_rect: Rect,
        selected: bool,
    );
    /// Draw one picker popup row state layer.
    fn draw_picker_popup_row_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _row_rect: Rect,
        _selected: bool,
        _state: WidgetInteractionState,
    ) {
    }
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
    /// Draw radio picker state layer.
    fn draw_radio_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _selected: bool,
        _state: WidgetInteractionState,
    ) {
    }

    /// Return slider metrics.
    fn slider_metrics(&self) -> SliderMetrics;
    /// Draw slider track chrome.
    fn draw_slider_track(&self, draw: &mut dyn DrawContext, track_rect: Rect, fill_rect: Rect);
    /// Draw slider thumb chrome.
    fn draw_slider_thumb(&self, draw: &mut dyn DrawContext, center: Point, radius: f64);
    /// Draw slider thumb state layer.
    fn draw_slider_thumb_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _state: WidgetInteractionState,
    ) {
    }

    /// Return progress indicator metrics.
    fn progress_metrics(&self, style: ProgressIndicatorStyle) -> ProgressMetrics;
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
    /// Draw a tab button state layer.
    fn draw_tabs_button_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _selected: bool,
        _state: WidgetInteractionState,
    ) {
    }

    /// Draw a scroll indicator.
    fn draw_scroll_indicator(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Draw a list row background.
    fn draw_list_row_background(&self, draw: &mut dyn DrawContext, bounds: Rect, alternate: bool);
    /// Draw a list move affordance.
    fn draw_list_move_control(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a list move affordance state layer.
    fn draw_list_move_control_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }
    /// Draw a list delete affordance.
    fn draw_list_delete_control(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a list delete affordance state layer.
    fn draw_list_delete_control_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }
    /// Draw a list separator.
    fn draw_list_separator(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Draw a table header background.
    fn draw_table_header_background(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a table cell border.
    fn draw_table_cell_border(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a table column separator.
    fn draw_table_column_separator(&self, draw: &mut dyn DrawContext, from: Point, to: Point);
}

/// Progress indicator visual style understood by backend theme packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressIndicatorStyle {
    /// Horizontal linear indicator.
    Linear,
    /// Circular indicator.
    Circular,
}
