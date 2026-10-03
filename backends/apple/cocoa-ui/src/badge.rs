//! Shared types for the badge indicator views in `appkit` and `uikit`.

/// The geometry a badge indicator draws at: the dot a zero count shows,
/// the capsule a count carries and the offsets the consumer uses to place
/// the indicator over its content.
///
/// The values are the caller's chrome choice — the view only draws and
/// measures what it is given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BadgeMetrics {
    /// The dot diameter a zero count draws.
    pub dot_size: f64,
    /// The capsule's fixed height; the width grows with the count.
    pub capsule_height: f64,
    /// The capsule's horizontal inset around the count text.
    pub capsule_horizontal_padding: f64,
    /// The count label's point size.
    pub capsule_font_size: f64,
    /// The inset from the content's trailing edge to a non-zero indicator's
    /// leading edge.
    pub count_horizontal_offset: f64,
    /// The distance below the content's top edge a non-zero indicator's top
    /// sits.
    pub count_vertical_offset: f64,
}
