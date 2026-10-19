//! A view controller's light or dark appearance, and its changes.
//!
//! # Safety
//!
//! The `unsafe` here reads trait collections, which `UIKit` answers on the
//! main thread.

use std::fmt;

use objc2::ClassType;
use objc2_ui_kit::{
    UITraitCollection, UITraitEnvironment, UITraitUserInterfaceStyle, UIUserInterfaceStyle,
    UIViewController,
};

use super::trait_change::{TraitChangeObservation, register_trait_change};
use super::view_controller::ViewController;
use crate::color_scheme::ColorScheme;

impl ViewController {
    /// Whether the controller's views are currently drawn light or dark.
    #[must_use]
    pub fn color_scheme(&self) -> ColorScheme {
        // SAFETY: see the module safety note.
        let style = unsafe { self.traitCollection().userInterfaceStyle() };
        scheme_for_style(style)
    }

    /// Calls `handler` with the new scheme every time the controller's
    /// light or dark appearance changes, until the returned guard is dropped.
    ///
    /// A panic in `handler` aborts the process (see the
    /// [crate documentation](crate)).
    ///
    /// # Panics
    ///
    /// If `UIKit` reports a trait change off the main thread.
    pub fn observe_color_scheme(
        &self,
        handler: impl Fn(ColorScheme) + 'static,
    ) -> ColorSchemeObservation {
        ColorSchemeObservation {
            observation: register_trait_change::<UIViewController, _>(
                self,
                UITraitUserInterfaceStyle::class().as_ref(),
                move |controller: &Self| handler(controller.color_scheme()),
            ),
        }
    }
}

/// Keeps a color-scheme handler registered; dropping it unregisters the
/// handler.
#[must_use = "the handler is unregistered as soon as this guard is dropped"]
pub struct ColorSchemeObservation {
    observation: TraitChangeObservation,
}

impl fmt::Debug for ColorSchemeObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorSchemeObservation")
            .field("observation", &self.observation)
            .finish_non_exhaustive()
    }
}

/// Whether the application's interface is currently drawn light or dark,
/// from the current trait collection.
///
/// Use this for the scheme a `ViewController` does not exist yet to answer —
/// before the first scene connects, for example. Once a controller exists,
/// prefer [`ViewController::color_scheme`], which follows its own overridden
/// traits.
#[must_use]
pub fn current_scheme() -> ColorScheme {
    // SAFETY: see the module safety note.
    let style = unsafe { UITraitCollection::currentTraitCollection().userInterfaceStyle() };
    scheme_for_style(style)
}

/// Dark only for an explicitly dark style; an unspecified style is drawn
/// light, as `UIKit` draws it.
fn scheme_for_style(style: UIUserInterfaceStyle) -> ColorScheme {
    if style == UIUserInterfaceStyle::Dark {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}
