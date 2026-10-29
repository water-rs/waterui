//! The accessibility tree publish and action routing of an Apple view.
//!
//! One `accesskit_ios::SubclassingAdapter` per scene serves the view's
//! accessibility elements from the renderer's tree. The adapter's handlers
//! only stash: the initial tree is the last published update, and an action
//! joins a queue the next frame transaction performs, so no handler enters
//! the session while a transaction runs. Without the `accessibility` feature
//! the bridge is an empty type and every operation vanishes.

#[cfg(feature = "accessibility")]
pub(super) use enabled::AccessibilityBridge;

#[cfg(not(feature = "accessibility"))]
pub(super) use disabled::AccessibilityBridge;

#[cfg(feature = "accessibility")]
mod enabled {
    use std::cell::RefCell;
    use std::rc::Rc;

    use accesskit::{
        ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate,
    };
    use accesskit_ios::SubclassingAdapter;
    use cocoa_ui::PlatformView;
    use waterui_core::Environment;

    use crate::platform::PlatformWindow;
    use crate::runner::window::{RuntimeWindow, runtime_window_origin};
    use crate::text::ParleyEngine;

    /// What the adapter's handlers share with the bridge.
    #[derive(Default)]
    struct Mailbox {
        /// The update last offered to the adapter, replayed to an assistive
        /// technology that attaches later.
        last: RefCell<Option<TreeUpdate>>,
        actions: RefCell<Vec<ActionRequest>>,
    }

    struct Activation {
        mailbox: Rc<Mailbox>,
        request_frame: Rc<dyn Fn()>,
    }

    impl ActivationHandler for Activation {
        fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
            let cached = self.mailbox.last.borrow().clone();
            if cached.is_none() {
                // No frame has produced a tree yet: schedule one, whose
                // publish reaches the adapter as a full update.
                (self.request_frame)();
            }
            cached
        }
    }

    struct Actions {
        mailbox: Rc<Mailbox>,
        request_frame: Rc<dyn Fn()>,
    }

    impl ActionHandler for Actions {
        fn do_action(&mut self, request: ActionRequest) {
            self.mailbox.actions.borrow_mut().push(request);
            (self.request_frame)();
        }
    }

    struct Deactivation;

    impl DeactivationHandler for Deactivation {
        fn deactivate_accessibility(&mut self) {}
    }

    /// The scene's adapter and the mailbox its handlers fill.
    pub(in crate::runner::apple) struct AccessibilityBridge {
        adapter: SubclassingAdapter,
        mailbox: Rc<Mailbox>,
    }

    impl AccessibilityBridge {
        /// Attaches an adapter to `view`; `request_frame` schedules the frame
        /// an attach or an action needs.
        ///
        /// Must run before the view is first shown.
        pub(in crate::runner::apple) fn new(
            view: &PlatformView,
            request_frame: &Rc<dyn Fn()>,
        ) -> Self {
            let mailbox = Rc::new(Mailbox::default());
            let pointer = std::ptr::from_ref(view).cast_mut().cast();
            // SAFETY: `pointer` names the live view the caller borrows, and
            // the adapter retains it.
            let adapter = unsafe {
                SubclassingAdapter::new(
                    pointer,
                    Activation {
                        mailbox: Rc::clone(&mailbox),
                        request_frame: Rc::clone(request_frame),
                    },
                    Actions {
                        mailbox: Rc::clone(&mailbox),
                        request_frame: Rc::clone(request_frame),
                    },
                    Deactivation,
                )
            };
            Self { adapter, mailbox }
        }

        /// Performs the actions assistive technologies queued since the last
        /// frame against the renderer, scheduling a frame for each one it
        /// handled.
        pub(in crate::runner::apple) fn perform_actions<P: PlatformWindow>(
            &self,
            runtime: &mut RuntimeWindow<P, ParleyEngine>,
            env: &Environment,
        ) {
            let queued = std::mem::take(&mut *self.mailbox.actions.borrow_mut());
            for request in queued {
                tracing::trace!(
                    target: "waterui::hydrolysis::a11y",
                    action = ?request.action,
                    node = ?request.target_node,
                    "accessibility action requested"
                );
                let env = env.extending(runtime_window_origin(runtime));
                if runtime.renderer.handle_accessibility_action(request, &env) {
                    runtime.request_refresh();
                    runtime.request_redraw();
                }
            }
        }

        /// Offers the renderer's latest tree update, if the frame produced
        /// one, to the adapter and raises the events it queues.
        pub(in crate::runner::apple) fn publish<P: PlatformWindow>(
            &self,
            runtime: &mut RuntimeWindow<P, ParleyEngine>,
        ) {
            let Some(update) = runtime.renderer.take_accessibility_tree_update() else {
                return;
            };
            self.mailbox.last.replace(Some(update.clone()));
            // The events are raised with no session state held by the
            // adapter, as its contract requires.
            if let Some(events) = self.adapter.update_if_active(|| update) {
                events.raise();
            }
        }
    }
}

#[cfg(not(feature = "accessibility"))]
mod disabled {
    use std::rc::Rc;

    use cocoa_ui::PlatformView;
    use waterui_core::Environment;

    use crate::platform::PlatformWindow;
    use crate::runner::window::RuntimeWindow;
    use crate::text::ParleyEngine;

    /// Without the feature there is no tree to serve.
    pub(in crate::runner::apple) struct AccessibilityBridge;

    impl AccessibilityBridge {
        pub(in crate::runner::apple) const fn new(
            _view: &PlatformView,
            _request_frame: &Rc<dyn Fn()>,
        ) -> Self {
            Self
        }

        pub(in crate::runner::apple) const fn perform_actions<P: PlatformWindow>(
            &self,
            _runtime: &mut RuntimeWindow<P, ParleyEngine>,
            _env: &Environment,
        ) {
        }

        pub(in crate::runner::apple) const fn publish<P: PlatformWindow>(
            &self,
            _runtime: &mut RuntimeWindow<P, ParleyEngine>,
        ) {
        }
    }
}
