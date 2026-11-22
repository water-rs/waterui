//! Standard conformance suites for `WaterUI` core components and Hydrolysis
//! [`Style`] packages.

use core::fmt;

use waterui::component::button::button;
use waterui::component::progress::progress;
use waterui::component::{hstack, vstack};
use waterui::form::picker::{PickerItem, PickerStyle, picker};
use waterui::layout::scroll::ScrollView;
use waterui::prelude::slider::slider;
use waterui::prelude::stepper::stepper;
use waterui::prelude::{TextField, Toggle, ToggleStyle, text};
use waterui::reactive::{Binding, Signal as _, SignalExt as _, binding};
use waterui::theme::color as theme_color;
use waterui::{Str, View, ViewExt as _};
use waterui_testing::{Role, Style};

use crate::case::{Appearance, RenderCase, ShowcaseSuite};
use crate::error::ConformanceError;
use crate::runner::{GalleryConfig, GalleryRunOutput, GalleryRunner};

/// Builds the standard `WaterUI` conformance suite covering core components,
/// typography (including CJK), theme tokens, and multi-stage interactive states
/// across Light and Dark appearances.
#[must_use]
pub fn waterui_standard_suite() -> ShowcaseSuite {
    ShowcaseSuite::new("waterui-standard")
        .with_both_appearances("typography", |name, _| {
            RenderCase::new(name, typography_view)
                .viewport(440, 260)
                .interaction_stage("00_initial", |app| {
                    app.query()
                        .role(Role::LABEL)
                        .label("Typography")
                        .assert_exists();
                })
        })
        .with_both_appearances("buttons", |name, _| {
            RenderCase::new(name, buttons_view)
                .viewport(420, 320)
                .interaction_stage("00_idle", |app| {
                    app.query().label("Primary Action").assert_exists();
                    app.query().label("Taps: 0").assert_exists();
                })
                .interaction_stage("01_tapped", |app| {
                    app.query().label("Primary Action").tap();
                    app.query().label("Taps: 1").assert_exists();
                })
        })
        .with_both_appearances("toggles", |name, _| {
            RenderCase::new(name, toggles_view)
                .viewport(420, 220)
                .interaction_stage("00_idle", |app| {
                    app.query().label("Wi-Fi").assert_exists();
                    app.query().label("Bluetooth").assert_exists();
                    app.query()
                        .label("Wi-Fi: false · Bluetooth: true")
                        .assert_exists();
                })
                .interaction_stage("01_toggled", |app| {
                    app.query().label("Wi-Fi").tap();
                    app.query()
                        .label("Wi-Fi: true · Bluetooth: true")
                        .assert_exists();
                })
        })
        .with_both_appearances("slider", |name, _| {
            RenderCase::new(name, slider_view)
                .viewport(420, 220)
                .interaction_stage("00_idle", |app| {
                    app.query().label("Volume").assert_exists();
                })
        })
        .with_both_appearances("stepper", |name, _| {
            RenderCase::new(name, stepper_view)
                .viewport(420, 180)
                .interaction_stage("00_idle", |app| {
                    app.query().label("Quantity").assert_exists();
                    app.query().label("Count: 2").assert_exists();
                })
                .interaction_stage("01_incremented", |app| {
                    app.query().label("Quantity").increment();
                    app.query().label("Count: 3").assert_exists();
                })
        })
        .with_both_appearances("text-field", |name, _| {
            RenderCase::new(name, text_field_view)
                .viewport(420, 200)
                .interaction_stage("00_idle", |app| {
                    app.query().label("Name").assert_exists();
                })
                .interaction_stage("01_entered", |app| {
                    app.query().label("Name").set_text("WaterUI");
                    app.query().label("Echo: WaterUI").assert_exists();
                })
        })
        .with_both_appearances("picker", |name, _| {
            RenderCase::new(name, picker_view)
                .viewport(440, 320)
                .interaction_stage("00_idle", |app| {
                    app.query().label("Segmented").assert_exists();
                    app.query().label("Selected: Medium").assert_exists();
                })
        })
        .with_both_appearances("progress", |name, _| {
            RenderCase::new(name, progress_view)
                .viewport(420, 220)
                .interaction_stage("00_idle", |app| {
                    app.query().label("Downloading").assert_exists();
                })
        })
        .with_both_appearances("surfaces", |name, _| {
            RenderCase::new(name, surfaces_view)
                .viewport(440, 240)
                .interaction_stage("00_idle", |app| {
                    app.query().label("Primary Surface").assert_exists();
                    app.query().label("Variant Surface").assert_exists();
                })
        })
}

