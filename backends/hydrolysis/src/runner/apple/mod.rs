//! The Apple hosts: Hydrolysis windows realized on `cocoa-ui` views.
//!
//! Each host owns the application and its windows through `cocoa-ui` and
//! drives the shared frame transaction from a display link; this module
//! holds everything that does not depend on which kit — `UIKit` or `AppKit` —
//! hosts the view.
//!
//! - [`surface`]: the engine-presented Metal surface over a view handle
//! - [`session`]: one window's frame transaction and its platform publishes
//! - [`text_input`]: the `TextInputClient` document over the editing session
//! - [`input`]: `cocoa-ui` surface events as renderer input events
//! - [`accessibility`]: the accessibility tree publish and action routing
//! - [`uikit`]: the `UIKit` host

mod accessibility;
mod input;
mod session;
mod surface;
mod text_input;
pub(super) mod uikit;
