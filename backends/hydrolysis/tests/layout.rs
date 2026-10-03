//! Renderer presentation tests for layout geometry: stack bounds
//! relationships, layout priority, growing-child heights, snackbar width
//! bounds, and text line limits.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation); every case names its origin file and asserts what
//! it asserted there, mounted under `Material3::defaults()` on the rendered
//! runtime.

use std::time::Duration;

use waterui::View;
use waterui::graphics::color::Srgb;
use waterui::prelude::*;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarTheme};
use waterui_testing::{OffscreenApp, Role, Styled, UiBuilder, ui as test_ui};

fn visual_shell<V: View>(content: V) -> impl View {
    content.padding_with(20.0).background(Srgb::BLACK)
}

fn labeled_card(label: &'static str, width: f32, height: f32, color: Srgb) -> impl View {
    text(label)
        .body()
        .foreground(Srgb::WHITE)
        .size(width, height)
        .background(color)
}

fn vstack_view() -> impl View {
    visual_shell(
        vstack((
            labeled_card("Upper card", 80.0, 40.0, Srgb::new(1.0, 0.1, 0.1)),
            labeled_card("Lower card", 80.0, 40.0, Srgb::new(0.1, 1.0, 0.1)),
        ))
        .spacing(12.0),
    )
}

// Origin: waterui `testing/tests/layout.rs`.
#[waterui::test(vstack_view, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (180, 180))]
fn vstack_renders_children_vertically(app: &mut OffscreenApp) {
    let upper = app.query().role(Role::LABEL).label("Upper card").single();
    let lower = app.query().role(Role::LABEL).label("Lower card").single();
    assert!(
        upper.bounds().y() + upper.bounds().height() <= lower.bounds().y(),
        "vstack should place the upper card above the lower card: upper={:?} lower={:?}",
        upper.bounds(),
        lower.bounds()
    );
}

fn hstack_view() -> impl View {
    visual_shell(
        hstack((
            labeled_card("Left card", 40.0, 80.0, Srgb::new(1.0, 0.1, 0.1)),
            labeled_card("Right card", 40.0, 80.0, Srgb::new(0.1, 1.0, 0.1)),
        ))
        .spacing(12.0),
    )
}

// Origin: waterui `testing/tests/layout.rs`.
#[waterui::test(hstack_view, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (180, 180))]
fn hstack_renders_children_horizontally(app: &mut OffscreenApp) {
    let left = app.query().role(Role::LABEL).label("Left card").single();
    let right = app.query().role(Role::LABEL).label("Right card").single();
    assert!(
        left.bounds().x() + left.bounds().width() <= right.bounds().x(),
        "hstack should place the left card before the right card: left={:?} right={:?}",
        left.bounds(),
        right.bounds()
    );
}

fn zstack_view() -> impl View {
    visual_shell(zstack((
        labeled_card("Background layer", 120.0, 120.0, Srgb::new(1.0, 0.1, 0.1)),
        labeled_card("Overlay layer", 60.0, 60.0, Srgb::new(0.1, 1.0, 0.1)),
    )))
}

// Origin: waterui `testing/tests/layout.rs`.
#[waterui::test(zstack_view, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (180, 180))]
fn zstack_overlays_children(app: &mut OffscreenApp) {
    let background = app
        .query()
        .role(Role::LABEL)
        .label("Background layer")
        .single();
    let overlay = app
        .query()
        .role(Role::LABEL)
        .label("Overlay layer")
        .single();
    let background_center = background.center();
    let overlay_center = overlay.center();
    assert!(
        (background_center.0 - overlay_center.0).abs() <= 1.0
            && (background_center.1 - overlay_center.1).abs() <= 1.0,
        "zstack should center overlay and background together: background={:?} overlay={:?}",
        background.bounds(),
        overlay.bounds()
    );
}

/// Two texts too wide for the row. Without a priority they share the shortfall;
/// the prioritized one should keep its width while the other gives way.
fn layout_priority_view() -> impl View {
    visual_shell(
        hstack((
            text("Keep me whole")
                .body()
                .foreground(Srgb::WHITE)
                .background(Srgb::new(1.0, 0.1, 0.1))
                .layout_priority(1),
            text("I can shrink")
                .body()
                .foreground(Srgb::WHITE)
                .background(Srgb::new(0.1, 1.0, 0.1)),
        ))
        .spacing(0.0),
    )
}

// Origin: waterui `testing/tests/layout.rs`.
#[waterui::test(layout_priority_view, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (160, 120))]
fn layout_priority_protects_the_prioritized_child(app: &mut OffscreenApp) {
    let kept = app
        .query()
        .role(Role::LABEL)
        .label("Keep me whole")
        .single();
    let yielded = app.query().role(Role::LABEL).label("I can shrink").single();

    assert!(
        kept.bounds().width() > yielded.bounds().width(),
        "the prioritized child must keep more width than the one that gives way: \
         kept={:?} yielded={:?}",
        kept.bounds(),
        yielded.bounds()
    );
}

