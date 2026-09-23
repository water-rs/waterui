//! SF Symbol icons for `WaterUI`.
//!
//! This crate exposes [`SystemIcon`] constructors for SF Symbols. SF Symbols
//! are Apple's icon system available on iOS, macOS, watchOS, and tvOS.
//!
//! Function-based entry points are used here to match the shape of the
//! `lucide` and `material-icon` icon packs.
//!
//! # Usage
//!
//! ```ignore
//! use waterui_icons_sf_symbol as sf;
//!
//! sf::house()
//! sf::gear()
//! sf::chevron_right()
//! ```
//!
//! # Platform Support
//!
//! - **Apple**: renders as native SF Symbols.
//! - **Other platforms**: the resulting [`SystemIcon`] falls back to a
//!   placeholder rendering chosen by the active backend.

#![no_std]

use waterui_icon::SystemIcon;

macro_rules! sf_icons {
    ($($(#[$meta:meta])* $name:ident => $sf:literal,)*) => {
        $(
            $(#[$meta])*
            #[must_use]
            pub const fn $name() -> SystemIcon {
                SystemIcon::from_static($sf)
            }
        )*
    };
}

sf_icons! {
    // ========================================================================
    // General Icons
    // ========================================================================
    /// House icon (`"house"`).
    house => "house",
    /// House filled icon (`"house.fill"`).
    house_fill => "house.fill",
    /// Gear/settings icon (`"gear"`).
    gear => "gear",
    /// Gear badge icon (`"gearshape"`).
    gearshape => "gearshape",
    /// Magnifying glass/search icon (`"magnifyingglass"`).
    magnifyingglass => "magnifyingglass",
    /// Person icon (`"person"`).
    person => "person",
    /// Person filled icon (`"person.fill"`).
    person_fill => "person.fill",
    /// Person circle icon (`"person.circle"`).
    person_circle => "person.circle",
    /// Plus icon (`"plus"`).
    plus => "plus",
    /// Plus circle icon (`"plus.circle"`).
    plus_circle => "plus.circle",
    /// Plus circle filled icon (`"plus.circle.fill"`).
    plus_circle_fill => "plus.circle.fill",
    /// Minus icon (`"minus"`).
    minus => "minus",
    /// Minus circle icon (`"minus.circle"`).
    minus_circle => "minus.circle",
    /// Trash icon (`"trash"`).
    trash => "trash",
    /// Trash filled icon (`"trash.fill"`).
    trash_fill => "trash.fill",
    /// Xmark/close icon (`"xmark"`).
    xmark => "xmark",
    /// Xmark circle icon (`"xmark.circle"`).
    xmark_circle => "xmark.circle",
    /// Xmark circle filled icon (`"xmark.circle.fill"`).
    xmark_circle_fill => "xmark.circle.fill",
    /// Checkmark icon (`"checkmark"`).
    checkmark => "checkmark",
    /// Checkmark circle icon (`"checkmark.circle"`).
    checkmark_circle => "checkmark.circle",
    /// Checkmark circle filled icon (`"checkmark.circle.fill"`).
    checkmark_circle_fill => "checkmark.circle.fill",

    // ========================================================================
    // Navigation Icons
    // ========================================================================
    /// Chevron right icon (`"chevron.right"`).
    chevron_right => "chevron.right",
    /// Chevron left icon (`"chevron.left"`).
    chevron_left => "chevron.left",
    /// Chevron up icon (`"chevron.up"`).
    chevron_up => "chevron.up",
    /// Chevron down icon (`"chevron.down"`).
    chevron_down => "chevron.down",
    /// Arrow right icon (`"arrow.right"`).
    arrow_right => "arrow.right",
    /// Arrow left icon (`"arrow.left"`).
    arrow_left => "arrow.left",
    /// Arrow up icon (`"arrow.up"`).
    arrow_up => "arrow.up",
    /// Arrow down icon (`"arrow.down"`).
    arrow_down => "arrow.down",

    // ========================================================================
    // Media Icons
    // ========================================================================
    /// Star icon (`"star"`).
    star => "star",
    /// Star filled icon (`"star.fill"`).
    star_fill => "star.fill",
    /// Heart icon (`"heart"`).
    heart => "heart",
    /// Heart filled icon (`"heart.fill"`).
    heart_fill => "heart.fill",
    /// Bell icon (`"bell"`).
    bell => "bell",
    /// Bell filled icon (`"bell.fill"`).
    bell_fill => "bell.fill",
    /// Play icon (`"play"`).
    play => "play",
    /// Play filled icon (`"play.fill"`).
    play_fill => "play.fill",
    /// Pause icon (`"pause"`).
    pause => "pause",
    /// Pause filled icon (`"pause.fill"`).
    pause_fill => "pause.fill",
    /// Stop icon (`"stop"`).
    stop => "stop",
    /// Stop filled icon (`"stop.fill"`).
    stop_fill => "stop.fill",

    // ========================================================================
    // Communication Icons
    // ========================================================================
    /// Envelope/mail icon (`"envelope"`).
    envelope => "envelope",
    /// Envelope filled icon (`"envelope.fill"`).
    envelope_fill => "envelope.fill",
    /// Phone icon (`"phone"`).
    phone => "phone",
    /// Phone filled icon (`"phone.fill"`).
    phone_fill => "phone.fill",
    /// Message/bubble icon (`"message"`).
    message => "message",
    /// Message filled icon (`"message.fill"`).
    message_fill => "message.fill",

    // ========================================================================
    // Editing Icons
    // ========================================================================
    /// Pencil icon (`"pencil"`).
    pencil => "pencil",
    /// Square and pencil icon (`"square.and.pencil"`).
    square_and_pencil => "square.and.pencil",
    /// Slider horizontal icon (`"slider.horizontal.3"`).
    slider_horizontal_3 => "slider.horizontal.3",

    // ========================================================================
    // Sharing Icons
    // ========================================================================
    /// Square and arrow up/share icon (`"square.and.arrow.up"`).
    square_and_arrow_up => "square.and.arrow.up",
    /// Square and arrow down/download icon (`"square.and.arrow.down"`).
    square_and_arrow_down => "square.and.arrow.down",
    /// Link icon (`"link"`).
    link => "link",

    // ========================================================================
    // System Icons
    // ========================================================================
    /// Info circle icon (`"info.circle"`).
    info_circle => "info.circle",
    /// Info circle filled icon (`"info.circle.fill"`).
    info_circle_fill => "info.circle.fill",
    /// Question mark circle icon (`"questionmark.circle"`).
    questionmark_circle => "questionmark.circle",
    /// Exclamation mark circle icon (`"exclamationmark.circle"`).
    exclamationmark_circle => "exclamationmark.circle",
    /// Exclamation mark triangle/warning icon (`"exclamationmark.triangle"`).
    exclamationmark_triangle => "exclamationmark.triangle",

    // ========================================================================
    // Menu Icons
    // ========================================================================
    /// Three horizontal lines/menu icon (`"line.3.horizontal"`).
    line_3_horizontal => "line.3.horizontal",
    /// Ellipsis icon (`"ellipsis"`).
    ellipsis => "ellipsis",
    /// Ellipsis circle icon (`"ellipsis.circle"`).
    ellipsis_circle => "ellipsis.circle",
}
