//! `AppKit` cases — real `AppKit` objects built through the public API on
//! the real main thread.

use std::cell::Cell;
use std::rc::Rc;

use cocoa_ui::appkit::input_view::InputView;
use cocoa_ui::appkit::{HostView, Label, Window, WindowLevel, WindowStyle};
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2::runtime::Bool;
use cocoa_ui::objc2::{msg_send, sel};
use cocoa_ui::objc2_app_kit::{
    NSBitmapFormat, NSColor, NSTextAlignment, NSTextField, NSTextFieldCell, NSView,
};
use cocoa_ui::objc2_foundation::{
    NSAlignmentOptions, NSArray, NSAttributedString, NSNotFound, NSPoint, NSRange, NSRect, NSSize,
    NSString,
};
use cocoa_ui::{MainThreadMarker, Rect, Size, bitmap, font, text, view};
use libtest_mimic::Trial;

use crate::harness::marker;

/// The suite's `AppKit` cases, named after the module and test they moved
/// from.
pub fn trials() -> Vec<Trial> {
    macro_rules! case {
        ($module:literal, $test:ident) => {
            Trial::test(concat!($module, "::", stringify!($test)), || {
                $test();
                Ok(())
            })
        };
    }
    crate::harness::trials(vec![
        case!(
            "appkit::colors",
            extended_linear_display_p3_preserves_space_and_hdr_channels
        ),
        case!(
            "appkit::host_view",
            a_child_invalidation_inside_layout_does_not_reenter_the_pass
        ),
        case!(
            "appkit::input_view",
            the_text_input_client_selectors_register_under_appkits_names
        ),
        case!(
            "appkit::input_view",
            a_fresh_input_view_reports_no_marked_text
        ),
        case!(
            "appkit::input_view",
            valid_attributes_for_marked_text_returns_attribute_names
        ),
        case!(
            "appkit::display_link",
            a_window_without_a_screen_drives_no_frame_clock
        ),
        case!("appkit::label", a_factory_label_survives_debug_ivar_checks),
        case!(
            "appkit::label",
            a_label_at_a_fractional_origin_rasters_text_on_pixel_bounds
        ),
        case!(
            "appkit::window",
            a_window_starts_hidden_with_the_requested_style
        ),
        case!("appkit::window", closing_a_window_fires_on_close),
        case!(
            "appkit::window",
            the_wrapper_forwards_title_level_size_and_content
        ),
        case!(
            "capture",
            a_capture_claim_restores_containment_and_survives_release
        ),
        case!(
            "capture",
            a_detached_capture_renders_and_teardown_stays_clean
        ),
    ])
}

/// The extended linear Display-P3 constructor must land in that space —
/// not extended linear sRGB — and pass HDR channels straight through
/// instead of clamping or baking headroom away.
fn extended_linear_display_p3_preserves_space_and_hdr_channels() {
    use cocoa_ui::appkit::colors;
    use cocoa_ui::objc2_core_graphics::{
        CGColor, CGColorSpace, kCGColorSpaceExtendedLinearDisplayP3,
    };

    let color = colors::extended_linear_display_p3(1.5, 0.25, 0.5, 0.8);
    let cg = color.CGColor();
    let space = CGColor::color_space(Some(&cg)).expect("an RGB CGColor has a color space");
    let name = CGColorSpace::name(Some(&space)).expect("a named color space reports its name");
    // SAFETY: the static is a `CFString` constant exported by Core Graphics.
    let expected = unsafe { kCGColorSpaceExtendedLinearDisplayP3 }.to_string();
    assert_eq!(name.to_string(), expected);
    assert_eq!(CGColor::number_of_components(Some(&cg)), 4);
    // SAFETY: `components` is valid for `number_of_components` entries.
    let channels = unsafe { std::slice::from_raw_parts(CGColor::components(Some(&cg)), 4) };
    assert!((channels[0] - 1.5).abs() < f64::EPSILON);
    assert!((channels[3] - 0.8).abs() < f64::EPSILON);
}

/// Regression test for the layout-invalidation loop: a label whose width
/// changes inside the parent's layout pass invalidates upward; the old
/// ancestor walk re-entered `layout` synchronously and looped at 85% CPU.
/// The fixed path only marks ancestors dirty, so the handler's call count
/// stays bounded across a few passes.
fn a_child_invalidation_inside_layout_does_not_reenter_the_pass() {
    let mtm = marker();
    let host = HostView::new(mtm, Rect::new(0.0, 0.0, 400.0, 300.0));
    let label = Label::new(mtm);
    label.set_text("content");
    host.add_subview(&label);

    let calls = Rc::new(Cell::new(0));
    host.set_layout_handler({
        let calls = Rc::clone(&calls);
        move |view| {
            calls.set(calls.get() + 1);
            let remaining = view.bounds().size.width - label.intrinsicContentSize().width;
            if calls.get() == 1 && remaining > 40.0 {
                // Mutate a child mid-pass: the label's intrinsic size
                // invalidates upward. A synchronous ancestor walk would
                // recurse into this handler before it returns.
                label.setFrameSize(cocoa_ui::objc2_foundation::NSSize::new(
                    label.frame().size.width + 20.0,
                    label.frame().size.height,
                ));
            }
        }
    });

    host.set_needs_layout();
    host.layout_if_needed();
    // The second pass is itself the flush the mid-pass invalidation waits
    // on: `layoutSubtreeIfNeeded` is the platform's synchronous layout API —
    // no run-loop wait stands in for it.
    host.layout_if_needed();

    assert!(
        calls.get() <= 4,
        "layout re-entered {} times — invalidation must mark, not recurse",
        calls.get(),
    );
    assert!(calls.get() >= 1);
}

