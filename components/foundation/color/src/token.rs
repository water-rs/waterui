//! Semantic theme colour tokens.
//!
//! Each token is a zero-sized unit struct implementing
//! [`Resolvable<Resolved = WorkingColor>`](Resolvable), [`ColorSlot`], and
//! [`View`], with a matching constructor on [`Color`] (for example
//! [`Color::accent`] and [`Color::surface`]).

use nami::{Computed, Signal, impl_constant};
use pastey::paste;
use waterui_core::{Environment, View, resolve::Resolvable};

use crate::slot::{ColorSlot, Sealed, rederive_color_slot, resolve_color_slot};
use crate::{Color, ColorScheme, WorkingColor};

macro_rules! define_color_tokens {
    ($( $name:ident => $doc:literal ),* $(,)?) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Clone, Copy, Default)]
            pub struct $name;

            impl Resolvable for $name {
                type Resolved = WorkingColor;

                fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
                    resolve_color_slot::<Self>(env)
                }
            }

            impl_constant!($name);

            impl Sealed for $name {}

            impl ColorSlot for $name {}

            impl View for $name {
                fn body(self, _env: &Environment) -> impl View {
                    Color::new(self)
                }
            }

            paste! {
                impl Color {
                    #[doc = $doc]
                    #[must_use]
                    pub fn [<$name:snake>]() -> Self {
                        Self::new($name)
                    }
                }
            }
        )*

        /// Re-derives from `scheme` every slot installed as a derivation.
        pub(crate) fn rederive(env: &mut Environment, scheme: &Computed<ColorScheme>) {
            $( rederive_color_slot::<$name>(env, scheme); )*
        }
    };
}

define_color_tokens! {
    Background => "Primary background color.",
    Surface => "Elevated surface color (cards, sheets).",
    SurfaceVariant => "Alternate surface color.",
    Border => "Border and divider color.",
    Foreground => "Primary text and icon color.",
    MutedForeground => "Secondary/dimmed text color.",
    TertiaryForeground => "Tertiary text color, fainter than the muted tier: a month grid's days outside the shown month.",
    DisabledForeground => "Text and glyph color of a control or item that cannot be used.",
    Accent => "Accent color for interactive elements.",
    AccentContainer => "Container color associated with the accent.",
    AccentForeground => "Foreground on accent backgrounds.",
    Tertiary => "Contrasting accent color for complementary emphasis.",
    TertiaryContainer => "Container color associated with the tertiary accent.",
    SelectionContainer => "The fill a platform paints behind a selected item.",
    SelectionForeground => "Foreground drawn on the selection container.",
    UnemphasizedSelection => "The fill painted behind a selected item while its control is not focused.",
    Error => "Destructive / error emphasis: badges, destructive buttons, validation.",
    ErrorForeground => "Foreground drawn on the error color.",
    Scrim => "Dimming backdrop a modal layer paints between the content and a presented dialog.",
}
