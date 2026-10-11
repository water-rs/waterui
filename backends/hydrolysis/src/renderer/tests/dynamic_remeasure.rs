//! Row re-measurement regression test for water-rs/hydrolysis#210.
//!
//! A `List` row whose content hangs off a `Dynamic` — the reproduction's
//! `when(has_photo, …)` arm lands on one — must re-measure when an
//! async-decoded image inside it gets its natural size. Completing the decode
//! publishes new dimensions into the retained `DynamicHostNode` child in
//! place — no fresh view is staged — so the row's next transient measure must
//! consult that child's live answer, not the size the `Dynamic` measured and
//! cached before the decode landed.

use core::time::Duration;
use std::time::Instant;

use accesskit::{Rect, Role, TreeUpdate};
use nami::Binding;
use nami::collection::SignalCollection;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_core::{AnyView, Dynamic};
use waterui_image::{Image, ReactiveImageHandle, reactive_image};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::vstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

/// The reproduction's viewport: the issue mounts the chat at 400x700.
const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 700;
/// `MinimalTestTheme`'s one-line row floor — where the row rests while the
/// image still reports no intrinsic size.
const ROW_FLOOR: f64 = 56.0;
/// The frame the deferred decode lands, sized inside the row width.
const IMAGE_WIDTH: u32 = 320;
const IMAGE_HEIGHT: u32 = 240;
/// The row's own insets: 8pt above and below the photo.
const ROW_INSETS: EdgeInsets = EdgeInsets::symmetric(8.0, 12.0);
/// Slack on the grown-row assertion for the caption line the row carries
/// beside the photo — the invariant is "row = content + insets", not a magic
/// caption height.
const CAPTION_SLACK: f64 = 40.0;

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

/// Pumps until the runtime reports quiet — never fewer than `min_frames` —
/// then returns the settled merged tree (as the vec `node_bounds` reads).
fn settle(runtime: &mut HeadlessRuntime, at: &mut Instant, min_frames: u32) -> Vec<TreeUpdate> {
    let mut frame = 0;
    loop {
        frame += 1;
        *at += Duration::from_millis(16);
        let _ = runtime.pump_at(false, *at);
        if frame >= min_frames
            && (frame >= 300 || (runtime.is_settled() && !runtime.has_pending_semantic_update()))
        {
            break;
        }
    }
    runtime.accessibility_tree().into_iter().collect()
}

/// The bounds the settled tree carries for the node with `role` and derived
/// `label` — `None` when the node is absent.
fn node_bounds(updates: &[TreeUpdate], role: Role, label: &str) -> Option<Rect> {
    updates.iter().rev().find_map(|update| {
        update.nodes.iter().find_map(|(_, node)| {
            if node.role() == role && node.label() == Some(label) {
                node.bounds()
            } else {
                None
            }
        })
    })
}

/// A one-row list whose row content is the issue's `when(has_photo, …)` arm: a
/// `Dynamic` subtree holding a `ReactiveImage` that is still decoding, beside
/// a caption so the row measures (and thus retains) its subtree before the
/// decode lands — a row that is all image never builds a retained child while
/// the image reports zero size. The returned handle publishes the decoded
/// frame — the exact call the image pipeline makes when the decoder lands.
fn photo_row_list() -> (ReactiveImageHandle, AnyViewBuilder<AnyView>) {
    let (decode, photo) = reactive_image();
    let (set_content, dynamic) = Dynamic::new();
    set_content.set(vstack((
        text("a photo"),
        photo
            .a11y_role(AccessibilityRole::Image)
            .a11y_label("photo"),
    )));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let dynamic = dynamic.clone();
        let rows = Binding::container(vec![SelfId::new(1_u64)]);
        AnyView::new(List::for_each(SignalCollection::new(rows), move |_| {
            ListItem::new(dynamic.clone().a11y_label("photo row")).insets(ROW_INSETS)
        }))
    });
    (decode, builder)
}

/// The issue's reproduction: the row measures while the photo is still
/// decoding, so its extent caches the zero-size image; completing the decode
/// must re-measure the row to the image's natural size, and the image must not
/// paint outside the row that holds it.
#[test]
fn list_row_remeasures_when_async_image_decode_lands() {
    let (decode, builder) = photo_row_list();
    let mut runtime = runtime(builder);
    let mut at = Instant::now();

    let updates = settle(&mut runtime, &mut at, 4);
    let initial =
        node_bounds(&updates, Role::ListItem, "photo row").expect("the row must publish bounds");
    assert!(
        (initial.height() - ROW_FLOOR).abs() < 2.0,
        "an undecoded image has no size, so the row sits on the one-line floor \
         (got {:.1})",
        initial.height()
    );

    // The decode completes: the `ReactiveImage` publishes the frame's pixel
    // size in place — the `Dynamic` stages no replacement view.
    decode.set(Image::new(
        vec![0xFF; IMAGE_WIDTH as usize * IMAGE_HEIGHT as usize * 4],
        IMAGE_WIDTH,
        IMAGE_HEIGHT,
    ));
    let updates = settle(&mut runtime, &mut at, 4);
    let grown = node_bounds(&updates, Role::ListItem, "photo row")
        .expect("the row must publish bounds after the decode");
    let photo =
        node_bounds(&updates, Role::Image, "photo").expect("the decoded image must publish bounds");
    assert!(
        grown.height() >= f64::from(IMAGE_HEIGHT) + 16.0
            && grown.height() <= f64::from(IMAGE_HEIGHT) + 16.0 + CAPTION_SLACK,
        "the row must re-measure to the caption plus the image's height plus \
         the row's insets (got {:.1})",
        grown.height()
    );
    assert!(
        photo.y0 >= grown.y0 - 0.5 && photo.y1 <= grown.y1 + 0.5,
        "the image must not paint outside the row (image {:.1}..{:.1}, row \
         {:.1}..{:.1})",
        photo.y0,
        photo.y1,
        grown.y0,
        grown.y1
    );
}
