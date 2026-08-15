//! Widget chrome contracts shared by rendering backends and theme packages.

use core::time::Duration;
use kurbo::{Affine, BezPath, Point, Rect, RoundedRectRadii};
use nami::signal::IntoComputed;
use waterui_controls::button::ButtonStyle;
use waterui_controls::toggle::ToggleStyle;
use waterui_core::EasingCurve;
use waterui_core::animation::Animation;
use waterui_core::handler::SharedAction;
use waterui_core::plugin::Plugin;
use waterui_core::{Binding, Computed, Signal};
use waterui_form::picker::PickerStyle;
use waterui_graphics::color::Color;
use waterui_text::font::Font;

/// Paint source used by backend widget chrome.
#[derive(Debug, Clone)]
pub enum Brush {
    /// A solid peniko color.
    Solid(peniko::Color),
    /// A peniko gradient.
    Gradient(peniko::Gradient),
}

impl From<peniko::Color> for Brush {
    fn from(value: peniko::Color) -> Self {
        Self::Solid(value)
    }
}

impl From<peniko::Gradient> for Brush {
    fn from(value: peniko::Gradient) -> Self {
        Self::Gradient(value)
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

/// Backend-neutral presentation for a semantic interactive surface whose visual
/// chrome is composed by its content view.
///
/// Installing this plugin on a `Button` or tappable view keeps the control's
/// input, keyboard, focus, and accessibility semantics while allowing theme
/// packages to supply exact state-layer geometry and colors for composed
/// controls such as icon buttons, FABs, chips, and navigation destinations.
#[derive(Debug, Clone)]
pub struct InteractionStyle {
    /// Layout metrics used by the semantic button wrapper.
    pub metrics: ButtonMetrics,
    /// State-layer and ripple color.
    pub state_layer_color: Color,
    /// State-layer clipping radii.
    pub state_layer_radii: RoundedRectRadii,
    /// Optional fixed state-layer width.
    pub state_layer_width: Option<f64>,
    /// Optional fixed state-layer height.
    pub state_layer_height: Option<f64>,
    /// Horizontal alignment of a fixed state layer in the hit bounds.
    pub state_layer_alignment_x: f64,
    /// Vertical alignment of a fixed state layer in the hit bounds.
    pub state_layer_alignment_y: f64,
    /// Optional enabled label color.
    pub label_color: Option<Color>,
    /// Optional disabled label color.
    pub disabled_label_color: Option<Color>,
    /// Whether this pointer action participates in keyboard focus traversal.
    pub keyboard_focusable: bool,
}

impl InteractionStyle {
    /// Creates an interaction presentation with transparent semantic-button
    /// chrome and no label-color override.
    #[must_use]
    pub fn new(
        metrics: ButtonMetrics,
        state_layer_color: impl Into<Color>,
        state_layer_radii: impl Into<RoundedRectRadii>,
    ) -> Self {
        Self {
            metrics,
            state_layer_color: state_layer_color.into(),
            state_layer_radii: state_layer_radii.into(),
            state_layer_width: None,
            state_layer_height: None,
            state_layer_alignment_x: 0.5,
            state_layer_alignment_y: 0.5,
            label_color: None,
            disabled_label_color: None,
            keyboard_focusable: true,
        }
    }

    /// Uses a fixed state-layer size aligned within the semantic hit bounds.
    #[must_use]
    pub const fn fixed_state_layer(
        mut self,
        width: f64,
        height: f64,
        alignment_x: f64,
        alignment_y: f64,
    ) -> Self {
        self.state_layer_width = Some(width);
        self.state_layer_height = Some(height);
        self.state_layer_alignment_x = alignment_x;
        self.state_layer_alignment_y = alignment_y;
        self
    }

    /// Resolves the state-layer drawing bounds inside the semantic hit bounds.
    #[must_use]
    pub fn state_layer_bounds(&self, hit_bounds: Rect) -> Rect {
        let width = self
            .state_layer_width
            .unwrap_or_else(|| hit_bounds.width())
            .min(hit_bounds.width());
        let height = self
            .state_layer_height
            .unwrap_or_else(|| hit_bounds.height())
            .min(hit_bounds.height());
        let x0 = (hit_bounds.width() - width)
            .mul_add(self.state_layer_alignment_x.clamp(0.0, 1.0), hit_bounds.x0);
        let y0 = (hit_bounds.height() - height)
            .mul_add(self.state_layer_alignment_y.clamp(0.0, 1.0), hit_bounds.y0);
        Rect::new(x0, y0, x0 + width, y0 + height)
    }

    /// Sets enabled and disabled label colors for semantic text labels.
    #[must_use]
    pub fn label_colors(mut self, enabled: impl Into<Color>, disabled: impl Into<Color>) -> Self {
        self.label_color = Some(enabled.into());
        self.disabled_label_color = Some(disabled.into());
        self
    }

    /// Returns the label color for the current enabled state.
    #[must_use]
    pub fn resolved_label_color(&self, disabled: bool) -> Option<Color> {
        if disabled {
            self.disabled_label_color.clone()
        } else {
            self.label_color.clone()
        }
    }

    /// Keeps the interaction pointer-only, for modal scrims and similar layers.
    #[must_use]
    pub const fn pointer_only(mut self) -> Self {
        self.keyboard_focusable = false;
        self
    }
}

impl Plugin for InteractionStyle {}

/// Publishes whether an interactive subtree owns keyboard focus.
#[derive(Debug, Clone)]
pub struct InteractionFocusBinding {
    focused: Binding<bool>,
    escape_action: Option<SharedAction>,
}

impl InteractionFocusBinding {
    /// Creates a focus observer backed by component-owned state.
    #[must_use]
    pub fn new(focused: &Binding<bool>) -> Self {
        Self {
            focused: focused.clone(),
            escape_action: None,
        }
    }

    /// Handles Escape while this interactive subtree owns keyboard focus.
    #[must_use]
    pub fn escape_action(mut self, action: SharedAction) -> Self {
        self.escape_action = Some(action);
        self
    }

    /// Returns the component-owned focus state.
    #[must_use]
    pub const fn focused(&self) -> &Binding<bool> {
        &self.focused
    }

    /// Returns the configured Escape action.
    #[must_use]
    pub const fn escape_action_handle(&self) -> Option<&SharedAction> {
        self.escape_action.as_ref()
    }
}

impl Plugin for InteractionFocusBinding {}

/// Marks a subtree as a modal interaction scope.
#[derive(Debug, Clone)]
pub struct ModalInteraction {
    active: Computed<bool>,
    close_on_escape: bool,
    escape_action: SharedAction,
}

impl ModalInteraction {
    /// Creates a modal scope with an optional Escape-key action.
    #[must_use]
    pub fn new(close_on_escape: bool, escape_action: SharedAction) -> Self {
        Self {
            active: Computed::constant(true),
            close_on_escape,
            escape_action,
        }
    }

    /// Controls whether the modal scope currently traps interaction.
    #[must_use]
    pub fn active(mut self, active: impl IntoComputed<bool>) -> Self {
        self.active = active.into_computed();
        self
    }

    /// Whether this modal scope currently traps interaction.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.get()
    }

    /// Whether Escape should dispatch the modal action.
    #[must_use]
    pub const fn close_on_escape(&self) -> bool {
        self.close_on_escape
    }

    /// Dispatches the modal Escape action.
    pub fn handle_escape(&self, env: &waterui_core::Environment) {
        self.escape_action.call(env);
    }
}

impl Plugin for ModalInteraction {}

/// Maximum number of press ripple waves tracked concurrently per widget.
///
/// Every press spawns its own wave (Material/mdui semantics), so rapid
/// re-presses overlap: older waves keep fading while the newest grows. A wave
/// lives for at most its grow plus fade-out duration (a few hundred
/// milliseconds), so this bound sits well above what any human click rate can
/// keep alive; when it is exceeded the oldest wave's slot is recycled.
pub const MAX_PRESS_WAVES: usize = 4;

/// One in-flight press ripple wave.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressWave {
    /// Absolute logical-unit origin of the press that spawned this wave.
    /// `None` centers the wave in the target (synthetic/keyboard presses).
    pub origin: Option<Point>,
    /// Animated grow progress in the 0.0..=1.0 range.
    pub progress: f32,
    /// Animated wave opacity sampled by the renderer.
    pub opacity: f32,
}

