//! Types shared by the two platform pickers.
//!
//! The style decides which concrete control the picker wraps; the platforms
//! choose their closest primitive for it.

/// The presentation a `Picker` renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickerStyle {
    /// A pull-down menu — `NSPopUpButton` on `AppKit`, a `UIButton` with a
    /// `UIMenu` on `UIKit`.
    Menu,
    /// A single-choice radio group — a vertical stack of native radio buttons
    /// on both `AppKit` and `UIKit`.
    Radio,
    /// A row of labeled segments — `NSSegmentedControl` /
    /// `UISegmentedControl`.
    Segmented,
    /// A scrollable vertical wheel drum — a native drum composition on
    /// `AppKit`, a single-column `UIPickerView` on `UIKit`.
    Wheel,
}