/// A list under a header must render its rows. A column that left its growing
/// child out of its own height handed the list nothing to draw in, which is how
/// the Tasks pane lost its table.
///
/// Origin: waterui `components/devtools/inspector/app/tests/layout_reproducers.rs`.
#[test]
fn a_list_under_a_header_renders_its_rows() {
    let rows = ["FramePump", "VideoTick", "LayoutPass"];

    let mut app: OffscreenApp = test_ui()
        .viewport(600, 400)
        .theme(hydrolysis_m3::Material3::defaults())
        .mount_offscreen(move || {
            vstack((
                text("Task").caption(),
                vstack(
                    rows.iter()
                        .map(|row| text(*row).anyview())
                        .collect::<Vec<_>>(),
                )
                .alignment(HorizontalAlignment::Leading)
                .spacing(8.0),
            ))
            .alignment(HorizontalAlignment::Leading)
            .spacing(12.0)
        });

    for row in ["FramePump", "VideoTick", "LayoutPass"] {
        app.query().role(Role::LABEL).label(row).assert_exists();
    }
}

// ============================================================================
// Snackbar container width bounds — origin: waterui `tests/snackbar_layout.rs`
// ============================================================================

/// The width bounds these tests pin the bar to, distinctive on purpose so an
/// ambient theme silently overriding them shows up as a failed assertion.
const MIN_WIDTH: f32 = 300.0;
const MAX_WIDTH: f32 = 600.0;

/// Mounts an app whose window shows `snackbar`, and returns it settled.
fn show(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
    snackbar: Snackbar,
) -> (OffscreenApp, SnackbarManager) {
    let (manager, overlay) = SnackbarManager::new();
    let theme = SnackbarTheme {
        min_width: MIN_WIDTH,
        max_width: MAX_WIDTH,
        ..SnackbarTheme::default()
    };
    let mut env = Environment::new();
    env.insert(manager.clone());
    env.insert(theme);
    let mut app = ui
        .environment(env)
        .mount_offscreen(move || zstack((text("app content"), overlay.clone())));
    manager.show(snackbar);
    app.settle();
    (app, manager)
}

/// A closeable bar with a short message sits at the `min_width` floor with its
/// close control pinned to the trailing edge — nowhere near the `max_width`
/// cap it used to stretch to.
///
/// Origin: waterui `tests/snackbar_layout.rs`.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (900, 600))]
fn a_short_closeable_snackbar_hugs_the_min_width_floor(
    ui: UiBuilder<Styled<hydrolysis_m3::Material3>>,
) {
    let (mut app, _manager) = show(
        ui,
        Snackbar::new("Saved").duration(Duration::ZERO).closeable(),
    );

    let message = app.query().label("Saved").single().bounds();
    let close = app.query().label("Close").single().bounds();

    let span = close.x() + close.width() - message.x();
    assert!(
        span <= MIN_WIDTH,
        "the bar's content spans {span} logical pixels, wider than the \
         {MIN_WIDTH} floor — it is still stretching toward the {MAX_WIDTH} cap"
    );
    assert!(
        span > MIN_WIDTH * 0.7,
        "the close control sits {span} from the message's leading edge; the \
         spacer should pin it near the {MIN_WIDTH} floor's trailing edge"
    );
}

/// A plain bar hugs its message rather than stretching to the cap.
///
/// Origin: waterui `tests/snackbar_layout.rs`.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (900, 600))]
fn a_plain_snackbar_hugs_its_message(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let (mut app, _manager) = show(
        ui,
        Snackbar::new("Copied to clipboard").duration(Duration::ZERO),
    );

    let message = app.query().label("Copied to clipboard").single().bounds();
    assert!(
        message.width() < MIN_WIDTH,
        "a short message stays narrower than the bar's own floor"
    );
}

// ============================================================================
// Text line limits — origin: waterui `tests/text_line_limit.rs`
// ============================================================================

const LONG: &str = "A label long enough to wrap into several lines at this width";

/// A limited text reserves height for its visible lines only.
///
/// Origin: waterui `tests/text_line_limit.rs`.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (600, 600))]
fn a_line_limit_caps_the_reserved_height(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let mut app = ui.mount_offscreen(|| {
        vstack((
            text(LONG).a11y_label("unlimited").width(150.0),
            text(LONG)
                .line_limit(core::num::NonZeroUsize::MIN)
                .a11y_label("limited")
                .width(150.0),
        ))
    });

    let unlimited = app.query().label("unlimited").single().bounds();
    let limited = app.query().label("limited").single().bounds();
    assert!(
        unlimited.height() > limited.height() * 2.0,
        "the unlimited text wraps ({}) while the limited one stays one line ({})",
        unlimited.height(),
        limited.height(),
    );
}

