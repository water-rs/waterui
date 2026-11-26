//! Floating-surface presentation metadata for elevated interactive views.

use waterui_core::metadata::MetadataKey;

/// Marks a view as promoted to the floating interaction layer.
///
/// Floating presentation is a semantic attribute realized by each backend:
/// Apple platforms realize it as a native elevated floating container, while
/// Hydrolysis realizes the active style package's floating surface look.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Floating;

impl MetadataKey for Floating {}
