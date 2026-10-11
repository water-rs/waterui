//! The `drop_destination` metadata: `Metadata<DropDestination>` wrapped
//! around a child.
//!
//! Mirrors `WuiDropDestination`: a transparent `HostView` container —
//! measure, stretch and placement all answer for the mounted child — that
//! registers for the pasteboard types its `TransferKind` maps to and
//! delivers dropped values to the destination's handler. A same-process
//! `WaterUI` drag carries its typed `DragPayload` on the drag item (iOS) or
//! the dragging source (macOS); the FFI `accepts` check discriminates
//! `InProcess` type identity, so those payloads deliver unserialized. Every
//! other drag is judged by the pasteboard types it offers and rebuilt into
//! a fresh `DragPayload` (`Str`, `Url` or `Files`) on delivery.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use cocoa_ui::Rect;
use cocoa_ui::view;
use waterui::drag_drop::{DragPayload, DropDestination, Files, TransferKind};
use waterui::{Str, Url};
use waterui_backend_core::Environment;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, drag_drop as kit};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HostView, drag_drop as kit};

#[cfg(target_os = "macos")]
/// The pasteboard type a process-scoped drag's marker item carries — the
/// same identifier `draggable` writes.
const IN_PROCESS_TYPE: &str = "dev.waterui.inProcessDragPayload";

/// The leaf's live state: the mounted child the layout face forwards to,
/// the destination accepting drops, and the environment the handlers run
/// in.
struct DropLeafState {
    /// The mounted content.
    child: Mounted,
    /// The destination; `deliver`, `enter` and `exit` take `&mut`, so it
    /// sits behind the shared `RefCell`.
    destination: DropDestination,
    /// The environment `deliver`, `enter` and `exit` receive.
    env: Environment,
}

impl core::fmt::Debug for DropLeafState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DropLeafState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: the content's own answers everywhere.
struct DropSubView {
    /// The leaf's state.
    state: Rc<RefCell<DropLeafState>>,
}

impl core::fmt::Debug for DropSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DropSubView").finish_non_exhaustive()
    }
}

impl SubView for DropSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.borrow().child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.borrow().child.layout().priority()
    }
}

/// The `DragPayload` a same-process drag carries, when it is one.
fn shared_payload(any: &alloc::rc::Rc<dyn core::any::Any>) -> Option<DragPayload> {
    any.downcast_ref::<DragPayload>().cloned()
}

/// Whether `payload` is accepted; the check includes `InProcess` type
/// identity, so same-process payload handles must go through it.
fn accepts(state: &RefCell<DropLeafState>, payload: &DragPayload) -> bool {
    state.borrow().destination.accepts(payload)
}

/// Delivers a payload the destination accepts.
fn deliver(state: &RefCell<DropLeafState>, payload: DragPayload) {
    let mut state = state.borrow_mut();
    let env = state.env.clone();
    state.destination.deliver(payload, &env);
}

/// Reports an accepted drag entering the destination's bounds.
fn call_enter(state: &RefCell<DropLeafState>) {
    let mut state = state.borrow_mut();
    let env = state.env.clone();
    state.destination.enter(&env);
}

/// Reports an accepted drag leaving the destination's bounds.
fn call_exit(state: &RefCell<DropLeafState>) {
    let mut state = state.borrow_mut();
    let env = state.env.clone();
    state.destination.exit(&env);
}

