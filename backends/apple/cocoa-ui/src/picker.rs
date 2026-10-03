//! Types shared by the two platform pickers.
//!
//! The style decides which concrete control the picker wraps; the platforms
//! choose their closest primitive for it.

/// The presentation a [`Picker`] renders.
///
/// The mapping to platform controls is asymmetric on purpose: `UIKit` has
/// no radio-group primitive, so [`PickerStyle::Radio`] drives a
/// `UIPickerView` wheel there while `AppKit` builds a stack of radio
/// buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickerStyle {
    /// A pull-down menu — `NSPopUpButton` on `AppKit`, a `UIButton` with a
    /// `UIMenu` on `UIKit`.
    Menu,
    /// A single-choice group — a vertical stack of radio buttons on
    /// `AppKit`, a single-column `UIPickerView` wheel on `UIKit`.
    Radio,
    /// A row of labeled segments — `NSSegmentedControl` /
    /// `UISegmentedControl`.
    Segmented,
}