/// The press ripple waves currently visible on a widget, ordered oldest to
/// newest.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PressWaves {
    waves: [Option<PressWave>; MAX_PRESS_WAVES],
}

impl PressWaves {
    /// No visible press waves.
    pub const EMPTY: Self = Self {
        waves: [None; MAX_PRESS_WAVES],
    };

    /// Appends a wave as the newest entry, recycling the oldest slot when the
    /// set is full.
    pub fn push(&mut self, wave: PressWave) {
        if let Some(slot) = self.waves.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(wave);
            return;
        }
        self.waves.rotate_left(1);
        self.waves[MAX_PRESS_WAVES - 1] = Some(wave);
    }

    /// Iterates the visible waves from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = PressWave> + '_ {
        self.waves.iter().filter_map(|wave| *wave)
    }

    /// Whether no wave is visible.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.waves.iter().all(Option::is_none)
    }

    /// The most recently spawned wave.
    #[must_use]
    pub fn latest(&self) -> Option<PressWave> {
        self.waves.iter().rev().find_map(|wave| *wave)
    }

    /// Maps every wave origin, e.g. from window space into a widget-local
    /// frame.
    pub fn map_origins(&mut self, mut map: impl FnMut(Point) -> Point) {
        for wave in self.waves.iter_mut().flatten() {
            wave.origin = wave.origin.map(&mut map);
        }
    }
}