/// Regression test for the selector-override defect: `#[unsafe(method(..))]`
/// names compile regardless of spelling, and a method declared under a
/// `snake_case` name was never installed under the `AppKit` selector —
/// `NSTextInputContext::initWithClient:` then threw on the missing
/// `NSTextInputClient` methods. Assert the `ObjC` selectors `AppKit` calls
/// actually resolve on the class.
fn the_text_input_client_selectors_register_under_appkits_names() {
    let mtm = marker();
    // `new` itself performs `initWithClient:` — the call that threw when
    // the selectors were missing.
    let view = InputView::new(mtm);
    for selector in [
        sel!(insertText:replacementRange:),
        sel!(setMarkedText:selectedRange:replacementRange:),
        sel!(unmarkText),
        sel!(selectedRange),
        sel!(markedRange),
        sel!(hasMarkedText),
        sel!(attributedSubstringForProposedRange:actualRange:),
        sel!(validAttributesForMarkedText),
        sel!(firstRectForCharacterRange:actualRange:),
        sel!(characterIndexForPoint:),
        sel!(isFlipped),
    ] {
        // SAFETY: `respondsToSelector:` is a plain `NSObject` query on the
        // live view.
        let responds: Bool = unsafe { msg_send![&*view, respondsToSelector: selector] };
        assert!(
            responds.as_bool(),
            "CocoaUiInputView must respond to {}",
            selector.name().to_string_lossy(),
        );
    }
}

/// `markedRange`/`selectedRange` answer UTF-16 `NSRange`s: an empty document
/// reports `NSNotFound`.
fn a_fresh_input_view_reports_no_marked_text() {
    let mtm = marker();
    let view = InputView::new(mtm);
    // SAFETY: `markedRange` is the view's own getter.
    let marked: NSRange = unsafe { msg_send![&*view, markedRange] };
    assert_eq!(marked.location, NSNotFound.cast_unsigned());
    assert_eq!(marked.length, 0);
    // SAFETY: `hasMarkedText` is the view's own getter.
    let has_marked: Bool = unsafe { msg_send![&*view, hasMarkedText] };
    assert!(!has_marked.as_bool());
}

/// `validAttributesForMarkedText` must answer an `NSArray` of attribute
/// names — `NSTextInputContext` reads it during composition setup.
fn valid_attributes_for_marked_text_returns_attribute_names() {
    let mtm = marker();
    let view = InputView::new(mtm);
    let attributes: Retained<NSArray<NSString>> =
        // SAFETY: `validAttributesForMarkedText` is the view's own getter.
        unsafe { msg_send![&*view, validAttributesForMarkedText] };
    assert!(attributes.count() >= 3);
}

