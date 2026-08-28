//! Apple system icons for `WaterUI`.
//!
//! This crate provides curated SF Symbol names for Apple platforms. `SystemIcon`
//! is intentionally unsupported on Android, Linux, Web, terminal, and self-drawn
//! backends because those platforms do not expose Apple's system icon catalog.
//! Portable applications must use a packaged icon crate.
//!
//! Function-based entry points match the shape of the `lucide`,
//! `material-icon`, and `sf-symbol` packs.
//!
//! # Usage
//!
//! ```ignore
//! use waterui_icons_native as icons;
//!
//! icons::home()
//! icons::settings()
//! icons::search()
//! ```

#![no_std]

use waterui_icon::SystemIcon;

macro_rules! native_icons {
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

native_icons! {
    // ========================================================================
    // Home & Navigation
    // ========================================================================
    /// Home icon.
    home => "house",
    /// Home filled icon.
    home_fill => "house.fill",
    /// Settings/gear icon.
    settings => "gear",
    /// Settings shape icon.
    settings_shape => "gearshape",
    /// Search/magnifying glass icon.
    search => "magnifyingglass",
    /// Back/chevron left icon.
    back => "chevron.left",
    /// Forward/chevron right icon.
    forward => "chevron.right",
    /// Up/chevron up icon.
    up => "chevron.up",
    /// Down/chevron down icon.
    down => "chevron.down",
    /// Arrow back icon.
    arrow_back => "arrow.left",
    /// Arrow forward icon.
    arrow_forward => "arrow.right",
    /// Arrow up icon.
    arrow_up => "arrow.up",
    /// Arrow down icon.
    arrow_down => "arrow.down",

    // ========================================================================
    // Actions
    // ========================================================================
    /// Add/plus icon.
    add => "plus",
    /// Add circle icon.
    add_circle => "plus.circle",
    /// Add circle filled icon.
    add_circle_fill => "plus.circle.fill",
    /// Remove/minus icon.
    remove => "minus",
    /// Remove circle icon.
    remove_circle => "minus.circle",
    /// Delete/trash icon.
    delete => "trash",
    /// Delete filled icon.
    delete_fill => "trash.fill",
    /// Close/X icon.
    close => "xmark",
    /// Close circle icon.
    close_circle => "xmark.circle",
    /// Close circle filled icon.
    close_circle_fill => "xmark.circle.fill",
    /// Check/checkmark icon.
    check => "checkmark",
    /// Check circle icon.
    check_circle => "checkmark.circle",
    /// Check circle filled icon.
    check_circle_fill => "checkmark.circle.fill",
    /// Edit/pencil icon.
    edit => "pencil",
    /// Compose/square and pencil icon.
    compose => "square.and.pencil",

    // ========================================================================
    // User & Account
    // ========================================================================
    /// Person/user icon.
    person => "person",
    /// Person filled icon.
    person_fill => "person.fill",
    /// Person circle icon.
    person_circle => "person.circle",

    // ========================================================================
    // Favorites & Ratings
    // ========================================================================
    /// Star icon.
    star => "star",
    /// Star filled icon.
    star_fill => "star.fill",
    /// Heart icon.
    heart => "heart",
    /// Heart filled icon.
    heart_fill => "heart.fill",

    // ========================================================================
    // Media Playback
    // ========================================================================
    /// Play icon.
    play => "play",
    /// Play filled icon.
    play_fill => "play.fill",
    /// Pause icon.
    pause => "pause",
    /// Pause filled icon.
    pause_fill => "pause.fill",
    /// Stop icon.
    stop => "stop",
    /// Stop filled icon.
    stop_fill => "stop.fill",

    // ========================================================================
    // Communication
    // ========================================================================
    /// Email/envelope icon.
    email => "envelope",
    /// Email filled icon.
    email_fill => "envelope.fill",
    /// Phone icon.
    phone => "phone",
    /// Phone filled icon.
    phone_fill => "phone.fill",
    /// Message/chat bubble icon.
    message => "message",
    /// Message filled icon.
    message_fill => "message.fill",
    /// Bell/notification icon.
    notification => "bell",
    /// Bell filled icon.
    notification_fill => "bell.fill",

    // ========================================================================
    // Sharing & Links
    // ========================================================================
    /// Share icon.
    share => "square.and.arrow.up",
    /// Download icon.
    download => "square.and.arrow.down",
    /// Link icon.
    link => "link",

    // ========================================================================
    // System & Status
    // ========================================================================
    /// Info icon.
    info => "info.circle",
    /// Info filled icon.
    info_fill => "info.circle.fill",
    /// Help/question mark icon.
    help => "questionmark.circle",
    /// Warning/exclamation mark icon.
    warning => "exclamationmark.circle",
    /// Error/exclamation triangle icon.
    error => "exclamationmark.triangle",

    // ========================================================================
    // Menu & Layout
    // ========================================================================
    /// Menu/hamburger icon.
    menu => "line.3.horizontal",
    /// More/ellipsis icon.
    more => "ellipsis",
    /// More circle icon.
    more_circle => "ellipsis.circle",
    /// Filter/sliders icon.
    filter => "slider.horizontal.3",
}