/// Builds the Hydrolysis [`Style`] conformance suite.
///
/// Validates every `WidgetTheme` look across Light and Dark appearances and
/// interactive states without coupling any style-specific shader or animation
/// model into the contract.
#[must_use]
pub fn hydrolysis_style_suite<S>(style: S) -> StyleConformanceSuite<S>
where
    S: Style + Clone + 'static,
{
    StyleConformanceSuite {
        style_factory: Box::new(move |_| style.clone()),
        suite: build_hydrolysis_style_cases(),
        config: GalleryConfig::default().title("Hydrolysis Style Conformance Gallery"),
    }
}

/// Variant of [`hydrolysis_style_suite`] for styles constructed per
/// [`Appearance`] or that do not implement [`Clone`].
#[must_use]
pub fn hydrolysis_style_suite_with<S, F>(style_factory: F) -> StyleConformanceSuite<S>
where
    S: Style,
    F: Fn(Appearance) -> S + 'static,
{
    StyleConformanceSuite {
        style_factory: Box::new(style_factory),
        suite: build_hydrolysis_style_cases(),
        config: GalleryConfig::default().title("Hydrolysis Style Conformance Gallery"),
    }
}

/// Configured Hydrolysis [`Style`] conformance runner returned by
/// [`hydrolysis_style_suite`].
pub struct StyleConformanceSuite<S: Style> {
    style_factory: Box<dyn Fn(Appearance) -> S>,
    suite: ShowcaseSuite,
    config: GalleryConfig,
}

impl<S: Style> fmt::Debug for StyleConformanceSuite<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StyleConformanceSuite")
            .field("suite", &self.suite)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl<S: Style> StyleConformanceSuite<S> {
    /// Returns the underlying [`ShowcaseSuite`].
    #[must_use]
    pub const fn suite(&self) -> &ShowcaseSuite {
        &self.suite
    }

    /// Returns a mutable reference to the underlying [`ShowcaseSuite`] so
    /// style packages can append extra style-specific cases.
    #[must_use]
    pub const fn suite_mut(&mut self) -> &mut ShowcaseSuite {
        &mut self.suite
    }

    /// Replaces the [`GalleryConfig`] used when running this suite.
    #[must_use]
    pub fn with_config(mut self, config: GalleryConfig) -> Self {
        self.config = config;
        self
    }

    /// Runs the Hydrolysis style conformance suite and emits gallery artifacts.
    ///
    /// # Errors
    ///
    /// Returns [`ConformanceError`] if image encoding, reference comparison, or
    /// gallery emission fails.
    pub fn run(&self) -> Result<GalleryRunOutput, ConformanceError> {
        let runner = GalleryRunner::new(self.config.clone());
        runner.run_suites_with(&[&self.suite], |appearance| {
            (self.style_factory)(appearance)
        })
    }

    /// Runs the Hydrolysis style conformance suite and panics if any stage fails.
    ///
    /// # Panics
    ///
    /// Panics if running fails or any stage fails blank-frame or FLIP checks.
    #[must_use]
    #[track_caller]
    pub fn assert_pass(&self) -> GalleryRunOutput {
        let output = self
            .run()
            .expect("hydrolysis style conformance suite should run without I/O errors");
        output.assert_pass();
        output
    }
}

fn build_hydrolysis_style_cases() -> ShowcaseSuite {
    let mut suite = ShowcaseSuite::new("hydrolysis-style");
    for case in waterui_standard_suite().cases {
        suite.add_case(case);
    }
    suite.with_both_appearances("scroll-container", |name, _| {
        RenderCase::new(name, scroll_container_view)
            .viewport(360, 220)
            .interaction_stage("00_top", |app| {
                app.query().label("Row 1").assert_exists();
            })
    })
}

fn backdrop<V: View>(content: V) -> impl View {
    content
        .padding_with(16.0)
        .background(theme_color::Background)
}

fn typography_view() -> impl View {
    backdrop(
        vstack((
            text("Typography").title(),
            text("Headline · Subheadline · Body").headline(),
            text("Fine-grained reactive UI across native and self-drawn backends.").body(),
            text("WaterUI 界面 · テスト · 인터페이스").body(),
            text("Caption muted metadata")
                .caption()
                .foreground(theme_color::MutedForeground),
        ))
        .spacing(10.0)
        .leading(),
    )
}