/// Interactive state snapshot for widget chrome.
#[expect(
    clippy::struct_excessive_bools,
    reason = "a per-frame state snapshot of independent interaction flags, not a configuration"
)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WidgetInteractionState {
    /// The widget is disabled: it must render its inactive appearance and the
    /// renderer suppresses hover, press, and focus for it, so the remaining
    /// fields stay at rest while this is `true`.
    pub disabled: bool,
    /// Pointer is inside the widget bounds.
    pub hovered: bool,
    /// Primary pointer is actively pressing the widget.
    pub pressed: bool,
    /// Keyboard focus is visible on the widget.
    pub focus_visible: bool,
    /// Animated focus affordance progress in the 0.0..=1.0 range.
    pub focus_progress: f32,
    /// Animated state-layer opacity sampled by the renderer.
    pub state_layer_opacity: f32,
    /// The press ripple waves currently visible on the widget: one per press,
    /// so rapid re-presses overlap while older waves fade out.
    pub press_waves: PressWaves,
}

impl WidgetInteractionState {
    /// No active interaction state.
    pub const NONE: Self = Self {
        disabled: false,
        hovered: false,
        pressed: false,
        focus_visible: false,
        focus_progress: 0.0,
        state_layer_opacity: 0.0,
        press_waves: PressWaves::EMPTY,
    };
}

/// Motion policy for interactive widget chrome.
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionMotion {
    /// Hover state-layer opacity.
    pub hover_opacity: f32,
    /// Focus state-layer opacity.
    pub focus_opacity: f32,
    /// Pressed state-layer opacity.
    pub pressed_opacity: f32,
    /// Dragged state-layer opacity.
    pub dragged_opacity: f32,
    /// Animation used when hover appears.
    pub hover_enter: Animation,
    /// Animation used when hover disappears.
    pub hover_exit: Animation,
    /// Animation used when focus appears.
    pub focus_enter: Animation,
    /// Animation used when focus disappears.
    pub focus_exit: Animation,
    /// Animation used when the press layer fades in.
    pub press_fade_in: Animation,
    /// Animation used when the press layer fades out.
    pub press_fade_out: Animation,
    /// Animation used when the pressed ripple grows from origin to final size.
    pub press_grow: Animation,
    /// Minimum amount of time a press must remain visible.
    pub minimum_press_duration: Duration,
    /// Touch delay before showing a press ripple.
    pub touch_delay: Duration,
}

