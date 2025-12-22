//! Native drag and drop support for `WaterUI`.
//!
//! This module provides types for making views draggable and enabling views to receive
//! dropped content. The drag and drop system integrates with native platform APIs:
//!
//! - **macOS**: `NSDraggingSource` / `NSDraggingDestination`
//! - **iOS**: `UIDragInteraction` / `UIDropInteraction`
//! - **Android**: `View.startDragAndDrop()` / `OnDragListener`
//!
//! # Example
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! // Make a view draggable
//! text("Drag me!")
//!     .draggable(DragData::text("Hello, World!"));
//!
//! // Create a drop destination
//! text("Drop here")
//!     .drop_destination(|data: DragData| {
//!         println!("Received: {:?}", data);
//!     });
//! ```

use alloc::{boxed::Box, string::String};
use core::fmt;
use nami::Computed;
use nami::signal::IntoComputed;
use waterui_core::{
    handler::{BoxHandler, HandlerFn, into_handler},
    metadata::MetadataKey,
};

/// Data that can be transferred via drag and drop.
///
/// This enum represents the types of content that can be dragged between views.
/// Backend implementations convert these to platform-native formats:
///
/// - **Text**: Plain text (UTType.plainText, MIME text/plain)
/// - **Url**: URLs (UTType.url, MIME text/uri-list)
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DragData {
    /// Plain text content.
    Text(String),
    /// A URL (as a string).
    Url(String),
}

impl DragData {
    /// Creates a text drag data item.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    /// Creates a URL drag data item.
    #[must_use]
    pub fn url(url: impl Into<String>) -> Self {
        Self::Url(url.into())
    }

    /// Returns the content as a string, regardless of the type.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Text(s) | Self::Url(s) => s,
        }
    }

    /// Returns `true` if this is text data.
    #[must_use]
    pub const fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    /// Returns `true` if this is URL data.
    #[must_use]
    pub const fn is_url(&self) -> bool {
        matches!(self, Self::Url(_))
    }
}

// Implement IntoSignal/IntoComputed so DragData can be passed directly to .draggable()
nami::impl_constant!(DragData);

/// Metadata that makes a view draggable.
///
/// When attached to a view, the view becomes a drag source. Users can initiate
/// a drag operation by:
/// - **macOS**: Click and drag
/// - **iOS/Android**: Long-press and drag
///
/// The data provider is evaluated when the drag begins.
pub struct Draggable {
    /// The data to transfer when dragging.
    pub data: Computed<DragData>,
}

impl fmt::Debug for Draggable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Draggable").finish_non_exhaustive()
    }
}

impl MetadataKey for Draggable {}

impl Draggable {
    /// Creates a new draggable metadata with the given data.
    #[must_use]
    pub fn new(data: impl IntoComputed<DragData>) -> Self {
        Self {
            data: data.into_computed(),
        }
    }
}

/// Metadata that makes a view a drop destination.
///
/// When attached to a view, the view can receive dropped content. The `on_drop`
/// handler is called when compatible data is dropped onto the view.
///
/// The handler can extract the dropped data using `Use<DragData>`:
///
/// ```rust,ignore
/// .drop_destination(|Use(data): Use<DragData>| {
///     println!("Received: {:?}", data);
/// })
/// ```
pub struct DropDestination {
    /// Callback invoked when data is dropped onto this view.
    /// The handler extracts `DragData` from the environment using `Use<DragData>`.
    pub on_drop: BoxHandler<()>,
    /// Optional callback when a drag enters the view bounds.
    pub on_enter: Option<BoxHandler<()>>,
    /// Optional callback when a drag exits the view bounds.
    pub on_exit: Option<BoxHandler<()>>,
}

impl fmt::Debug for DropDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropDestination").finish_non_exhaustive()
    }
}

impl MetadataKey for DropDestination {}

impl DropDestination {
    /// Creates a drop destination with only an `on_drop` handler.
    pub fn new<P>(on_drop: impl HandlerFn<P, ()> + 'static) -> Self
    where
        P: 'static,
    {
        Self {
            on_drop: Box::new(into_handler(on_drop)),
            on_enter: None,
            on_exit: None,
        }
    }

    /// Adds a callback for when a drag enters the view bounds.
    #[must_use]
    pub fn on_enter<P>(mut self, handler: impl HandlerFn<P, ()> + 'static) -> Self
    where
        P: 'static,
    {
        self.on_enter = Some(Box::new(into_handler(handler)));
        self
    }

    /// Adds a callback for when a drag exits the view bounds.
    #[must_use]
    pub fn on_exit<P>(mut self, handler: impl HandlerFn<P, ()> + 'static) -> Self
    where
        P: 'static,
    {
        self.on_exit = Some(Box::new(into_handler(handler)));
        self
    }
}
