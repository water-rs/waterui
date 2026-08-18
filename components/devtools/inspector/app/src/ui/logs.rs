//! Application logs.
//!
//! The same records `water run --logs debug` prints, so a developer does not
//! have to choose between watching a terminal and watching the app.

use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::reactive::collection::SignalCollection;
use waterui::widget::condition::when;
use waterui_inspector_protocol::LogLevel;

use crate::model::{LogRow, Model};

use super::parts;

/// The logs pane.
#[expect(
    clippy::needless_pass_by_value,
    reason = "view builders take owned reactive handles: the views they return capture them for 'static, so a borrow would not do"
)]
pub fn view(model: Model) -> impl View {
    let filter = Binding::container(Str::from(""));
    let minimum = Binding::container(LogLevel::Debug);
    let rows = visible_rows(&model, &filter, &minimum);
    let empty = rows.map(|rows| rows.is_empty()).computed();

    vstack((
        vstack((
            parts::section_header("Logs", "Records emitted by the target application."),
            hstack((
                TextField::new(&filter).prompt("Filter"),
                level_picker(&minimum),
            ))
            .spacing(12.0),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(8.0)
        .padding_with(EdgeInsets::all(16.0)),
        Divider,
        when(empty, || {
            parts::empty_state("Nothing logged at this level yet.")
        })
        .otherwise(move || List::for_each(SignalCollection::new(rows.clone()), row_view)),
    ))
    .alignment(HorizontalAlignment::Leading)
}

/// Rows surviving the text filter and the minimum severity.
///
/// Both inputs are signals, so the list re-derives itself as they change rather
/// than being rebuilt by the view.
fn visible_rows(
    model: &Model,
    filter: &Binding<Str>,
    minimum: &Binding<LogLevel>,
) -> Computed<Vec<LogRow>> {
    model
        .logs
        .clone()
        .zip(&filter.clone().zip(minimum))
        .map(|(rows, (query, minimum))| {
            let query = query.to_lowercase();
            rows.into_iter()
                .filter(|row| row.level >= minimum)
                .filter(|row| {
                    query.is_empty()
                        || str::contains(&row.message.to_lowercase(), query.as_str())
                        || str::contains(&row.target.to_lowercase(), query.as_str())
                })
                .collect()
        })
        .computed()
}

fn level_picker(minimum: &Binding<LogLevel>) -> impl View {
    let levels = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];

    levels
        .iter()
        .map(|level| {
            let level = *level;
            let selected = minimum.clone().map(move |current| current == level);
            button(text(level.name()))
                .action(move |State(minimum): State<Binding<LogLevel>>| minimum.set(level))
                .state(minimum)
                .opacity(selected.select(1.0, 0.5))
                .anyview()
        })
        .collect::<HStack<_>>()
        .spacing(4.0)
}

fn row_view(row: LogRow) -> ListItem {
    ListItem::new(
        hstack((
            text(row.level.name())
                .caption()
                .foreground(parts::level_color(row.level))
                .width(52.0),
            vstack((text(row.message), origin_view(row.target, row.origin)))
                .alignment(HorizontalAlignment::Leading)
                .spacing(2.0),
        ))
        .spacing(10.0)
        .padding_with(EdgeInsets::new(6.0, 6.0, 12.0, 12.0)),
    )
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the caller has just destructured its row and has nothing left to borrow from"
)]
fn origin_view(target: Str, origin: Option<Str>) -> impl View {
    let line = origin.map_or_else(
        || target.clone(),
        |origin| Str::from(format!("{target}  {origin}")),
    );

    text(line).caption().foreground(MutedForeground)
}