/// Native attachment lifecycle: a window that is truly off every screen
/// reports `screen() == nil`, so `start` leaves the clock disarmed and the
/// bounded run-loop drain stays silent — the screenless safety the removed
/// run-loop arm (a queued raw-pointer hop) violated by ticking anyway.
/// Moving the same window onto a real display lets `reselect` re-arm the
/// still-active request; `stop`/`drop` leave nothing queued.
fn a_window_without_a_screen_drives_no_frame_clock() {
    use cocoa_ui::display_link::FrameClock;
    use cocoa_ui::objc2_app_kit::{NSScreen, NSView};
    use cocoa_ui::objc2_foundation::{NSPoint, NSRect, NSSize};

    let mtm = marker();
    // A point strictly beyond every real screen's frame, derived from the
    // display layout — a borderless window accepts the position verbatim
    // because AppKit's keep-on-screen constraint only applies to windows
    // with a title bar.
    let screens = NSScreen::screens(mtm);
    let offscreen_x = screens
        .iter()
        .map(|screen| screen.frame().origin.x + screen.frame().size.width)
        .fold(0.0, f64::max)
        + 10_000.0;
    let window = Window::new(
        mtm,
        Rect::new(offscreen_x, 0.0, 200.0, 200.0),
        WindowStyle::empty(),
    );
    let frame = window.native().frame();
    let overlaps = |a: NSRect, b: NSRect| {
        a.origin.x < b.origin.x + b.size.width
            && b.origin.x < a.origin.x + a.size.width
            && a.origin.y < b.origin.y + b.size.height
            && b.origin.y < a.origin.y + a.size.height
    };
    assert!(
        screens.iter().all(|s| !overlaps(s.frame(), frame)),
        "the fixture window must sit outside every screen: {frame:?}"
    );
    assert!(
        window.native().screen().is_none(),
        "a window outside every screen frame must report no screen"
    );
    let view = NSView::new(mtm);
    view.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(200.0, 200.0)));
    window.native().setContentView(Some(&view));

    let ticks = Rc::new(Cell::new(0u32));
    let clock = FrameClock::new(mtm, {
        let ticks = Rc::clone(&ticks);
        move || ticks.set(ticks.get() + 1)
    });

    // No screen: `start` leaves the clock disarmed, and a bounded run-loop
    // drain must deliver nothing — no fallback and nothing queued.
    clock.start(&view);
    assert!(
        !clock.is_running(),
        "a screenless window must not arm the clock"
    );
    for _ in 0..10 {
        crate::harness::pump_main_turn();
    }
    assert_eq!(ticks.get(), 0, "a screenless clock must not tick");

    // Re-attach to the actual display: move the window onto a real screen's
    // frame and `reselect` the same clock — the request stayed active, so
    // the link arms without a second `start`.
    let main = NSScreen::screens(mtm)
        .iter()
        .next()
        .expect("the suite requires a real screen")
        .frame();
    window
        .native()
        .setFrameOrigin(NSPoint::new(main.origin.x + 40.0, main.origin.y + 40.0));
    assert!(window.native().screen().is_some());
    clock.reselect(&view);
    assert!(
        clock.is_running(),
        "attaching to a screen must arm the same clock's display link"
    );
    // A visible window's link ticks under the run loop the suite pumps.
    window.native().orderFrontRegardless();
    let fired = crate::harness::pump_main_until(2.0, || ticks.get() > 0);
    assert!(
        fired,
        "a display link on a real visible screen must tick within 2s"
    );

    // `stop`, then `drop`: the queue stays quiet — the removed run-loop arm
    // is where a pending hop could outlive the clock before.
    clock.stop();
    let at_stop = ticks.get();
    for _ in 0..10 {
        crate::harness::pump_main_turn();
    }
    assert!(!clock.is_running());
    assert_eq!(
        ticks.get(),
        at_stop,
        "a stopped clock must deliver no further ticks"
    );
    drop(clock);
    for _ in 0..10 {
        crate::harness::pump_main_turn();
    }
    assert_eq!(
        ticks.get(),
        at_stop,
        "a dropped clock must leave nothing queued"
    );
    window.close();
}

/// The `labelWithString:` factory allocs through `self` without running
/// `set_ivars`: `setFrameSize:` — delivered during the factory's own
/// layout — must not trip the debug initialized-ivars check, and every
/// later `ivars()` access must see the marked flag.
fn a_factory_label_survives_debug_ivar_checks() {
    let mtm = marker();
    let label = Label::label_with_string(mtm, "hello");
    label.set_text("world");
    assert!(label.source_text().is_some());
    label.set_line_limit(1);
}

