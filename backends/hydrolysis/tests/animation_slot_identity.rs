//! Regression coverage for water-rs/waterui#2240: per-view animation slots
//! must be keyed by the view that draws them, not by the driving signal's
//! identity. Nami keys derived signals by source plus call site, so two
//! views whose signals come from one `map`/`mapping` call site share a
//! `SignalIdentity`; signal-keyed slots then alias each other and neither
//! view animates correctly. Every test here mounts two views fed by signals
//! minted at one call site, drives only the second, and asserts the first
//! stays still while the second transitions over time.

mod support {
    /// Converts a `f32` coordinate to `usize` with `as` saturating truncation.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "test coordinates are non-negative and within usize range"
    )]
    pub const fn f32_as_usize(v: f32) -> usize {
        v as usize
    }

    /// Widens a `usize` count to `f64`, rounding to nearest.
    #[expect(clippy::cast_precision_loss, reason = "test counts are far below 2^53")]
    pub const fn usize_as_f64(v: usize) -> f64 {
        v as f64
    }
}

use core::time::Duration;

use hydrolysis_m3::Material3;
use waterui::animation::Animation;
use waterui::component::progress;
use waterui::component::vstack;
use waterui::shape::{Circle, Rectangle, ShapeExt};
use waterui::{Binding, Color, Signal, Str, View};
use waterui::{SignalExt as _, ViewExt as _};
use waterui_controls::field;
use waterui_form::picker::{Picker, PickerStyle};
use waterui_testing::{NodeBounds, OffscreenApp, Role, Snapshot, ui};
use waterui_text::text;

/// The transform tests' explicit animation: long enough that half a second
/// of virtual frames lands mid-transition, short enough that two pumps cover
/// start, middle and end.
const TRANSITION: Duration = Duration::from_millis(400);
/// Half of the transition — the mid-animation sample point.
const MIDPOINT: Duration = Duration::from_millis(200);

/// The signal a transform or morph test hands to view `index`. Both views'
/// signals are minted by the same `map`/`with` call site in this helper, so
/// they share one nami `SignalIdentity` — the shape of the #2240
/// reproduction.
fn channel(source: &Binding<i32>, index: i32, rest: f32, end: f32) -> impl Signal<Output = f32> {
    source
        .map(move |v| if v == index { end } else { rest })
        .with(Animation::linear(TRANSITION))
}

/// The progress values: index 1 climbs to 0.8 while index 0 rests at 0.2.
fn fraction(source: &Binding<i32>, index: i32) -> impl Signal<Output = f64> {
    source.map(move |v| if v == index { 0.8 } else { 0.2 })
}

/// A `Binding<Str>` per text field, from one `Binding::mapping` call site.
fn text_channel(source: &Binding<(String, String)>, index: usize) -> Binding<Str> {
    Binding::mapping(
        source,
        move |value: (String, String)| {
            if index == 0 {
                Str::from(value.0)
            } else {
                Str::from(value.1)
            }
        },
        move |src: &Binding<(String, String)>, new: Str| {
            src.with_mut(|current| {
                if index == 0 {
                    current.0 = new.to_string();
                } else {
                    current.1 = new.to_string();
                }
            });
        },
    )
}

/// A `Binding<i32>` selection per picker, from one `Binding::mapping` call
/// site.
fn selection_channel(source: &Binding<(i32, i32)>, index: usize) -> Binding<i32> {
    Binding::mapping(
        source,
        move |value: (i32, i32)| if index == 0 { value.0 } else { value.1 },
        move |src: &Binding<(i32, i32)>, new: i32| {
            src.with_mut(|current| {
                if index == 0 {
                    current.0 = new;
                } else {
                    current.1 = new;
                }
            });
        },
    )
}

/// Mounts the view produced by `make` under the default Material 3 theme at
/// a fixed viewport.
fn mount<V: View + 'static>(make: impl Fn() -> V + 'static) -> OffscreenApp {
    ui().theme(Material3::defaults())
        .viewport(320, 240)
        .mount_offscreen(make)
}

