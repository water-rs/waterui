//! The `dialog` metadata: a platform alert — an `NSAlert` sheet on macOS, a
//! `UIAlertController` on iOS — presented while `is_presented` reads `true`
//! (water-rs/waterui#1210).
//!
//! The leaf is the transparent container the overlay contract asks for: the
//! wrapped content mounts and lays out untouched, and the dialog itself is
//! the platform object beside it. `is_presented` drives present/dismiss,
//! `Dialog::run_action` runs an action's handler and writes the binding
//! back to `false`, and the reactive title/message keep streaming into the
//! live alert.
//!
//! One alert per window at a time: an [`AlertQueue`] shared by every leaf the
//! dispatcher renders keeps each window's presented dialogs in presentation
//! order, and a dialog presented while another is up in its window waits
//! until that one is gone — on `UIKit`, which would otherwise stack the
//! second alert on top, and on `AppKit` alike.

use alloc::rc::{Rc, Weak};
use alloc::vec::Vec;
use core::cell::RefCell;

use cocoa_ui::geometry::Rect as KitRect;
#[cfg(target_os = "macos")]
use cocoa_ui::objc2_app_kit::NSWindow as PlatformWindow;
#[cfg(target_os = "ios")]
use cocoa_ui::objc2_ui_kit::UIWindow as PlatformWindow;
use cocoa_ui::view;
use cocoa_ui::{PlatformView, Retained};
#[cfg(target_os = "macos")]
use waterui::dialog::DialogAction;
use waterui::dialog::{Dialog, DialogRole};
use waterui::reactive::{Binding, Computed, Signal};
use waterui::text::StyledStr;
use waterui_backend_core::Environment;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, SheetAlert, window_of};
#[cfg(target_os = "macos")]
use cocoa_ui::objc2_foundation::NSString;
#[cfg(target_os = "ios")]
use cocoa_ui::objc2_ui_kit::UIAlertActionStyle;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{AlertController, HostView};

/// The leaf's live state: the mounted content child, the dialog being
/// presented, its resolved texts, and the platform alert while it is up.
struct DialogState {
    /// Proof of the main thread for kit calls inside callbacks.
    mtm: cocoa_ui::MainThreadMarker,
    /// The wrapper's own view — the window the alert presents from.
    host: Retained<HostView>,
    /// The dispatcher's alert queue — one alert per window at a time.
    queue: Rc<AlertQueue>,
    /// The mounted content — the dialog presents beside it.
    child: Mounted,
    /// The environment handlers resolve in — the environment the wrapper
    /// was rendered under.
    env: Environment,
    /// The dialog: binding, actions, and the cancel path.
    dialog: Dialog,
    /// The presentation binding — the watcher that drives the alert.
    is_presented: Binding<bool>,
    /// The reactive title, kept live while the alert is up.
    title: Computed<StyledStr>,
    /// The reactive message, when the dialog declared one.
    message: Option<Computed<StyledStr>>,
    /// The platform alert while presented; `None` once it ends.
    #[cfg(target_os = "macos")]
    alert: RefCell<Option<SheetAlert>>,
    /// The platform alert while presented; `None` once it ends.
    #[cfg(target_os = "ios")]
    alert: RefCell<Option<AlertController>>,
}

impl core::fmt::Debug for DialogState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DialogState").finish_non_exhaustive()
    }
}

/// One presented dialog in its window's line.
struct QueuedAlert {
    /// The window the dialog presents in.
    window: Retained<PlatformWindow>,
    /// The dialog's leaf — compared by identity, so an entry whose leaf was
    /// dropped still answers for the alert it left on screen.
    state: Weak<DialogState>,
}

/// Every window's presented dialogs, in presentation order: the first entry
/// of a window is the alert on screen, every later one waits for the
/// entries before it to close.
#[derive(Default)]
struct AlertQueue {
    entries: RefCell<Vec<QueuedAlert>>,
}

