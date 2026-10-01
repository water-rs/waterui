use super::*;

mod accessibility_impl;
#[cfg(feature = "accessibility")]
mod remap;

/// Why [`SemanticCore::accessibility_activation_point`] could not resolve a
/// point a pointer can reach (water-rs/hydrolysis#27).
#[cfg(feature = "accessibility")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityActivationPointError {
    /// No node with this id exists in the emitted tree.
    NoNode,
    /// The node carries no bounds — the semantic walk emits none, and a
    /// suppressed subtree registers nothing.
    NoBounds,
    /// The node's clip chain and the window bounds leave no visible fragment,
    /// so there is no on-screen point to return.
    EmptyFragment,
}

#[cfg(feature = "accessibility")]
impl core::fmt::Display for AccessibilityActivationPointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NoNode => "no accessibility node with that id exists in the tree",
            Self::NoBounds => "the accessibility node carries no bounds",
            Self::EmptyFragment => {
                "the accessibility node's clip chain and the window bounds leave no visible fragment"
            }
        })
    }
}

#[cfg(feature = "accessibility")]
impl std::error::Error for AccessibilityActivationPointError {}

pub(crate) use accessibility_impl::*;
#[cfg(feature = "accessibility")]
pub(crate) use remap::*;
