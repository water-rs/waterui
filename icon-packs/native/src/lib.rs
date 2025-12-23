//! Cross-platform native icons for WaterUI.
//!
//! This crate provides a curated set of common icons that work across platforms:
//! - **Apple**: Renders as native SF Symbols
//! - **Android**: Falls back to placeholder icon (future: Material Icons mapping)
//!
//! # Usage
//!
//! ```ignore
//! use waterui_icons_native as icons;
//!
//! // Use icons directly
//! icons::HOME
//! icons::SETTINGS
//! icons::SEARCH
//! ```
//!
//! # Icon Names
//!
//! Icons use semantic names that map to platform-appropriate symbols:
//! - `HOME` → "house" (SF Symbol) / placeholder (Android)
//! - `SETTINGS` → "gear" (SF Symbol) / placeholder (Android)
//! - `SEARCH` → "magnifyingglass" (SF Symbol) / placeholder (Android)

#![no_std]

use waterui_icon::SystemIcon;

// ============================================================================
// Home & Navigation
// ============================================================================

/// Home icon.
pub const HOME: SystemIcon = SystemIcon::from_static("house");

/// Home filled icon.
pub const HOME_FILL: SystemIcon = SystemIcon::from_static("house.fill");

/// Settings/gear icon.
pub const SETTINGS: SystemIcon = SystemIcon::from_static("gear");

/// Settings shape icon.
pub const SETTINGS_SHAPE: SystemIcon = SystemIcon::from_static("gearshape");

/// Search/magnifying glass icon.
pub const SEARCH: SystemIcon = SystemIcon::from_static("magnifyingglass");

/// Back/chevron left icon.
pub const BACK: SystemIcon = SystemIcon::from_static("chevron.left");

/// Forward/chevron right icon.
pub const FORWARD: SystemIcon = SystemIcon::from_static("chevron.right");

/// Up/chevron up icon.
pub const UP: SystemIcon = SystemIcon::from_static("chevron.up");

/// Down/chevron down icon.
pub const DOWN: SystemIcon = SystemIcon::from_static("chevron.down");

/// Arrow back icon.
pub const ARROW_BACK: SystemIcon = SystemIcon::from_static("arrow.left");

/// Arrow forward icon.
pub const ARROW_FORWARD: SystemIcon = SystemIcon::from_static("arrow.right");

/// Arrow up icon.
pub const ARROW_UP: SystemIcon = SystemIcon::from_static("arrow.up");

/// Arrow down icon.
pub const ARROW_DOWN: SystemIcon = SystemIcon::from_static("arrow.down");

// ============================================================================
// Actions
// ============================================================================

/// Add/plus icon.
pub const ADD: SystemIcon = SystemIcon::from_static("plus");

/// Add circle icon.
pub const ADD_CIRCLE: SystemIcon = SystemIcon::from_static("plus.circle");

/// Add circle filled icon.
pub const ADD_CIRCLE_FILL: SystemIcon = SystemIcon::from_static("plus.circle.fill");

/// Remove/minus icon.
pub const REMOVE: SystemIcon = SystemIcon::from_static("minus");

/// Remove circle icon.
pub const REMOVE_CIRCLE: SystemIcon = SystemIcon::from_static("minus.circle");

/// Delete/trash icon.
pub const DELETE: SystemIcon = SystemIcon::from_static("trash");

/// Delete filled icon.
pub const DELETE_FILL: SystemIcon = SystemIcon::from_static("trash.fill");

/// Close/X icon.
pub const CLOSE: SystemIcon = SystemIcon::from_static("xmark");

/// Close circle icon.
pub const CLOSE_CIRCLE: SystemIcon = SystemIcon::from_static("xmark.circle");

/// Close circle filled icon.
pub const CLOSE_CIRCLE_FILL: SystemIcon = SystemIcon::from_static("xmark.circle.fill");

/// Check/checkmark icon.
pub const CHECK: SystemIcon = SystemIcon::from_static("checkmark");

/// Check circle icon.
pub const CHECK_CIRCLE: SystemIcon = SystemIcon::from_static("checkmark.circle");

/// Check circle filled icon.
pub const CHECK_CIRCLE_FILL: SystemIcon = SystemIcon::from_static("checkmark.circle.fill");

/// Edit/pencil icon.
pub const EDIT: SystemIcon = SystemIcon::from_static("pencil");

