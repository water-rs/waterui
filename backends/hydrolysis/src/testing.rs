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

/// A toggle control with no parts, 10 by 20 points and 3 points from its
/// label: what a test style's toggles measure, place and hit-test as.
#[must_use]
pub fn plain_toggle_control() -> crate::theme::ToggleControl {
    crate::theme::ToggleControl::new(
        kurbo::Size::new(10.0, 20.0),
        crate::theme::Motion::uniform(waterui::animation::Animation::linear(
            std::time::Duration::from_millis(100),
        )),
    )
    .label_spacing(3.0)
}

/// A chrome drawing nothing: a clear square surface and a state layer with no
/// feedback, over a 15 millisecond linear motion.
#[must_use]
pub fn plain_chrome() -> crate::theme::Chrome {
    use crate::theme::Surface;
    use crate::theme::{Chrome, CornerShape, FocusIndicator, Motion, PressFeedback, StateLayer};
    Chrome::new(
        Surface::clear(CornerShape::radius(0.0)),
        StateLayer::new(waterui::graphics::Color::new(
            waterui::theme::color::Foreground,
        )),
        PressFeedback::new(),
        FocusIndicator::Tint,
        Motion::uniform(waterui::animation::Animation::linear(
            std::time::Duration::from_millis(15),
        )),
    )
}

/// A stepper look with ends 14 by 14 points, 4 points apart and 8 points from
/// the label, drawing nothing: what a test style's steppers measure, place
/// and hit-test as.
#[must_use]
pub fn plain_stepper_look() -> crate::theme::StepperLook {
    crate::theme::StepperLook::new(plain_chrome(), plain_chrome(), kurbo::Size::new(14.0, 14.0))
        .spacing(4.0)
        .label_spacing(8.0)
}