/// Motion policy for progress indicators.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressMotion {
    /// Animation used for linear determinate value changes.
    pub linear_determinate: Animation,
    /// Animation used for circular determinate value changes.
    pub circular_determinate: Animation,
    /// Full cycle duration for linear indeterminate progress.
    pub linear_indeterminate_cycle: Duration,
    /// Full cycle duration for circular indeterminate progress.
    pub circular_indeterminate_cycle: Duration,
}

/// Motion policy for radio selection indicators.
#[derive(Debug, Clone, PartialEq)]
pub struct RadioSelectionMotion {
    /// Animation used when the selected inner dot grows.
    pub inner_grow: Animation,
    /// Animation used when the inner dot fades in or out.
    pub inner_opacity: Animation,
    /// Animation used when the outer ring changes between selected and
    /// unselected colors.
    pub outer_color: Animation,
}

/// Animated visual state for a radio indicator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadioIndicatorState {
    /// Whether the radio option is currently selected semantically.
    pub selected: bool,
    /// Progress from unselected outer-ring color to selected outer-ring color.
    pub outer_selected_progress: f32,
    /// Inner dot radius progress in the 0.0..=1.0 range.
    pub inner_scale: f32,
    /// Inner dot opacity in the 0.0..=1.0 range.
    pub inner_opacity: f32,
}

/// Motion policy for text input carets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextCaretMotion {
    /// Full fade cycle duration.
    pub fade_cycle_duration: Duration,
    /// Time between caret animation redraws.
    pub frame_interval: Duration,
    /// Minimum opacity at the dimmest point of the fade cycle.
    pub min_opacity: f32,
}

/// Motion policy for backend-rendered navigation transitions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationMotion {
    /// Duration for a complete navigation transition.
    pub transition_duration: Duration,
    /// Easing applied to programmatic navigation progress.
    pub transition_easing: EasingCurve,
    /// Absolute travel distance for the Material shared-axis X pattern.
    pub shared_axis_slide_distance: f64,
    /// Progress at which the outgoing layer has faded out and the incoming
    /// layer begins fading in.
    pub fade_through_threshold: f32,
}

/// Toggle layout metrics.
#[derive(Debug, Clone, Copy)]
pub struct ToggleMetrics {
    /// Toggle control width.
    pub width: f64,
    /// Toggle control height.
    pub height: f64,
    /// Spacing between label and toggle control.
    pub label_spacing: f64,
}

