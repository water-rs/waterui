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
//! ```rust
//! use waterui::prelude::*;
//!
//! // Make a view draggable
//! text("Drag me!")
//!     .draggable(DragData::text("Hello, World!"));
//!
//! // Create a drop destination that receives the dropped data
//! text("Drop here")
//!     .drop_destination(|data: DragData| {
//!         println!("Received: {:?}", data);
//!     });
//! ```

use core::fmt;
use nami::Computed;
use nami::signal::IntoComputed;
use waterui_core::{
    Environment, Error,
    extract::Extractor,
    handler::{BoxedAction, Handler, boxed_action},
    metadata::{Metadata, MetadataKey},
};
use waterui_str::Str;

use crate::reactive::Binding;

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
    Text(Str),
    /// A URL (as a string).
    Url(Str),
}

impl DragData {
    /// Creates a text drag data item.
    #[must_use]
    pub fn text(text: impl Into<Str>) -> Self {
        Self::Text(text.into())
    }

    /// Creates a URL drag data item.
    #[must_use]
    pub fn url(url: impl Into<Str>) -> Self {
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

// ============================================================================
// Drop Handler with DragData extraction
// ============================================================================

impl Extractor for DragData {
    fn extract(env: &Environment) -> Result<Self, Error> {
        env.get::<Self>()
            .cloned()
            .ok_or_else(|| Error::msg("DragData not found in environment"))
    }
}

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
/// # Example
///
/// ```rust
/// .drop_destination(|data: DragData| {
///     println!("Received: {:?}", data);
/// })
/// ```
pub struct DropDestination {
    /// Callback invoked when data is dropped onto this view.
    /// The handler receives `DragData` extracted from the environment.
    pub on_drop: BoxedAction<()>,
    /// Optional callback when a drag enters the view bounds.
    pub on_enter: Option<BoxedAction<()>>,
    /// Optional callback when a drag exits the view bounds.
    pub on_exit: Option<BoxedAction<()>>,
}

impl fmt::Debug for DropDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropDestination").finish_non_exhaustive()
    }
}

impl MetadataKey for DropDestination {}

impl DropDestination {
    /// Creates a drop destination with an `on_drop` handler that receives the dropped data.
    ///
    /// # Example
    ///
    /// ```rust
    /// DropDestination::new(|data: DragData| {
    ///     println!("Dropped: {:?}", data);
    /// })
    /// ```
    pub fn new<Args>(on_drop: impl Handler<Args, ()>) -> Self {
        Self {
            on_drop: boxed_action(on_drop),
            on_enter: None,
            on_exit: None,
        }
    }

    /// Adds a callback for when a drag enters the view bounds.
    #[must_use]
    pub fn on_enter(mut self, handler: impl FnMut() + 'static) -> Self {
        self.on_enter = Some(boxed_action(handler));
        self
    }

    /// Adds a callback for when a drag exits the view bounds.
    #[must_use]
    pub fn on_exit(mut self, handler: impl FnMut() + 'static) -> Self {
        self.on_exit = Some(boxed_action(handler));
        self
    }
}

// ============================================================================
// Drop Destination Extension
// ============================================================================

/// Extension trait for `Metadata<DropDestination>` to easily bind hover state.
pub trait DropDestinationExt {
    /// Binds the drag hover state to a `Binding<bool>`.
    ///
    /// This is a convenience method that automatically sets the binding to `true`
    /// when a drag enters the view and `false` when it exits.
    ///
    /// # Example
    ///
    /// ```rust
    /// let is_hovering = Binding::bool(false);
    ///
    /// view
    ///     .drop_destination()
    ///     .on_drop_simple(|data| handle_drop(data))
    ///     .drop_hover(&is_hovering)
    /// ```
    #[must_use]
    fn drop_hover(self, is_hovering: &Binding<bool>) -> Self;

    /// Adds a callback for when a drag enters the view bounds.
    ///
    /// This chains with any existing `on_enter` handler, executing both.
    #[must_use]
    fn on_enter(self, handler: impl FnMut() + 'static) -> Self;

    /// Adds a callback for when a drag exits the view bounds.
    ///
    /// This chains with any existing `on_exit` handler, executing both.
    #[must_use]
    fn on_exit(self, handler: impl FnMut() + 'static) -> Self;
}

impl DropDestinationExt for Metadata<DropDestination> {
    fn drop_hover(mut self, is_hovering: &Binding<bool>) -> Self {
        let enter = is_hovering.clone();
        let exit = is_hovering.clone();

        // Chain with existing handlers if present
        let mut prev_enter = self.value.on_enter.take();
        let mut prev_exit = self.value.on_exit.take();

        self.value.on_enter = Some(Box::new(move |env| {
            if let Some(ref mut prev) = prev_enter {
                prev(env);
            }
            enter.set(true);
        }));

        self.value.on_exit = Some(Box::new(move |env| {
            if let Some(ref mut prev) = prev_exit {
                prev(env);
            }
            exit.set(false);
        }));

        self
    }

    fn on_enter(mut self, mut handler: impl FnMut() + 'static) -> Self {
        let mut prev = self.value.on_enter.take();
        self.value.on_enter = Some(Box::new(move |env| {
            if let Some(ref mut prev) = prev {
                prev(env);
            }
            handler();
        }));
        self
    }

    fn on_exit(mut self, mut handler: impl FnMut() + 'static) -> Self {
        let mut prev = self.value.on_exit.take();
        self.value.on_exit = Some(Box::new(move |env| {
            if let Some(ref mut prev) = prev {
                prev(env);
            }
            handler();
        }));
        self
    }
}