/// The measures of a test style's menus: 56 point rows, 12 points of vertical
/// padding, 16 of horizontal, at least 112 points wide and at most 320.
#[must_use]
pub const fn plain_menu_layout() -> crate::theme::MenuLayout {
    crate::theme::MenuLayout {
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

/// A menu look measured by [`plain_menu_layout`], drawing nothing: what a
/// test style's picker popups, text context menus and popup windows measure,
/// place and hit-test as.
#[must_use]
pub fn plain_menu_look() -> crate::theme::MenuLook {
    use crate::theme::Custom;
    crate::theme::MenuLook::new(
        plain_menu_layout(),
        Custom::new((), |_recorder, _frame, ()| {}),
        Custom::new((), |_recorder, _frame, ()| {}),
    )
}

/// A menu picker look drawing nothing: a field at least 72 by 28 points,
/// inset 8 by 6 points with 18 points for the indicator, 8 points below its
/// label and 4 points above the popup it opens.
#[must_use]
pub fn plain_menu_picker_look() -> crate::theme::MenuPickerLook {
    crate::theme::MenuPickerLook::new(
        plain_chrome(),
        crate::theme::MenuPickerLayout {
            min_width: 72.0,
            min_height: 28.0,
            horizontal_inset: 8.0,
            vertical_inset: 6.0,
            label_spacing: 8.0,
            indicator_space: 18.0,
            popup_gap: 4.0,
        },
    )
}

/// A radio group look drawing nothing: indicators 16 points square and
/// 8 points from their labels, in rows 8 points apart, in a group at least
/// 72 by 28 points inset 8 by 6 points.
#[must_use]
pub fn plain_radio_group_look() -> crate::theme::RadioGroupLook {
    use crate::theme::{Motion, RadioGroupLook, RadioLayout, ToggleControl};
    RadioGroupLook::new(
        ToggleControl::new(
            kurbo::Size::new(16.0, 16.0),
            Motion::uniform(waterui::animation::Animation::linear(
                std::time::Duration::from_millis(1),
            )),
        )
        .label_spacing(8.0),
        RadioLayout {
            min_width: 72.0,
            min_height: 28.0,
            horizontal_inset: 8.0,
            vertical_inset: 6.0,
            label_spacing: 8.0,
            row_spacing: 8.0,
        },
    )
}

/// A segmented look drawing nothing: what a test style's segmented pickers
/// measure, place and hit-test as.
///
/// Its row is at least 28 points tall, its labels are padded 8 by 6 points
/// in segments at least 58 points wide, 8 points below the heading, in the
/// default font and foreground.
#[must_use]
pub fn plain_segmented_look() -> crate::theme::SegmentedLook {
    use crate::theme::{Motion, SegmentLabel, SegmentedLayout, SegmentedLook};
    SegmentedLook::new(
        SegmentedLayout {
            row_height: 28.0,
            label_padding: kurbo::Vec2::new(8.0, 6.0),
            minimum_segment_width: 58.0,
            minimum_width: 72.0,
            label_spacing: 8.0,
            reach: 0.0,
        },
        SegmentLabel::new(
            waterui_text::font::Font::default(),
            waterui::graphics::Color::new(waterui::theme::color::Foreground),
        ),
        Motion::uniform(waterui::animation::Animation::linear(
            std::time::Duration::from_millis(15),
        )),
    )
}

/// A text field look over `container` and `metrics`.
///
/// Its floating label moves by a 15 ms linear motion, and its caret, selection
/// and placeholder are plain: what a test style's fields measure, place and
/// hit-test as.
#[must_use]
pub fn plain_text_field_look(
    container: crate::theme::Chrome,
    metrics: crate::theme::InputFieldMetrics,
) -> crate::theme::TextFieldLook {
    use crate::theme::{Caret, FloatingLabel, Label, TextCaretMotion, TextFieldLook, ValueMotion};
    use cherenkov_record::Paint;
    use std::time::Duration;
    use waterui::animation::Animation;
    use waterui::graphics::color::Srgb;
    let label = FloatingLabel::new(
        Label::new(),
        ValueMotion::Follow(Animation::linear(Duration::from_millis(15))),
    )
    .scale(0.75)
    .top(4.0);
    let caret = Caret::new(
        Paint::Solid(Srgb::new(0.12, 0.14, 0.18).resolve()),
        TextCaretMotion {
            fade_cycle_duration: Duration::from_millis(1_060),
            frame_interval: Duration::from_millis(530),
            min_opacity: 0.2,
        },
    );
    let selection = Paint::Solid(Srgb::new(0.20, 0.45, 0.90).resolve().with_alpha(0.28));
    TextFieldLook::new(container, metrics, label, caret, selection)
        .placeholder(Label::new().color(waterui::graphics::Color::srgb(0, 0, 0)))
}

/// The metrics a test style's fields lay out by.
#[must_use]
pub const fn plain_input_metrics() -> crate::theme::InputFieldMetrics {
    crate::theme::InputFieldMetrics {
        label_height: 14.0,
        min_width: 100.0,
        min_height: 32.0,
        horizontal_inset: 8.0,
        vertical_inset: 6.0,
    }
}

mod test_style {
    use std::time::Duration;

    use crate::theme::{
        BadgeMetrics, ButtonLook, ButtonSpec, Chrome, CornerShape, DividerMetrics, DragMapping,
        FocusIndicator, HitRegion, IndicatorLook, InteractionMotion, Knob, Label,
        LinearProgressLook, ListMetrics, MenuLook, MenuPickerLook, MenuSpec, Motion,
        NavigationMetrics, NavigationMotion, PickerSpec, PressFeedback, ProgressControl,
        ProgressSpec, RadioGroupLook, SliderLayout, SliderLook, SliderSpec, StateLayer,
        StepperLook, StepperSpec, Surface, TabItemGeometry, TabItemLayout, TableMetrics,
        TabsMetrics, TextFieldLook, TextFieldSpec, ToggleSpec, WidgetTheme,
    };
    use crate::theme::{CheckboxLook, SwitchDrag, SwitchLook};
    use cherenkov_record::{Paint, Recorder};
    use kurbo::{Point, Rect};
    use waterui::animation::Animation;
    use waterui::{Color, EasingCurve, Environment};
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

        fn navigation_motion(&self) -> NavigationMotion {
            NavigationMotion {
                transition_duration: Duration::from_millis(450),
                transition_easing: EasingCurve::bezier(0.2, 0.0, 0.0, 1.0),
                shared_axis_slide_distance: 30.0,
                fade_through_threshold: 0.35,
            }
        }

        /// A button with no visible chrome: a clear surface, a state layer
        /// with no opacity in any state and no press feedback, so a test
        /// sees the button's content and layout alone.
        fn button(&self, _spec: &ButtonSpec, _env: &Environment) -> ButtonLook {
            let chrome = Chrome::new(
                Surface::clear(CornerShape::radius(0.0)),
                StateLayer::new(Color::new(waterui::theme::color::Foreground)),
                PressFeedback::new(),
                FocusIndicator::Tint,
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            )
            // The theme's `touch_delay`.
            .hit(HitRegion::NONE.press_delay(Duration::from_millis(150)));
            ButtonLook::new(chrome, Label::new())
                .padding(1.0, 2.0)
                .minimum_size(kurbo::Size::new(123.0, 45.0))
        }

        fn switch(&self, spec: &ToggleSpec, _env: &Environment) -> SwitchLook {
            SwitchLook::new(
                super::plain_toggle_control(),
                SwitchDrag::new(6.0, spec.direction()),
            )
        }

        fn checkbox(&self, _spec: &ToggleSpec, _env: &Environment) -> CheckboxLook {
            CheckboxLook::new(super::plain_toggle_control())
        }

        fn stepper(&self, _spec: &StepperSpec, _env: &Environment) -> StepperLook {
            super::plain_stepper_look()
        }

        fn text_field(&self, _spec: &TextFieldSpec, _env: &Environment) -> TextFieldLook {
            super::plain_text_field_look(super::plain_chrome(), super::plain_input_metrics())
        }

        fn menu(&self, _spec: &MenuSpec, _env: &Environment) -> MenuLook {
            super::plain_menu_look()
        }

        fn menu_picker(&self, _spec: &PickerSpec, _env: &Environment) -> MenuPickerLook {
            super::plain_menu_picker_look()
        }

        fn radio_group(&self, _spec: &PickerSpec, _env: &Environment) -> RadioGroupLook {
            super::plain_radio_group_look()
        }

        fn segmented_picker(
            &self,
            _spec: &crate::theme::SegmentedSpec,
            _env: &Environment,
        ) -> crate::theme::SegmentedLook {
            super::plain_segmented_look()
        }

        fn slider(&self, _spec: &SliderSpec, _env: &Environment) -> SliderLook {
            SliderLook::new(
                SliderLayout {
                    row_height: 44.0,
                    track_inset: 12.0,
                    label_inset: 12.0,
                    label_gap: 8.0,
                    minimum_track: 72.0,
                    label_spacing: Some(6.0),
                },
                Knob {
                    size: kurbo::Size::new(4.0, 44.0),
                    hit: HitRegion::NONE,
                },
                DragMapping::Absolute,
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            )
        }

        /// A bar with no parts: a 72 wide, 8 tall track inset 8 from each
        /// side, below a label band 18 tall and 10 away and above a value
        /// label 6 away.
        fn linear_progress(&self, _spec: &ProgressSpec, _env: &Environment) -> LinearProgressLook {
            LinearProgressLook::new(
                ProgressControl::new(
                    kurbo::Size::new(72.0, 8.0),
                    Motion::uniform(Animation::linear(Duration::from_millis(15))),
                ),
                8.0,
            )
            .label_band(18.0, 10.0)
            .value_label_band(6.0)
        }

        /// A 32 square with no parts.
        fn circular_progress(&self, _spec: &ProgressSpec, _env: &Environment) -> IndicatorLook {
            IndicatorLook::new(ProgressControl::new(
                kurbo::Size::new(32.0, 32.0),
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            ))
        }

        /// A 48 square with no parts.
        fn loading_indicator(&self, _spec: &ProgressSpec, _env: &Environment) -> IndicatorLook {
            IndicatorLook::new(ProgressControl::new(
                kurbo::Size::new(48.0, 48.0),
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            ))
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