impl ToggleMetrics {
    /// Create toggle layout metrics.
    #[must_use]
    pub const fn new(width: f64, height: f64, label_spacing: f64) -> Self {
        Self {
            width,
            height,
            label_spacing,
        }
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
    /// Spacing between stepper buttons.
    pub button_spacing: f64,
    /// Spacing between label and buttons.
    pub label_spacing: f64,
}

impl StepperMetrics {
    /// Create stepper layout metrics.
    #[must_use]
    pub const fn new(
        button_min_size: f64,
        button_max_size: f64,
        button_intrinsic_size: f64,
        button_spacing: f64,
        label_spacing: f64,
    ) -> Self {
        Self {
            button_min_size,
            button_max_size,
            button_intrinsic_size,
            button_spacing,
            label_spacing,
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

/// Layout metrics for backend-rendered context menus.
#[derive(Debug, Clone, Copy)]
pub struct TextContextMenuMetrics {
    /// One-line menu row height.
    pub row_height: f64,
    /// Horizontal text inset.
    pub horizontal_padding: f64,
    /// Vertical text inset.
    pub vertical_padding: f64,
    /// Minimum menu width.
    pub min_width: f64,
    /// Maximum menu width.
    pub max_width: f64,
    /// Estimated width used before text layout is available.
    pub width_per_char: f64,
    /// Menu container corner radius.
    pub corner_radius: f64,
    /// Horizontal inset for separators.
    pub separator_horizontal_inset: f64,
    /// Separator thickness.
    pub separator_thickness: f64,
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
    /// Gap between an external picker label and the picker field.
    pub label_spacing: f64,
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
    /// Popup menu row minimum height.
    pub popup_row_height: f64,
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
    /// Circular progress stroke width.
    pub circular_stroke_width: f64,
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
            circular_stroke_width: 0.0,
        }
    }

    /// Create circular progress layout metrics.
    #[must_use]
    pub const fn circular(circular_diameter: f64, circular_stroke_width: f64) -> Self {
        Self {
            label_height: 0.0,
            bar_top_offset: 0.0,
            bar_height: 0.0,
            bar_horizontal_inset: 0.0,
            value_label_top_spacing: 0.0,
            min_track_width: 0.0,
            circular_diameter,
            circular_stroke_width,
        }
    }
}

/// Layout metrics for tab containers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabsMetrics {
    /// Tab bar height.
    pub bar_height: f64,
    /// Minimum width for one tab button.
    pub button_min_width: f64,
    /// Horizontal label inset inside one tab button.
    pub button_horizontal_inset: f64,
    /// Selected tab active indicator height.
    pub active_indicator_height: f64,
    /// Selected tab active indicator corner radius.
    pub active_indicator_radius: f64,
}

impl TabsMetrics {
    /// Create tab layout metrics.
    #[must_use]
    pub const fn new(
        bar_height: f64,
        button_min_width: f64,
        button_horizontal_inset: f64,
        active_indicator_height: f64,
        active_indicator_radius: f64,
    ) -> Self {
        Self {
            bar_height,
            button_min_width,
            button_horizontal_inset,
            active_indicator_height,
            active_indicator_radius,
        }
    }
}

/// Layout metrics for lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListMetrics {
    /// Minimum one-line row height.
    pub one_line_row_height: f64,
    /// Horizontal content inset.
    pub horizontal_inset: f64,
    /// Vertical content inset.
    pub vertical_inset: f64,
    /// Divider leading inset.
    pub divider_leading_inset: f64,
    /// Divider trailing inset.
    pub divider_trailing_inset: f64,
    /// Width of the row move affordance.
    pub move_control_width: f64,
    /// Width of the row delete affordance.
    pub delete_control_width: f64,
    /// Gap between trailing row affordances.
    pub trailing_control_spacing: f64,
    /// Vertical inset for trailing row affordances.
    pub trailing_control_vertical_inset: f64,
}

/// Row metrics for lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListRowMetrics {
    /// Minimum one-line row height.
    pub one_line_row_height: f64,
    /// Horizontal content inset.
    pub horizontal_inset: f64,
    /// Vertical content inset.
    pub vertical_inset: f64,
}

impl ListRowMetrics {
    /// Create list row layout metrics.
    #[must_use]
    pub const fn new(one_line_row_height: f64, horizontal_inset: f64, vertical_inset: f64) -> Self {
        Self {
            one_line_row_height,
            horizontal_inset,
            vertical_inset,
        }
    }
}

/// Divider metrics for lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListDividerMetrics {
    /// Divider leading inset.
    pub leading_inset: f64,
    /// Divider trailing inset.
    pub trailing_inset: f64,
}

impl ListDividerMetrics {
    /// Create list divider layout metrics.
    #[must_use]
    pub const fn new(leading_inset: f64, trailing_inset: f64) -> Self {
        Self {
            leading_inset,
            trailing_inset,
        }
    }
}

/// Trailing control metrics for lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListTrailingControlMetrics {
    /// Width of the row move affordance.
    pub move_width: f64,
    /// Width of the row delete affordance.
    pub delete_width: f64,
    /// Gap between trailing row affordances.
    pub spacing: f64,
    /// Vertical inset for trailing row affordances.
    pub vertical_inset: f64,
}

