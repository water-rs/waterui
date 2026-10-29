//! Deterministic environment setup for Hydrolysis-backed tests.

use std::path::PathBuf;

use waterui::Environment;

/// Installs the framework default colour and font tokens required by
/// Hydrolysis rendering, underneath any entries the environment already
/// carries.
///
/// This goes through the same assembly the runtimes use
/// (`crate::theme::install_theme_tokens`) with no style. It is deliberately
/// *not* a [`crate::Style`]: it carries no widget theme, so tests that mount
/// widgets on the rendered runtime must still supply a real style.
pub fn install_theme(env: &mut Environment) {
    crate::theme::install_theme_tokens(env, None, waterui::theme::ColorScheme::Light);
}

/// Returns the root directory Hydrolysis's own test artifacts are written
/// under.
///
/// `WATERUI_TEST_ARTIFACTS_DIR` wins when set — that is the directory CI
/// uploads after a test run — and otherwise artifacts land in a stable
/// temp-directory location.
#[must_use]
pub fn artifacts_root() -> PathBuf {
    std::env::var_os("WATERUI_TEST_ARTIFACTS_DIR").map_or_else(
        || std::env::temp_dir().join("waterui-testing-artifacts"),
        PathBuf::from,
    )
}

/// Returns the directory one logical test case's artifacts live in, inside
/// `suite`.
#[must_use]
pub fn artifact_case_dir(suite: &str, case: &str) -> PathBuf {
    artifacts_root().join(suite).join(case)
}

/// Builds the canonical PNG path for a named snapshot stage of one case.
///
/// The path is the layout `waterui-testing`'s `TestArtifacts` writes into,
/// so an artifact CI collects keeps its `suite/case/stage.png` shape
/// regardless of which crate wrote it.
#[must_use]
pub fn artifact_snapshot_path(suite: &str, case: &str, stage: &str) -> PathBuf {
    artifact_case_dir(suite, case).join(format!("{stage}.png"))
}

mod test_style {
    use std::time::Duration;

    use crate::theme::{
        BadgeMetrics, ButtonMetrics, DividerMetrics, InputFieldMetrics, InteractionMotion,
        ListMetrics, NavigationMetrics, NavigationMotion, PickerMetrics, ProgressIndicatorStyle,
        ProgressMetrics, ProgressMotion, RadioIndicatorState, RadioSelectionMotion, SliderMetrics,
        SliderValueIndicatorMetrics, StepperEnd, StepperMetrics, TabItemGeometry, TabItemLayout,
        TableMetrics, TabsMetrics, TextCaretMotion, TextContextMenuMetrics, ToggleMetrics,
        WidgetInteractionState, WidgetTheme,
    };
    use kurbo::{BezPath, Point, Rect};
    use waterui::animation::Animation;
    use waterui::{Color, EasingCurve, Environment};
    use waterui_controls::ControlSize;
    use waterui_controls::button::ButtonStyle;
    use waterui_controls::toggle::ToggleStyle;
    use waterui_form::picker::PickerStyle;
    use waterui_graphics::color::Srgb;
    use waterui_graphics::draw::{Paint, Recorder};
    use waterui_text::font::Font;

    type TokenInstaller = std::sync::Arc<dyn Fn(&mut Environment) + Send + Sync>;

    /// The smallest [`crate::Style`] a rendered mount can run under.
    ///
    /// `TestStyle` exists for tests whose subject is the backend's own
    /// machinery — layout, dispatch, scroll, accessibility — rather than a
    /// style package's chrome: its metrics are coherent (a button lays out,
    /// a field proposes a floor) so widgets take real frames, and its draw
    /// calls record nothing so no pixel depends on it. A test asserting a
    /// style package's metrics or paint belongs with that package instead.
    ///
    /// `install_tokens` is a no-op by default; a test that probes the token
    /// layer supplies its own installer through [`Self::with_token_installer`]
    /// instead of re-implementing [`WidgetTheme`].
    #[derive(Clone)]
    pub struct TestStyle {
        token_installer: Option<TokenInstaller>,
    }