/// Compose/square and pencil icon.
pub const COMPOSE: SystemIcon = SystemIcon::from_static("square.and.pencil");

// ============================================================================
// User & Account
// ============================================================================

/// Person/user icon.
pub const PERSON: SystemIcon = SystemIcon::from_static("person");

/// Person filled icon.
pub const PERSON_FILL: SystemIcon = SystemIcon::from_static("person.fill");

/// Person circle icon.
pub const PERSON_CIRCLE: SystemIcon = SystemIcon::from_static("person.circle");

// ============================================================================
// Favorites & Ratings
// ============================================================================

/// Star icon.
pub const STAR: SystemIcon = SystemIcon::from_static("star");

/// Star filled icon.
pub const STAR_FILL: SystemIcon = SystemIcon::from_static("star.fill");

/// Heart icon.
pub const HEART: SystemIcon = SystemIcon::from_static("heart");

/// Heart filled icon.
pub const HEART_FILL: SystemIcon = SystemIcon::from_static("heart.fill");

// ============================================================================
// Media Playback
// ============================================================================

/// Play icon.
pub const PLAY: SystemIcon = SystemIcon::from_static("play");

/// Play filled icon.
pub const PLAY_FILL: SystemIcon = SystemIcon::from_static("play.fill");

/// Pause icon.
pub const PAUSE: SystemIcon = SystemIcon::from_static("pause");

/// Pause filled icon.
pub const PAUSE_FILL: SystemIcon = SystemIcon::from_static("pause.fill");

/// Stop icon.
pub const STOP: SystemIcon = SystemIcon::from_static("stop");

/// Stop filled icon.
pub const STOP_FILL: SystemIcon = SystemIcon::from_static("stop.fill");

// ============================================================================
// Communication
// ============================================================================

/// Email/envelope icon.
pub const EMAIL: SystemIcon = SystemIcon::from_static("envelope");

/// Email filled icon.
pub const EMAIL_FILL: SystemIcon = SystemIcon::from_static("envelope.fill");

/// Phone icon.
pub const PHONE: SystemIcon = SystemIcon::from_static("phone");

/// Phone filled icon.
pub const PHONE_FILL: SystemIcon = SystemIcon::from_static("phone.fill");

/// Message/chat bubble icon.
pub const MESSAGE: SystemIcon = SystemIcon::from_static("message");

/// Message filled icon.
pub const MESSAGE_FILL: SystemIcon = SystemIcon::from_static("message.fill");

/// Bell/notification icon.
pub const NOTIFICATION: SystemIcon = SystemIcon::from_static("bell");

/// Bell filled icon.
pub const NOTIFICATION_FILL: SystemIcon = SystemIcon::from_static("bell.fill");

// ============================================================================
// Sharing & Links
// ============================================================================

/// Share icon.
pub const SHARE: SystemIcon = SystemIcon::from_static("square.and.arrow.up");

/// Download icon.
pub const DOWNLOAD: SystemIcon = SystemIcon::from_static("square.and.arrow.down");

/// Link icon.
pub const LINK: SystemIcon = SystemIcon::from_static("link");

// ============================================================================
// System & Status
// ============================================================================

/// Info icon.
pub const INFO: SystemIcon = SystemIcon::from_static("info.circle");

/// Info filled icon.
pub const INFO_FILL: SystemIcon = SystemIcon::from_static("info.circle.fill");

/// Help/question mark icon.
pub const HELP: SystemIcon = SystemIcon::from_static("questionmark.circle");

/// Warning/exclamation mark icon.
pub const WARNING: SystemIcon = SystemIcon::from_static("exclamationmark.circle");

/// Error/exclamation triangle icon.
pub const ERROR: SystemIcon = SystemIcon::from_static("exclamationmark.triangle");

// ============================================================================
// Menu & Layout
// ============================================================================

/// Menu/hamburger icon.
pub const MENU: SystemIcon = SystemIcon::from_static("line.3.horizontal");

/// More/ellipsis icon.
pub const MORE: SystemIcon = SystemIcon::from_static("ellipsis");

/// More circle icon.
pub const MORE_CIRCLE: SystemIcon = SystemIcon::from_static("ellipsis.circle");

/// Filter/sliders icon.
pub const FILTER: SystemIcon = SystemIcon::from_static("slider.horizontal.3");
