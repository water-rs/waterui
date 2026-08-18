//! Small views shared across sections.
//!
//! These exist so a metric looks the same wherever it appears, and so a value
//! that is genuinely unknown renders as unknown rather than as zero.

use waterui::color::signal_color;
use waterui::prelude::theme_color::{Accent, MutedForeground};
use waterui::prelude::*;
use waterui_inspector_protocol::LogLevel;

use crate::model::{Model, Section};

/// A filled dot, accent-coloured when live and muted when not.
#[expect(
    clippy::needless_pass_by_value,
    reason = "a signal handle is cloned into the view it drives, not borrowed"
)]
pub fn status_dot(live: Computed<bool>) -> impl View {
    signal_color(live.select(Color::from(Accent), Color::from(MutedForeground)))
        .clip(shape::Circle)
        .size(9.0, 9.0)
}

/// A labelled figure — the unit of every statistics row in the app.
#[expect(
    clippy::needless_pass_by_value,
    reason = "a signal handle is cloned into the view it drives, not borrowed"
)]
pub fn metric(label: impl Into<Str>, value: Computed<Str>) -> impl View {
    let label = label.into();
    vstack((
        text!("{value}").headline(),
        text!("{label}").caption().foreground(MutedForeground),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(2.0)
}

/// The number shown against a section in the sidebar.
pub fn count_badge(section: Section, model: &Model) -> impl View {
    let count: Computed<Str> = match section {
        Section::Overview => model
            .dropped
            .clone()
            .map(|dropped| {
                if dropped == 0 {
                    Str::from("")
                } else {
                    Str::from(format!("{dropped} lost"))
                }
            })
            .computed(),
        Section::Frames => model.frames_seen.clone().map(count_label).computed(),
        Section::Tree => model.tree.clone().map(row_count).computed(),
        Section::Tasks => model.tasks.clone().map(row_count).computed(),
        Section::Logs => model.logs.clone().map(row_count).computed(),
        Section::Signals => model.signals.clone().map(row_count).computed(),
    };

    text!("{count}").caption().foreground(MutedForeground)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "a signal's map receives the value by move"
)]
fn row_count<T>(rows: Vec<T>) -> Str {
    count_label(rows.len() as u64)
}

fn count_label(count: u64) -> Str {
    if count == 0 {
        Str::from("")
    } else if count >= 10_000 {
        Str::from(format!("{}k", count / 1000))
    } else {
        Str::from(count.to_string())
    }
}

/// Colour for a log severity.
pub fn level_color(level: LogLevel) -> Color {
    match level {
        LogLevel::Error => Color::from(Red),
        LogLevel::Warn => Color::from(Orange),
        LogLevel::Info => Color::from(Accent),
        LogLevel::Debug | LogLevel::Trace => Color::from(MutedForeground),
    }
}

/// Formats microseconds the way a developer reads them.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a duration large enough to lose f64 precision here would be centuries"
)]
pub fn format_us(micros: u64) -> Str {
    if micros >= 10_000 {
        Str::from(format!("{:.1}ms", micros as f64 / 1000.0))
    } else if micros >= 1_000 {
        Str::from(format!("{:.2}ms", micros as f64 / 1000.0))
    } else {
        Str::from(format!("{micros}µs"))
    }
}

/// What a section shows when it has nothing yet.
///
/// An empty pane should say why it is empty; a blank one is indistinguishable
/// from a broken one.
pub fn empty_state(message: impl Into<Str>) -> impl View {
    let message = message.into();
    vstack((text!("{message}").foreground(MutedForeground),)).padding_with(EdgeInsets::all(32.0))
}

/// A section heading with an explanatory subtitle.
pub fn section_header(title: impl Into<Str>, subtitle: impl Into<Str>) -> impl View {
    let title = title.into();
    let subtitle = subtitle.into();
    vstack((
        text!("{title}").title(),
        text!("{subtitle}").caption().foreground(MutedForeground),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(4.0)
}
