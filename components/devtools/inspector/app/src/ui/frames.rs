//! The frame timeline.
//!
//! One line per frame's wall time against the frame budget. This is the fastest
//! way to see whether a hitch is layout, encoding, or presentation, and whether
//! the renderer is skipping work it should be doing.

use waterui::chart::{DataPoint, LineChart};
use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::widget::condition::when;
use waterui_inspector_protocol::{FrameKind, FrameSample};

use crate::model::Model;

use super::parts;

/// The frames pane.
pub fn view(model: Model) -> impl View {
    let has_frames = model.frames_seen.gt(0);

    vstack((
        parts::section_header(
            "Frame timeline",
            "Wall time per frame, newest at the right.",
        ),
        when(has_frames, {
            move || {
                let model = model.clone();
                // The chart is greedy: a `LineChart` placed above its siblings
                // in a stack leaves them no height and they draw on top of each
                // other. Keeping it last confines that to the bottom of the
                // pane, where there is nothing after it to starve.
                vstack((
                    breakdown(&model.last_frame),
                    Divider,
                    chart(model.frame_series),
                ))
                .spacing(16.0)
            }
        })
        .otherwise(|| {
            parts::empty_state("No frames yet. The target draws only when something changes.")
        }),
        spacer(),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(16.0)
    .padding_with(EdgeInsets::all(20.0))
}

fn chart(frame_series: Binding<Vec<DataPoint>>) -> impl View {
    LineChart::new(frame_series)
        .line_width(1.5)
        .fill(0.15)
        .height(220.0)
}

/// What the most recent frame actually spent its time on.
fn breakdown(last_frame: &Binding<Option<FrameSample>>) -> impl View + use<> {
    let frame = last_frame.clone();

    vstack((
        hstack((
            parts::metric("Kind", field(&frame, |sample| Str::from(sample.kind.name()))),
            parts::metric(
                "Total",
                field(&frame, |sample| {
                    parts::format_us(u64::from(sample.total_us))
                }),
            ),
            parts::metric(
                "Budget",
                field(&frame, |sample| {
                    sample.budget_usage().map_or_else(
                        || Str::from("—"),
                        |usage| Str::from(format!("{:.0}%", usage * 100.0)),
                    )
                }),
            ),
            spacer(),
        ))
        .spacing(32.0)
        .height(parts::METRIC_HEIGHT),
        hstack((
            parts::metric(
                "Layout",
                field(&frame, |sample| {
                    parts::format_us(u64::from(sample.layout_us))
                }),
            ),
            parts::metric(
                "Encode",
                field(&frame, |sample| {
                    parts::format_us(u64::from(sample.encode_us))
                }),
            ),
            parts::metric(
                "Present",
                field(&frame, |sample| {
                    parts::format_us(u64::from(sample.present_us))
                }),
            ),
            spacer(),
        ))
        .spacing(32.0)
        .height(parts::METRIC_HEIGHT),
        hstack((
            parts::metric(
                "Layers",
                field(&frame, |sample| Str::from(sample.layers.to_string())),
            ),
            parts::metric(
                "Clip depth",
                field(&frame, |sample| Str::from(sample.clip_depth.to_string())),
            ),
            parts::metric(
                "Refresh",
                field(&frame, |sample| {
                    Str::from(format!("{:.0} Hz", sample.refresh_hz))
                }),
            ),
            spacer(),
        ))
        .spacing(32.0)
        .height(parts::METRIC_HEIGHT),
        idle_note(last_frame),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(16.0)
}

/// Explains an idle frame rather than leaving a row of zeroes unexplained.
fn idle_note(last_frame: &Binding<Option<FrameSample>>) -> impl View + use<> {
    let idle = last_frame
        .map(|frame| frame.is_some_and(|sample| sample.kind == FrameKind::Idle))
        .computed();

    when(idle, || {
        text("This frame did no work — the window had nothing new to show.")
            .caption()
            .foreground(MutedForeground)
    })
    .otherwise(|| ())
}

/// Projects one field of the latest frame into displayable text.
fn field(
    frame: &Binding<Option<FrameSample>>,
    project: impl Fn(&FrameSample) -> Str + Clone + 'static,
) -> Computed<Str> {
    frame
        .clone()
        .map(move |sample| {
            sample
                .as_ref()
                .map_or_else(|| Str::from("—"), &project)
        })
        .computed()
}
