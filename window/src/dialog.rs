//! Dialog components and utilities for modal interactions.
//!
//! This module provides structures and traits for creating and managing
//! modal dialogs with configurable options and actions.

use waterui_core::handler::BoxHandler;

/// Represents a modal dialog with a title, content, and action options.
///
/// A `Dialog` contains all the information needed to display a modal dialog,
/// including its text content and the available user actions.
#[derive(Debug)]
#[allow(dead_code)] // These will be used by dialog managers
pub struct Dialog {
    /// The dialog's title text.
    title: String,
    /// The main content text of the dialog.
    content: String,
    /// Available action options for the user.
    options: Vec<DialogOption>,
}

/// Represents an action option within a dialog.
///
/// Each `DialogOption` defines a button or action that the user can take
/// to respond to the dialog.
#[derive(Debug)]
#[allow(dead_code)] // These will be used by dialog managers
pub struct DialogOption {
    /// The text label for the option button.
    label: String,
    /// The action to execute when the option is selected.
    action: BoxHandler<()>,
    /// Whether this action is destructive (e.g., "Delete", "Cancel").
    is_destructive: bool,
}

/// Trait for handling dialog lifecycle events.
///
/// Implementors of this trait can respond to dialog events such as closing.
pub trait DialogHandler {
    /// Closes the dialog.
    fn close(self);
}

/// Trait for managing modal dialogs.
///
/// A `DialogManager` is responsible for creating and displaying dialogs
/// across different platforms.
pub trait DialogManager {
    /// Opens a new dialog with the specified configuration.
    ///
    /// # Arguments
    ///
    /// * `dialog` - The dialog configuration to display.
    ///
    /// # Returns
    ///
    /// A dialog handler that can be used to control the dialog.
    fn open(&self, dialog: Dialog) -> impl DialogHandler;
}

trait DialogHandlerImpl {
    fn close(self: Box<Self>);
}

/// Type-erased dialog handler that can store any dialog handler implementation.
pub struct AnyDialogHandler(Box<dyn DialogHandlerImpl>);

impl std::fmt::Debug for AnyDialogHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyDialogHandler").finish_non_exhaustive()
    }
}

impl DialogHandler for AnyDialogHandler {
    fn close(self) {
        self.0.close();
    }
}

trait DialogManagerImpl {
    fn open(&self, dialog: Dialog) -> AnyDialogHandler;
}

/// Type-erased dialog manager that can store any dialog manager implementation.
pub struct AnyDialogManager(Box<dyn DialogManagerImpl>);

impl std::fmt::Debug for AnyDialogManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyDialogManager").finish_non_exhaustive()
    }
}

impl DialogManager for AnyDialogManager {
    fn open(&self, dialog: Dialog) -> impl DialogHandler {
        self.0.open(dialog)
    }
}
