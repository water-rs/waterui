//! Theme selection for test sessions.
//!
//! Every mounted test declares which widget presentation it runs under:
//!
//! - [`install_test_theme`] installs the [`TestWidgetTheme`] fixture — the
//!   harness's synthetic presentation for semantic contract tests. Its
//!   metrics are deterministic and valid but they are not any application's;
//!   offscreen, snapshot, and performance entry points reject it.
//! - [`theme_with`] composes the renderer's base setup with a real theme
//!   package installer (`theme_with(hydrolysis_m3::install)` reproduces the
//!   presentation generated `WaterUI` applications ship with).
//! - [`UiBuilder::theme`](crate::UiBuilder::theme) installs a caller-supplied
//!   installer verbatim, replacing the base setup entirely.
//!
//! Hydrolysis requires a valid `WidgetTheme` in the environment before it
//! will build, measure, or draw any widget — semantic sessions included.
//! [`crate::ui()`] carries no implicit theme: an unconfigured builder fails
//! at mount with an actionable error, so no test silently runs under a
//! presentation it did not choose.

use core::time::Duration;

use kurbo::{BezPath, Point, Rect};
use peniko::Color as PenikoColor;
use waterui_backend_core::widget::{
    BadgeMetrics, Brush, ButtonMetrics, DividerMetrics, DrawContext, InputFieldMetrics,
    InteractionMotion, ListMetrics, NavigationMetrics, NavigationMotion, PickerMetrics,
    ProgressIndicatorStyle, ProgressMetrics, ProgressMotion, RadioIndicatorState,
    RadioSelectionMotion, SliderMetrics, StepperEnd, StepperMetrics, TableMetrics, TabsMetrics,
    TextCaretMotion, TextContextMenuMetrics, ToggleMetrics, WidgetInteractionState, WidgetTheme,
};
use waterui_controls::button::{ButtonSize, ButtonStyle};
use waterui_controls::toggle::ToggleStyle;
use waterui_core::animation::Animation;
use waterui_core::{EasingCurve, Environment};
use waterui_form::picker::PickerStyle;
use waterui_graphics::color::Color;
use waterui_text::font::Font;

use crate::app::ThemeInstaller;

/// Marker installed by [`install_test_theme`] so visual and performance
/// entry points can reject the synthetic fixture: its draw methods are
/// no-ops, so a snapshot or a frame-time measurement taken under it describes
/// a workload no real theme produces.
pub struct SyntheticThemeMarker;

/// Composes the renderer's base theme setup with a caller's theme installer.
///
/// The base installer (`hydrolysis::testing::install_theme`) supplies the
/// deterministic renderer tokens tests rely on — a light color scheme and the
/// bundled `FontSettings::default_scale`. The caller's installer then applies
/// its own tokens and widget realizations on top, exactly as it would in an
/// application environment.
///
/// `theme_with(hydrolysis_m3::install)` reproduces the presentation that
/// generated `WaterUI` applications ship with; Material reads the installed
/// color scheme and preserves the bundled font defaults, so this composition
/// — not a bare `theme(hydrolysis_m3::install)` — is what previously
/// un-themed tests ran under.
///
/// The composer installs only the base tokens before delegating; an
/// incomplete custom installer does not silently inherit the synthetic
/// fixture's metrics.
pub fn theme_with<U: ThemeInstaller>(theme: U) -> impl ThemeInstaller {
    move |env: &mut Environment| {
        hydrolysis::testing::install_theme(env);
        theme.install(env);
    }
}

/// Installs the harness's synthetic test presentation: the renderer's base
/// tokens plus [`TestWidgetTheme`].
///
/// Use this for semantic contract tests — role, label, value, action, state,
/// and tree-structure assertions, and interaction routing that resolves
/// through the same computed bounds the assertions read. The fixture reports
/// the session's presentation as synthetic via a marker the visual entry
/// points check: `mount_offscreen`, `snapshot`, and the `perf` paths reject
/// it, because captures and timings taken under no-op chrome would describe a
/// workload no real theme produces.
pub fn install_test_theme(env: &mut Environment) {
    hydrolysis::testing::install_theme(env);
    env.insert(Box::new(TestWidgetTheme) as Box<dyn WidgetTheme>);
    env.insert(SyntheticThemeMarker);
}

