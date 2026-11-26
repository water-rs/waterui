//! Disclosure component with a focusable header, indicator, and lazy expandable content.
//!
//! # Examples
//! ```rust
//! use waterui::prelude::*;
//! use waterui::widget::{Disclosure, disclosure};
//!
//! fn basic() -> impl View {
//!     disclosure("Advanced", || text("Details"))
//! }
//!
//! fn controlled(open: &Binding<bool>) -> impl View {
//!     Disclosure::new("Advanced", || text("Details")).expanded(open)
//! }
//! ```

pub use waterui_controls::disclosure::{Disclosure, DisclosureConfig, disclosure};
pub use waterui_controls::slot::SlotBuilder;
