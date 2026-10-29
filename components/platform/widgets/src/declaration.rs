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

impl WidgetDeclaration {
    /// The attributed function's name — the `waterui_widget_*` export's
    /// suffix after the crate name.
    #[must_use]
    pub const fn function(&self) -> &str {
        self.function.as_str()
    }

    /// The `WidgetConfiguration` kind string; defaults to `function`.
    #[must_use]
    pub const fn kind(&self) -> &str {
        self.kind.as_str()
    }

    /// The name shown in the widget gallery.
    #[must_use]
    pub const fn display_name(&self) -> &str {
        self.display_name.as_str()
    }

    /// The description shown in the widget gallery.
    #[must_use]
    pub const fn description(&self) -> &str {
        self.description.as_str()
    }

    /// The `WidgetFamily` case names the widget supports
    /// (`systemSmall`, `systemMedium`, …).
    #[must_use]
    pub const fn families(&self) -> &[String] {
        self.families.as_slice()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (String, String, String, String, Vec<String>) {
        (
            self.function,
            self.kind,
            self.display_name,
            self.description,
            self.families,
        )
    }
}

/// What a `#[widget_intent]` action declares — the `waterui_meta_intent_*`
/// record payload. `kind` names the generated `Swift` `AppIntent` type
/// (`swift_type` is its stable hash-derived identifier).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentDeclaration {
    kind: String,
    display_name: String,
    description: String,
    swift_type: String,
}

impl IntentDeclaration {
    /// The declared kind — unique per application.
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }
    /// The intent's display title.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
    /// The intent's description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
    /// The generated `Swift` `AppIntent` type name.
    #[must_use]
    pub fn swift_type(&self) -> &str {
        &self.swift_type
    }
}
