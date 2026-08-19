//! The accessibility tree.
//!
//! This is the view hierarchy as `WaterUI` actually presents it — the same tree
//! assistive technology sees, and the same one `waterui-testing` asserts
//! against. A node missing a label here is missing it for a screen reader too,
//! which makes this pane an accessibility check as much as a layout one.

use waterui::prelude::theme_color::{Accent, MutedForeground};
use waterui::prelude::*;
use waterui::reactive::collection::SignalCollection;
use waterui::widget::condition::when;

use crate::model::{Model, TreeRow};

use super::parts;

/// The tree pane.
pub fn view(model: Model) -> impl View {
    let filter = Binding::container(Str::from(""));
    let empty = model.tree.map(|rows| rows.is_empty()).computed();

    vstack((
        vstack((
            parts::section_header(
                "View tree",
                "The accessibility tree, as assistive technology sees it.",
            ),
            TextField::new(&filter).prompt("Filter by label or role"),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(8.0)
        .padding_with(EdgeInsets::all(16.0)),
        Divider,
        when(empty, || {
            parts::empty_state("No tree yet. Open the tree channel, then interact with the target.")
        })
        .otherwise({
            let rows = rows(&model, &filter);
            let revealed = model.revealed;
            move || {
                let revealed = revealed.clone();
                List::for_each(rows.clone(), move |row: TreeRow| {
                    let id = row.id;
                    let is_revealed = revealed
                        .clone()
                        .map(move |node| node == Some(id))
                        .computed();
                    row_view(row, is_revealed)
                })
            }
        }),
    ))
    .alignment(HorizontalAlignment::Leading)
}

/// The rows to display, filtered reactively.
///
/// Filtering derives a new collection from the source and the query, so typing
/// updates the list without disturbing the connection or the tree itself.
fn rows(model: &Model, filter: &Binding<Str>) -> SignalCollection<Computed<Vec<TreeRow>>> {
    SignalCollection::new(
        model
            .tree
            .clone()
            .zip(filter)
            .map(|(rows, query)| {
                if query.is_empty() {
                    return rows;
                }
                let query = query.to_lowercase();
                rows.into_iter()
                    .filter(|row| {
                        // `SignalExt::contains` shadows the inherent `str`
                        // method here, so call it by path.
                        str::contains(&row.role.to_lowercase(), query.as_str())
                            || row.label.as_ref().is_some_and(|label| {
                                str::contains(&label.to_lowercase(), query.as_str())
                            })
                    })
                    .collect()
            })
            .computed(),
    )
}

/// One node: indented by depth, with its role, label, and geometry.
///
/// A node the application asked to inspect is marked, so that opening the
/// inspector on an element lands the eye on that element rather than on the
/// top of a long tree.
fn row_view(row: TreeRow, revealed: Computed<bool>) -> ListItem {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a view tree deep enough to lose f32 precision could not be rendered"
    )]
    let indent = (row.depth as f32) * 14.0;

    let content = hstack((
        spacer().width(indent),
        vstack((
            hstack((
                text(row.role).sub_headline().foreground(Accent),
                label_view(row.label),
                hidden_badge(row.hidden),
                revealed_badge(revealed),
            ))
            .spacing(8.0),
            detail_view(row.value, row.bounds),
        ))
        .alignment(HorizontalAlignment::Leading)
        .spacing(2.0),
    ))
    .padding_with(EdgeInsets::new(6.0, 6.0, 12.0, 12.0))
    .opacity(if row.enabled { 1.0 } else { 0.55 });

    ListItem::new(content)
}

/// A node without a label is called out rather than rendered as blank space —
/// an unlabelled control is invisible to a screen reader.
fn label_view(label: Option<Str>) -> AnyView {
    label.map_or_else(
        || text("(no label)").caption().foreground(Orange).anyview(),
        |label| text(label).anyview(),
    )
}

/// Marks the node the application asked to inspect.
///
/// Reactive rather than baked in, so a second "inspect element" moves the mark
/// instead of rebuilding the tree.
fn revealed_badge(revealed: Computed<bool>) -> AnyView {
    when(revealed, || text("inspected").caption().foreground(Accent)).anyview()
}

fn hidden_badge(hidden: bool) -> AnyView {
    if hidden {
        text("hidden")
            .caption()
            .foreground(MutedForeground)
            .anyview()
    } else {
        ().anyview()
    }
}

fn detail_view(value: Option<Str>, bounds: Option<Str>) -> AnyView {
    let mut parts: Vec<Str> = Vec::new();
    if let Some(value) = value {
        parts.push(Str::from(format!("value {value}")));
    }
    if let Some(bounds) = bounds {
        parts.push(bounds);
    }
    if parts.is_empty() {
        return ().anyview();
    }
    text(Str::from(parts.join("   ")))
        .caption()
        .foreground(MutedForeground)
        .anyview()
}