    impl TestStyle {
        /// The minimal style: framework-default tokens only.
        #[must_use]
        pub fn new() -> Self {
            Self {
                token_installer: None,
            }
        }

        /// A minimal style that additionally runs `install` during
        /// `install_tokens` — the seam a token-layer probe hooks.
        #[must_use]
        pub fn with_token_installer(
            install: impl Fn(&mut Environment) + Send + Sync + 'static,
        ) -> Self {
            Self {
                token_installer: Some(std::sync::Arc::new(install)),
            }
        }
    }

    impl Default for TestStyle {
        fn default() -> Self {
            Self::new()
        }
    }

    impl core::fmt::Debug for TestStyle {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("TestStyle").finish_non_exhaustive()
        }
    }

    impl crate::Style for TestStyle {
        fn install_tokens(&self, env: &mut Environment) {
            if let Some(install) = &self.token_installer {
                install(env);
            }
        }
    }

    impl WidgetTheme for TestStyle {
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
                press_shape: Animation::linear(Duration::from_millis(15)),
                minimum_press_duration: Duration::from_millis(225),
                touch_delay: Duration::from_millis(150),
            }
        }

        fn progress_motion(&self) -> ProgressMotion {
            ProgressMotion {
                linear_determinate: Animation::bezier(
                    Duration::from_millis(250),
                    0.4,
                    0.0,
                    0.6,
                    1.0,
                ),
                circular_determinate: Animation::bezier(
                    Duration::from_millis(500),
                    0.0,
                    0.0,
                    0.2,
                    1.0,
                ),
                linear_indeterminate_cycle: Duration::from_secs(2),
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

        fn button_metrics(&self, _style: ButtonStyle, _size: ControlSize) -> ButtonMetrics {
            ButtonMetrics {
                padding_x: 1.0,
                padding_y: 2.0,
                min_width: 123.0,
                min_height: 45.0,
            }
        }

        fn draw_button_chrome(
            &self,
            _recorder: &mut Recorder,
            _bounds: Rect,
            _style: ButtonStyle,
            _icon_only: bool,
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
            _recorder: &mut Recorder,
            _bounds: Rect,
            _progress: f32,
            _selected: bool,
            _state: WidgetInteractionState,
        ) {
        }

        fn draw_toggle_checkbox(
            &self,
            _recorder: &mut Recorder,
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
            _recorder: &mut Recorder,
            _bounds: Rect,
            _end: StepperEnd,
            _state: WidgetInteractionState,
        ) {
        }
        fn draw_stepper_decrement_icon(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_stepper_increment_icon(&self, _recorder: &mut Recorder, _bounds: Rect) {}

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

        fn input_selection_paint(&self) -> Paint {
            Paint::Solid(Srgb::new(0.20, 0.45, 0.90).resolve().with_alpha(0.28))
        }

        fn input_caret_paint(&self, opacity: f32) -> Paint {
            Paint::Solid(Srgb::new(0.12, 0.14, 0.18).resolve().with_alpha(opacity))
        }

        fn draw_input_field(
            &self,
            _recorder: &mut Recorder,
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

        fn draw_text_context_menu_panel(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_text_context_menu_separator(&self, _recorder: &mut Recorder, _bounds: Rect) {}

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

        fn draw_picker_indicator(&self, _recorder: &mut Recorder, _bounds: Rect) {}

        fn draw_picker_popup(&self, _recorder: &mut Recorder, _popup_rect: Rect) {}

        fn draw_picker_popup_row_background(
            &self,
            _recorder: &mut Recorder,
            _row_rect: Rect,
            _selected: bool,
        ) {
        }

        fn draw_picker_separator(&self, _recorder: &mut Recorder, _separator: Rect) {}

        fn draw_radio_indicator(
            &self,
            _recorder: &mut Recorder,
            _center: Point,
            _radius: f64,
            _state: RadioIndicatorState,
        ) {
        }

        fn slider_metrics(&self, _size: ControlSize) -> SliderMetrics {
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
            _recorder: &mut Recorder,
            _track_rect: Rect,
            _fill_rect: Rect,
            _size: ControlSize,
            _state: WidgetInteractionState,
        ) {
        }

        fn draw_slider_thumb(
            &self,
            _recorder: &mut Recorder,
            _center: Point,
            _radius: f64,
            _size: ControlSize,
            _state: WidgetInteractionState,
        ) {
        }

        fn slider_value_indicator_metrics(&self) -> SliderValueIndicatorMetrics {
            SliderValueIndicatorMetrics::new(8.0, 4.0, 4.0, 0.0, 0.0)
        }

        fn slider_value_indicator_color(&self) -> Color {
            Color::srgb(255, 255, 255)
        }

        fn slider_value_indicator_font(&self) -> Font {
            Font::default()
        }

        fn draw_slider_value_indicator(&self, _recorder: &mut Recorder, _bounds: Rect) {}

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
            _recorder: &mut Recorder,
            _bounds: Rect,
            _active_end: Option<f64>,
        ) {
        }
        fn draw_progress_linear_fill(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_progress_linear_indeterminate(
            &self,
            _recorder: &mut Recorder,
            _bounds: Rect,
            _elapsed: Duration,
            _four_color: bool,
        ) {
        }
        fn draw_progress_circular_track(
            &self,
            _recorder: &mut Recorder,
            _center: Point,
            _radius: f64,
            _width: f64,
            _active_turns: Option<f64>,
        ) {
        }
        fn draw_progress_circular_fill(
            &self,
            _recorder: &mut Recorder,
            _path: &BezPath,
            _width: f64,
        ) {
        }
        fn draw_progress_loading(
            &self,
            _recorder: &mut Recorder,
            _bounds: Rect,
            _elapsed: Duration,
            _four_color: bool,
        ) {
        }

        fn draw_progress_circular_indeterminate(
            &self,
            _recorder: &mut Recorder,
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

        fn draw_navigation_bar(
            &self,
            _recorder: &mut Recorder,
            _bounds: Rect,
            _background: &Paint,
        ) {
        }

        fn draw_navigation_bar_separator(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_navigation_back_button(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn tabs_metrics(&self, _layout: TabItemLayout) -> TabsMetrics {
            TabsMetrics {
                bar_height: 48.0,
                button_min_width: 48.0,
                button_horizontal_inset: 16.0,
                icon_label_spacing: 4.0,
            }
        }
        fn draw_tabs_bar(&self, _recorder: &mut Recorder, _surface: Rect) {}
        fn draw_tabs_bar_divider(&self, _recorder: &mut Recorder, _divider: Rect) {}
        fn draw_tabs_highlight(&self, _recorder: &mut Recorder, _item: TabItemGeometry) {}
        fn draw_scroll_indicator(&self, _recorder: &mut Recorder, _bounds: Rect) {}

        fn divider_metrics(&self) -> DividerMetrics {
            DividerMetrics { thickness: 1.0 }
        }

        fn draw_divider(&self, _recorder: &mut Recorder, _bounds: Rect) {}

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

        fn draw_badge_small(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_badge_large(&self, _recorder: &mut Recorder, _bounds: Rect) {}

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
            _recorder: &mut Recorder,
            _bounds: Rect,
            _alternate: bool,
        ) {
        }
        fn draw_list_move_control(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_list_delete_control(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_list_separator(&self, _recorder: &mut Recorder, _bounds: Rect) {}

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

        fn draw_table_background(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_table_header_background(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_table_cell_border(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_table_column_separator(&self, _recorder: &mut Recorder, _from: Point, _to: Point) {}
    }
}

pub use test_style::TestStyle;
