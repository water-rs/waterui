//! water-rs/waterui#1254 and water-rs/hydrolysis#127 — typed drag payloads
//! and OS file drops.
//!
//! In-app drags carry the source's typed [`DragPayload`]; a drop destination
//! is hovered and delivered to only when it accepts the payload's kind. OS
//! file drops arrive from winit as one `HoveredFile`/`DroppedFile` event per
//! file and become a single [`Files`] payload, delivered once at the drop.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use waterui::drag_drop::{DropDestinationExt as _, Files, Transferable};
use waterui::reactive::impl_constant;
use waterui::{AnyView, Str, Url, ViewExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::stack::hstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const POINTER_ID: u64 = 11;
const SIZE: f32 = 160.0;

/// An application type: `TransferKind::of::<TabId>()` is `InProcess`, so the
/// payload travels within the process only.
#[derive(Debug, Clone, PartialEq)]
struct TabId(u64);

impl Transferable for TabId {}
impl_constant!(TabId);

fn runtime_with(view: AnyView) -> HeadlessRuntime {
    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        crate::num_cast::f32_as_u32(SIZE),
        crate::num_cast::f32_as_u32(SIZE),
        MinimalTestTheme::default(),
    );
    // Settle the tree so this frame's hit targets are registered.
    for _ in 0..4 {
        let _ = runtime.pump(false);
    }
    runtime
}

fn push(runtime: &mut HeadlessRuntime, event: InputEvent) {
    runtime.push_input_event(event);
    let _ = runtime.pump(false);
}

