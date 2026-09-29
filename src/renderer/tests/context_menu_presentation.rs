//! water-rs/hydrolysis#200 (water-rs/waterui#1245): the drawn `.context_menu`
//! presentation — the lifted preview over a dimmed backdrop, the interactive
//! accessory anchored to its edge, and the accessory's dismiss requests —
//! plus destructive and subtitled commands in the menu itself.
//!
//! A context menu with neither preview nor accessory mounts none of this:
//! its popup stays a pure platform menu.

use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId, TreeUpdate};
use nami::Binding;
use nami::Signal as _;
use waterui::ViewExt as _;
use waterui::prelude::{ContextMenu, DismissContextMenu, Use};
use waterui_backend_core::WidgetTheme;
use waterui_controls::button::button;
use waterui_controls::menu::{CommandExt as _, MenuItem};
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::Color;
use waterui_layout::frame::Frame;
use waterui_layout::stack::vstack;

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::{HeadlessRuntime, HeadlessSnapshot};

const WINDOW: (u32, u32) = (320, 240);

fn secondary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
    ]
}

fn primary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 2,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: 2,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
    ]
}

/// The host view every test mounts: a button carrying the context menu inside
/// a `160x80` positioning frame, sat below a `spacer`-point gap. The menu rides
/// on the button itself so its a11y bounds are the menu target's own bounds —
/// what `bounds_of` returns is what the preview lifts and the menu clears.
/// `menu` is a factory because the builder closure is `Fn`, and `ContextMenu`
/// is not `Clone`.
fn host_view_with_spacer(
    spacer: f32,
    menu: impl Fn() -> ContextMenu + 'static,
) -> AnyViewBuilder<AnyView> {
    AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(vstack((
            ().size(0.0, spacer),
            Frame::new(button("host").action(|| {}).context_menu(menu()))
                .width(160.0)
                .height(80.0),
        )))
    })
}

fn host_view(menu: impl Fn() -> ContextMenu + 'static) -> AnyViewBuilder<AnyView> {
    host_view_with_spacer(110.0, menu)
}

fn runtime(menu: impl Fn() -> ContextMenu + 'static) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        host_view(menu),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    )
}

/// Same, in a taller window: under `debug_assertions` the inspector joins
/// every context menu with a divider and an "Inspect element" row, so the
/// menu stands `3 * row_height + padding` tall — a `240`-high window leaves
/// no room beside any source for the below/above placements to show.
fn runtime_sized(
    spacer: f32,
    window: (u32, u32),
    menu: impl Fn() -> ContextMenu + 'static,
) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        host_view_with_spacer(spacer, menu),
        window.0,
        window.1,
        MinimalTestTheme::default(),
    )
}

/// Pumps until the runtime settles (with a cap), then returns the merged
/// tree as of that settle — `None` when no window has ever produced one.
fn pump_until_settled(runtime: &mut HeadlessRuntime) -> Option<TreeUpdate> {
    for _ in 0..64 {
        let _ = runtime.pump_at(false, Instant::now());
        if runtime.is_settled() {
            break;
        }
    }
    runtime.accessibility_tree()
}

/// Same, capturing the composited frame on every pump — the pixel assertions
/// read the last one.
fn capture_until_settled(runtime: &mut HeadlessRuntime) -> HeadlessSnapshot {
    let mut snapshot = None;
    for _ in 0..64 {
        let result = runtime.pump_at(true, Instant::now());
        if let Some(frame) = result.snapshot {
            snapshot = Some(frame);
        }
        if runtime.is_settled() {
            break;
        }
    }
    snapshot.expect("a settled runtime must capture a frame")
}

/// The a11y bounds `label` was last seen with, read off the settled merged
/// tree.
fn bounds_of(runtime: &mut HeadlessRuntime, label: &str) -> accesskit::Rect {
    pump_until_settled(runtime)
        .as_ref()
        .and_then(|update| find_by_label(update, Role::Button, label))
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("{label} must emit a button with bounds"))
}

fn bounds_in(update: &TreeUpdate, label: &str) -> accesskit::Rect {
    find_by_label(update, Role::Button, label)
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("{label} must emit a button with bounds"))
}