impl ListTrailingControlMetrics {
    /// Create list trailing control layout metrics.
    #[must_use]
    pub const fn new(
        move_width: f64,
        delete_width: f64,
        spacing: f64,
        vertical_inset: f64,
    ) -> Self {
        Self {
            move_width,
            delete_width,
            spacing,
            vertical_inset,
        }
    }
}

impl ListMetrics {
    /// Create list layout metrics.
    #[must_use]
    pub const fn new(
        row: ListRowMetrics,
        divider: ListDividerMetrics,
        trailing_controls: ListTrailingControlMetrics,
    ) -> Self {
        Self {
            one_line_row_height: row.one_line_row_height,
            horizontal_inset: row.horizontal_inset,
            vertical_inset: row.vertical_inset,
            divider_leading_inset: divider.leading_inset,
            divider_trailing_inset: divider.trailing_inset,
            move_control_width: trailing_controls.move_width,
            delete_control_width: trailing_controls.delete_width,
            trailing_control_spacing: trailing_controls.spacing,
            trailing_control_vertical_inset: trailing_controls.vertical_inset,
        }
    }
}

/// Layout metrics for dividers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DividerMetrics {
    /// Divider stroke thickness.
    pub thickness: f64,
}

impl DividerMetrics {
    /// Create divider layout metrics.
    #[must_use]
    pub const fn new(thickness: f64) -> Self {
        Self { thickness }
    }
}

/// Layout metrics for badges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BadgeMetrics {
    /// Diameter of a small dot badge.
    pub small_size: f64,
    /// Height and minimum width of a large labeled badge.
    pub large_size: f64,
    /// Horizontal padding inside a large labeled badge.
    pub large_horizontal_padding: f64,
    /// Horizontal offset from the content center for a small badge.
    pub small_offset_x: f64,
    /// Vertical offset from the content top for a small badge.
    pub small_offset_y: f64,
    /// Horizontal offset from the content center for a large badge.
    pub large_offset_x: f64,
    /// Vertical offset from the content top for a large badge.
    pub large_offset_y: f64,
}

impl BadgeMetrics {
    /// Create badge layout metrics.
    #[must_use]
    pub const fn new(
        small_size: f64,
        large_size: f64,
        large_horizontal_padding: f64,
        small_offset_x: f64,
        small_offset_y: f64,
        large_offset_x: f64,
        large_offset_y: f64,
    ) -> Self {
        Self {
            small_size,
            large_size,
            large_horizontal_padding,
            small_offset_x,
            small_offset_y,
            large_offset_x,
            large_offset_y,
        }
    }
}

/// Layout metrics for navigation bars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationMetrics {
    /// Automatic title-mode bar height.
    pub automatic_bar_height: f64,
    /// Inline title-mode bar height.
    pub inline_bar_height: f64,
    /// Large title-mode bar height.
    pub large_bar_height: f64,
    /// Navigation title slot height for inline bars.
    pub inline_title_height: f64,
    /// Navigation title slot height for large bars.
    pub large_title_height: f64,
    /// Title leading inset when no leading bar item is present.
    pub title_leading_inset: f64,
    /// Title trailing inset when no trailing bar item is present.
    pub title_trailing_inset: f64,
    /// Bottom inset for large navigation titles.
    pub large_title_bottom_inset: f64,
    /// Horizontal leading/trailing bar inset.
    pub horizontal_inset: f64,
    /// Gap between title and adjacent items.
    pub item_spacing: f64,
    /// Search field height.
    pub search_height: f64,
    /// Vertical search inset.
    pub search_vertical_inset: f64,
    /// Navigation back button size.
    pub back_button_size: f64,
    /// Navigation back button leading inset.
    pub back_button_leading_inset: f64,
    /// Navigation back button top inset.
    pub back_button_top_inset: f64,
}