fn pointer_down(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    push(
        runtime,
        InputEvent::PointerDown {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
    );
}

fn pointer_move(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    push(
        runtime,
        InputEvent::PointerMove {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
        },
    );
}

fn pointer_up(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    push(
        runtime,
        InputEvent::PointerUp {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
    );
}

#[test]
fn in_process_typed_drop_is_delivered() {
    let dropped = Rc::new(RefCell::new(None::<TabId>));
    let dropped_target = Rc::clone(&dropped);
    let view = hstack((
        ().size(60.0, 60.0).draggable(TabId(7)),
        ().size(60.0, 60.0).drop_destination(move |id: TabId| {
            *dropped_target.borrow_mut() = Some(id);
        }),
    ))
    .spacing(20.0);
    let mut runtime = runtime_with(AnyView::new(view));

    pointer_down(&mut runtime, 30.0, 80.0);
    pointer_move(&mut runtime, 110.0, 80.0);
    pointer_up(&mut runtime, 110.0, 80.0);

    assert_eq!(*dropped.borrow(), Some(TabId(7)));
}

#[test]
fn non_matching_payload_is_neither_hovered_nor_delivered() {
    let calls = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let (on_drop, on_enter, on_exit) = (Rc::clone(&calls), Rc::clone(&calls), Rc::clone(&calls));
    // A `Str` destination under a `TabId` drag: nothing may reach it.
    let view = hstack((
        ().size(60.0, 60.0).draggable(TabId(7)),
        ().size(60.0, 60.0)
            .drop_destination(move |text: Str| {
                let _received = text;
                on_drop.borrow_mut().push("drop");
            })
            .on_enter(move || on_enter.borrow_mut().push("enter"))
            .on_exit(move || on_exit.borrow_mut().push("exit")),
    ))
    .spacing(20.0);
    let mut runtime = runtime_with(AnyView::new(view));

    pointer_down(&mut runtime, 30.0, 80.0);
    pointer_move(&mut runtime, 110.0, 80.0);
    pointer_move(&mut runtime, 30.0, 80.0);
    pointer_move(&mut runtime, 110.0, 80.0);
    pointer_up(&mut runtime, 110.0, 80.0);

    assert!(
        calls.borrow().is_empty(),
        "a destination that does not accept the payload is untouched: {:?}",
        calls.borrow()
    );
}

#[test]
fn os_file_drop_delivers_one_files_payload_with_every_url() {
    let file_a = PathBuf::from("/tmp/drop-a.png");
    // A non-UTF-8 path must not panic: `Url::from_file_path` carries it.
    let file_b = {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt as _;
            PathBuf::from(std::ffi::OsString::from_vec(
                b"/tmp/drop-\xFF\xFE.png".to_vec(),
            ))
        }
        #[cfg(not(unix))]
        PathBuf::from("C:\\drops\\drop-b.png")
    };
    let drops = Rc::new(RefCell::new(Vec::<Files>::new()));
    let enters = Rc::new(Cell::new(0u32));
    let exits = Rc::new(Cell::new(0u32));
    let view = {
        let (drops, enters, exits) = (Rc::clone(&drops), Rc::clone(&enters), Rc::clone(&exits));
        ().size(SIZE, SIZE)
            .drop_destination(move |files: Files| drops.borrow_mut().push(files))
            .on_enter(move || enters.set(enters.get() + 1))
            .on_exit(move || exits.set(exits.get() + 1))
    };
    let mut runtime = runtime_with(AnyView::new(view));

    // winit delivers one event per file; the pointer position the drop lands
    // at is the last cursor position it reported.
    pointer_move(&mut runtime, 80.0, 80.0);
    for event in [
        InputEvent::FileHovered {
            path: file_a.clone(),
        },
        InputEvent::FileHovered {
            path: file_b.clone(),
        },
        InputEvent::FileDropped {
            path: file_a.clone(),
        },
        InputEvent::FileDropped {
            path: file_b.clone(),
        },
    ] {
        runtime.push_input_event(event);
    }
    // The drop is delivered when the first drain adds no more files — winit
    // emits no drop-end event.
    let _ = runtime.pump(false);
    let _ = runtime.pump(false);

    assert_eq!(
        drops.borrow().as_slice(),
        &[Files::new([
            Url::from_file_path(&file_a),
            Url::from_file_path(&file_b),
        ])],
        "the drop delivers once, as one Files of both file URLs"
    );
    assert_eq!(enters.get(), 1, "the destination is hovered once");
    assert_eq!(exits.get(), 1, "the delivered destination exits once");
}

/// water-rs/hydrolysis#127 — the platforms that deliver no cursor events
/// while an OS drag owns the pointer (the OLE grab on Windows, an
/// `NSDraggingSession` on macOS) send winit's `HoveredFile`/`DroppedFile`
/// unaccompanied: the only position the runner can know for them is the
/// host's own answer. A drop the host can place must still land even when
/// the stream reported no position at all.
#[test]
fn os_file_drop_delivers_at_the_host_reported_position() {
    let file_a = PathBuf::from("/tmp/drop-a.png");
    let drops = Rc::new(RefCell::new(Vec::<Files>::new()));
    let enters = Rc::new(Cell::new(0u32));
    let view = {
        let (drops, enters) = (Rc::clone(&drops), Rc::clone(&enters));
        ().size(SIZE, SIZE)
            .drop_destination(move |files: Files| drops.borrow_mut().push(files))
            .on_enter(move || enters.set(enters.get() + 1))
    };
    let mut runtime = runtime_with(AnyView::new(view));

    // The host knows where the pointer is even though it delivered no
    // cursor event — exactly the OLE/AppKit situation winit's file events
    // arrive in.
    runtime.set_pointer_position(Some((80.0, 80.0)));
    for event in [
        InputEvent::FileHovered {
            path: file_a.clone(),
        },
        InputEvent::FileDropped {
            path: file_a.clone(),
        },
    ] {
        runtime.push_input_event(event);
    }
    let _ = runtime.pump(false);
    let _ = runtime.pump(false);

    assert_eq!(
        drops.borrow().as_slice(),
        &[Files::new([Url::from_file_path(&file_a)])],
        "the drop lands at the host's reported position even without cursor events"
    );
    assert_eq!(enters.get(), 1, "the destination is hovered once");
}

/// A drop's `DroppedFile` events straddling two pumps still merge into one
/// delivery carrying every file — the collected list lives on the drag, not
/// on the batch.
#[test]
fn os_file_drop_split_across_pumps_delivers_once_with_every_url() {
    let file_a = PathBuf::from("/tmp/split-a.png");
    let file_b = PathBuf::from("/tmp/split-b.png");
    let drops = Rc::new(RefCell::new(Vec::<Files>::new()));
    let view = {
        let drops = Rc::clone(&drops);
        ().size(SIZE, SIZE)
            .drop_destination(move |files: Files| drops.borrow_mut().push(files))
    };
    let mut runtime = runtime_with(AnyView::new(view));

    pointer_move(&mut runtime, 80.0, 80.0);
    for event in [
        InputEvent::FileHovered {
            path: file_a.clone(),
        },
        InputEvent::FileHovered {
            path: file_b.clone(),
        },
        InputEvent::FileDropped {
            path: file_a.clone(),
        },
    ] {
        runtime.push_input_event(event);
    }
    let _ = runtime.pump(false);
    runtime.push_input_event(InputEvent::FileDropped {
        path: file_b.clone(),
    });
    let _ = runtime.pump(false);
    let _ = runtime.pump(false);

    assert_eq!(
        drops.borrow().as_slice(),
        &[Files::new([
            Url::from_file_path(&file_a),
            Url::from_file_path(&file_b),
        ])],
        "the straddled drop delivers once, as one Files of both file URLs"
    );
}

/// A dropped drag must deliver even if no further input ever arrives: the
/// drain that collected the `DroppedFile`s marks the runtime unsettled and
/// requests the one follow-up pump that runs the delivering drain.
#[test]
fn os_file_drop_is_delivered_by_the_pump_the_runner_schedules() {
    let file_a = PathBuf::from("/tmp/scheduled-a.png");
    let file_b = PathBuf::from("/tmp/scheduled-b.png");
    let drops = Rc::new(RefCell::new(Vec::<Files>::new()));
    let view = {
        let drops = Rc::clone(&drops);
        ().size(SIZE, SIZE)
            .drop_destination(move |files: Files| drops.borrow_mut().push(files))
    };
    let mut runtime = runtime_with(AnyView::new(view));

    pointer_move(&mut runtime, 80.0, 80.0);
    for event in [
        InputEvent::FileHovered {
            path: file_a.clone(),
        },
        InputEvent::FileHovered {
            path: file_b.clone(),
        },
        InputEvent::FileDropped {
            path: file_a.clone(),
        },
        InputEvent::FileDropped {
            path: file_b.clone(),
        },
    ] {
        runtime.push_input_event(event);
    }
    // No further input is delivered — the drop only lands if the runner
    // scheduled the drain that delivers it, which `is_settled` reports.
    let mut pumps = 0;
    while !runtime.is_settled() && pumps < 8 {
        let _ = runtime.pump(false);
        pumps += 1;
    }
    assert!(
        runtime.is_settled(),
        "the runner-scheduled pump lands the drop without further input"
    );
    assert_eq!(
        drops.borrow().as_slice(),
        &[Files::new([
            Url::from_file_path(&file_a),
            Url::from_file_path(&file_b),
        ])],
        "the drop is delivered exactly once, with every file it carried"
    );
}

#[test]
fn os_file_drop_produces_no_text() {
    let calls = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let (on_drop, on_enter, on_exit) = (Rc::clone(&calls), Rc::clone(&calls), Rc::clone(&calls));
    let view = ()
        .size(SIZE, SIZE)
        .drop_destination(move |text: Str| {
            let _received = text;
            on_drop.borrow_mut().push("drop");
        })
        .on_enter(move || on_enter.borrow_mut().push("enter"))
        .on_exit(move || on_exit.borrow_mut().push("exit"));
    let mut runtime = runtime_with(AnyView::new(view));

    pointer_move(&mut runtime, 80.0, 80.0);
    for event in [
        InputEvent::FileHovered {
            path: PathBuf::from("/tmp/drop.txt"),
        },
        InputEvent::FileDropped {
            path: PathBuf::from("/tmp/drop.txt"),
        },
    ] {
        runtime.push_input_event(event);
    }
    let _ = runtime.pump(false);
    // The deferred delivery point must be crossed for the assertion to mean
    // the destination rejected the payload rather than the drop never ran.
    let _ = runtime.pump(false);

    assert!(
        calls.borrow().is_empty(),
        "a file drop carries Files, not Str — the text destination stays untouched: {:?}",
        calls.borrow()
    );
}