impl AlertQueue {
    /// The dialog's binding reads `true` with its host in a window: it joins
    /// that window's line, and comes up when nothing is ahead of it.
    fn request(&self, state: &Rc<DialogState>) {
        let Some(window) = view::window(&state.host) else {
            // Not in a window yet: `attachment_changed` requests on attach.
            return;
        };
        let front = {
            let mut entries = self.entries.borrow_mut();
            if entries
                .iter()
                .any(|entry| Weak::as_ptr(&entry.state) == Rc::as_ptr(state))
            {
                return;
            }
            let front = !entries
                .iter()
                .any(|entry| Retained::as_ptr(&entry.window) == Retained::as_ptr(&window));
            entries.push(QueuedAlert {
                window,
                state: Rc::downgrade(state),
            });
            front
        };
        if front {
            present(state);
        }
    }

    /// The dialog closed without a button — its binding wrote `false`, or
    /// its host left the window. An alert on screen is dismissed, and its
    /// end ([`Self::finished`]) brings the next one up; a waiting dialog
    /// leaves the line.
    fn withdraw(&self, state: &Rc<DialogState>) {
        // Bound before the dismissal so the alert drops after it ends: the
        // dismissal may run the end handler synchronously.
        let alert = state.alert.borrow_mut().take();
        if let Some(alert) = alert {
            dismiss_alert(state, &alert);
            return;
        }
        let mut entries = self.entries.borrow_mut();
        let Some(index) = entries
            .iter()
            .position(|entry| Weak::as_ptr(&entry.state) == Rc::as_ptr(state))
        else {
            return;
        };
        let window = Retained::as_ptr(&entries[index].window);
        let in_front = entries
            .iter()
            .position(|entry| Retained::as_ptr(&entry.window) == window)
            == Some(index);
        // The front entry without a live alert is one whose end is running
        // right now — the button that answered it; `finished` removes it.
        if !in_front {
            entries.remove(index);
        }
    }

    /// An alert ended — a button answered it or a dismissal closed it. Its
    /// entry leaves the line and the next dialog in its window still
    /// presented comes up.
    fn finished(&self, state: &Weak<DialogState>) {
        let next = {
            let mut entries = self.entries.borrow_mut();
            let Some(index) = entries
                .iter()
                .position(|entry| Weak::ptr_eq(&entry.state, state))
            else {
                return;
            };
            let window = entries.remove(index).window;
            loop {
                let Some(index) = entries
                    .iter()
                    .position(|entry| Retained::as_ptr(&entry.window) == Retained::as_ptr(&window))
                else {
                    break None;
                };
                match entries[index].state.upgrade() {
                    Some(next) if next.is_presented.snapshot() => break Some(next),
                    _ => {
                        entries.remove(index);
                    }
                }
            }
        };
        if let Some(next) = next {
            present(&next);
        }
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct DialogSubView {
    /// The leaf's state.
    state: Rc<DialogState>,
}

impl core::fmt::Debug for DialogSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DialogSubView").finish_non_exhaustive()
    }
}

impl SubView for DialogSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.child.layout().is_empty()
    }
}

/// The actions `run_action` dispatches on: the platform order — `AppKit`'s
/// most-to-least-prominent add order (primary first) — so a button index
/// always reaches the declared action.
#[cfg(target_os = "macos")]
struct ActionRun {
    /// The resolved actions in the order they were handed to the platform.
    ordered: Vec<DialogAction>,
}