/// Compressed buttons keep their labels on one line instead of folding them
/// into paragraphs — the webview example's toolbar rendered "Back" as
/// "Bac / k" before button labels defaulted to a single truncated line.
///
/// Origin: waterui `tests/text_line_limit.rs`.
#[waterui::test(theme = hydrolysis_m3::Material3::defaults(), viewport = (600, 600))]
fn compressed_button_labels_stay_on_one_line(ui: UiBuilder<Styled<hydrolysis_m3::Material3>>) {
    let mut app = ui.mount_offscreen(|| {
        hstack((
            button("Back").action(|| {}),
            button("Forward").action(|| {}),
            button("Reload").action(|| {}),
            button("Stop").action(|| {}),
        ))
        .width(250.0)
    });

    let mut heights = Vec::new();
    for label in ["Back", "Forward", "Reload", "Stop"] {
        heights.push(app.query().label(label).single().bounds().height());
    }
    let min = heights.iter().copied().fold(f32::INFINITY, f32::min);
    let max = heights.iter().copied().fold(0.0_f32, f32::max);
    assert!(
        (max - min) < 1.0,
        "buttons disagree about line count, so a label folded: {heights:?}"
    );
}

/// A text field is a `Horizontal` leaf: inside an hstack offered 340 wide,
/// two fields split the row instead of overflowing at the 280-pt intrinsic
/// floor. water-rs/hydrolysis#107.
fn text_field_row_view() -> impl View {
    let first = Binding::container(Str::from(""));
    let last = Binding::container(Str::from(""));
    hstack((field("First name", &first), field("Last name", &last))).size(340.0, 60.0)
}

#[waterui::test(text_field_row_view, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (400, 120))]
fn text_fields_share_a_340_row(app: &mut OffscreenApp) {
    let first = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("First name")
        .single()
        .bounds();
    let last = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Last name")
        .single()
        .bounds();
    let half = (340.0_f32 - 10.0) / 2.0;
    assert!(
        (first.width() - half).abs() <= 1.0 && (last.width() - half).abs() <= 1.0,
        "each field should get about half of the row: first={first:?} last={last:?}"
    );
    assert!(
        first.x() + first.width() <= last.x(),
        "fields should share the row without overlapping: first={first:?} last={last:?}"
    );
    assert!(
        last.x() + last.width() - first.x() <= 340.0,
        "fields should stay inside the 340-wide row: first={first:?} last={last:?}"
    );
}

/// With no width proposal on its stretch axis the field keeps its intrinsic
/// width — the theme's ideal `min_width` (280 under Material 3), not a hard
/// floor. A horizontal scroll is the container that measures content at
/// `None` on the scrolling axis; a viewport narrower than 280 keeps the field
/// at its ideal and lets it overflow into scrollable content.
fn lone_text_field_view() -> impl View {
    let value = Binding::container(Str::from(""));
    scroll_horizontal(field("Field", &value))
}

#[waterui::test(lone_text_field_view, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (200, 120))]
fn lone_text_field_keeps_ideal_width(app: &mut OffscreenApp) {
    let field = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Field")
        .single()
        .bounds();
    assert!(
        (field.width() - 280.0).abs() <= 1.0,
        "an unproposed field should keep the 280-pt ideal width: {field:?}"
    );
}

/// In a 600-wide container the field answers the proposal and fills it.
fn text_field_in_container_view() -> impl View {
    let value = Binding::container(Str::from(""));
    field("Field", &value).size(600.0, 60.0)
}

#[waterui::test(text_field_in_container_view, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (640, 120))]
fn text_field_fills_wide_container(app: &mut OffscreenApp) {
    let field = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Field")
        .single()
        .bounds();
    assert!(
        (field.width() - 600.0).abs() <= 1.0,
        "a field should fill its 600-wide container: {field:?}"
    );
}

/// The conversation row from water-rs/waterui#1219: a one-line preview that
/// truncates in the second row. The truncated leaf reports the width it
/// drew — its laid-out line including the ellipsis — so what it declines
/// reaches the row's spacer and the trailing badge ends on the row's
/// trailing edge, where the timestamp in the row above already lands.
fn compressed_row_view() -> impl View {
    hstack((
        text("RC").padding_with(44.0),
        vstack((
            hstack((
                text("Ada").a11y_label("title"),
                spacer(),
                text("14:32").a11y_label("stamp"),
            ))
            .spacing(4.0),
            hstack((
                text("On my way — see you at the gate in about ten minutes")
                    .caption()
                    .line_limit(core::num::NonZeroUsize::MIN)
                    .a11y_label("preview"),
                spacer(),
                text("2").caption().a11y_label("badge"),
            ))
            .spacing(4.0),
        ))
        .spacing(2.0)
        .leading(),
    ))
    .spacing(10.0)
    .padding_with((6.0, 10.0))
}