/// The harness's synthetic `WidgetTheme` fixture.
///
/// Every required trait method returns deterministic, valid values:
/// meaningful control dimensions, internally consistent track/thumb and
/// sizing relationships, and finite non-zero motion timings. Painting
/// methods are no-ops — the fixture exists so semantic tests exercise real
/// widget build, measure, interaction, and accessibility paths without
/// depending on an application theme package.
///
/// These are test inputs, not application presentation: assertions on exact
/// geometry, wrapping, clipping, or pixels belong to tests that select a real
/// theme explicitly.
#[derive(Debug, Default)]
pub struct TestWidgetTheme;

impl WidgetTheme for TestWidgetTheme {
    fn interaction_motion(&self) -> InteractionMotion {
        InteractionMotion {
            hover_opacity: 0.08,
            focus_opacity: 0.12,
            pressed_opacity: 0.12,
            dragged_opacity: 0.16,
            hover_enter: Animation::linear(Duration::from_millis(15)),
            hover_exit: Animation::linear(Duration::from_millis(15)),
            focus_enter: Animation::linear(Duration::from_millis(15)),
            focus_exit: Animation::linear(Duration::from_millis(15)),
            press_fade_in: Animation::linear(Duration::from_millis(105)),
            press_fade_out: Animation::linear(Duration::from_millis(375)),
            press_grow: Animation::bezier(Duration::from_millis(450), 0.2, 0.0, 0.0, 1.0),
            minimum_press_duration: Duration::from_millis(225),
            touch_delay: Duration::from_millis(150),
        }
    }

    fn progress_motion(&self) -> ProgressMotion {
        ProgressMotion {
            linear_determinate: Animation::bezier(Duration::from_millis(250), 0.4, 0.0, 0.6, 1.0),
            circular_determinate: Animation::bezier(Duration::from_millis(500), 0.0, 0.0, 0.2, 1.0),
            linear_indeterminate_cycle: Duration::from_millis(2_000),
            loading_cycle: Duration::from_millis(4_666),
            circular_indeterminate_cycle: Duration::from_millis(5_332),
        }
    }

    fn text_caret_motion(&self) -> TextCaretMotion {
        TextCaretMotion {
            fade_cycle_duration: Duration::from_millis(1_060),
            frame_interval: Duration::from_millis(530),
            min_opacity: 0.2,
        }
    }

    fn navigation_motion(&self) -> NavigationMotion {
        NavigationMotion {
            transition_duration: Duration::from_millis(450),
            transition_easing: EasingCurve::bezier(0.2, 0.0, 0.0, 1.0),
            shared_axis_slide_distance: 30.0,
            fade_through_threshold: 0.35,
        }
    }

    fn button_metrics(&self, _style: ButtonStyle, _size: ButtonSize) -> ButtonMetrics {
        ButtonMetrics {
            padding_x: 1.0,
            padding_y: 2.0,
            min_width: 123.0,
            min_height: 45.0,
        }
    }

    fn draw_button_chrome(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _style: ButtonStyle,
        _state: WidgetInteractionState,
    ) {
    }

    fn toggle_metrics(&self, _style: ToggleStyle) -> ToggleMetrics {
        ToggleMetrics {
            width: 10.0,
            height: 20.0,
            label_spacing: 3.0,
        }
    }

    fn toggle_value_animation(&self) -> Animation {
        Animation::linear(Duration::from_millis(100))
    }

    fn draw_toggle_switch(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _progress: f32,
        _selected: bool,
        _state: WidgetInteractionState,
    ) {
    }

    fn draw_toggle_checkbox(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _progress: f32,
        _state: WidgetInteractionState,
    ) {
    }

    fn stepper_metrics(&self) -> StepperMetrics {
        StepperMetrics {
            button_min_size: 12.0,
            button_max_size: 18.0,
            button_intrinsic_size: 14.0,
            button_spacing: 4.0,
            label_spacing: 8.0,
        }
    }