/// A secondary click at `label`'s centre — the open path every context menu
/// takes — followed by a settle. Returns the label's bounds and the last
/// merged update the runtime published (it carries the menu and accessory
/// nodes emitted by the open frame).
fn secondary_click_label(
    runtime: &mut HeadlessRuntime,
    label: &str,
) -> (accesskit::Rect, TreeUpdate) {
    let bounds = bounds_of(runtime, label);
    let (x, y) = (
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for event in secondary_click(x, y) {
        runtime.push_input_event(event);
    }
    let update = pump_until_settled(runtime).expect("the open frame publishes");
    (bounds, update)
}

/// A primary click at `label`'s centre inside `update`, followed by a settle.
fn primary_click_label(runtime: &mut HeadlessRuntime, label: &str, update: &TreeUpdate) {
    let bounds = bounds_in(update, label);
    let (x, y) = (
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for event in primary_click(x, y) {
        runtime.push_input_event(event);
    }
    let _ = pump_until_settled(runtime);
}

fn pixel(snapshot: &HeadlessSnapshot, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * snapshot.width + x) * 4) as usize;
    snapshot.rgba8[index..index + 4]
        .try_into()
        .expect("a pixel is four channels")
}

fn near(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(left, right)| left.abs_diff(*right) <= tolerance)
}

/// The accessory's button mounts into the open presentation, hit-testable on
/// the main window, and runs its action without closing the menu — only an
/// item choice, an outside press, or a dismiss request does.
#[test]
fn the_accessory_is_hit_testable_and_acts_without_closing_the_menu() {
    let fired = Binding::container(false);
    let fired_for_view = fired.clone();
    let mut runtime = runtime(move || {
        let fired = fired_for_view.clone();
        ContextMenu::new(vec!["Copy".action(|| {})]).accessory(Frame::new(button("Like").action(
            move || {
                fired.set(true);
            },
        )))
    });
    let (_, update) = secondary_click_label(&mut runtime, "host");
    assert!(
        runtime.context_menu_presentation_frames().is_some(),
        "the drawn menu mounts"
    );

    // The accessory's own button lands on the merged tree: pressing it must
    // resolve like a pointer, not like a dismissive outside press.
    primary_click_label(&mut runtime, "Like", &update);

    assert!(
        fired.snapshot(),
        "the accessory's button must run its action"
    );
    assert!(
        runtime.context_menu_presentation_frames().is_some(),
        "acting inside the accessory does not close the menu"
    );
}

/// `DismissContextMenu` is the accessory's own way out: a button extracting it
/// and calling `dismiss()` flips `dismiss_requests`, which the open menu
/// observes and closes on.
#[test]
fn a_dismiss_request_from_the_accessory_closes_the_menu() {
    let fired = Binding::container(false);
    let fired_for_view = fired.clone();
    let mut runtime = runtime(move || {
        let fired = fired_for_view.clone();
        ContextMenu::new(vec!["Copy".action(|| {})]).accessory(Frame::new(button("Done").action(
            move |Use(dismiss): Use<DismissContextMenu>| {
                fired.set(true);
                dismiss.dismiss();
            },
        )))
    });
    let (_, update) = secondary_click_label(&mut runtime, "host");
    assert!(
        runtime.context_menu_presentation_frames().is_some(),
        "the drawn menu mounts"
    );

    primary_click_label(&mut runtime, "Done", &update);

    assert!(fired.snapshot(), "the accessory's button must run");
    assert!(
        runtime.context_menu_presentation_frames().is_none(),
        "a dismiss request closes the open menu"
    );
}

