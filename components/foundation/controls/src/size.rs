//! The size a control is drawn at.

use waterui_core::Environment;
use waterui_core::plugin::Plugin;

/// How large a control is drawn.
///
/// Material 3 Expressive scales a control through five sizes that change its
/// height, padding, icon size and corner shape together — not just its height
/// — so the size is a semantic choice, like the style, rather than a frame the
/// caller imposes from outside. Each control documents the size it takes when
/// none is chosen; platforms with fewer sizes map these onto theirs.
///
/// Installing a `ControlSize` on a subtree (typically via
/// `ViewExt::control_size`) sets the ambient size inherited by every control in
/// that subtree that did not specify an explicit `.size(...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ControlSize {
    /// The most compact size, for dense rows of controls.
    ExtraSmall,
    /// A compact size.
    Small,
    /// A roomier size, for a screen's main control.
    Medium,
    /// A prominent size, large enough to anchor a section.
    Large,
    /// The largest size, for hero controls on big surfaces.
    ExtraLarge,
}

/// A control size installed on a subtree becomes the ambient size for controls
/// in that subtree that did not choose one explicitly.
impl Plugin for ControlSize {}

impl ControlSize {
    /// Resolves an optional explicit control size against the nearest ambient
    /// [`ControlSize`] in `env`, falling back to `default`.
    #[must_use]
    pub fn resolve(explicit: Option<Self>, env: &Environment, default: Self) -> Self {
        explicit
            .or_else(|| env.get::<Self>().copied())
            .unwrap_or(default)
    }
}
