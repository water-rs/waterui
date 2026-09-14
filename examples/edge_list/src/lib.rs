//! Edge List Example - variable-height rows in a 200,000-row lazy list.
//!
//! Unlike `examples/list` (uniform rows), every row's height depends on its
//! index: rows carry one to three text lines plus a colored chip, exercising
//! lazy realization, measurement, and recycling of non-uniform cells.

use waterui::Identifiable;
use waterui::app::App;
use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::collection::List as ReactiveList;
use waterui::shape::{Circle, ShapeExt};

const ROW_COUNT: u64 = 200_000;

#[derive(Clone, Copy, Identifiable)]
struct Row {
    #[id]
    id: u64,
}

fn chip_color(index: u64) -> Color {
    match index % 4 {
        0 => Color::srgb_hex("#3B82F6"),
        1 => Color::srgb_hex("#10B981"),
        2 => Color::srgb_hex("#F59E0B"),
        _ => Color::srgb_hex("#EF4444"),
    }
}

fn detail_line(n: u64, label: &'static str) -> impl View {
    text(format!("detail line {n} {label}"))
        .caption()
        .foreground(MutedForeground)
}

fn record_row(index: u64) -> ListItem {
    let lines = index % 3 + 1;
    let header = hstack((
        Circle.fill(chip_color(index)).size(10.0, 10.0),
        text(format!("Row #{index} - {lines} detail line(s)")).sub_headline(),
    ))
    .spacing(8.0)
    .alignment(VerticalAlignment::Center);

    let content = match lines {
        1 => AnyView::new(vstack((header, detail_line(1, "always present"))).alignment(HorizontalAlignment::Leading)),
        2 => AnyView::new(vstack((
            header,
            detail_line(1, "always present"),
            detail_line(2, "makes this row taller"),
        ))
        .alignment(HorizontalAlignment::Leading)),
        _ => AnyView::new(vstack((
            header,
            detail_line(1, "always present"),
            detail_line(2, "makes this row taller"),
            detail_line(3, "tallest variant"),
        ))
        .alignment(HorizontalAlignment::Leading)),
    };

    ListItem::new(
        content.padding_with(EdgeInsets::symmetric(8.0, 16.0)),
    )
}

#[preview]
pub fn demo() -> impl View {
    let records =
        ReactiveList::from((0..ROW_COUNT).map(|id| Row { id }).collect::<Vec<_>>());
    let list = List::for_each(records, |row| record_row(row.id));

    vstack((
        vstack((
            text("200,000 variable-height rows").title(),
            text("Rows cycle 1-3 detail lines; only viewport rows materialize.")
                .sub_headline()
                .foreground(MutedForeground),
        ))
        .alignment(HorizontalAlignment::Leading)
        .padding(),
        Divider,
        list,
    ))
    .alignment(HorizontalAlignment::Leading)
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
