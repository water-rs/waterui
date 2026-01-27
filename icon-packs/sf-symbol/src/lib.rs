//! SF Symbol icons for WaterUI.
//!
//! This crate provides const `SystemIcon` definitions for SF Symbols.
//! SF Symbols are Apple's icon system available on iOS, macOS, watchOS, and tvOS.
//!
//! # Usage
//!
//! ```ignore
//! use waterui_icons_sf_symbol as sf;
//!
//! // Use icons directly
//! sf::HOUSE
//! sf::GEAR
//! sf::CHEVRON_RIGHT
//! ```
//!
//! # Platform Support
//!
//! - **Apple**: Renders as native SF Symbols
//! - **Android**: Falls back to placeholder icon (SF Symbols not available)

#![no_std]

use waterui_icon::SystemIcon;

// ============================================================================
// General Icons
// ============================================================================

/// House icon ("house").
pub const HOUSE: SystemIcon = SystemIcon::from_static("house");

/// House filled icon ("house.fill").
pub const HOUSE_FILL: SystemIcon = SystemIcon::from_static("house.fill");

/// Gear/settings icon ("gear").
pub const GEAR: SystemIcon = SystemIcon::from_static("gear");

/// Gear badge icon ("gearshape").
pub const GEARSHAPE: SystemIcon = SystemIcon::from_static("gearshape");

/// Magnifying glass/search icon ("magnifyingglass").
pub const MAGNIFYINGGLASS: SystemIcon = SystemIcon::from_static("magnifyingglass");

/// Person icon ("person").
pub const PERSON: SystemIcon = SystemIcon::from_static("person");

/// Person filled icon ("person.fill").
pub const PERSON_FILL: SystemIcon = SystemIcon::from_static("person.fill");

/// Person circle icon ("person.circle").
pub const PERSON_CIRCLE: SystemIcon = SystemIcon::from_static("person.circle");

/// Plus icon ("plus").
pub const PLUS: SystemIcon = SystemIcon::from_static("plus");

/// Plus circle icon ("plus.circle").
pub const PLUS_CIRCLE: SystemIcon = SystemIcon::from_static("plus.circle");

/// Plus circle filled icon ("plus.circle.fill").
pub const PLUS_CIRCLE_FILL: SystemIcon = SystemIcon::from_static("plus.circle.fill");

/// Minus icon ("minus").
pub const MINUS: SystemIcon = SystemIcon::from_static("minus");

/// Minus circle icon ("minus.circle").
pub const MINUS_CIRCLE: SystemIcon = SystemIcon::from_static("minus.circle");

/// Trash icon ("trash").
pub const TRASH: SystemIcon = SystemIcon::from_static("trash");

/// Trash filled icon ("trash.fill").
pub const TRASH_FILL: SystemIcon = SystemIcon::from_static("trash.fill");

/// Xmark/close icon ("xmark").
pub const XMARK: SystemIcon = SystemIcon::from_static("xmark");

/// Xmark circle icon ("xmark.circle").
pub const XMARK_CIRCLE: SystemIcon = SystemIcon::from_static("xmark.circle");

/// Xmark circle filled icon ("xmark.circle.fill").
pub const XMARK_CIRCLE_FILL: SystemIcon = SystemIcon::from_static("xmark.circle.fill");

/// Checkmark icon ("checkmark").
pub const CHECKMARK: SystemIcon = SystemIcon::from_static("checkmark");

/// Checkmark circle icon ("checkmark.circle").
pub const CHECKMARK_CIRCLE: SystemIcon = SystemIcon::from_static("checkmark.circle");

/// Checkmark circle filled icon ("checkmark.circle.fill").
pub const CHECKMARK_CIRCLE_FILL: SystemIcon = SystemIcon::from_static("checkmark.circle.fill");

// ============================================================================
// Navigation Icons
// ============================================================================

/// Chevron right icon ("chevron.right").
pub const CHEVRON_RIGHT: SystemIcon = SystemIcon::from_static("chevron.right");

/// Chevron left icon ("chevron.left").
pub const CHEVRON_LEFT: SystemIcon = SystemIcon::from_static("chevron.left");

/// Chevron up icon ("chevron.up").
pub const CHEVRON_UP: SystemIcon = SystemIcon::from_static("chevron.up");

/// Chevron down icon ("chevron.down").
pub const CHEVRON_DOWN: SystemIcon = SystemIcon::from_static("chevron.down");

/// Arrow right icon ("arrow.right").
pub const ARROW_RIGHT: SystemIcon = SystemIcon::from_static("arrow.right");

/// Arrow left icon ("arrow.left").
pub const ARROW_LEFT: SystemIcon = SystemIcon::from_static("arrow.left");

/// Arrow up icon ("arrow.up").
pub const ARROW_UP: SystemIcon = SystemIcon::from_static("arrow.up");

/// Arrow down icon ("arrow.down").
pub const ARROW_DOWN: SystemIcon = SystemIcon::from_static("arrow.down");

