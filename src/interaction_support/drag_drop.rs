//! Native drag and drop support for `WaterUI`.
//!
//! A drag carries one typed value. [`ViewExt::draggable`](crate::ViewExt::draggable)
//! takes any [`Transferable`] value, and
//! [`ViewExt::drop_destination`](crate::ViewExt::drop_destination) accepts exactly
//! the drags whose value has the type of its handler's first argument. A
//! destination is neither highlighted nor called for a drag of another type.
//!
//! The framework's own transferable types map onto the platform pasteboard, so
//! they travel between applications:
//!
//! - [`Str`](crate::Str): plain text (`UTType.plainText`, MIME `text/plain`)
//! - [`Url`](crate::Url): a URL (`UTType.url`, MIME `text/uri-list`)
//! - [`Files`]: a list of files (`UTType.fileURL`, Android `ClipData` URIs)
//!
//! Any other type an application marks [`Transferable`] travels within the
//! process only: it never reaches the pasteboard, and drags from other
//! applications never produce it.
//!
//! The drag and drop system integrates with native platform APIs:
//!
//! - **macOS**: `NSDraggingSource` / `NSDraggingDestination`
//! - **iOS**: `UIDragInteraction` / `UIDropInteraction`
//! - **Android**: `View.startDragAndDrop()` / `OnDragListener`
//!
//! # Example
//!
//! ```rust
//! use waterui::drag_drop::Transferable;
//! use waterui::prelude::*;
//! use waterui::Str;
//!
//! // Text travels to other applications as plain text.
//! let source = text!("Drag me!").draggable(Str::from("Hello, World!"));
//! let target = text!("Drop text here").drop_destination(|text: Str| {
//!     let _received = text;
//! });
//!
//! // An application type travels within the process only.
//! #[derive(Debug, Clone, PartialEq, Transferable)]
//! struct TabId(u64);
//!
//! let tab = text!("Tab 1").draggable(TabId(1));
//! let tab_bar = text!("Tabs").drop_destination(|tab: TabId| {
//!     let _moved = tab;
//! });
//! ```

pub use waterui_interaction::drag_drop::{
    DragPayload, Draggable, DropDestination, DropDestinationExt, DropInfo, Files, IntoDragPayload,
    PlatformRepresentation, TransferKind, Transferable,
};
pub use waterui_macros::Transferable;
