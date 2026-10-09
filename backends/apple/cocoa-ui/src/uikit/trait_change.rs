//! A `UITraitChangeObservable` registration for one trait class.
//!
//! # Safety
//!
//! The `unsafe` here registers a trait-change handler with
//! `registerForTraitChanges:withHandler:`, which `objc2-ui-kit` does not
//! bind: its `traits` parameter is an array of trait classes, a type the
//! binding generator skips. The declaration is
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
use objc2::{MainThreadMarker, MainThreadOnly, Message, msg_send};
use objc2_foundation::NSArray;
use objc2_ui_kit::{
    UITraitChangeObservable, UITraitChangeRegistration, UITraitCollection, UITraitEnvironment,
};

use crate::callback::guarded;

/// Registers `handler` on `observable` for changes of the one trait class
/// `trait_class`, until the returned guard is dropped.
///
/// `handler` runs on the main thread with the observable that reported the
/// change. A panic in it aborts the process (see the
/// [crate documentation](crate)).
///
/// `E` is the concrete view or controller class, and `O` the `UIKit` class
/// in its inheritance chain that implements `UITraitChangeObservable` —
/// `UIView` or `UIViewController`. `handler` receives `E` itself, however
/// far below `O` it sits.
///
/// # Panics
///
/// If `UIKit` reports a trait change off the main thread.
pub(super) fn register_trait_change<O, E>(
    observable: &E,
    trait_class: &AnyObject,
    handler: impl Fn(&E) + 'static,
) -> TraitChangeObservation
where
    E: Message + MainThreadOnly + AsRef<O>,
    O: Message + UITraitChangeObservable,
{
    // The block may be released wherever `UIKit` lets it go, so what it
    // captures is bound to the main thread and dropped there. The
    // observable is held weakly: it owns the registration, and so the
    // block.
    let state = MainThreadBound::new((Weak::new(observable), handler), observable.mtm());
    let block = RcBlock::new(
        move |_environment: NonNull<ProtocolObject<dyn UITraitEnvironment>>,
              _previous: NonNull<UITraitCollection>| {
            guarded("trait change handler", || {
                let mtm = MainThreadMarker::new()
                    .expect("UIKit must report trait changes on the main thread");
                let (observable, handler) = state.get(mtm);
                if let Some(observable) = observable.load() {
                    handler(&observable);
                }
            });
        },
    );
    let traits = NSArray::from_slice(&[trait_class]);
    // SAFETY: see the module safety note.
    let registration: Retained<ProtocolObject<dyn UITraitChangeRegistration>> = unsafe {
        msg_send![
            observable,
            registerForTraitChanges: &*traits,
            withHandler: &*block
        ]
    };
    TraitChangeObservation {
        observable: Weak::new(ProtocolObject::from_ref(AsRef::<O>::as_ref(observable))),
        registration,
    }
}

/// Keeps a trait-change handler registered; dropping it unregisters the
/// handler.
#[must_use = "the handler is unregistered as soon as this guard is dropped"]
pub(super) struct TraitChangeObservation {
    observable: Weak<ProtocolObject<dyn UITraitChangeObservable>>,
    registration: Retained<ProtocolObject<dyn UITraitChangeRegistration>>,
}

impl fmt::Debug for TraitChangeObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraitChangeObservation")
            .finish_non_exhaustive()
    }
}

impl Drop for TraitChangeObservation {
    fn drop(&mut self) {
        // An observable that is gone took its registrations with it.
        if let Some(observable) = self.observable.load() {
            observable.unregisterForTraitChanges(&self.registration);
        }
    }
}