#[waterui::test(compressed_row_view, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (340, 200))]
fn a_compressed_limited_text_leaves_its_spacer_nothing(app: &mut OffscreenApp) {
    let stamp = app.query().label("stamp").single().bounds();
    let badge = app.query().label("badge").single().bounds();
    // The outer padding insets horizontally by 10, so the trailing edge of
    // either stretched row lands at 340 - 10.
    let trailing = 330.0;
    assert!(
        (stamp.x() + stamp.width() - trailing).abs() <= 1.0,
        "the timestamp should end on the row's trailing edge: {stamp:?}"
    );
    assert!(
        (badge.x() + badge.width() - trailing).abs() <= 1.0,
        "the badge should end on the row's trailing edge: {badge:?}"
    );
}

/// water-rs/hydrolysis#141 end to end: a line-limited text beside a trailing
/// sibling compresses into its share and draws the truncation mark — three
/// ink dots riding the baseline at the leaf's trailing edge — while the
/// sibling keeps the width it takes when the row has room to spare.
fn truncated_row_view() -> impl View {
    vstack((
        hstack((
            text("On my way — see you at the gate in about ten minutes")
                .line_limit(core::num::NonZeroUsize::MIN)
                .a11y_label("preview"),
            text("2").a11y_label("tight"),
        ))
        .spacing(4.0),
        hstack((text("x").a11y_label("short"), text("2").a11y_label("free"))).spacing(4.0),
    ))
    .spacing(8.0)
    .leading()
    .padding_with((6.0, 10.0))
}

#[waterui::test(truncated_row_view, theme = hydrolysis_m3::Material3::defaults(), offscreen, viewport = (220, 120))]
fn a_limited_text_truncates_with_an_ellipsis_beside_its_sibling(app: &mut OffscreenApp) {
    let preview = app.query().label("preview").single().bounds();
    let tight = app.query().label("tight").single().bounds();
    let free = app.query().label("free").single().bounds();

    assert!(
        (tight.width() - free.width()).abs() <= 1.0,
        "the sibling keeps the width it takes unconstrained: tight={tight:?} free={free:?}"
    );
    assert!(
        preview.x() + preview.width() <= tight.x(),
        "text and sibling share the row without overlapping: preview={preview:?} tight={tight:?}"
    );

    // The respelled line ends in "…" — three separated ink dots riding the
    // baseline. Walk left from the leaf's last inked column while columns
    // carry ink only below the midline; a clipped line instead ends
    // mid-glyph with tall ink at its trailing edge.
    let snapshot = app.snapshot();
    let x0 = preview.x().max(0.0) as usize;
    let x1 = ((preview.x() + preview.width()) as usize).min(snapshot.width as usize);
    let y0 = preview.y().max(0.0) as usize;
    let y1 = ((preview.y() + preview.height()) as usize).min(snapshot.height as usize);
    let mid = y0 + (y1 - y0) / 2;
    let bg: [u8; 4] = snapshot.rgba8[(y0 * snapshot.width as usize + x0.saturating_sub(6)) * 4..]
        [..4]
        .try_into()
        .unwrap();
    let ink = |x: usize, y: usize| -> bool {
        let px: [u8; 4] = snapshot.rgba8[(y * snapshot.width as usize + x) * 4..][..4]
            .try_into()
            .unwrap();
        (0..3).any(|c| px[c].abs_diff(bg[c]) > 48)
    };

    let last_ink_x = (x0..x1)
        .rev()
        .find(|&x| (y0..y1).any(|y| ink(x, y)))
        .expect("the truncated text drew ink");
    let tall = |x: usize| (y0..mid).any(|y| ink(x, y));
    let low = |x: usize| (mid..y1).any(|y| ink(x, y));
    assert!(
        !tall(last_ink_x),
        "the leaf's trailing ink should sit on the baseline — a tall trailing column is a clipped glyph: {preview:?}"
    );
    let zone_x0 = (x0..last_ink_x)
        .rev()
        .find(|&x| tall(x))
        .map_or(x0, |x| x + 1);

    let mut runs = 0usize;
    let mut in_run = false;
    for x in zone_x0..=last_ink_x {
        if low(x) && !in_run {
            runs += 1;
        }
        in_run = low(x);
    }
    assert!(
        runs >= 3,
        "the truncated line should end in a '…' — {runs} baseline ink runs in its tail: {preview:?}"
    );
}