/// Puts the dialog's alert on screen — the dialog is at the front of its
/// window's line in the [`AlertQueue`].
#[cfg(target_os = "macos")]
fn present(state: &Rc<DialogState>) {
    let window = window_of(&state.host)
        .expect("a dialog reaches the front of its window's line only from a host in a window");
    let title = state.title.snapshot().to_semantic().to_string();
    let message = state
        .message
        .as_ref()
        .map(|message| message.snapshot().to_semantic().to_string());
    let alert = SheetAlert::new(state.mtm, &title, message.as_deref());

    // AppKit ordering: the primary — the first Default — is added first so
    // it is the rightmost button bound to Return; every other action
    // follows in resolved order (Cancel, then Destructive, then the
    // remaining Defaults), landing leftward.
    let resolved = state.dialog.resolved_actions();
    let primary = resolved
        .iter()
        .position(|action| action.role() == DialogRole::Default);
    let mut ordered = Vec::with_capacity(resolved.len());
    if let Some(primary) = primary {
        ordered.push(resolved[primary].clone());
    }
    ordered.extend(
        resolved
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != primary)
            .map(|(_, action)| action.clone()),
    );

    let actions = Rc::new(ActionRun { ordered });
    // A dialog declared without actions answers with `NSAlert`'s own
    // acknowledgement: the alert adds its localized OK button, bound to
    // Return, and reports it as the first button — the single resolved
    // acknowledgement action.
    let platform_buttons: &[DialogAction] = if state.dialog.actions().is_empty() {
        &[]
    } else {
        &actions.ordered
    };
    for action in platform_buttons {
        let button = alert.add_button(
            action
                .title()
                .resolve(&state.env)
                .content
                .snapshot()
                .to_semantic()
                .as_ref(),
        );
        match action.role() {
            DialogRole::Cancel => {
                button.setKeyEquivalent(&NSString::from_str("\u{1b}"));
            }
            DialogRole::Destructive => button.setHasDestructiveAction(true),
            DialogRole::Default => {}
        }
    }
    // The sheet's end — a button, or `endSheet` from a dismissal, whose
    // stop code is no button index — runs the answered action, then hands
    // the window to the next dialog in line.
    alert.begin_sheet(&window, {
        let weak = Rc::downgrade(state);
        let queue = Rc::clone(&state.queue);
        let actions = Rc::clone(&actions);
        move |index| {
            if let Some(state) = weak.upgrade() {
                state.alert.borrow_mut().take();
                if let Some(action) = actions.ordered.get(index) {
                    state.dialog.run_action(action, &state.env);
                }
            }
            queue.finished(&weak);
        }
    });
    *state.alert.borrow_mut() = Some(alert);
}

/// Puts the dialog's alert on screen — the dialog is at the front of its
/// window's line in the [`AlertQueue`].
#[cfg(target_os = "ios")]
fn present(state: &Rc<DialogState>) {
    let title = state.title.snapshot().to_semantic().to_string();
    let message = state
        .message
        .as_ref()
        .map(|message| message.snapshot().to_semantic().to_string());
    let controller = AlertController::new(state.mtm, &title, message.as_deref());

    // UIKit takes the resolved order verbatim — the platform lays Cancel
    // out separately and emboldens the preferred action, pinned to the
    // first Default.
    let resolved = state.dialog.resolved_actions();
    let mut preferred = None;
    for action in &resolved {
        let style = match action.role() {
            DialogRole::Default => UIAlertActionStyle::Default,
            DialogRole::Cancel => UIAlertActionStyle::Cancel,
            DialogRole::Destructive => UIAlertActionStyle::Destructive,
        };
        let title = action
            .title()
            .resolve(&state.env)
            .content
            .snapshot()
            .to_semantic()
            .to_string();
        // `UIKit` runs an action's handler once the alert has dismissed
        // itself, so the window is free for the next dialog in line.
        let run = {
            let weak = Rc::downgrade(state);
            let queue = Rc::clone(&state.queue);
            let action = action.clone();
            move || {
                if let Some(state) = weak.upgrade() {
                    state.alert.borrow_mut().take();
                    state.dialog.run_action(&action, &state.env);
                }
                queue.finished(&weak);
            }
        };
        let platform_action = controller.add_action(&title, style, run);
        if preferred.is_none() && action.role() == DialogRole::Default {
            preferred = Some(platform_action);
        }
    }
    if let Some(preferred) = preferred {
        controller.set_preferred(&preferred);
    }
    assert!(
        controller.present(&state.host),
        "a dialog reaches the front of its window's line only from a host in a window with a root view controller"
    );
    *state.alert.borrow_mut() = Some(controller);
}