fn buttons_view() -> impl View {
    let taps: Binding<u32> = binding(0_u32);
    let bump = {
        let taps = taps.clone();
        move || {
            let next = taps.snapshot() + 1;
            taps.set(next);
        }
    };
    backdrop(
        vstack((
            text!("Taps: {taps}").headline(),
            button("Primary Action").bordered_prominent().action(bump),
            button("Bordered").bordered().action(|| {}),
            button("Automatic").action(|| {}),
            button("Plain").plain().action(|| {}),
            button("Borderless").borderless().action(|| {}),
        ))
        .spacing(10.0)
        .leading(),
    )
}

fn toggles_view() -> impl View {
    let wifi = binding(false);
    let bluetooth = binding(true);
    backdrop(
        vstack((
            Toggle::new("Wi-Fi", &wifi).style(ToggleStyle::Switch),
            Toggle::new("Bluetooth", &bluetooth).style(ToggleStyle::Checkbox),
            text!("Wi-Fi: {wifi} · Bluetooth: {bluetooth}").body(),
        ))
        .spacing(12.0)
        .leading(),
    )
}

fn slider_view() -> impl View {
    let volume = binding(50.0_f64);
    backdrop(
        vstack((
            slider("Volume", &volume).range(0.0..=100.0),
            progress(volume.clone().map(|v| v / 100.0)).label("Volume Level"),
            text!("Volume: {volume}").body(),
        ))
        .spacing(12.0)
        .leading(),
    )
}

fn stepper_view() -> impl View {
    let quantity = binding(2_i32);
    backdrop(
        vstack((
            stepper("Quantity", &quantity).range(0..=10),
            text!("Count: {quantity}").body(),
        ))
        .spacing(12.0)
        .leading(),
    )
}

fn text_field_view() -> impl View {
    let name: Binding<Str> = binding(Str::from(""));
    backdrop(
        vstack((
            TextField::new("Name", &name).prompt("Type your name"),
            text!("Echo: {name}").body(),
        ))
        .spacing(12.0)
        .leading(),
    )
}

fn picker_items() -> Vec<PickerItem<&'static str>> {
    vec![
        text("Small").tag("Small"),
        text("Medium").tag("Medium"),
        text("Large").tag("Large"),
    ]
}

fn picker_view() -> impl View {
    let size = binding("Medium");
    backdrop(
        vstack((
            text("Segmented").sub_headline(),
            picker("Size", picker_items(), &size).style(PickerStyle::Segmented),
            text("Menu").sub_headline(),
            picker("Size", picker_items(), &size).style(PickerStyle::Menu),
            text("Radio").sub_headline(),
            picker("Size", picker_items(), &size).style(PickerStyle::Radio),
            text!("Selected: {size}").body(),
        ))
        .spacing(10.0)
        .leading(),
    )
}

fn progress_view() -> impl View {
    backdrop(
        vstack((
            progress(0.25).label("Downloading"),
            progress(0.50).label("Processing"),
            progress(0.75).label("Finalizing"),
        ))
        .spacing(14.0)
        .leading(),
    )
}

fn surfaces_view() -> impl View {
    backdrop(
        hstack((
            vstack((
                text("Primary Surface").headline(),
                text("Standard card surface token")
                    .caption()
                    .foreground(theme_color::MutedForeground),
            ))
            .spacing(6.0)
            .leading()
            .padding_with(14.0)
            .background(theme_color::Surface),
            vstack((
                text("Variant Surface").headline(),
                text("Accent highlight")
                    .caption()
                    .foreground(theme_color::Accent),
            ))
            .spacing(6.0)
            .leading()
            .padding_with(14.0)
            .background(theme_color::SurfaceVariant),
        ))
        .spacing(12.0),
    )
}

fn scroll_container_view() -> impl View {
    backdrop(ScrollView::vertical(
        vstack((
            text("Row 1").body(),
            text("Row 2").body(),
            text("Row 3").body(),
            text("Row 4").body(),
            text("Row 5").body(),
            text("Row 6").body(),
            text("Row 7").body(),
            text("Row 8").body(),
            text("Row 9").body(),
            text("Row 10").body(),
        ))
        .spacing(12.0)
        .leading(),
    ))
}