/// With no custom `preview`, the lifted content is the source view itself:
/// the dim backdrop parts around its frame — inside stays lit, outside dims.
#[test]
fn the_source_view_lifts_through_a_hole_in_the_dim_backdrop() {
    let mut runtime = runtime_sized(110.0, (320, 480), || {
        ContextMenu::new(vec!["Copy".action(|| {})])
            .accessory(Frame::new(button("Like").action(|| {})))
    });
    let bounds = bounds_of(&mut runtime, "host");
    let baseline = runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("a captured frame");

    let (cx, cy) = (
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for event in secondary_click(cx, cy) {
        runtime.push_input_event(event);
    }
    let opened = capture_until_settled(&mut runtime);

    // A sample inside the source's top-left interior — clear of the menu,
    // which opens at the press point and grows right and down.
    let (inside_x, inside_y) = ((bounds.x0 + 8.0) as u32, (bounds.y0 + 8.0) as u32);
    assert!(
        near(
            pixel(&opened, inside_x, inside_y),
            pixel(&baseline, inside_x, inside_y),
            16
        ),
        "the lifted source stays lit inside the backdrop's hole"
    );
    // A window corner far from the source and the menu dims.
    let (outside_x, outside_y) = (6, 6);
    assert!(
        !near(
            pixel(&opened, outside_x, outside_y),
            pixel(&baseline, outside_x, outside_y),
            16
        ),
        "the backdrop dims outside the lifted source"
    );
}

/// A custom `preview` replaces the source in the lift: the rect the source
/// occupied draws the preview's pixels instead.
#[test]
fn a_custom_preview_lifts_at_the_source_frame() {
    let mut runtime = runtime_sized(110.0, (320, 480), || {
        ContextMenu::new(vec!["Copy".action(|| {})])
            .preview(Color::srgb(255, 0, 0))
            .accessory(Frame::new(button("Like").action(|| {})))
    });
    let bounds = bounds_of(&mut runtime, "host");
    let (cx, cy) = (
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for event in secondary_click(cx, cy) {
        runtime.push_input_event(event);
    }
    let opened = capture_until_settled(&mut runtime);

    // The preview fills the source's rect; several interior samples land on it.
    for (x, y) in [
        (bounds.x0 + 8.0, bounds.y0 + 8.0),
        (bounds.x0 + 8.0, (bounds.y0 + bounds.y1) / 2.0 - 8.0),
        ((bounds.x0 + bounds.x1) / 2.0 - 8.0, bounds.y0 + 8.0),
    ] {
        let px = pixel(&opened, x as u32, y as u32);
        assert!(
            px[0] > 150 && px[1] < 100 && px[2] < 100,
            "the custom preview draws at the source's frame, got {px:?} at {x},{y}"
        );
    }
}

/// A destructive command's label draws in the theme's error colour; a subtitle
/// adds a second, smaller line under its label and grows the row — and with it
/// the menu.
#[test]
fn destructive_draws_in_error_colour_and_subtitle_grows_its_row() {
    let mut runtime = runtime_sized(10.0, (320, 520), || {
        ContextMenu::new(vec![
            "Keep".action(|| {}),
            "Delete".command().action(|| {}).destructive(),
            "Info".command().action(|| {}).subtitle("Details"),
        ])
        .accessory(Frame::new(button("Like").action(|| {})))
    });

    let _ = secondary_click_label(&mut runtime, "host");
    let (menu_frame, _) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn menu mounts");
    // The test theme is not M3 — derive the expectation from its metrics so
    // the test cannot drift from the tokens it is fed.
    let menu_metrics = MinimalTestTheme::default().text_context_menu_metrics();
    let three_plain_rows = 3.0 * menu_metrics.row_height;
    assert!(
        menu_frame.height() > three_plain_rows + menu_metrics.vertical_padding,
        "a subtitled row grows the menu, height {}",
        menu_frame.height()
    );

    // The drawn menu renders inside the main window's frame — read its rows
    // off the captured snapshot under `menu_frame`.
    let opened = capture_until_settled(&mut runtime);
    let (x0, y0) = (menu_frame.x0 as u32, menu_frame.y0 as u32);
    let (x1, y1) = (menu_frame.x1 as u32, menu_frame.y1 as u32);
    let menu_pixel = |x: u32, y: u32| pixel(&opened, x0 + x, y0 + y);

    let mut error_colored = 0_u32;
    for y in 0..y1.saturating_sub(y0) {
        for x in 0..x1.saturating_sub(x0) {
            let px = menu_pixel(x, y);
            // The default theme's error is srgb(220, 38, 38).
            if px[0] > 160 && px[1] < 110 && px[2] < 110 {
                error_colored += 1;
            }
        }
    }
    assert!(
        error_colored > 40,
        "the destructive command's label draws in the theme's error colour"
    );

    // The caption draws in the muted supporting style: mid-grey ink a
    // second line leaves under the label, distinct from the labels'
    // near-black and the destructive row's error red.
    let mut second_line_ink = 0_u32;
    for y in 0..y1.saturating_sub(y0) {
        for x in 0..x1.saturating_sub(x0) {
            let px = menu_pixel(x, y);
            if (55..150).contains(&px[0])
                && (55..150).contains(&px[1])
                && (55..150).contains(&px[2])
            {
                second_line_ink += 1;
            }
        }
    }
    assert!(
        second_line_ink > 60,
        "a subtitle draws a second line under the label"
    );
}

/// With no preview and no accessory the menu is the same pure platform popup
/// it always was: the window does not dim and no presentation mounts.
#[test]
fn a_menu_with_no_preview_or_accessory_stays_a_pure_platform_menu() {
    let mut runtime = runtime(|| ContextMenu::new(vec!["Copy".action(|| {})]));

    let bounds = bounds_of(&mut runtime, "host");
    let baseline = runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("a captured frame");
    let (cx, cy) = (
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for event in secondary_click(cx, cy) {
        runtime.push_input_event(event);
    }
    let opened = capture_until_settled(&mut runtime);
    assert_eq!(runtime.popup_frames().len(), 1, "the menu mounts");

    assert!(
        near(pixel(&opened, 6, 6), pixel(&baseline, 6, 6), 16)
            && near(pixel(&opened, 160, 20), pixel(&baseline, 160, 20), 16),
        "without a preview or accessory the window does not dim"
    );
}

/// A preview on its own already takes the drawn presentation — the lift is
/// not gated on the accessory.
#[test]
fn a_preview_without_an_accessory_lifts_too() {
    let mut runtime = runtime_sized(110.0, (320, 480), || {
        ContextMenu::new(vec!["Copy".action(|| {})]).preview(Color::srgb(255, 0, 0))
    });
    let bounds = bounds_of(&mut runtime, "host");
    let baseline = runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("a captured frame");

    let (cx, cy) = (
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for event in secondary_click(cx, cy) {
        runtime.push_input_event(event);
    }
    let opened = capture_until_settled(&mut runtime);
    assert!(
        runtime.context_menu_presentation_frames().is_some(),
        "the drawn menu mounts"
    );

    let px = pixel(&opened, (bounds.x0 + 8.0) as u32, (bounds.y0 + 8.0) as u32);
    assert!(
        px[0] > 150 && px[1] < 100 && px[2] < 100,
        "the preview lifts at the source's frame without an accessory, got {px:?}"
    );
    assert!(
        !near(pixel(&opened, 6, 6), pixel(&baseline, 6, 6), 16),
        "the backdrop dims for a preview-only presentation"
    );
}

/// A touch press held past its threshold opens through the same drawn
/// presentation a preview would take — the source lifts and the menu sits
/// beside it — while a secondary click on the same menu stays a plain popup.
#[test]
fn a_touch_hold_lifts_and_mounts_the_menu_beside_the_source() {
    const HOLD: Duration = Duration::from_millis(500);
    let mut runtime = runtime_sized(110.0, (320, 480), || {
        ContextMenu::new(vec!["Copy".action(|| {})])
    });
    let bounds = bounds_of(&mut runtime, "host");
    let baseline = runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("a captured frame");
    let start = Instant::now();
    let (cx, cy) = (
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );

    runtime.push_input_event(InputEvent::PointerDown {
        id: 3,
        kind: PointerKind::Touch,
        x: cx,
        y: cy,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump_at(false, start);
    let open = runtime.pump_at(true, start + HOLD);
    assert!(
        open.tree_update
            .as_ref()
            .and_then(|update| find_by_label(update, Role::Button, "Copy"))
            .is_some(),
        "a held touch press mounts the menu"
    );
    runtime.push_input_event(InputEvent::PointerUp {
        id: 3,
        kind: PointerKind::Touch,
        x: cx,
        y: cy,
        button: PointerButton::Primary,
    });

    let (menu, _) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn menu mounts");
    assert!(
        menu.y0 >= bounds.y1,
        "the menu sits beside the lifted source, menu {menu:?} source {bounds:?}"
    );
    let opened = open.snapshot.expect("the hold frame captures");
    assert!(
        !near(pixel(&opened, 6, 6), pixel(&baseline, 6, 6), 16),
        "the backdrop dims for a touch-hold presentation"
    );
}

/// The menu's placement is pinned relative to the lifted preview: below its
/// bottom edge when that fits, above it when the window has no room below,
/// always aligned to the preview's leading edge, never overlapping it.
#[test]
fn the_menu_sits_beside_the_lifted_preview() {
    // Below by default: the source sits high in the window.
    let mut runtime = runtime_sized(20.0, (320, 480), || {
        ContextMenu::new(vec!["Copy".action(|| {})]).preview(Color::srgb(255, 0, 0))
    });
    let (bounds, _) = secondary_click_label(&mut runtime, "host");
    let (menu, _) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn menu mounts");
    assert!(
        (menu.x0 - bounds.x0).abs() < 1.0,
        "the menu's leading edge aligns with the preview's, menu {menu:?} source {bounds:?}"
    );
    assert!(
        (menu.y0 - (bounds.y1 + 8.0)).abs() < 1.0,
        "the menu sits below the preview's bottom edge, menu {menu:?} source {bounds:?}"
    );
    assert!(
        menu.y0 >= bounds.y1,
        "the menu never overlaps the preview, menu {menu:?} source {bounds:?}"
    );

    // Above when below does not fit: the source sits near the window's
    // bottom edge.
    let mut runtime = runtime_sized(320.0, (320, 480), || {
        ContextMenu::new(vec!["Copy".action(|| {})]).preview(Color::srgb(255, 0, 0))
    });
    let (bounds, _) = secondary_click_label(&mut runtime, "host");
    let (menu, _) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn menu mounts");
    assert!(
        (menu.y1 - (bounds.y0 - 8.0)).abs() < 1.0,
        "the menu sits above the preview near the bottom edge, menu {menu:?} source {bounds:?}"
    );
    assert!(
        menu.y1 <= bounds.y0,
        "the menu never overlaps the preview, menu {menu:?} source {bounds:?}"
    );
}

/// Every row's label shares one leading x: a plain command, a subtitled
/// command and a destructive command all start their text at the same x —
/// the row's label sits at its button's leading edge regardless of kind.
#[test]
fn every_rows_label_shares_one_leading_x() {
    let mut runtime = runtime_sized(20.0, (320, 560), || {
        ContextMenu::new(vec![
            "Reply".action(|| {}),
            "Forward"
                .command()
                .action(|| {})
                .subtitle("To another chat"),
            "Delete".command().action(|| {}).destructive(),
        ])
        .accessory(Frame::new(button("Like").action(|| {})))
    });
    let _ = secondary_click_label(&mut runtime, "host");
    let rows = runtime.context_menu_row_frames();
    // Three app rows at minimum — debug builds add the inspector's command.
    assert!(
        rows.len() >= 3,
        "expected at least the three row buttons, got {rows:?}"
    );
    let opened = capture_until_settled(&mut runtime);
    // The leftmost ink pixel of each row band is the label's leading x —
    // near-black for plain and subtitled labels, error red for the
    // destructive one, mid-grey for the caption.
    let is_ink = |px: [u8; 4]| {
        (px[0] < 140 && px[1] < 140 && px[2] < 140) || (px[0] > 150 && px[1] < 120 && px[2] < 120)
    };
    let mut leading_xs = Vec::new();
    for row in &rows {
        let mut found = None;
        'row: for x in row.x0 as u32..row.x1 as u32 {
            for y in row.y0 as u32..row.y1 as u32 {
                if is_ink(pixel(&opened, x, y)) {
                    found = Some(x);
                    break 'row;
                }
            }
        }
        leading_xs.push(found.unwrap_or_else(|| panic!("row {row:?} has no ink")));
    }
    let leading = leading_xs[0];
    assert!(
        leading_xs.iter().all(|x| x.abs_diff(leading) <= 1),
        "every row's label shares one leading x, got {leading_xs:?}"
    );
}

/// The accessory anchors to the preview: leading-aligned with its leading
/// edge, with the same gap the menu keeps on its own side.
#[test]
fn the_accessory_anchors_to_the_previews_leading_edge() {
    // The source sits low enough that the accessory fits above it while the
    // menu fits below.
    let mut runtime = runtime_sized(160.0, (320, 560), || {
        ContextMenu::new(vec!["Copy".action(|| {})])
            .accessory(Frame::new(button("Like").action(|| {})))
    });
    let (bounds, _) = secondary_click_label(&mut runtime, "host");
    let (menu, accessory) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn menu mounts");
    let accessory = accessory.expect("the accessory mounts");
    assert!(
        (accessory.x0 - bounds.x0).abs() < 1.0,
        "the accessory's leading edge aligns with the preview's, accessory {accessory:?} preview {bounds:?}"
    );
    assert!(
        accessory.y1 <= bounds.y0,
        "the accessory defaults above the preview, accessory {accessory:?} preview {bounds:?}"
    );
    let accessory_gap = bounds.y0 - accessory.y1;
    let menu_gap = menu.y0 - bounds.y1;
    assert!(
        (accessory_gap - menu_gap).abs() < 1.0,
        "the accessory keeps the menu's gap, accessory {accessory:?} menu {menu:?} preview {bounds:?}"
    );
}

/// The panels are rounded: a pixel inside a panel's corner cut-off shows the
/// scrim — the dimmed page — never the panel's surface; and the panel's
/// elevation shadow lands on the scrim just outside its edge.
#[test]
fn the_panels_corners_show_the_scrim_and_their_shadow_lands_on_it() {
    let mut runtime = runtime_sized(160.0, (320, 560), || {
        ContextMenu::new(vec!["Copy".action(|| {})])
            .accessory(Frame::new(button("Like").action(|| {})))
    });
    let _ = secondary_click_label(&mut runtime, "host");
    let (menu, accessory) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn menu mounts");
    let accessory = accessory.expect("the accessory mounts");
    let opened = capture_until_settled(&mut runtime);

    let luma = |px: [u8; 4]| (px[0] as i32 + px[1] as i32 + px[2] as i32) / 3;
    // The bare scrim: dimmed page, sampled clear of every panel and shadow.
    let scrim = pixel(&opened, 6, 550);
    for frame in [menu, accessory] {
        // The panel's interior beside its top-left corner is the surface.
        let surface = pixel(&opened, frame.x0 as u32 + 8, frame.y0 as u32 + 2);
        // Inside the corner's cut-off region — within the frame's rect, past
        // the 4pt radius — the scrim shows through: never the surface.
        let wedge = pixel(&opened, frame.x0 as u32 + 1, frame.y0 as u32 + 1);
        assert!(
            !near(wedge, surface, 40) && luma(wedge) <= luma(scrim) + 20,
            "the corner cut-off shows the scrim, not the surface — wedge {wedge:?}, scrim {scrim:?}, surface {surface:?}"
        );
        // Just under the panel's bottom edge its elevation shadow deepens the
        // scrim — a level-2 shadow exists and lands on the dim.
        let below = pixel(&opened, (frame.x0 + 24.0) as u32, frame.y1 as u32 + 3);
        assert!(
            luma(below) < luma(scrim) - 8,
            "the panel's shadow darkens the scrim below it — below {below:?}, scrim {scrim:?}"
        );
    }
}

/// The whole stack — accessory, gap, preview, gap, menu — fits inside the
/// window around a source near the bottom edge: the lifted preview moves so
/// nothing overlaps and every frame stays inside.
#[test]
fn a_source_near_the_bottom_edge_stacks_without_overlap() {
    let mut runtime = runtime_sized(380.0, (320, 480), || {
        ContextMenu::new(vec!["Copy".action(|| {})])
            .preview(Color::srgb(255, 0, 0))
            .accessory(Frame::new(button("Like").action(|| {})))
    });
    let _ = secondary_click_label(&mut runtime, "host");
    let (menu, accessory) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn menu mounts");
    let accessory = accessory.expect("the accessory mounts");
    let lift = runtime
        .context_menu_lift_frame()
        .expect("the lift frame is laid out");
    let window = kurbo::Rect::new(0.0, 0.0, 320.0, 480.0);
    for frame in [menu, accessory, lift] {
        assert!(
            frame.x0 >= window.x0
                && frame.y0 >= window.y0
                && frame.x1 <= window.x1
                && frame.y1 <= window.y1,
            "every frame stays inside the window, got {frame:?} in {window:?}"
        );
    }
    for (a, b) in [(menu, accessory), (menu, lift), (accessory, lift)] {
        assert!(
            a.x0 >= b.x1 || b.x0 >= a.x1 || a.y0 >= b.y1 || b.y0 >= a.y1,
            "no pair overlaps: {a:?} vs {b:?}"
        );
    }
}

/// Same, with the source near the top edge: the stack fits inside the window
/// in whichever order has room, without overlap.
#[test]
fn a_source_near_the_top_edge_stacks_without_overlap() {
    let mut runtime = runtime_sized(10.0, (320, 480), || {
        ContextMenu::new(vec!["Copy".action(|| {})])
            .preview(Color::srgb(255, 0, 0))
            .accessory(Frame::new(button("Like").action(|| {})))
    });
    let _ = secondary_click_label(&mut runtime, "host");
    let (menu, accessory) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn menu mounts");
    let accessory = accessory.expect("the accessory mounts");
    let lift = runtime
        .context_menu_lift_frame()
        .expect("the lift frame is laid out");
    let window = kurbo::Rect::new(0.0, 0.0, 320.0, 480.0);
    for frame in [menu, accessory, lift] {
        assert!(
            frame.x0 >= window.x0
                && frame.y0 >= window.y0
                && frame.x1 <= window.x1
                && frame.y1 <= window.y1,
            "every frame stays inside the window, got {frame:?} in {window:?}"
        );
    }
    for (a, b) in [(menu, accessory), (menu, lift), (accessory, lift)] {
        assert!(
            a.x0 >= b.x1 || b.x0 >= a.x1 || a.y0 >= b.y1 || b.y0 >= a.y1,
            "no pair overlaps: {a:?} vs {b:?}"
        );
    }
}

/// A divider row takes the theme's divider spacing — the separator line
/// inside the menu's vertical padding — not a full row's height.
#[test]
fn a_divider_row_takes_the_themes_divider_spacing() {
    let mut runtime = runtime_sized(20.0, (320, 560), || {
        ContextMenu::new(vec![
            MenuItem::from("Copy".action(|| {})),
            MenuItem::Divider,
            MenuItem::from("Paste".action(|| {})),
        ])
    });
    let _ = secondary_click_label(&mut runtime, "host");
    let popup = runtime.popup_frames();
    let frame = *popup.first().expect("the menu mounts");
    // The test theme is not M3 — derive the expectation from its metrics so
    // the test cannot drift from the tokens it is fed: two rows + a divider
    // (separator_thickness + vertical_padding above and below) + the theme's
    // container vertical padding; the window frame adds the panel margin on
    // every side. The debug build's "Inspect element" affordance appends its
    // own divider and row.
    let menu_metrics = MinimalTestTheme::default().text_context_menu_metrics();
    let divider = crate::renderer::input::popup_menu_divider_height(menu_metrics);
    let row = menu_metrics.row_height;
    let rows = row
        + row
        + divider
        + if cfg!(debug_assertions) {
            divider + row
        } else {
            0.0
        };
    let expected =
        rows + menu_metrics.vertical_padding * 2.0 + crate::renderer::POPUP_MENU_PANEL_MARGIN * 2.0;
    assert!(
        (f64::from(frame.height()) - expected).abs() < 1.0,
        "the divider takes the theme's divider spacing, height {} vs {expected}",
        frame.height()
    );
}

/// The plain popup path — secondary click with neither preview nor accessory —
/// draws the theme's context-menu panel inside its own window: an opaque
/// surface with rounded corners and an elevation shadow, not a bare list.
#[test]
fn the_plain_popup_draws_the_theme_panel() {
    let mut runtime = runtime_sized(110.0, (320, 480), || {
        ContextMenu::new(vec!["Copy".action(|| {})])
    });
    let _ = secondary_click_label(&mut runtime, "host");
    let pop = runtime.popup_frame(0).expect("the popup mounts");

    let m = crate::renderer::POPUP_MENU_PANEL_MARGIN as u32;
    // The window's transparent ring: well outside the panel.
    assert!(
        pixel(&pop, 1, 1)[3] < 16,
        "the popup's margin stays transparent, got {:?}",
        pixel(&pop, 1, 1)
    );
    // Inside the panel the surface fill is exactly the test theme's panel
    // colour (0.96, 0.94, 0.97) and fully opaque — an open-appear-animation
    // opacity below 1.0 (or any bleed from content behind) fails here.
    let inside = pixel(&pop, m + 8, m + 2);
    let surface = [245_u8, 240_u8, 248_u8];
    assert!(
        inside[3] == 255
            && inside[0].abs_diff(surface[0]) <= 4
            && inside[1].abs_diff(surface[1]) <= 4
            && inside[2].abs_diff(surface[2]) <= 4,
        "the popup panel interior is the opaque theme surface, got {inside:?}"
    );
    // Inside the panel's corner cut-off the pixel is transparent or the
    // shadow — never the opaque surface.
    let wedge = pixel(&pop, m + 1, m + 1);
    assert!(
        wedge[3] < inside[3] || wedge[0] < inside[0] - 40,
        "the panel's corner is rounded, wedge {wedge:?} vs surface {inside:?}"
    );
    // Under the panel's bottom edge, inside the margin ring, its shadow
    // darkens: a visible level-2 shadow.
    let panel_bottom = pop.height - m;
    let below = pixel(&pop, pop.width / 2, panel_bottom + 2);
    assert!(
        below[3] > 16 && below[0] < 90,
        "the panel's shadow draws in the window's margin ring, got {below:?}"
    );
}

/// water-rs/hydrolysis#248: a disabled menu command's label draws at
/// Material's disabled contrast — on-surface at 38% opacity — not at the
/// enabled row's full contrast, and its press stays inert.
#[test]
fn a_disabled_command_draws_its_label_dimmed_and_stays_inert() {
    let copied = Binding::container(false);
    let copied_for_view = copied.clone();
    let mut runtime = runtime(move || {
        let copied = copied_for_view.clone();
        ContextMenu::new(vec![
            "Copy".action(move || copied.set(true)).disabled(true),
            "Paste".action(|| {}),
        ])
    });
    let (_, update) = secondary_click_label(&mut runtime, "host");
    let pop = runtime.popup_frame(0).expect("the popup mounts");

    // Rows lay out under the panel margin: first the disabled `Copy`, then
    // `Paste` — each `row_height` tall inside `vertical_padding`.
    let metrics = MinimalTestTheme::default().text_context_menu_metrics();
    let margin = crate::renderer::POPUP_MENU_PANEL_MARGIN as u32;
    let row_top = margin + metrics.vertical_padding as u32;
    let row_bottom = row_top + metrics.row_height as u32;
    let (x0, x1) = (margin + 2, pop.width - margin - 2);
    let darkest_ink = |y0: u32, y1: u32| -> u32 {
        let mut darkest = u32::MAX;
        for y in y0..y1 {
            for x in x0..x1 {
                let px = pixel(&pop, x, y);
                darkest = darkest.min((px[0] as u32 + px[1] as u32 + px[2] as u32) / 3);
            }
        }
        darkest
    };
    let disabled_ink = darkest_ink(row_top, row_bottom);
    let enabled_ink = darkest_ink(row_bottom, row_bottom + metrics.row_height as u32);

    assert!(
        enabled_ink < 80,
        "the enabled row's label draws at full contrast, got darkest {enabled_ink}"
    );
    assert!(
        disabled_ink > enabled_ink + 60,
        "the disabled row's label draws at 38% on-surface, got darkest {disabled_ink} \
         vs enabled {enabled_ink}"
    );

    // The press stays inert: invoking the disabled row runs nothing and the
    // menu stays open.
    let (copy_id, _) =
        find_by_label(&update, Role::Button, "Copy").expect("the disabled row stays in the tree");
    runtime.perform_accessibility_action(ActionRequest {
        action: Action::Click,
        target_node: copy_id,
        target_tree: TreeId::ROOT,
        data: None,
    });
    let _ = pump_until_settled(&mut runtime);
    assert!(!copied.snapshot(), "the disabled row's press stays inert");
    assert_eq!(runtime.popup_frames().len(), 1, "the menu stays open");
}

/// A destructive command draws in the theme's error colour on the plain
/// popup path too — the role's span colour rides the shared menu content.
#[test]
fn destructive_rows_draw_in_error_colour_on_the_plain_popup_path() {
    let mut runtime = runtime_sized(110.0, (320, 480), || {
        ContextMenu::new(vec!["Delete".command().action(|| {}).destructive()])
    });
    let _ = secondary_click_label(&mut runtime, "host");
    let pop = runtime.popup_frame(0).expect("the popup mounts");

    let mut error_colored = 0_u32;
    for y in 0..pop.height {
        for x in 0..pop.width {
            let px = pixel(&pop, x, y);
            if px[0] > 160 && px[1] < 110 && px[2] < 110 && px[3] > 100 {
                error_colored += 1;
            }
        }
    }
    assert!(
        error_colored > 20,
        "the destructive command's label draws in the theme's error colour on the plain popup path"
    );
}
