//! Shared types for the badge indicator views in `appkit` and `uikit`.

/// The presentation carried by a native badge indicator.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BadgeDisplay {
    /// No indicator is visible.
    #[default]
    Hidden,
    /// A small indicator dot without a label.
    Dot,
    /// A capsule pill displaying `text`.
    Text(String),
}

/// The geometry a badge indicator draws at: the dot, the capsule carrying text,
/// and the offsets the consumer uses to place the indicator over its content.
///
/// The values are the caller's chrome choice — the view only draws and
/// measures what it is given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BadgeMetrics {
    /// The dot diameter.
    pub dot_size: f64,
    /// The capsule's fixed height; the width grows with the text.
    pub capsule_height: f64,
    /// The capsule's horizontal inset around the label text.
    pub capsule_horizontal_padding: f64,
    /// The label's point size.
    pub capsule_font_size: f64,
    /// The inset from the content's trailing edge to a pill indicator's
    /// leading edge.
    pub count_horizontal_offset: f64,
    /// The distance below the content's top edge a pill indicator's top
    /// sits.
    pub count_vertical_offset: f64,
}