/// `Label`'s cell anchors its interior's origin on the control view's
/// backing pixels: text at a fractional backing origin must raster onto
/// pixel bounds rather than smearing across them. A stock `NSTextField`
/// pair is the un-anchored reference; a `Label` pair must produce
/// byte-identical glyph rasters translated by exactly the aligned
/// amount — matching at any other translation, or not at all, means
/// glyphs straddle pixels again.
fn a_label_at_a_fractional_origin_rasters_text_on_pixel_bounds() {
    let mtm = marker();
    let (host_w, host_h) = (140.0, 170.0);
    let window = bitmap::make_offscreen_window(mtm, Size::new(host_w, host_h));
    let host = window
        .contentView()
        .expect("the offscreen window's content");
    bitmap::show_capture_window(&window);
    let scale = window.backingScaleFactor();
    assert!(scale > 0.0, "the fixture requires a positive backing scale");
    // Exactly half a device pixel: the worst raster phase at any scale.
    let frac = 0.5 / scale;
    let face = font::system(mtm, 13.0, 0.0);
    let fields = raster_fields(mtm, &host, frac, &face);
    assert_eq!(
        fields.label_frac.frame().origin,
        NSPoint::new(8.0 + frac, 20.0),
        "the fractional frame must stay exactly as set — nothing may snap it"
    );

    // The expected ink translation comes from the view's own backing
    // anchor — the same native API the cell uses — not an assumed
    // rounding rule.
    let anchor = fields.label_frac.backingAlignedRect_options(
        NSRect::new(NSPoint::ZERO, NSSize::ZERO),
        NSAlignmentOptions::AlignMinXNearest
            | NSAlignmentOptions::AlignMinYNearest
            | NSAlignmentOptions::AlignWidthNearest
            | NSAlignmentOptions::AlignHeightNearest,
    );
    let shift = device_px((anchor.origin.x * scale).round());
    let (field_w, field_h) = (
        device_px((FIELD_SIZE.0 * scale).round()),
        device_px((FIELD_SIZE.1 * scale).round()),
    );

    let raster = HostRaster::capture(&host, host_w, host_h, scale);
    let stock_int_crop = raster.crop(fields.stock_int.frame());
    let stock_frac_crop = raster.crop(fields.stock_frac.frame());
    let label_int_crop = raster.crop(fields.label_int.frame());
    let label_frac_crop = raster.crop(fields.label_frac.frame());
    for (name, bytes) in [
        ("stock_int", &stock_int_crop),
        ("stock_frac", &stock_frac_crop),
        ("label_int", &label_int_crop),
        ("label_frac", &label_frac_crop),
    ] {
        assert!(
            glyph_ink(bytes) > 0,
            "the {name} crop must contain actual blue glyph ink"
        );
    }

    // The anchored cell must produce the identical glyph raster at a
    // fractional backing origin — byte-for-byte the integer raster
    // translated by the anchor's actual amount.
    assert_eq!(
        shifted_diffs(&label_frac_crop, &label_int_crop, field_w, field_h, shift),
        0,
        "fractional-origin ink must equal integer-origin ink translated \
         by the anchor's {shift} device pixel(s)"
    );
    // Negative control: the unfixed stock cell must still smear — if the
    // stock pair ever matches, the fixture stopped measuring raster phase
    // and the assertion above proves nothing.
    assert!(
        shifted_diffs(&stock_frac_crop, &stock_int_crop, field_w, field_h, shift) > 100,
        "the stock control must still show the unfixed smear"
    );

    // A text update re-renders through the same anchored interior: new ink
    // must differ from the old render's.
    let updated_content = text::build(
        mtm,
        &[blue_run("Updated wrapped content draws differently", &face)],
    );
    fields.label_frac.set_attributed_text(&updated_content);
    let updated = HostRaster::capture(&host, host_w, host_h, scale);
    let updated_crop = updated.crop(fields.label_frac.frame());
    assert!(
        glyph_ink(&updated_crop) > 0 && updated_crop != label_frac_crop,
        "a replaced attributed string must change the raster"
    );

    // A rotated host transform preserves the label's own layout — the
    // anchor rides the native transform rather than snapping geometry —
    // and the text still draws.
    fields.label_frac.setFrameRotation(30.0);
    let bounds_before = fields.label_frac.bounds().size;
    let rotated = HostRaster::capture(&host, host_w, host_h, scale);
    assert_eq!(
        fields.label_frac.bounds().size,
        bounds_before,
        "rotation must not disturb the label's own bounds"
    );
    assert!(
        glyph_ink(&rotated.crop(fields.label_frac.frame())) > 0,
        "a rotated label must still raster ink"
    );
    fields.label_frac.setFrameRotation(0.0);

    bitmap::close_capture_window(&window);
}

/// The frame size every fixture field shares.
const FIELD_SIZE: (f64, f64) = (96.0, 30.0);

/// The text run every fixture field draws.
const fn blue_run<'a>(content: &'a str, face: &'a font::Font) -> text::TextRun<'a> {
    text::TextRun {
        text: content,
        font: face,
        foreground: None,
        background: None,
        underline: false,
        strikethrough: false,
        letter_spacing: 0.0,
        line_height: 0.0,
    }
}

/// An already-rounded device-pixel count as `usize`. The fixture's device
/// extents are a few hundred pixels by construction; `f64` has no checked
/// `usize` conversion, so the bound is asserted by hand before the cast.
#[expect(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "no checked f64-to-usize conversion exists; the asserted bound proves the cast exact"
)]
const fn device_px(px: f64) -> usize {
    assert!(
        px >= 0.0 && px <= 1_000_000.0,
        "a fixture extent must land on a small nonnegative device pixel count"
    );
    px as usize
}

/// The four identically configured fields of the raster fixture.
struct RasterFields {
    stock_int: Retained<NSTextField>,
    stock_frac: Retained<NSTextField>,
    label_int: Retained<Label>,
    label_frac: Retained<Label>,
}

/// The identical configuration on every field — the same string, font,
/// color, alignment and wrapping, so crops differ only in raster phase.
/// Left alignment keeps every glyph's pen a pure translation of the
/// interior's origin — a centered line would place each glyph at its own
/// fractional phase and defeat the shifted-equality check.
fn configure_field(field: &NSTextField, content: &NSAttributedString, blue: &NSColor) {
    field.setAlignment(NSTextAlignment::Left);
    field.setAttributedStringValue(content);
    field.setTextColor(Some(blue));
    let cell = field
        .cell()
        .and_then(|cell| cell.downcast::<NSTextFieldCell>().ok())
        .expect("a label-style field carries a text cell");
    cell.setWraps(true);
    cell.setScrollable(false);
}

