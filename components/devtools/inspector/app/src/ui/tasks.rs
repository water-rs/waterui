//! Main-thread occupancy, and the stalls worth explaining.
//!
//! The table answers "what is running"; the stall list answers "what blocked the
//! frame". A stall without a stack is normal — capturing one costs milliseconds
//! on the thread being measured, so the target rations them.

use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::widget::condition::when;
use waterui::reactive::collection::SignalCollection;

use crate::model::{Model, StallRow, TaskRow};

use super::parts;

/// The tasks pane.
pub fn view(model: Model) -> impl View {
    let no_tasks = model.tasks.map(|rows| rows.is_empty()).computed();
    let no_stalls = model.stalls.map(|rows| rows.is_empty()).computed();

    scroll(
        vstack((
            parts::section_header(
                "Main thread",
                "Poll occupancy per task type, aggregated by the target.",
            ),
            when(no_tasks, || parts::empty_state("Nothing polled yet.")).otherwise({
                let tasks = model.tasks.clone();
                move || {
                    vstack((
                        task_header(),
                        List::for_each(SignalCollection::new(tasks.clone()), task_row)
                            .height(TASK_TABLE_HEIGHT),
                    ))
                    .alignment(HorizontalAlignment::Leading)
                }
            }),
            Divider,
            parts::section_header("Stalls", "Polls that overran the frame budget."),
            when(no_stalls, || {
                parts::empty_state("No stalls. The main thread kept up.")
            })
            .otherwise({
                let stalls = model.stalls;
                move || List::for_each(SignalCollection::new(stalls.clone()), stall_row)
            }),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(16.0)
        .padding_with(EdgeInsets::all(20.0)),
    )
}

fn task_header() -> impl View {
    hstack((
        text("Task").caption().foreground(MutedForeground),
        spacer(),
        column(text("polls")),
        column(text("mean")),
        column(text("max")),
        column(text("over")),
    ))
    .spacing(8.0)
    .height(ROW_HEIGHT)
}

fn task_row(row: TaskRow) -> ListItem {
    let over = row.over_budget;

    ListItem::new(
        hstack((
        text(row.id),
        spacer(),
        column(text(Str::from(row.polls.to_string()))),
        column(text(parts::format_us(row.mean_us))),
        column(text(parts::format_us(u64::from(row.max_us)))),
        column(text(Str::from(over.to_string())).foreground(if over > 0 {
            Color::from(Orange)
        } else {
            Color::from(MutedForeground)
        })),
        ))
        .spacing(8.0)
        .height(ROW_HEIGHT)
        .padding_with(EdgeInsets::new(6.0, 6.0, 0.0, 0.0)),
    )
}

/// Height of one table row. Explicit, because a row left to size itself is
/// allocated nothing when it shares a stack with a greedy sibling.
const ROW_HEIGHT: f32 = 32.0;

/// Height reserved for the task table.
///
/// A handful of task types is the normal case; the list scrolls beyond that.
const TASK_TABLE_HEIGHT: f32 = 200.0;

/// Fixed-width numeric column, so the table reads as a table.
fn column(content: impl View) -> impl View {
    hstack((spacer(), content)).width(84.0)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "a list row builder is handed ownership of its item"
)]
fn stall_row(row: StallRow) -> ListItem {
    let summary = Str::from(format!(
        "{} · {:.0}% of budget · {}",
        row.task_type,
        row.usage_pct,
        parts::format_us(row.wall_us)
    ));

    ListItem::new(
        vstack((
            text(summary).sub_headline().foreground(Orange),
            backtrace_view(&row.backtrace),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(4.0)
        .padding_with(EdgeInsets::new(8.0, 8.0, 0.0, 0.0)),
    )
}

/// The captured stack, or an explanation of why there isn't one.
fn backtrace_view(backtrace: &[Str]) -> AnyView {
    if backtrace.is_empty() {
        return text("stack not captured — rate-limited to avoid perturbing the target")
            .caption()
            .foreground(MutedForeground)
            .anyview();
    }

    backtrace
        .iter()
        .map(|frame| {
            text(frame.clone())
                .caption()
                .foreground(MutedForeground)
                .anyview()
        })
        .collect::<VStack<_>>()
        .alignment(HorizontalAlignment::Leading)
        .anyview()
}