/// Pixel-space region of an accessibility node's bounds.
fn region_of(b: NodeBounds) -> (usize, usize, usize, usize) {
    (
        support::f32_as_usize(b.x()),
        support::f32_as_usize(b.y()),
        support::f32_as_usize(b.x() + b.width()),
        support::f32_as_usize(b.y() + b.height()),
    )
}

/// The union of two pixel-space regions.
fn union_region(
    a: (usize, usize, usize, usize),
    b: (usize, usize, usize, usize),
) -> (usize, usize, usize, usize) {
    (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
}

/// How many pixels in `region` `(x0, y0, x1, y1)` differ between the two
/// snapshots by more than a small per-channel tolerance.
fn changed_pixels(
    before: &Snapshot,
    after: &Snapshot,
    region: (usize, usize, usize, usize),
) -> usize {
    let (x0, y0, x1, y1) = region;
    let width = before.width as usize;
    let mut changed = 0;
    for y in y0..y1.min(before.height as usize) {
        for x in x0..x1.min(width) {
            let i = (y * width + x) * 4;
            let difference: i32 = before.rgba8[i..i + 4]
                .iter()
                .zip(&after.rgba8[i..i + 4])
                .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
                .sum();
            if difference > 24 {
                changed += 1;
            }
        }
    }
    changed
}

/// Aggregates the pixels whose `channel` component (0 = red, 2 = blue)
/// dominates the other two: count, bounding box and centroid.
#[derive(Debug)]
struct ColorPixels {
    count: usize,
    min_x: usize,
    max_x: usize,
    min_y: usize,
    max_y: usize,
    sum_x: f64,
    sum_y: f64,
}

fn color_pixels(snapshot: &Snapshot, channel: usize) -> ColorPixels {
    let width = snapshot.width as usize;
    let other_a = (channel + 1) % 3;
    let other_b = (channel + 2) % 3;
    let mut pixels = ColorPixels {
        count: 0,
        min_x: usize::MAX,
        max_x: 0,
        min_y: usize::MAX,
        max_y: 0,
        sum_x: 0.0,
        sum_y: 0.0,
    };
    for y in 0..snapshot.height as usize {
        for x in 0..width {
            let i = (y * width + x) * 4;
            let rgba = &snapshot.rgba8[i..i + 4];
            let dominant = i32::from(rgba[channel]);
            let others = i32::from(rgba[other_a]).midpoint(i32::from(rgba[other_b]));
            if dominant - others > 60 {
                pixels.count += 1;
                pixels.min_x = pixels.min_x.min(x);
                pixels.max_x = pixels.max_x.max(x);
                pixels.min_y = pixels.min_y.min(y);
                pixels.max_y = pixels.max_y.max(y);
                pixels.sum_x += support::usize_as_f64(x);
                pixels.sum_y += support::usize_as_f64(y);
            }
        }
    }
    pixels
}

/// Total channel-dominance — dominated pixel count weighted by how dominant
/// each pixel is, so an alpha-blended fill loses strength smoothly.
fn color_strength(snapshot: &Snapshot, channel: usize) -> i64 {
    let width = snapshot.width as usize;
    let other_a = (channel + 1) % 3;
    let other_b = (channel + 2) % 3;
    let mut total = 0i64;
    for y in 0..snapshot.height as usize {
        for x in 0..width {
            let i = (y * width + x) * 4;
            let rgba = &snapshot.rgba8[i..i + 4];
            total += i64::from(
                i32::from(rgba[channel])
                    - i32::from(rgba[other_a]).midpoint(i32::from(rgba[other_b])),
            )
            .max(0);
        }
    }
    total
}

/// Asserts the two halves of the animation contract: `still` did not change
/// between `before` and `mid`, and `moving` changed before→mid and again
/// mid→after (so the driven view is genuinely transitioning, not snapped to
/// its target).
fn assert_independent_animation(
    before: &Snapshot,
    mid: &Snapshot,
    after: &Snapshot,
    still: (usize, usize, usize, usize),
    moving: (usize, usize, usize, usize),
    context: &str,
) {
    let still_delta = changed_pixels(before, mid, still);
    assert_eq!(
        still_delta, 0,
        "{context}: the untouched view must not redraw while its sibling animates ({still_delta} pixels changed)"
    );
    let moved_before = changed_pixels(before, mid, moving);
    assert!(
        moved_before > 0,
        "{context}: the driven view must be mid-transition ({moved_before} pixels changed before→mid)"
    );
    let moved_after = changed_pixels(mid, after, moving);
    assert!(
        moved_after > 0,
        "{context}: the driven view must still be transitioning after the midpoint ({moved_after} pixels changed mid→after)"
    );
}

#[test]
fn two_views_with_one_callsite_opacity_signal_fade_independently() {
    let source = Binding::container(-1_i32);
    let feed = source.clone();
    let mut app = mount(move || {
        vstack((
            Color::srgb_hex("#FF0000")
                .size(48.0, 48.0)
                .opacity(channel(&feed, 0, 1.0, 0.0)),
            Color::srgb_hex("#0000FF")
                .size(48.0, 48.0)
                .opacity(channel(&feed, 1, 1.0, 0.0)),
        ))
    });
    app.settle();
    let before = app.snapshot();
    let red_init = color_strength(&before, 0);
    let blue_init = color_strength(&before, 2);
    assert!(red_init > 0 && blue_init > 0, "both squares must render");

    source.set(1);
    app.pump_for(MIDPOINT);
    let mid = app.snapshot();
    app.pump_for(TRANSITION + Duration::from_millis(200));
    let after = app.snapshot();

    // The untriggered square never fades.
    let red_mid = color_strength(&mid, 0);
    assert!(
        (red_mid - red_init).abs() * 20 <= red_init,
        "first square's opacity must not move: init {red_init}, mid {red_mid}"
    );
    // The faded square is partway out at the midpoint — under signal-identity
    // keying the shared slot snaps it to the target on the first frame.
    let blue_mid = color_strength(&mid, 2);
    let blue_after = color_strength(&after, 2);
    assert!(
        blue_mid < blue_init * 4 / 5 && blue_mid > blue_after + 500,
        "second square must be mid-fade: init {blue_init}, mid {blue_mid}, after {blue_after}"
    );
}

#[test]
fn two_views_with_one_callsite_scale_signal_grow_independently() {
    let source = Binding::container(-1_i32);
    let feed = source.clone();
    let mut app = mount(move || {
        vstack((
            Color::srgb_hex("#FF0000")
                .size(40.0, 40.0)
                .scale(channel(&feed, 0, 1.0, 1.75), channel(&feed, 0, 1.0, 1.75)),
            Color::srgb_hex("#0000FF")
                .size(40.0, 40.0)
                .scale(channel(&feed, 1, 1.0, 1.75), channel(&feed, 1, 1.0, 1.75)),
        ))
    });
    app.settle();
    let before = app.snapshot();
    let red_init = color_pixels(&before, 0).count;
    let blue_init = color_pixels(&before, 2).count;
    assert!(
        red_init > 400 && blue_init > 400,
        "both squares must render"
    );

    source.set(1);
    app.pump_for(MIDPOINT);
    let mid = app.snapshot();
    app.pump_for(TRANSITION + Duration::from_millis(200));
    let after = app.snapshot();

    let red_mid = color_pixels(&mid, 0).count;
    assert!(
        red_mid.abs_diff(red_init) * 20 <= red_init,
        "first square's scale must not move: init {red_init}, mid {red_mid}"
    );
    let blue_mid = color_pixels(&mid, 2).count;
    let blue_after = color_pixels(&after, 2).count;
    assert!(
        blue_mid > blue_init + 200 && blue_mid + 100 < blue_after,
        "second square must be mid-grow: init {blue_init}, mid {blue_mid}, after {blue_after}"
    );
}

#[test]
fn two_views_with_one_callsite_rotation_signal_turn_independently() {
    let source = Binding::container(-1_i32);
    let feed = source.clone();
    let mut app = mount(move || {
        vstack((
            Color::srgb_hex("#FF0000")
                .size(64.0, 20.0)
                .rotation(channel(&feed, 0, 0.0, 90.0)),
            Color::srgb_hex("#0000FF")
                .size(64.0, 20.0)
                .rotation(channel(&feed, 1, 0.0, 90.0)),
        ))
    });
    app.settle();
    let before = app.snapshot();
    let red_init = color_pixels(&before, 0);
    let blue_init = color_pixels(&before, 2);
    assert!(red_init.count > 100 && blue_init.count > 100);
    let red_init_wide = red_init.max_x - red_init.min_x;

    source.set(1);
    app.pump_for(MIDPOINT);
    let mid = app.snapshot();
    app.pump_for(TRANSITION + Duration::from_millis(200));
    let after = app.snapshot();

    let red_mid = color_pixels(&mid, 0);
    let red_mid_wide = red_mid.max_x - red_mid.min_x;
    assert!(
        red_mid_wide.abs_diff(red_init_wide) <= 4,
        "first bar's footprint must not move: init w {red_init_wide}, mid w {red_mid_wide}"
    );
    // Mid-rotation the bar's bounding box is nearly square; a snapped slot
    // would already show the fully-vertical 20×64 footprint.
    let blue_mid = color_pixels(&mid, 2);
    let mid_w = blue_mid.max_x - blue_mid.min_x;
    let mid_h = blue_mid.max_y - blue_mid.min_y;
    assert!(
        mid_w > 30 && mid_h > 30,
        "second bar must be mid-turn, not snapped vertical: {mid_w}x{mid_h}"
    );
    let blue_after = color_pixels(&after, 2);
    assert!(
        blue_after.max_y - blue_after.min_y > blue_after.max_x - blue_after.min_x,
        "second bar must end vertical"
    );
}

#[test]
fn two_views_with_one_callsite_offset_signal_slide_independently() {
    let source = Binding::container(-1_i32);
    let feed = source.clone();
    let mut app = mount(move || {
        vstack((
            Color::srgb_hex("#FF0000")
                .size(40.0, 40.0)
                .offset(channel(&feed, 0, 0.0, 48.0), 0.0),
            Color::srgb_hex("#0000FF")
                .size(40.0, 40.0)
                .offset(channel(&feed, 1, 0.0, 48.0), 0.0),
        ))
    });
    app.settle();
    let before = app.snapshot();
    let red_init = color_pixels(&before, 0);
    let blue_init = color_pixels(&before, 2);
    assert!(red_init.count > 100 && blue_init.count > 100);
    let red_cx = red_init.sum_x / support::usize_as_f64(red_init.count);
    let blue_cx = blue_init.sum_x / support::usize_as_f64(blue_init.count);

    source.set(1);
    app.pump_for(MIDPOINT);
    let mid = app.snapshot();
    app.pump_for(TRANSITION + Duration::from_millis(200));
    let after = app.snapshot();

    let red_mid = color_pixels(&mid, 0);
    let red_mid_cx = red_mid.sum_x / support::usize_as_f64(red_mid.count);
    assert!(
        (red_mid_cx - red_cx).abs() <= 2.0,
        "first square's x-centroid must not move: {red_cx} -> {red_mid_cx}"
    );
    let blue_mid = color_pixels(&mid, 2);
    let blue_mid_cx = blue_mid.sum_x / support::usize_as_f64(blue_mid.count);
    assert!(
        blue_mid_cx > blue_cx + 6.0 && blue_mid_cx < blue_cx + 40.0,
        "second square must be mid-slide: {blue_cx} -> {blue_mid_cx}"
    );
    let blue_after = color_pixels(&after, 2);
    let blue_after_cx = blue_after.sum_x / support::usize_as_f64(blue_after.count);
    assert!(
        blue_after_cx > blue_mid_cx + 6.0,
        "second square must keep moving after the midpoint: {blue_mid_cx} -> {blue_after_cx}"
    );
}

/// Two filled fields whose value bindings come from one `Binding::mapping`
/// call site: focusing the second floats only its label.
#[test]
fn two_fields_with_one_callsite_value_bindings_float_labels_independently() {
    let feed = Binding::container((String::new(), String::new()));
    let mut app = ui()
        .theme(Material3::defaults())
        .viewport(320, 140)
        .mount_offscreen(move || {
            let first = text_channel(&feed, 0);
            let second = text_channel(&feed, 1);
            vstack((
                field("Alpha", &first).size(280.0, 56.0),
                field("Beta", &second).size(280.0, 56.0),
            ))
        });
    app.settle();
    let mut fields: Vec<_> = app
        .query()
        .role(Role::TEXT_INPUT)
        .all()
        .iter()
        .cloned()
        .collect();
    fields.sort_by(|a, b| a.bounds().y().total_cmp(&b.bounds().y()));
    assert_eq!(fields.len(), 2, "two fields must be queryable");
    let first_region = region_of(fields[0].bounds());
    let second_region = region_of(fields[1].bounds());
    let before = app.snapshot();

    fields[1].focus(&mut app);
    // Half of M3's 280ms focus-enter motion.
    app.pump_for(Duration::from_millis(140));
    let mid = app.snapshot();
    app.pump_for(Duration::from_millis(600));
    let after = app.snapshot();

    assert_independent_animation(
        &before,
        &mid,
        &after,
        first_region,
        second_region,
        "text-field floating label",
    );
}

/// Two linear progress indicators whose fractions come from one `map` call
/// site: raising the second's value animates only its fill.
#[test]
fn two_linear_progress_views_with_one_callsite_fill_signal_fill_independently() {
    let source = Binding::container(-1_i32);
    let feed = source.clone();
    let mut app =
        mount(move || vstack((progress(fraction(&feed, 0)), progress(fraction(&feed, 1)))));
    app.settle();
    let mut bars: Vec<_> = app
        .query()
        .role(Role::PROGRESS_INDICATOR)
        .all()
        .iter()
        .cloned()
        .collect();
    bars.sort_by(|a, b| a.bounds().y().total_cmp(&b.bounds().y()));
    assert_eq!(bars.len(), 2, "two progress bars must be queryable");
    let first_region = region_of(bars[0].bounds());
    let second_region = region_of(bars[1].bounds());
    let before = app.snapshot();

    source.set(1);
    // Half of M3's 250ms linear-determinate motion.
    app.pump_for(Duration::from_millis(120));
    let mid = app.snapshot();
    app.pump_for(Duration::from_millis(500));
    let after = app.snapshot();

    assert_independent_animation(
        &before,
        &mid,
        &after,
        first_region,
        second_region,
        "linear progress fill",
    );
}

/// Two circular progress indicators whose fractions come from one `map`
/// call site.
#[test]
fn two_circular_progress_views_with_one_callsite_fill_signal_fill_independently() {
    let source = Binding::container(-1_i32);
    let feed = source.clone();
    let mut app = mount(move || {
        vstack((
            progress(fraction(&feed, 0)).circular().size(80.0, 80.0),
            progress(fraction(&feed, 1)).circular().size(80.0, 80.0),
        ))
    });
    app.settle();
    let mut bars: Vec<_> = app
        .query()
        .role(Role::PROGRESS_INDICATOR)
        .all()
        .iter()
        .cloned()
        .collect();
    bars.sort_by(|a, b| a.bounds().y().total_cmp(&b.bounds().y()));
    assert_eq!(bars.len(), 2, "two progress indicators must be queryable");
    let first_region = region_of(bars[0].bounds());
    let second_region = region_of(bars[1].bounds());
    let before = app.snapshot();

    source.set(1);
    // Half of M3's 500ms circular-determinate motion.
    app.pump_for(Duration::from_millis(250));
    let mid = app.snapshot();
    app.pump_for(Duration::from_millis(700));
    let after = app.snapshot();

    assert_independent_animation(
        &before,
        &mid,
        &after,
        first_region,
        second_region,
        "circular progress fill",
    );
}

/// Two radio pickers whose selection bindings come from one
/// `Binding::mapping` call site: re-selecting the second animates only its
/// indicators.
#[test]
fn two_radio_pickers_with_one_callsite_selection_animates_independently() {
    let picks = Binding::container((0i32, 0i32));
    let feed = picks.clone();
    let mut app = mount(move || {
        let first = selection_channel(&feed, 0);
        let second = selection_channel(&feed, 1);
        let items = || vec![text("One").tag(0i32), text("Two").tag(1i32)];
        vstack((
            Picker::new("First", items(), &first).style(PickerStyle::Radio),
            Picker::new("Second", items(), &second).style(PickerStyle::Radio),
        ))
    });
    app.settle();
    let mut rows: Vec<_> = app
        .query()
        .role(Role::RADIO_BUTTON)
        .all()
        .iter()
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.bounds().y().total_cmp(&b.bounds().y()));
    assert_eq!(rows.len(), 4, "two pickers of two rows each");
    let first_region = union_region(region_of(rows[0].bounds()), region_of(rows[1].bounds()));
    let second_region = union_region(region_of(rows[2].bounds()), region_of(rows[3].bounds()));
    let before = app.snapshot();

    picks.set((0, 1));
    // M3's indicator grow runs 300ms; the fades run 50ms.
    app.pump_for(Duration::from_millis(120));
    let mid = app.snapshot();
    app.pump_for(Duration::from_millis(500));
    let after = app.snapshot();

    assert_independent_animation(
        &before,
        &mid,
        &after,
        first_region,
        second_region,
        "radio indicator selection",
    );
}

