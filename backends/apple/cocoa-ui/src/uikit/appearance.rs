//! A view controller's light or dark appearance, and its changes.
//!
//! # Safety
//!
//! The `unsafe` here reads trait collections on the main thread, and registers
//! a trait-change handler with `registerForTraitChanges:withHandler:`, which
//! `objc2-ui-kit` does not bind: its `traits` parameter is an array of trait
//! classes, a type the binding generator skips. The declaration is
//!
//! ```text
//! - (id<UITraitChangeRegistration>)registerForTraitChanges:(NSArray<UITrait> *)traits
//!                                              withHandler:(UITraitChangeHandler)handler;
//! ```
//!
//! with `UITraitChangeHandler` a block taking the trait environment and its
//! previous trait collection and returning nothing; the call below passes
//! exactly that, and receives the registration as a retained object.
//! `UIKit` copies the block and calls it on the main thread.

use std::fmt;
use std::ptr::NonNull;

use block2::RcBlock;
use dispatch2::MainThreadBound;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, msg_send};
use objc2_foundation::NSArray;
use objc2_ui_kit::{
    UITraitChangeObservable, UITraitChangeRegistration, UITraitCollection, UITraitEnvironment,
    UITraitUserInterfaceStyle, UIUserInterfaceStyle,
};

use super::view_controller::ViewController;
use crate::callback::guarded;
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
        // The block may be released wherever `UIKit` lets it go, so what it
        // captures is bound to the main thread and dropped there. The
        // controller is held weakly: it owns the registration, and so the
        // block.
        let state = MainThreadBound::new((Weak::new(self), handler), self.mtm());
        let block = RcBlock::new(
            move |_environment: NonNull<ProtocolObject<dyn UITraitEnvironment>>,
                  _previous: NonNull<UITraitCollection>| {
                guarded("trait change handler", || {
                    let mtm = MainThreadMarker::new()
                        .expect("UIKit must report trait changes on the main thread");
                    let (controller, handler) = state.get(mtm);
                    if let Some(controller) = controller.load() {
                        handler(controller.color_scheme());
                    }
                });
            },
        );
        let style_trait: &AnyObject = UITraitUserInterfaceStyle::class().as_ref();
        let traits = NSArray::from_slice(&[style_trait]);
        // SAFETY: see the module safety note.
        let registration: Retained<ProtocolObject<dyn UITraitChangeRegistration>> = unsafe {
            msg_send![
                self,
                registerForTraitChanges: &*traits,
                withHandler: &*block
            ]
        };
        ColorSchemeObservation {
            controller: Weak::new(self),
            registration,
        }
    }
}

/// Keeps a color-scheme handler registered; dropping it unregisters the
/// handler.
#[must_use = "the handler is unregistered as soon as this guard is dropped"]
pub struct ColorSchemeObservation {
    controller: Weak<ViewController>,
    registration: Retained<ProtocolObject<dyn UITraitChangeRegistration>>,
}

impl fmt::Debug for ColorSchemeObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorSchemeObservation")
            .finish_non_exhaustive()
    }
}

impl Drop for ColorSchemeObservation {
    fn drop(&mut self) {
        // A controller that is gone took its registrations with it.
        if let Some(controller) = self.controller.load() {
            controller.unregisterForTraitChanges(&self.registration);
        }
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