/// Builds the four-field fixture on `host`: a stock `NSTextField` pair —
/// the unfixed reference, whose smear proves the fixture sees raster
/// phase at all — and a `Label` pair, one of each at an integer and a
/// `frac`-device-pixel origin, stacked in disjoint vertical rows.
fn raster_fields(
    mtm: MainThreadMarker,
    host: &NSView,
    frac: f64,
    face: &font::Font,
) -> RasterFields {
    let content = text::build(
        mtm,
        &[blue_run(
            "Fractional origins still raster on pixel bounds",
            face,
        )],
    );
    let blue = NSColor::systemBlueColor();
    let fields = RasterFields {
        stock_int: NSTextField::labelWithString(&NSString::from_str(""), mtm),
        stock_frac: NSTextField::labelWithString(&NSString::from_str(""), mtm),
        label_int: Label::label_with_string(mtm, ""),
        label_frac: Label::label_with_string(mtm, ""),
    };
    let all: [&NSTextField; 4] = [
        &fields.stock_int,
        &fields.stock_frac,
        &fields.label_int,
        &fields.label_frac,
    ];
    for field in all {
        configure_field(field, &content, &blue);
    }
    let (width, height) = FIELD_SIZE;
    let rows: [(&NSTextField, f64, f64); 4] = [
        (&fields.stock_int, 8.0, 134.0),
        (&fields.stock_frac, 8.0 + frac, 96.0),
        (&fields.label_int, 8.0, 58.0),
        (&fields.label_frac, 8.0 + frac, 20.0),
    ];
    for (field, x, y) in rows {
        field.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
        host.addSubview(field);
    }
    fields
}

/// A rendered host's raw device pixels plus the crop helper the raster
/// assertions are stated over.
struct HostRaster {
    pixels: Vec<u8>,
    pixel_w: usize,
    scale: f64,
    host_h: f64,
}

impl HostRaster {
    /// `cacheDisplay`s `host` into its own rep and returns the raw bytes,
    /// top-down RGBA — no CGImage/context redraw, so comparisons read
    /// exactly what the native paint path wrote. The rep's declared
    /// geometry and byte layout are verified before a byte is read.
    fn capture(host: &NSView, host_w: f64, host_h: f64, scale: f64) -> Self {
        bitmap::force_text_fields_display(host);
        let rep = view::bitmap_rep_for_caching_display(host).expect("a caching rep");
        view::cache_display(host, &rep);
        let pixel_w = device_px((host_w * scale).round());
        let pixel_h = device_px((host_h * scale).round());
        assert_eq!(
            (
                usize::try_from(rep.pixelsWide()),
                usize::try_from(rep.pixelsHigh()),
            ),
            (Ok(pixel_w), Ok(pixel_h)),
            "the rep must cover the host's whole device rect"
        );
        assert_eq!(
            (
                usize::try_from(rep.samplesPerPixel()),
                usize::try_from(rep.bitsPerPixel()),
            ),
            (Ok(4), Ok(32)),
            "the rep must be interleaved 8-bit RGBA"
        );
        assert!(!rep.isPlanar(), "the rep must be interleaved, not planar");
        assert!(
            !rep.bitmapFormat().contains(NSBitmapFormat::AlphaFirst),
            "the rep must lay out alpha last — the ink predicate reads RGBA"
        );
        let row_bytes =
            usize::try_from(rep.bytesPerRow()).expect("the rep's row stride is nonnegative");
        assert!(
            row_bytes >= pixel_w * 4,
            "the rep's stride must cover a full RGBA row"
        );
        let data = rep.bitmapData();
        assert!(!data.is_null(), "the rep must expose its raw pixels");
        let mut pixels = vec![0u8; pixel_w * pixel_h * 4];
        for row in 0..pixel_h {
            // SAFETY: `data` is the rep's live bitmap storage; `row_bytes`
            // spans `pixel_w * 4` bytes and `row` stays inside `pixelsHigh`.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.add(row * row_bytes),
                    pixels.as_mut_ptr().add(row * pixel_w * 4),
                    pixel_w * 4,
                );
            }
        }
        Self {
            pixels,
            pixel_w,
            scale,
            host_h,
        }
    }

    /// A field's device rect as a top-down row/column crop (view y is
    /// bottom-up).
    fn crop(&self, frame: NSRect) -> Vec<u8> {
        let (crop_w, crop_h) = (
            device_px((frame.size.width * self.scale).round()),
            device_px((frame.size.height * self.scale).round()),
        );
        let x0 = device_px((frame.origin.x * self.scale).floor());
        let y0 =
            device_px(((self.host_h - frame.origin.y - frame.size.height) * self.scale).round());
        (0..crop_h)
            .flat_map(|row| {
                let start = (y0 + row) * self.pixel_w * 4 + x0 * 4;
                self.pixels[start..start + crop_w * 4].to_vec()
            })
            .collect()
    }
}

