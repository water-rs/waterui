//! The colours a diagram is drawn in.
//!
//! Mermaid carries its own theme, expressed as CSS. We do not use it. A diagram
//! embedded in a `WaterUI` application is part of that application's surface,
//! and it reads the same theme tokens every other component reads, so it
//! follows a light/dark switch and a custom accent without knowing either
//! happened. This is Principle 6: the framework owns default appearance, and a
//! component that hard-codes its own colours is a bug.

use nami::{Signal, SignalExt as _};
use waterui_core::{Environment, resolve::Resolvable};
use waterui_graphics::color::{
    BorderColor, ForegroundColor, MutedForegroundColor, ResolvedColor, SurfaceColor,
    SurfaceVariantColor,
};

/// The colours one diagram is drawn with.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Fill of a node box.
    pub node_fill: ResolvedColor,
    /// Outline of a node box.
    pub node_border: ResolvedColor,
    /// Connectors and their decorations.
    pub edge: ResolvedColor,
    /// Fill of a subgraph or participant frame.
    pub cluster: ResolvedColor,
    /// Frame outlines and dividers.
    pub border: ResolvedColor,
    /// Label text, for the rare label the scene draws itself.
    pub foreground: ResolvedColor,
}

/// The environment key that resolves a [`Palette`].
#[derive(Debug, Clone, Copy)]
pub struct DiagramPalette;

impl Resolvable for DiagramPalette {
    type Resolved = Palette;

    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        SurfaceColor
            .resolve(env)
            .zip(&BorderColor.resolve(env))
            .zip(&MutedForegroundColor.resolve(env))
            .zip(&SurfaceVariantColor.resolve(env))
            .zip(&ForegroundColor.resolve(env))
            .map(
                |((((surface, border), muted), surface_variant), foreground)| Palette {
                    node_fill: surface,
                    node_border: border,
                    edge: muted,
                    cluster: surface_variant,
                    border,
                    foreground,
                },
            )
    }
}