    fn draw_stepper_button(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _end: StepperEnd,
        _state: WidgetInteractionState,
    ) {
    }
    fn draw_stepper_decrement_icon(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_stepper_increment_icon(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn input_field_metrics(&self) -> InputFieldMetrics {
        InputFieldMetrics {
            label_height: 14.0,
            min_width: 100.0,
            min_height: 32.0,
            horizontal_inset: 8.0,
            vertical_inset: 6.0,
        }
    }

    fn input_placeholder_color(&self) -> Color {
        Color::srgb(0, 0, 0)
    }

    fn input_selection_brush(&self) -> Brush {
        Brush::from(PenikoColor::new([0.20, 0.45, 0.90, 0.28]))
    }

    fn input_caret_brush(&self, opacity: f32) -> Brush {
        Brush::from(PenikoColor::new([0.12, 0.14, 0.18, opacity]))
    }

    fn draw_input_field(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _state: WidgetInteractionState,
    ) {
    }

    fn text_context_menu_metrics(&self) -> TextContextMenuMetrics {
        TextContextMenuMetrics {
            row_height: 56.0,
            horizontal_padding: 16.0,
            vertical_padding: 12.0,
            min_width: 112.0,
            max_width: 320.0,
            width_per_char: 8.5,
            corner_radius: 4.0,
            separator_horizontal_inset: 16.0,
            separator_thickness: 1.0,
        }
    }

    fn draw_text_context_menu_panel(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_text_context_menu_separator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn picker_metrics(&self, _style: PickerStyle) -> PickerMetrics {
        PickerMetrics {
            min_width: 72.0,
            min_height: 28.0,
            horizontal_inset: 8.0,
            vertical_inset: 6.0,
            label_spacing: 8.0,
            indicator_space: 18.0,
            radio_indicator_size: 16.0,
            radio_label_spacing: 8.0,
            radio_row_spacing: 8.0,
            popup_top_spacing: 4.0,
            popup_row_height: 48.0,
            popup_corner_radius: 6.0,
            segment_min_width: 58.0,
        }
    }

    fn radio_selection_motion(&self) -> RadioSelectionMotion {
        RadioSelectionMotion {
            inner_grow: Animation::linear(Duration::from_millis(1)),
            inner_opacity: Animation::linear(Duration::from_millis(1)),
            outer_color: Animation::linear(Duration::from_millis(1)),
        }
    }

    fn draw_picker_indicator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_picker_popup(&self, _draw: &mut dyn DrawContext, _popup_rect: Rect) {}
    fn draw_picker_popup_row_background(
        &self,
        _draw: &mut dyn DrawContext,
        _row_rect: Rect,
        _selected: bool,
    ) {
    }
    fn draw_picker_separator(&self, _draw: &mut dyn DrawContext, _separator: Rect) {}
    fn draw_radio_indicator(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _state: RadioIndicatorState,
    ) {
    }

    fn slider_metrics(&self) -> SliderMetrics {
        SliderMetrics {
            horizontal_inset: 12.0,
            horizontal_spacing: 8.0,
            vertical_spacing: 6.0,
            min_track_width: 72.0,
            track_height: 6.0,
            handle_width: 4.0,
            handle_height: 44.0,
        }
    }

    fn draw_slider_track(
        &self,
        _draw: &mut dyn DrawContext,
        _track_rect: Rect,
        _fill_rect: Rect,
        _state: WidgetInteractionState,
    ) {
    }
    fn draw_slider_thumb(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _state: WidgetInteractionState,
    ) {
    }

    fn progress_metrics(&self, style: ProgressIndicatorStyle) -> ProgressMetrics {
        match style {
            ProgressIndicatorStyle::Loading => ProgressMetrics::loading(48.0, 38.0),
            ProgressIndicatorStyle::Linear => ProgressMetrics {
                label_height: 18.0,
                bar_top_offset: 10.0,
                bar_height: 8.0,
                bar_horizontal_inset: 8.0,
                value_label_top_spacing: 6.0,
                min_track_width: 72.0,
                circular_diameter: 0.0,
                circular_stroke_width: 0.0,
                loading_indicator_size: 0.0,
            },
            ProgressIndicatorStyle::Circular => ProgressMetrics {
                label_height: 0.0,
                bar_top_offset: 0.0,
                bar_height: 0.0,
                bar_horizontal_inset: 0.0,
                value_label_top_spacing: 0.0,
                min_track_width: 0.0,
                circular_diameter: 32.0,
                circular_stroke_width: 5.0,
                loading_indicator_size: 0.0,
            },
        }
    }

    fn draw_progress_linear_track(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _active_end: Option<f64>,
    ) {
    }
    fn draw_progress_linear_fill(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_progress_linear_indeterminate(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _elapsed: Duration,
        _four_color: bool,
    ) {
    }
    fn draw_progress_circular_track(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _width: f64,
        _active_turns: Option<f64>,
    ) {
    }
    fn draw_progress_circular_fill(
        &self,
        _draw: &mut dyn DrawContext,
        _path: &BezPath,
        _width: f64,
    ) {
    }
    fn draw_progress_loading(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _elapsed: Duration,
        _four_color: bool,
    ) {
    }
    fn draw_progress_circular_indeterminate(
        &self,
        _draw: &mut dyn DrawContext,
        _center: Point,
        _radius: f64,
        _width: f64,
        _elapsed: Duration,
        _four_color: bool,
    ) {
    }

    fn navigation_metrics(&self) -> NavigationMetrics {
        NavigationMetrics {
            automatic_bar_height: 64.0,
            inline_bar_height: 64.0,
            medium_bar_height: 112.0,
            large_bar_height: 152.0,
            inline_title_height: 28.0,
            medium_title_height: 36.0,
            large_title_height: 36.0,
            title_leading_inset: 16.0,
            title_trailing_inset: 16.0,
            large_title_bottom_inset: 28.0,
            horizontal_inset: 4.0,
            item_spacing: 0.0,
            search_height: 56.0,
            search_vertical_inset: 4.0,
            back_button_size: 40.0,
            back_button_leading_inset: 4.0,
            back_button_top_inset: 12.0,
        }
    }

    fn draw_navigation_bar(&self, _draw: &mut dyn DrawContext, _bounds: Rect, _background: &Brush) {
    }
    fn draw_navigation_bar_separator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_navigation_back_button(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn tabs_metrics(&self) -> TabsMetrics {
        TabsMetrics {
            bar_height: 48.0,
            button_min_width: 48.0,
            button_horizontal_inset: 16.0,
            active_indicator_height: 3.0,
            active_indicator_radius: 3.0,
        }
    }
    fn draw_tabs_bar(&self, _draw: &mut dyn DrawContext, _bounds: Rect, _top_edge: bool) {}
    fn draw_tabs_highlight(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_scroll_indicator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn divider_metrics(&self) -> DividerMetrics {
        DividerMetrics { thickness: 1.0 }
    }
    fn draw_divider(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn badge_metrics(&self) -> BadgeMetrics {
        BadgeMetrics {
            small_size: 6.0,
            large_size: 16.0,
            large_horizontal_padding: 4.0,
            small_offset_x: 6.0,
            small_offset_y: 6.0,
            large_offset_x: 12.0,
            large_offset_y: 14.0,
        }
    }

    fn badge_label_color(&self) -> Color {
        Color::srgb(255, 255, 255)
    }

    fn badge_label_font(&self) -> Font {
        Font::default()
    }

    fn draw_badge_small(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_badge_large(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn list_metrics(&self) -> ListMetrics {
        ListMetrics {
            one_line_row_height: 56.0,
            horizontal_inset: 16.0,
            vertical_inset: 10.0,
            divider_leading_inset: 16.0,
            divider_trailing_inset: 16.0,
            move_control_width: 20.0,
            delete_control_width: 26.0,
            trailing_control_spacing: 6.0,
            trailing_control_vertical_inset: 6.0,
            section_header_height: 48.0,
            section_footer_height: 40.0,
        }
    }

    fn draw_list_row_background(
        &self,
        _draw: &mut dyn DrawContext,
        _bounds: Rect,
        _alternate: bool,
    ) {
    }
    fn draw_list_move_control(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_list_delete_control(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_list_separator(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}

    fn table_metrics(&self) -> TableMetrics {
        TableMetrics {
            min_column_width: 72.0,
            cell_horizontal_padding: 32.0,
            cell_vertical_inset: 16.0,
            header_height: 56.0,
            row_height: 52.0,
            outline_width: 1.0,
        }
    }

    fn draw_table_background(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_table_header_background(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_table_cell_border(&self, _draw: &mut dyn DrawContext, _bounds: Rect) {}
    fn draw_table_column_separator(&self, _draw: &mut dyn DrawContext, _from: Point, _to: Point) {}
}
