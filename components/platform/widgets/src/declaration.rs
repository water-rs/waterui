//! The `waterui_meta_widget_*` record payload the `water` CLI decodes.
//!
//! `#[widget]` emits one record per declared function into the crate's
//! `.wmeta` metadata directory; `water` enumerates the records on the
//! project crate's artifact, pairs each with the `waterui_widget_*` export
//! sharing its name suffix, and generates the `WidgetKit` extension from
//! them. This type is the wire contract of that payload.

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

/// What a `#[widget]` function declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct WidgetDeclaration {
    /// The attributed function's name — the `waterui_widget_*` export's
    /// suffix after the crate name.
    #[serde(rename = "fn")]
    pub function: String,
    /// The `WidgetConfiguration` kind string; defaults to `function`.
    pub kind: String,
    /// The name shown in the widget gallery.
    pub display_name: String,
    /// The description shown in the widget gallery.
    pub description: String,
    /// The `WidgetFamily` case names the widget supports
    /// (`systemSmall`, `systemMedium`, …).
    pub families: Vec<String>,
}
