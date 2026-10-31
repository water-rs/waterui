//! The environment marker every toolbar item renders under.

use waterui_backend_core::Environment;

/// Marks an environment whose content renders inside a toolbar: the window
/// toolbar on macOS, a navigation or bottom bar on iOS.
///
/// A button style resolves against the toolbar there, as it does in
/// `SwiftUI`. On macOS a borderless toolbar button draws its label in the
/// secondary label color inside the item's capsule; on iOS an automatic or
/// borderless one draws it in the primary label color, as a navigation bar
/// does. Neither takes the accent color a button has in content.
#[derive(Debug, Clone, Copy)]
pub struct ToolbarChrome;

/// `env` with [`ToolbarChrome`] set: the environment every toolbar item's
/// content renders under.
pub fn toolbar_environment(env: &Environment) -> Environment {
    let mut env = env.clone();
    env.insert(ToolbarChrome);
    env
}