/// Rebuilds a payload of `kind` from dropped platform text — `None` when
/// the text does not parse for the kind.
fn payload_from_strings(
    accepted_kind: TransferKind,
    text: Option<alloc::string::String>,
    file_urls: &[alloc::string::String],
) -> Option<DragPayload> {
    match accepted_kind {
        TransferKind::Text => text.map(|text| DragPayload::new(Str::from(text))),
        TransferKind::Url => text
            .and_then(|text| text.parse::<Url>().ok())
            .map(DragPayload::new),
        TransferKind::Files if !file_urls.is_empty() => Some(DragPayload::new(Files::new(
            file_urls.iter().filter_map(|url| url.parse::<Url>().ok()),
        ))),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
/// The pasteboard types `accepted_kind` registers for on macOS, as UTI
/// strings: text, URL and file URL map onto the standard types; `InProcess`
/// registers for the private marker only.
fn accepted_types(accepted_kind: TransferKind) -> Vec<&'static str> {
    match accepted_kind {
        TransferKind::Text => vec!["public.utf8-plain-text"],
        TransferKind::Url => vec!["public.url"],
        TransferKind::Files => vec!["public.file-url"],
        TransferKind::InProcess(_) => vec![IN_PROCESS_TYPE],
    }
}

#[cfg(target_os = "macos")]
/// Whether the drag over the view is acceptable — a same-process source's
/// typed payload through `accepts`, anything else by pasteboard types.
fn is_accepted(info: &kit::DragInfo<'_>, state: &RefCell<DropLeafState>) -> bool {
    if let Some(any) = info.source_payload()
        && let Some(payload) = shared_payload(&any)
    {
        return accepts(state, &payload);
    }
    let accepted_kind = state.borrow().destination.accepted_kind();
    accepted_types(accepted_kind)
        .iter()
        .any(|type_identifier| info.has_type(type_identifier))
        && accepted_kind.is_platform()
}

#[cfg(target_os = "ios")]
/// The payload of an accepted same-process drag item, if any.
fn accepted_local_payload(
    session: &kit::DropSession<'_>,
    state: &RefCell<DropLeafState>,
) -> Option<DragPayload> {
    session
        .local_payloads()
        .into_iter()
        .filter_map(|any| shared_payload(&any))
        .find(|payload| accepts(state, payload))
}

#[cfg(target_os = "ios")]
/// Whether the session's drag is acceptable, mirroring
/// `WuiDropDestination.isAcceptable`.
fn is_acceptable(session: &kit::DropSession<'_>, state: &RefCell<DropLeafState>) -> bool {
    if accepted_local_payload(session, state).is_some() {
        return true;
    }
    match state.borrow().destination.accepted_kind() {
        TransferKind::Text => session.can_load_strings(),
        TransferKind::Url => session.can_load_urls(),
        TransferKind::Files => session.has_file_urls(),
        TransferKind::InProcess(_) => false,
    }
}

/// Installs the `drop_destination` handler on the dispatcher.
#[expect(
    clippy::too_many_lines,
    reason = "one install wires both platforms' drop delegates, mirroring the baseline delegate extension"
)]
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<DropDestination>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(RefCell::new(DropLeafState {
            child: mounted,
            destination: metadata.value,
            env: ctx.env().clone(),
        }));

        // The content always fills the wrapper — `contentView.frame = bounds`.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                let state = state.borrow();
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });

        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            move |selected| {
                let state = state.borrow();
                proposal::deliver(state.child.view(), selected);
            }
        });

        #[cfg(target_os = "macos")]
        {
            use cocoa_ui::Retained;
            use cocoa_ui::objc2_app_kit::NSDragOperation;
            use objc2_foundation::NSString;

            let accepted_kind = state.borrow().destination.accepted_kind();
            let types: Vec<Retained<NSString>> = accepted_types(accepted_kind)
                .iter()
                .map(|type_identifier| NSString::from_str(type_identifier))
                .collect();
            let types: Vec<&NSString> = types.iter().map(|t| &**t).collect();
            host.set_drop_handlers(
                &types,
                Some(kit::DropHandlers {
                    entered: {
                        let state = Rc::clone(&state);
                        Rc::new(move |info| {
                            if is_accepted(info, &state) {
                                call_enter(&state);
                                NSDragOperation::Copy
                            } else {
                                NSDragOperation::None
                            }
                        })
                    },
                    updated: {
                        let state = Rc::clone(&state);
                        Rc::new(move |info| {
                            if is_accepted(info, &state) {
                                NSDragOperation::Copy
                            } else {
                                NSDragOperation::None
                            }
                        })
                    },
                    exited: {
                        let state = Rc::clone(&state);
                        Rc::new(move |info| {
                            if is_accepted(info, &state) {
                                call_exit(&state);
                            }
                        })
                    },
                    perform: {
                        let state = Rc::clone(&state);
                        Rc::new(move |info| {
                            if let Some(any) = info.source_payload()
                                && let Some(payload) = shared_payload(&any)
                                && accepts(&state, &payload)
                            {
                                deliver(&state, payload);
                                return true;
                            }
                            let accepted_kind = state.borrow().destination.accepted_kind();
                            let text = match accepted_kind {
                                TransferKind::Text => info
                                    .string("public.utf8-plain-text")
                                    .map(|text| text.to_string()),
                                TransferKind::Url => {
                                    info.string("public.url").map(|text| text.to_string())
                                }
                                _ => None,
                            };
                            let Some(payload) =
                                payload_from_strings(accepted_kind, text, &info.file_urls())
                            else {
                                return false;
                            };
                            deliver(&state, payload);
                            true
                        })
                    },
                }),
            );
        }

        #[cfg(target_os = "ios")]
        let drop_target = kit::drop_target(
            &host,
            kit::DropHandlers {
                can_handle: {
                    let state = Rc::clone(&state);
                    Rc::new(move |session| is_acceptable(session, &state))
                },
                entered: {
                    let state = Rc::clone(&state);
                    Rc::new(move |session| {
                        if is_acceptable(session, &state) {
                            call_enter(&state);
                        }
                    })
                },
                update: {
                    let state = Rc::clone(&state);
                    Rc::new(move |session| {
                        if is_acceptable(session, &state) {
                            kit::DropOperation::Copy
                        } else {
                            kit::DropOperation::Forbidden
                        }
                    })
                },
                exited: {
                    let state = Rc::clone(&state);
                    Rc::new(move |session| {
                        if is_acceptable(session, &state) {
                            call_exit(&state);
                        }
                    })
                },
                perform: {
                    let state = Rc::clone(&state);
                    Rc::new(move |session| {
                        if let Some(payload) = accepted_local_payload(session, &state) {
                            deliver(&state, payload);
                            return;
                        }
                        let accepted_kind = state.borrow().destination.accepted_kind();
                        match accepted_kind {
                            TransferKind::Files if session.has_file_urls() => {
                                let state = Rc::clone(&state);
                                session.load_urls(move |urls| {
                                    let file_urls: Vec<_> = urls
                                        .into_iter()
                                        .filter(|url| url.starts_with("file://"))
                                        .collect();
                                    if let Some(payload) =
                                        payload_from_strings(accepted_kind, None, &file_urls)
                                    {
                                        deliver(&state, payload);
                                    }
                                });
                            }
                            TransferKind::Url if session.can_load_urls() => {
                                let state = Rc::clone(&state);
                                session.load_urls(move |urls| {
                                    if let Some(payload) = payload_from_strings(
                                        accepted_kind,
                                        urls.into_iter().next(),
                                        &[],
                                    ) {
                                        deliver(&state, payload);
                                    }
                                });
                            }
                            TransferKind::Text if session.can_load_strings() => {
                                let state = Rc::clone(&state);
                                session.load_strings(move |strings| {
                                    let text =
                                        strings.first().map(std::string::ToString::to_string);
                                    if let Some(payload) =
                                        payload_from_strings(accepted_kind, text, &[])
                                    {
                                        deliver(&state, payload);
                                    }
                                });
                            }
                            _ => {}
                        }
                    })
                },
            },
        );

        let mut leaf = NativeLeaf::new(
            &*host,
            DropSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        #[cfg(target_os = "ios")]
        leaf.keep(drop_target);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::drag_drop::PlatformRepresentation;

    #[test]
    #[cfg(target_os = "macos")]
    fn accepted_types_map_each_kind_to_its_pasteboard_type() {
        assert_eq!(
            accepted_types(TransferKind::Text),
            ["public.utf8-plain-text"]
        );
        assert_eq!(accepted_types(TransferKind::Url), ["public.url"]);
        assert_eq!(accepted_types(TransferKind::Files), ["public.file-url"]);
        assert_eq!(
            accepted_types(TransferKind::InProcess(core::any::TypeId::of::<u8>())),
            [IN_PROCESS_TYPE]
        );
    }

    #[test]
    fn payload_from_strings_builds_the_accepted_kind() {
        let text =
            payload_from_strings(TransferKind::Text, Some("hi".into()), &[]).expect("text payload");
        assert!(matches!(
            text.platform_representation(),
            PlatformRepresentation::Text(value) if value.as_str() == "hi"
        ));

        let url =
            payload_from_strings(TransferKind::Url, Some("https://example.com/a".into()), &[])
                .expect("url payload");
        assert!(matches!(
            url.platform_representation(),
            PlatformRepresentation::Url(value) if value.as_str() == "https://example.com/a"
        ));

        let files = payload_from_strings(
            TransferKind::Files,
            None,
            &["file:///tmp/a.png".into(), "file:///tmp/b.png".into()],
        )
        .expect("files payload");
        assert!(matches!(
            files.platform_representation(),
            PlatformRepresentation::Files(files) if files.urls().len() == 2
        ));

        assert!(payload_from_strings(TransferKind::Files, None, &[]).is_none());
        assert!(payload_from_strings(TransferKind::Url, None, &[]).is_none());
        assert!(
            payload_from_strings(
                TransferKind::InProcess(core::any::TypeId::of::<u8>()),
                Some("anything".into()),
                &[],
            )
            .is_none()
        );
    }
}
