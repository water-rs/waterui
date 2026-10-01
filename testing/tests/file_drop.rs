//! Coverage for water-rs/cli#238: `SemanticApp<HeadlessRuntime>` dispatches
//! an OS file drop through the same input path a platform drag takes — the
//! host's pointer answer moves to the point, one `FileHovered` per path,
//! then one `FileDropped` per path — so a `drop_destination` accepting
//! `Files` observes the drag entering and receives every dropped path in
//! one payload.
//!
//! `hover_files_at` plus `cancel_files_hover` end the drag without a drop,
//! the hover-only half of the same input sequence.

use std::path::PathBuf;

use waterui::Binding;
use waterui::Signal as _;
use waterui::Url;
use waterui::ViewExt as _;
use waterui::component::text;
use waterui::drag_drop::{DropDestinationExt as _, Files};
use waterui_testing::{OffscreenApp, ui};

const CENTER: (f32, f32) = (180.0, 120.0);

fn dropped_paths() -> [PathBuf; 2] {
    [
        PathBuf::from("/tmp/report.pdf"),
        PathBuf::from("/tmp/photo.png"),
    ]
}

/// Mounts a drop destination covering the viewport that records every
/// delivered path, plus the `drop_hover` binding the drag's enter and exit
/// drive — the view's evidence a file drag reached it.
fn mount_destination() -> (OffscreenApp, Binding<Vec<PathBuf>>, Binding<bool>) {
    let received = Binding::container(Vec::<PathBuf>::new());
    let hovering = Binding::container(false);
    let recordings = received.clone();
    let hover_flag = hovering.clone();
    let mut app = ui()
        .theme(hydrolysis_m3::Material3::defaults())
        .viewport(360, 240)
        .mount_offscreen(move || {
            let recordings = recordings.clone();
            text("Drop files here")
                .width(360.0)
                .height(240.0)
                .drop_destination(move |files: Files| {
                    recordings.with_mut(|paths| {
                        paths.extend(files.urls().iter().filter_map(Url::to_file_path));
                    });
                })
                .drop_hover(&hover_flag)
        });
    app.settle();
    (app, received, hovering)
}

#[test]
fn drop_files_at_delivers_every_path_to_the_destination() {
    let (mut app, received, hovering) = mount_destination();
    app.drop_files_at(CENTER.0, CENTER.1, dropped_paths());
    assert_eq!(received.snapshot(), dropped_paths().to_vec());
    assert!(!hovering.snapshot());
}

#[test]
fn queued_drop_files_at_delivers_on_the_next_settle() {
    let (mut app, received, _) = mount_destination();
    app.queue_drop_files_at(CENTER.0, CENTER.1, dropped_paths());
    app.settle();
    assert_eq!(received.snapshot(), dropped_paths().to_vec());
}

#[test]
fn hover_then_cancel_files_hover_ends_the_drag_without_a_drop() {
    let (mut app, received, hovering) = mount_destination();
    app.hover_files_at(CENTER.0, CENTER.1, dropped_paths());
    assert!(hovering.snapshot());
    assert!(received.snapshot().is_empty());
    app.cancel_files_hover();
    assert!(!hovering.snapshot());
    assert!(received.snapshot().is_empty());
}