/// Two morphing shapes whose progress signals come from one `map` call
/// site: driving the second morphs only it.
#[test]
fn two_morphs_with_one_callsite_progress_signal_morph_independently() {
    let source = Binding::container(-1_i32);
    let feed = source.clone();
    let mut app = mount(move || {
        vstack((
            Circle
                .morph_to(Rectangle, Color::srgb_hex("#FF0000"))
                .progress(channel(&feed, 0, 0.0, 1.0))
                .size(60.0, 60.0),
            Circle
                .morph_to(Rectangle, Color::srgb_hex("#0000FF"))
                .progress(channel(&feed, 1, 0.0, 1.0))
                .size(60.0, 60.0),
        ))
    });
    app.settle();
    let before = app.snapshot();
    // Each shape's own pixels bound its region: the vstack packs at the
    // top of the viewport, so a midline split would cut through them.
    let pad = 8_usize;
    let a_px = color_pixels(&before, 0);
    let b_px = color_pixels(&before, 2);
    let top = (
        a_px.min_x.saturating_sub(pad),
        a_px.min_y.saturating_sub(pad),
        a_px.max_x + pad,
        a_px.max_y + pad,
    );
    let bottom = (
        b_px.min_x.saturating_sub(pad),
        b_px.min_y.saturating_sub(pad),
        b_px.max_x + pad,
        b_px.max_y + pad,
    );

    source.set(1);
    app.pump_for(MIDPOINT);
    let mid = app.snapshot();
    app.pump_for(TRANSITION + Duration::from_millis(200));
    let after = app.snapshot();

    assert_independent_animation(&before, &mid, &after, top, bottom, "morph progress");
}