/// The host moved into or out of a window: a presentation bound before the
/// host had a window joins its window's line on attach, and a host that left
/// the window closes its dialog with it — the binding is written back to
/// `false`.
fn attachment_changed(state: &Rc<DialogState>) {
    if !state.is_presented.snapshot() {
        return;
    }
    if view::has_window(&state.host) {
        state.queue.request(state);
    } else {
        state.queue.withdraw(state);
        state.is_presented.set(false);
    }
}

/// Ends a live alert without a button; its end handler hands the window to
/// the next dialog in line.
#[cfg(target_os = "macos")]
fn dismiss_alert(_state: &Rc<DialogState>, alert: &SheetAlert) {
    // `endSheet` runs the sheet's end handler, which reports the end.
    alert.dismiss();
}

/// Ends a live alert without a button; its end hands the window to the next
/// dialog in line.
#[cfg(target_os = "ios")]
fn dismiss_alert(state: &Rc<DialogState>, alert: &AlertController) {
    let weak = Rc::downgrade(state);
    let queue = Rc::clone(&state.queue);
    alert.dismiss(move || queue.finished(&weak));
}

/// Pushes the current resolved texts into a live alert — `text!` content
/// staying reactive while the dialog is up.
fn refresh_texts(state: &Rc<DialogState>) {
    let alert_slot = state.alert.borrow();
    let Some(alert) = alert_slot.as_ref() else {
        return;
    };
    let title = state.title.snapshot().to_semantic().to_string();
    alert.set_title(&title);
    let message = state
        .message
        .as_ref()
        .map(|message| message.snapshot().to_semantic().to_string());
    alert.set_message(message.as_deref());
}

/// Installs the `dialog` handler on the dispatcher: `Metadata<Dialog>` maps
/// to a transparent container that presents the platform alert while the
/// binding reads `true`.
pub fn install(dispatcher: &mut Dispatcher) {
    let queue = Rc::new(AlertQueue::default());
    dispatcher.register_view::<Metadata<Dialog>>(move |metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, KitRect::ZERO);
        let host_view: &PlatformView = &host;

        let mounted = ctx.render(metadata.content).mount(host_view);
        view::set_translates_autoresizing(mounted.view(), true);
        crate::primary_content::forward(&host, mounted.view());

        let dialog = metadata.value;
        let state = Rc::new(DialogState {
            mtm,
            host: host.clone(),
            queue: Rc::clone(&queue),
            child: mounted,
            env: ctx.env().clone(),
            is_presented: dialog.is_presented().clone(),
            title: dialog.title().resolve(ctx.env()).content,
            message: dialog
                .message_text()
                .map(|message| message.resolve(ctx.env()).content),
            dialog,
            alert: RefCell::new(None),
        });

        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });
        host.set_window_handler({
            let state = Rc::downgrade(&state);
            move |_| {
                if let Some(state) = state.upgrade() {
                    attachment_changed(&state);
                }
            }
        });
        let sink_guard = proposal::register_sink(host_view, {
            let state = Rc::clone(&state);
            move |selected| {
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            DialogSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(Rc::clone(&state));

        leaf.watch(&state.is_presented, {
            let state = Rc::downgrade(&state);
            move |ctx| {
                let Some(state) = state.upgrade() else {
                    return;
                };
                if *ctx.value() {
                    state.queue.request(&state);
                } else {
                    state.queue.withdraw(&state);
                }
            }
        });
        leaf.watch(&state.title, {
            let state = Rc::downgrade(&state);
            move |_ctx| {
                if let Some(state) = state.upgrade() {
                    refresh_texts(&state);
                }
            }
        });
        if let Some(message) = &state.message {
            leaf.watch(message, {
                let state = Rc::downgrade(&state);
                move |_ctx| {
                    if let Some(state) = state.upgrade() {
                        refresh_texts(&state);
                    }
                }
            });
        }

        if state.is_presented.snapshot() {
            state.queue.request(&state);
        }

        leaf
    });
}