/// Known-color glyph ink inside a crop: the explicit blue text color — a
/// bare alpha count would pass on an opaque background too. A presence
/// count only: coverage scales with backing density, so it carries no
/// fixed minimum.
fn glyph_ink(pixels: &[u8]) -> usize {
    crate::harness::count_pixels(pixels, [0, 80, 180, 200], [80, 170, 255, 255])
}

/// Two equal-size crops compared under a device-pixel x-translation: the
/// number of byte-differing pixels.
fn shifted_diffs(a: &[u8], b: &[u8], width: usize, height: usize, dx: usize) -> usize {
    (0..height)
        .map(|row| {
            let start = row * width * 4;
            a[start + dx * 4..start + width * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .zip(b[start..start + width * 4 - dx * 4].as_chunks::<4>().0)
                .filter(|(pa, pb)| pa != pb)
                .count()
        })
        .sum()
}

/// The case the old harness excluded: `-[NSWindow initWithContentRect:]`
/// throws `NSInternalInconsistency` off `pthread_main`, so a real `NSWindow`
/// could only exist once the suite moved to the true main thread. A fresh
/// window is hidden, keeps the style it was handed, and reports the content
/// area it was asked for. `FULL_SCREEN` is `AppKit`'s own state bit, not a
/// part to request, so the style goes in without it.
fn a_window_starts_hidden_with_the_requested_style() {
    let mtm = marker();
    let style = WindowStyle::all() - WindowStyle::FULL_SCREEN;
    let window = Window::new(mtm, Rect::new(100.0, 100.0, 480.0, 320.0), style);
    assert!(!window.is_visible());
    assert!(!window.is_fullscreen());
    assert_eq!(window.style_mask(), style);
    assert_eq!(window.content_rect().size, Size::new(480.0, 320.0));
    window.close();
}

/// `close()` must route through `windowWillClose:` — the notification the
/// kit's delegate observes — so the installed `on_close` handler runs.
fn closing_a_window_fires_on_close() {
    let mtm = marker();
    let window = Window::new(mtm, Rect::ZERO, WindowStyle::TITLED | WindowStyle::CLOSABLE);
    let closed = Rc::new(Cell::new(false));
    window.on_close({
        let closed = Rc::clone(&closed);
        move || closed.set(true)
    });
    window.close();
    assert!(
        closed.get(),
        "windowWillClose: never reached the on_close handler"
    );
}

/// Wrapper forwarding: property setters must land on the native `NSWindow`,
/// notifications must reach the `on_*` handlers, and a `HostView` installed
/// as content must report the window it lives in.
fn the_wrapper_forwards_title_level_size_and_content() {
    let mtm = marker();
    let window = Window::new(
        mtm,
        Rect::new(0.0, 0.0, 400.0, 300.0),
        WindowStyle::all() - WindowStyle::FULL_SCREEN,
    );

    window.set_title("native suite");
    assert_eq!(window.native().title().to_string(), "native suite");

    window.set_level(WindowLevel::Floating);
    assert_eq!(window.level(), Some(WindowLevel::Floating));
    window.set_level(WindowLevel::Normal);
    assert_eq!(window.level(), Some(WindowLevel::Normal));

    window.set_content_min_size(Size::new(200.0, 150.0));
    assert_eq!(window.content_min_size(), Size::new(200.0, 150.0));

    let resized = Rc::new(Cell::new(false));
    window.on_resize({
        let resized = Rc::clone(&resized);
        move || resized.set(true)
    });
    window.set_content_rect(Rect::new(0.0, 0.0, 500.0, 350.0), false);
    assert!(
        resized.get(),
        "windowDidResize: never reached the on_resize handler"
    );
    assert_eq!(window.content_rect().size, Size::new(500.0, 350.0));

    let host = HostView::new(mtm, window.content_rect());
    window.set_content_view(&host);
    assert!(cocoa_ui::view::has_window(&host));
    let reported = cocoa_ui::view::window(&host).expect("a hosted view reports its window");
    assert!(std::ptr::eq(&raw const *reported, window.native()));

    window.close();
}

/// The `CARenderer` root claim is scoped to the frame it encodes: two
/// cached `ViewCapture`s bound to nested views run child→parent claim
/// cycles, and after each the views report the same superview, sibling
/// order, superlayer, frame, and hidden flag as before — and they keep
/// answering them after both cached renderers are released on a later
/// main-queue turn. This is the nested-claim shape the production
/// ownership fix exists for.
#[allow(clippy::too_many_lines)] // The nested fixture plus the containment contract it checks.
fn a_capture_claim_restores_containment_and_survives_release() {
    use std::rc::Rc;

    use cocoa_ui::capture::ViewCapture;
    use cocoa_ui::objc2_app_kit::{NSColor, NSView};
    use cocoa_ui::objc2_foundation::NSSize;

    let mtm = marker();
    // AppKit only materializes the layer hierarchy under a window: a
    // detached view tree leaves `layer.superlayer` unwired, so the
    // containment-restore contract needs a real host window.
    let window = Window::new(
        mtm,
        Rect::new(0.0, 0.0, 400.0, 400.0),
        WindowStyle::all() - WindowStyle::FULL_SCREEN,
    );
    let parent = NSView::new(mtm);
    parent.setFrameSize(NSSize::new(400.0, 400.0));
    parent.setWantsLayer(true);
    window.native().setContentView(Some(&parent));
    window.native().orderFrontRegardless();
    crate::harness::pump_main_turn();

    let content = NSView::new(mtm);
    content.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(8.0, 40.0),
        NSSize::new(200.0, 200.0),
    ));
    content.setWantsLayer(true);
    content
        .layer()
        .expect("a wanted layer exists")
        .setBackgroundColor(Some(&NSColor::blueColor().CGColor()));
    // A non-identity transform is the preservation case. AppKit owns the
    // backing layer's transform and re-syncs it from `frameRotation`
    // during layout, so the fixture uses the view-level rotation the
    // platform persists rather than a raw `CATransform3D` AppKit would
    // revert on the next layout pass — the claim's restore must coexist
    // with that, not fight it.
    content.setFrameRotation(30.0);
    let label = Label::new(mtm);
    label.set_text("capture-claim");
    label.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(4.0, 8.0),
        NSSize::new(160.0, 24.0),
    ));
    content.addSubview(&label);
    // The nested claim: a second capturable subtree inside `content`.
    let child = NSView::new(mtm);
    child.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(20.0, 60.0),
        NSSize::new(120.0, 120.0),
    ));
    child.setWantsLayer(true);
    child
        .layer()
        .expect("a wanted layer exists")
        .setBackgroundColor(Some(&NSColor::orangeColor().CGColor()));
    let child_label = Label::new(mtm);
    child_label.set_text("nested");
    child_label.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(4.0, 8.0),
        NSSize::new(100.0, 24.0),
    ));
    child.addSubview(&child_label);
    content.addSubview(&child);
    parent.addSubview(&content);
    let sibling = NSView::new(mtm);
    sibling.setFrame(cocoa_ui::objc2_foundation::NSRect::new(
        cocoa_ui::objc2_foundation::NSPoint::new(8.0, 280.0),
        NSSize::new(40.0, 40.0),
    ));
    parent.addSubview(&sibling);
    // Let AppKit's layout pass wire the view hierarchy into the layer
    // hierarchy before the snapshots the claim will be held to.
    crate::harness::pump_main_turn();

    let frame = content.frame();
    let layer_transform = content.layer().expect("a wanted layer exists").transform();
    let child_frame = child.frame();

    let target = crate::harness::capture_target();
    let content_capture = Rc::new(ViewCapture::new(mtm, content.clone(), |_| None));
    content_capture.set_on_redraw(|| {});
    let child_capture = Rc::new(ViewCapture::new(mtm, child.clone(), |_| None));
    child_capture.set_on_redraw(|| {});

    // Everything the claim owes the tree: superview and ordered
    // siblings, the model layer's own superlayer, geometry, and the
    // full transform — checked after every cycle and again after both
    // cached renderers are released.
    let check = || {
        let restored_parent = cocoa_ui::view::superview(&content)
            .expect("the claim left the view detached from its parent");
        assert!(std::ptr::eq(
            &raw const *restored_parent,
            &raw const *parent
        ));
        let order = parent.subviews();
        assert_eq!(order.count(), 2);
        assert!(std::ptr::eq(
            &raw const *order.objectAtIndex(0),
            &raw const *content
        ));
        assert!(std::ptr::eq(
            &raw const *order.objectAtIndex(1),
            &raw const *sibling
        ));
        let child_parent =
            cocoa_ui::view::superview(&child).expect("the nested claim left the child detached");
        assert!(std::ptr::eq(&raw const *child_parent, &raw const *content));
        let content_order = content.subviews();
        assert_eq!(content_order.count(), 2);
        assert!(std::ptr::eq(
            &raw const *content_order.objectAtIndex(0),
            std::ptr::from_ref(&*label).cast()
        ));
        assert!(std::ptr::eq(
            &raw const *content_order.objectAtIndex(1),
            &raw const *child
        ));
        let child_order = child.subviews();
        assert_eq!(child_order.count(), 1);
        assert!(std::ptr::eq(
            &raw const *child_order.objectAtIndex(0),
            std::ptr::from_ref(&*child_label).cast()
        ));
        assert_eq!(content.frame(), frame);
        assert_eq!(child.frame(), child_frame);
        assert!(
            (content.frameRotation() - 30.0).abs() < 1e-6,
            "the claim must preserve the view's rotation"
        );
        let actual_transform = content.layer().expect("a wanted layer exists").transform();
        assert!(
            actual_transform.equal_to_transform(layer_transform),
            "the claim must preserve the layer's full transform: {actual_transform:?} vs {layer_transform:?}"
        );
        assert!(
            content
                .layer()
                .is_some_and(|layer| layer.superlayer().is_some()),
            "the model layer must keep its superlayer"
        );
        assert!(
            child
                .layer()
                .is_some_and(|layer| layer.superlayer().is_some()),
            "the nested model layer must keep its superlayer"
        );
        assert!(!content.isHidden());
    };

    // Three nested cycles: child claim first, then the enclosing content
    // claim — each against the same cached renderers, each proven by
    // its own completed, successful fence.
    for _cycle in 0..3 {
        let (flag, complete) = crate::harness::fence_flag();
        child_capture.capture(&target, complete);
        crate::harness::await_fence(&flag, "child");
        let (flag, complete) = crate::harness::fence_flag();
        content_capture.capture(&target, complete);
        crate::harness::await_fence(&flag, "content");
        check();
    }

    // A layer claimed out of a window's render context encodes an empty
    // frame on a host without an app compositor, so pixel fidelity is
    // proven by the detached arm below; this arm proves the capture
    // completed and the whole containment contract survived it.

    // Releasing the cached renderers must not invalidate the layers
    // they claimed: on the next real main-queue turn both trees answer,
    // still attached, and tear down normally.
    content_capture.shutdown();
    child_capture.shutdown();
    drop(content_capture);
    drop(child_capture);
    crate::harness::pump_main_turn();
    check();

    content.removeFromSuperview();
    sibling.removeFromSuperview();
    parent.removeFromSuperview();
    window.close();
}