/// Layout metrics for backend-rendered tables.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableMetrics {
    /// Minimum table column width.
    pub min_column_width: f64,
    /// Horizontal padding added to measured cell content.
    pub cell_horizontal_padding: f64,
    /// Inset used when placing header and row cell content.
    pub cell_vertical_inset: f64,
    /// Header row height.
    pub header_height: f64,
    /// Data row height.
    pub row_height: f64,
    /// Table outline width.
    pub outline_width: f64,
}

/// Theme contract for backend-rendered widgets.
pub trait WidgetTheme {
    /// Return motion policy for interactive widget chrome.
    fn interaction_motion(&self) -> InteractionMotion;
    /// Return motion policy for progress indicators.
    fn progress_motion(&self) -> ProgressMotion;
    /// Return motion policy for text input carets.
    fn text_caret_motion(&self) -> TextCaretMotion;
    /// Return motion policy for navigation transitions.
    fn navigation_motion(&self) -> NavigationMotion;

    /// Return metrics for a button style.
    fn button_metrics(&self, style: ButtonStyle) -> ButtonMetrics;
    /// Optional button label foreground override. `disabled` selects the
    /// inactive label color (e.g. Material's on-surface at 38%).
    fn button_label_color(&self, _style: ButtonStyle, _disabled: bool) -> Option<Color> {
        None
    }
    /// Optional button label font override.
    fn button_label_font(&self, _style: ButtonStyle) -> Option<Font> {
        None
    }
    /// Content alpha for labels accompanying a disabled control (Material's
    /// 38% disabled-content token). Backends dim a disabled control's label
    /// views with this alpha.
    fn disabled_content_alpha(&self) -> f32 {
        0.38
    }
    /// Draw button chrome for a style. `state` carries the disabled flag so
    /// themes can render the inactive container.
    fn draw_button_chrome(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        style: ButtonStyle,
        state: WidgetInteractionState,
    );
    /// Draw the button state layer for a style.
    fn draw_button_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _style: ButtonStyle,
        _state: WidgetInteractionState,
    ) {
    }
    /// Draw a state layer supplied by a composed semantic control.
    fn draw_interaction_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _radii: RoundedRectRadii,
        _color: peniko::Color,
        _state: WidgetInteractionState,
    ) {
    }

    /// Return metrics for a toggle style.
    fn toggle_metrics(&self, style: ToggleStyle) -> ToggleMetrics;
    /// Return the default animation for toggle value changes.
    fn toggle_value_animation(&self) -> Animation;
    /// Draw switch-style toggle chrome. `progress` is the animated 0..=1
    /// value and `selected` is the toggle's current target, so staged
    /// choreography (e.g. a thumb icon that enters late and exits early) can
    /// tell which direction the animation is heading.
    fn draw_toggle_switch(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        progress: f32,
        selected: bool,
        state: WidgetInteractionState,
    );
    /// Draw switch-style toggle state layer.
    fn draw_toggle_switch_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _progress: f32,
        _selected: bool,
        _state: WidgetInteractionState,
    ) {
    }
    /// Draw checkbox-style toggle chrome. `state` carries the disabled flag
    /// so themes can render the inactive container.
    fn draw_toggle_checkbox(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        progress: f32,
        state: WidgetInteractionState,
    );
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
    /// Draw a stepper decrement icon.
    fn draw_stepper_decrement_icon(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a stepper increment icon.
    fn draw_stepper_increment_icon(&self, draw: &mut dyn DrawContext, bounds: Rect);
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
    /// Return the text selection fill brush.
    fn input_selection_brush(&self) -> Brush;
    /// Return the caret brush for the current caret opacity.
    fn input_caret_brush(&self, opacity: f32) -> Brush;
    /// Draw text input chrome.
    fn draw_input_field(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        state: WidgetInteractionState,
    );
    /// Draw text input state layer.
    fn draw_input_field_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }
    /// Return text context menu metrics.
    fn text_context_menu_metrics(&self) -> TextContextMenuMetrics;
    /// Draw text context menu panel.
    fn draw_text_context_menu_panel(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw text context menu separator.
    fn draw_text_context_menu_separator(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Return picker metrics for a style.
    fn picker_metrics(&self, style: PickerStyle) -> PickerMetrics;
    /// Return the motion policy for radio picker selection changes.
    fn radio_selection_motion(&self) -> RadioSelectionMotion;
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
        state: RadioIndicatorState,
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
    /// Optional segmented picker label foreground override.
    fn segmented_picker_label_color(&self, _selected: bool) -> Option<Color> {
        None
    }
    /// Draw segmented picker container and dividers.
    fn draw_segmented_picker_container(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _segment_count: usize,
    ) {
    }
    /// Draw one segmented picker segment background.
    fn draw_segmented_picker_segment(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _selected: bool,
        _is_first: bool,
        _is_last: bool,
    ) {
    }
    /// Draw one segmented picker state layer.
    fn draw_segmented_picker_state_layer(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _selected: bool,
        _state: WidgetInteractionState,
    ) {
    }

    /// Return slider metrics.
    fn slider_metrics(&self) -> SliderMetrics;
    /// Draw slider track chrome. `state` carries the disabled flag so themes
    /// can render the inactive track.
    fn draw_slider_track(
        &self,
        draw: &mut dyn DrawContext,
        track_rect: Rect,
        fill_rect: Rect,
        state: WidgetInteractionState,
    );
    /// Draw slider thumb chrome.
    fn draw_slider_thumb(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        state: WidgetInteractionState,
    );
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
    /// Draw the linear indeterminate progress indicator.
    fn draw_progress_linear_indeterminate(
        &self,
        draw: &mut dyn DrawContext,
        bounds: Rect,
        elapsed: Duration,
        four_color: bool,
    );
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
    /// Draw the circular indeterminate progress indicator.
    fn draw_progress_circular_indeterminate(
        &self,
        draw: &mut dyn DrawContext,
        center: Point,
        radius: f64,
        width: f64,
        elapsed: Duration,
        four_color: bool,
    );

    /// Return navigation bar layout metrics.
    fn navigation_metrics(&self) -> NavigationMetrics;
    /// Draw a navigation bar background.
    fn draw_navigation_bar(&self, draw: &mut dyn DrawContext, bounds: Rect, background: &Brush);
    /// Draw a navigation bar separator.
    fn draw_navigation_bar_separator(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a navigation back button.
    fn draw_navigation_back_button(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Return tabs layout metrics.
    fn tabs_metrics(&self) -> TabsMetrics;
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

    /// Return divider layout metrics.
    fn divider_metrics(&self) -> DividerMetrics;
    /// Draw a divider.
    fn draw_divider(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Return badge layout metrics.
    fn badge_metrics(&self) -> BadgeMetrics;
    /// Return badge label foreground color.
    fn badge_label_color(&self) -> Color;
    /// Return badge label font.
    fn badge_label_font(&self) -> Font;
    /// Draw a small dot badge.
    fn draw_badge_small(&self, draw: &mut dyn DrawContext, bounds: Rect);
    /// Draw a large labeled badge.
    fn draw_badge_large(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Return list layout metrics.
    fn list_metrics(&self) -> ListMetrics;
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
    /// Draw the background revealed behind a row that is being swiped away.
    ///
    /// `progress` runs `0.0..=1.0` toward the dismiss threshold, and
    /// `toward_start` is `true` while the row travels toward the leading edge,
    /// so a theme can anchor its icon on the side the row is uncovering.
    fn draw_list_swipe_dismiss_background(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _progress: f64,
        _toward_start: bool,
    ) {
    }
    /// Draw the lifted treatment for a row being dragged to a new position.
    fn draw_list_row_lifted(&self, _draw: &mut dyn DrawContext, _bounds: Rect, _elevation: f64) {}
    /// Draw a list separator.
    fn draw_list_separator(&self, draw: &mut dyn DrawContext, bounds: Rect);

    /// Return table layout metrics.
    fn table_metrics(&self) -> TableMetrics;
    /// Draw a table background.
    fn draw_table_background(&self, draw: &mut dyn DrawContext, bounds: Rect);
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