// ============================================================================
// Media Icons
// ============================================================================

/// Star icon ("star").
pub const STAR: SystemIcon = SystemIcon::from_static("star");

/// Star filled icon ("star.fill").
pub const STAR_FILL: SystemIcon = SystemIcon::from_static("star.fill");

/// Heart icon ("heart").
pub const HEART: SystemIcon = SystemIcon::from_static("heart");

/// Heart filled icon ("heart.fill").
pub const HEART_FILL: SystemIcon = SystemIcon::from_static("heart.fill");

/// Bell icon ("bell").
pub const BELL: SystemIcon = SystemIcon::from_static("bell");

/// Bell filled icon ("bell.fill").
pub const BELL_FILL: SystemIcon = SystemIcon::from_static("bell.fill");

/// Play icon ("play").
pub const PLAY: SystemIcon = SystemIcon::from_static("play");

/// Play filled icon ("play.fill").
pub const PLAY_FILL: SystemIcon = SystemIcon::from_static("play.fill");

/// Pause icon ("pause").
pub const PAUSE: SystemIcon = SystemIcon::from_static("pause");

/// Pause filled icon ("pause.fill").
pub const PAUSE_FILL: SystemIcon = SystemIcon::from_static("pause.fill");

/// Stop icon ("stop").
pub const STOP: SystemIcon = SystemIcon::from_static("stop");

/// Stop filled icon ("stop.fill").
pub const STOP_FILL: SystemIcon = SystemIcon::from_static("stop.fill");

// ============================================================================
// Communication Icons
// ============================================================================

/// Envelope/mail icon ("envelope").
pub const ENVELOPE: SystemIcon = SystemIcon::from_static("envelope");

/// Envelope filled icon ("envelope.fill").
pub const ENVELOPE_FILL: SystemIcon = SystemIcon::from_static("envelope.fill");

/// Phone icon ("phone").
pub const PHONE: SystemIcon = SystemIcon::from_static("phone");

/// Phone filled icon ("phone.fill").
pub const PHONE_FILL: SystemIcon = SystemIcon::from_static("phone.fill");

/// Message/bubble icon ("message").
pub const MESSAGE: SystemIcon = SystemIcon::from_static("message");

/// Message filled icon ("message.fill").
pub const MESSAGE_FILL: SystemIcon = SystemIcon::from_static("message.fill");

// ============================================================================
// Editing Icons
// ============================================================================

/// Pencil icon ("pencil").
pub const PENCIL: SystemIcon = SystemIcon::from_static("pencil");

/// Square and pencil icon ("square.and.pencil").
pub const SQUARE_AND_PENCIL: SystemIcon = SystemIcon::from_static("square.and.pencil");

/// Slider horizontal icon ("slider.horizontal.3").
pub const SLIDER_HORIZONTAL_3: SystemIcon = SystemIcon::from_static("slider.horizontal.3");

// ============================================================================
// Sharing Icons
// ============================================================================

/// Square and arrow up/share icon ("square.and.arrow.up").
pub const SQUARE_AND_ARROW_UP: SystemIcon = SystemIcon::from_static("square.and.arrow.up");

/// Square and arrow down/download icon ("square.and.arrow.down").
pub const SQUARE_AND_ARROW_DOWN: SystemIcon = SystemIcon::from_static("square.and.arrow.down");

/// Link icon ("link").
pub const LINK: SystemIcon = SystemIcon::from_static("link");

// ============================================================================
// System Icons
// ============================================================================

/// Info circle icon ("info.circle").
pub const INFO_CIRCLE: SystemIcon = SystemIcon::from_static("info.circle");

/// Info circle filled icon ("info.circle.fill").
pub const INFO_CIRCLE_FILL: SystemIcon = SystemIcon::from_static("info.circle.fill");

/// Question mark circle icon ("questionmark.circle").
pub const QUESTIONMARK_CIRCLE: SystemIcon = SystemIcon::from_static("questionmark.circle");

/// Exclamation mark circle icon ("exclamationmark.circle").
pub const EXCLAMATIONMARK_CIRCLE: SystemIcon = SystemIcon::from_static("exclamationmark.circle");

/// Exclamation mark triangle/warning icon ("exclamationmark.triangle").
pub const EXCLAMATIONMARK_TRIANGLE: SystemIcon =
    SystemIcon::from_static("exclamationmark.triangle");

// ============================================================================
// Menu Icons
// ============================================================================

/// Three horizontal lines/menu icon ("line.3.horizontal").
pub const LINE_3_HORIZONTAL: SystemIcon = SystemIcon::from_static("line.3.horizontal");

/// Ellipsis icon ("ellipsis").
pub const ELLIPSIS: SystemIcon = SystemIcon::from_static("ellipsis");

/// Ellipsis circle icon ("ellipsis.circle").
pub const ELLIPSIS_CIRCLE: SystemIcon = SystemIcon::from_static("ellipsis.circle");