/// Capturing a view with no superview is supported — the claim restores
/// nothing, the frame still renders real content, and teardown after
/// the cached renderer's release stays clean.
fn a_detached_capture_renders_and_teardown_stays_clean() {
    use std::rc::Rc;

    use cocoa_ui::capture::ViewCapture;
    use cocoa_ui::objc2_app_kit::{NSColor, NSView};
    use cocoa_ui::objc2_foundation::{NSPoint, NSRect, NSSize};

    let mtm = marker();
    let content = NSView::new(mtm);
    content.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(200.0, 200.0),
    ));
    content.setWantsLayer(true);
    content
        .layer()
        .expect("a wanted layer exists")
        .setBackgroundColor(Some(&NSColor::orangeColor().CGColor()));
    let label = Label::new(mtm);
    // A contrasting glyph color so the readback can tell real label
    // rendering from a flat filled frame.
    label.setTextColor(Some(&NSColor::whiteColor()));
    label.set_text("detached");
    label.setWantsLayer(true);
    label.setFrame(NSRect::new(
        NSPoint::new(4.0, 60.0),
        NSSize::new(150.0, 24.0),
    ));
    content.addSubview(&label);

    // A parentless view gets no window update cycle, so a brief window
    // residency rasterizes the label's text into its backing layer
    // first — the renderer composites `layer.contents`, which only a
    // real display pass fills — before the claim detaches it again.
    {
        let window = Window::new(
            mtm,
            Rect::new(0.0, 0.0, 200.0, 200.0),
            WindowStyle::all() - WindowStyle::FULL_SCREEN,
        );
        let host = NSView::new(mtm);
        host.setFrameSize(NSSize::new(200.0, 200.0));
        host.setWantsLayer(true);
        window.native().setContentView(Some(&host));
        host.addSubview(&content);
        window.native().orderFrontRegardless();
        crate::harness::pump_main_turn();
        content.removeFromSuperview();
        window.close();
    }

    assert!(cocoa_ui::view::superview(&content).is_none());

    let target = crate::harness::capture_target();
    let capture = Rc::new(ViewCapture::new(mtm, content.clone(), |_| None));
    capture.set_on_redraw(|| {});
    let (flag, complete) = crate::harness::fence_flag();
    capture.capture(&target, complete);
    assert!(
        cocoa_ui::view::superview(&content).is_none(),
        "a detached capture must not invent a parent"
    );
    crate::harness::await_fence(&flag, "detached");

    let texels = crate::harness::readback(&target);
    assert!(
        crate::harness::count_pixels(&texels, [0, 110, 220, 255], [60, 200, 255, 255]) > 5_000,
        "a detached capture must render the view's own color"
    );
    assert!(
        crate::harness::count_pixels(&texels, [220, 220, 220, 255], [255, 255, 255, 255]) > 20,
        "a detached capture must render real label glyphs"
    );

    capture.shutdown();
    drop(capture);
    crate::harness::pump_main_turn();
    let _layer = content.layer(); // crashes on an invalidated layer
    label.removeFromSuperview();
}
