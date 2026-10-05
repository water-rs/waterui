//! Native safe-area policy shared by roots and transparent containers.
//!
//! The safe area has two regions on each edge — the *container* region
//! (system bars, cutouts, the home indicator) and the *keyboard* region
//! (the software keyboard). A view is laid out clear of every region it
//! does not ignore; a fill — a view whose painted surface is a color, a
//! gradient or a material — extends past its frame into the bands it
//! touches; a scroll surface or chrome container extends under the bands
//! and insets its content by them.
//!
//! `UIKit` reports the container region through `safeAreaInsets`. The
//! keyboard region is geometric: the window root tracks the keyboard's
//! frame from `UIKit`'s keyboard notifications (`cocoaUiKeyboardFrame`)
//! and the inset a view measures is the depth of that rect inside the
//! view's own window frame, on the edges the rect covers. A scroll surface
//! owns the keyboard's avoidance for its whole subtree, so inside one the
//! keyboard inset reads zero.

use cocoa_ui::objc2_foundation::NSObjectProtocol;
use cocoa_ui::{PlatformView, Rect, view};

#[cfg(target_os = "ios")]
use cocoa_ui::geometry::EdgeInsets;
#[cfg(target_os = "ios")]
use objc2_core_foundation::CGRect;

/// The mask bit `cocoaUiIgnoredSafeAreaEdges` sets on an ignorer; the
/// remaining bits are the container `Edges` mask in bits 0–3 and the
/// keyboard `Edges` mask in bits 4–7.
#[cfg(target_os = "ios")]
const IGNORER_MARK: u16 = 0x100;

fn owns_safe_area(view: &PlatformView) -> bool {
    #[cfg(target_os = "macos")]
    if view
        .downcast_ref::<cocoa_ui::objc2_app_kit::NSScrollView>()
        .is_some()
    {
        return true;
    }
    #[cfg(target_os = "ios")]
    if view
        .downcast_ref::<cocoa_ui::objc2_ui_kit::UIScrollView>()
        .is_some()
    {
        return true;
    }
    if view.respondsToSelector(objc2::sel!(cocoaUiManagesSafeArea)) {
        // SAFETY: cocoa-ui's host classes declare this selector as a boolean query.
        unsafe { objc2::msg_send![view, cocoaUiManagesSafeArea] }
    } else {
        false
    }
}

/// Whether `view` is a scroll surface — the boundary at which both the
/// keyboard inset and the ignored-edge accumulation stop.
#[cfg(target_os = "ios")]
fn is_scroll_surface(view: &PlatformView) -> bool {
    view.downcast_ref::<cocoa_ui::objc2_ui_kit::UIScrollView>()
        .is_some()
}

/// Whether `view` declares itself a fill through `cocoaUiIsFill` — the
/// color, gradient and material leaves do; every other view does not.
fn is_fill(view: &PlatformView) -> bool {
    view.respondsToSelector(objc2::sel!(cocoaUiIsFill))
        // SAFETY: cocoa-ui's classes declare this selector as a boolean query.
        && unsafe { objc2::msg_send![view, cocoaUiIsFill] }
}

pub fn manages_safe_area(view: &PlatformView) -> bool {
    if owns_safe_area(view) {
        return true;
    }
    view::primary_content(view).is_some_and(|child| manages_safe_area(&child))
}

/// Whether the extension rules apply to `view` as a child: safe-area
/// managers and fills both extend past their placed frame into the bands
/// their accumulated mask leaves reachable.
pub fn extends_into_bands(view: &PlatformView) -> bool {
    manages_safe_area(view) || is_fill(view)
}

/// The mask `view` itself declares through `cocoaUiIgnoredSafeAreaEdges`,
/// mark bit included.
#[cfg(target_os = "ios")]
fn declared_mask(view: &PlatformView) -> u16 {
    if view.respondsToSelector(objc2::sel!(cocoaUiIgnoredSafeAreaEdges)) {
        // SAFETY: cocoa-ui declares this selector as an NSInteger edge mask.
        let mask: isize = unsafe { objc2::msg_send![view, cocoaUiIgnoredSafeAreaEdges] };
        u16::try_from(mask & 0x1ff).unwrap_or(0)
    } else {
        0
    }
}

/// The `(region, edge)` pairs `view`'s ancestors mark ignored, plus its own
/// declaration — marks accumulate through containers and stop at a scroll
/// surface, which owns the safe-area contract for its subtree.
#[cfg(target_os = "ios")]
pub fn accumulated_mask(view: &PlatformView) -> u16 {
    let mut mask = declared_mask(view) & !IGNORER_MARK;
    let mut ancestor = view::superview(view);
    while let Some(current) = ancestor {
        mask |= declared_mask(&current) & !IGNORER_MARK;
        if is_scroll_surface(&current) {
            break;
        }
        ancestor = view::superview(&current);
    }
    mask
}

/// Whether `view` sits inside a scroll surface, whose subtree sees no
/// keyboard inset.
#[cfg(target_os = "ios")]
fn inside_scroll_surface(view: &PlatformView) -> bool {
    let mut ancestor = view::superview(view);
    while let Some(current) = ancestor {
        if is_scroll_surface(&current) {
            return true;
        }
        ancestor = view::superview(&current);
    }
    false
}

/// The keyboard's frame in the view's window coordinates, tracked by the
/// window root from `UIKit`'s keyboard notifications; `CGRect::ZERO` when
/// there is no keyboard, no window, or no tracking root.
#[cfg(target_os = "ios")]
fn keyboard_rect(view: &PlatformView) -> CGRect {
    let Some(window) = view::window(view) else {
        return CGRect::ZERO;
    };
    let Some(root) = window.rootViewController().and_then(|c| c.view()) else {
        return CGRect::ZERO;
    };
    if root.respondsToSelector(objc2::sel!(cocoaUiKeyboardFrame)) {
        // SAFETY: the kit's window root declares this selector as a `CGRect`
        // read of its tracked keyboard frame.
        unsafe { objc2::msg_send![&*root, cocoaUiKeyboardFrame] }
    } else {
        CGRect::ZERO
    }
}

/// The depth `band` eats into `frame` on each edge — positive only where
/// the band's extent reaches that edge of the frame and overlaps it on the
/// crossing axis. `frame` and `band` share a coordinate space.
#[cfg(target_os = "ios")]
fn band_depths(frame: CGRect, band: CGRect) -> EdgeInsets {
    let (frame_min_x, frame_max_x) = (frame.origin.x, frame.origin.x + frame.size.width);
    let (frame_min_y, frame_max_y) = (frame.origin.y, frame.origin.y + frame.size.height);
    let (band_min_x, band_max_x) = (band.origin.x, band.origin.x + band.size.width);
    let (band_min_y, band_max_y) = (band.origin.y, band.origin.y + band.size.height);
    let across_x = band_min_x < frame_max_x && band_max_x > frame_min_x;
    let across_y = band_min_y < frame_max_y && band_max_y > frame_min_y;
    // An edge's depth counts only where the band attaches to that edge and
    // does not continue past the opposite one: a band spanning the frame on
    // an axis intrudes only through the perpendicular edges it crosses.
    EdgeInsets::new(
        if band_min_y <= frame_min_y
            && band_max_y > frame_min_y
            && band_max_y < frame_max_y
            && across_x
        {
            (band_max_y - frame_min_y).min(frame.size.height)
        } else {
            0.0
        },
        if band_min_x <= frame_min_x
            && band_max_x > frame_min_x
            && band_max_x < frame_max_x
            && across_y
        {
            (band_max_x - frame_min_x).min(frame.size.width)
        } else {
            0.0
        },
        if band_max_y >= frame_max_y
            && band_min_y < frame_max_y
            && band_min_y > frame_min_y
            && across_x
        {
            (frame_max_y - band_min_y).min(frame.size.height)
        } else {
            0.0
        },
        if band_max_x >= frame_max_x
            && band_min_x < frame_max_x
            && band_min_x > frame_min_x
            && across_y
        {
            (frame_max_x - band_min_x).min(frame.size.width)
        } else {
            0.0
        },
    )
}

/// The two regions' insets as `view` measures them: the container region
/// `UIKit` reports, and the keyboard band's depth inside `view`'s own
/// window frame — zero inside a scroll surface.
#[cfg(target_os = "ios")]
fn region_insets(view: &PlatformView) -> (EdgeInsets, EdgeInsets) {
    let cocoa_ui::objc2_ui_kit::UIEdgeInsets {
        top,
        left,
        bottom,
        right,
    } = view.safeAreaInsets();
    let container = EdgeInsets::new(top, left, bottom, right);
    let keyboard = if inside_scroll_surface(view) {
        EdgeInsets::ZERO
    } else {
        let band = keyboard_rect(view);
        if band.size.width <= 0.0 || band.size.height <= 0.0 {
            EdgeInsets::ZERO
        } else {
            band_depths(view.convertRect_toView(view.bounds(), None), band)
        }
    };
    (container, keyboard)
}

/// `Edges`-bit helpers on a region mask: whether region `region` (0 =
/// container, 1 = keyboard) is marked on the bit `bit` (the `Edges::mask`
/// order — top, leading, bottom, trailing).
#[cfg(target_os = "ios")]
const fn mask_marks(mask: u16, region: u16, bit: u16) -> bool {
    mask & (1 << (region * 4 + bit)) != 0
}

/// The inset a non-ignoring child of `view` avoids on each edge: the
/// deepest unmarked region on that edge.
#[cfg(target_os = "ios")]
fn avoided_insets(container: &EdgeInsets, keyboard: &EdgeInsets, mask: u16) -> EdgeInsets {
    let pick = |bit: u16, c: f64, k: f64| {
        let c = if mask_marks(mask, 0, bit) { 0.0 } else { c };
        let k = if mask_marks(mask, 1, bit) { 0.0 } else { k };
        c.max(k)
    };
    EdgeInsets::new(
        pick(0, container.top, keyboard.top),
        pick(1, container.left, keyboard.left),
        pick(2, container.bottom, keyboard.bottom),
        pick(3, container.right, keyboard.right),
    )
}

#[cfg(target_os = "macos")]
pub fn safe_area_rect(view: &PlatformView) -> Rect {
    view.safeAreaRect().into()
}

#[cfg(target_os = "ios")]
pub fn safe_area_rect(view: &PlatformView) -> Rect {
    let (container, keyboard) = region_insets(view);
    let insets = avoided_insets(&container, &keyboard, accumulated_mask(view));
    let bounds = view::bounds(view);
    let width = bounds.size.width - insets.left - insets.right;
    let height = bounds.size.height - insets.top - insets.bottom;
    if width < 0.0 || height < 0.0 {
        Rect::new(bounds.origin.x, bounds.origin.y, 0.0, 0.0)
    } else {
        Rect::new(
            bounds.origin.x + insets.left,
            bounds.origin.y + insets.top,
            width,
            height,
        )
    }
}

/// The rect a band-reaching child of `host` may extend to, in `host`'s
/// coordinates.
///
/// A bare child (no accumulated marks) reaches the bounds edge on every
/// edge. A marked edge releases through the marked regions' zones, then
/// stops at the nearest unmarked zone — a view that ignores only the
/// keyboard still ends above the container band. An edge with no mark does
/// not extend: its target stays the avoided edge.
#[cfg(target_os = "ios")]
pub fn band_target(host: &PlatformView, mask: u16) -> Rect {
    let bounds = view::bounds(host);
    if mask == 0 {
        return bounds;
    }
    let (container, keyboard) = region_insets(host);
    // The depth from `edge` the child may not reach past: the avoided depth
    // itself when no region on the edge is marked (no extension), else the
    // shallowest unmarked depth — an extension never paints inside a region
    // it does not ignore — or zero when every region is marked, letting the
    // child run to the window's edge.
    let gap = |bit: u16, c: f64, k: f64| -> f64 {
        let regions = [(mask_marks(mask, 0, bit), c), (mask_marks(mask, 1, bit), k)];
        if !regions.iter().any(|(marked, _)| *marked) {
            return c.max(k);
        }
        let nearest_unmarked = regions
            .iter()
            .filter(|(marked, _)| !marked)
            .map(|(_, depth)| *depth)
            .fold(f64::INFINITY, f64::min);
        if nearest_unmarked.is_infinite() {
            0.0
        } else {
            nearest_unmarked
        }
    };
    let top = gap(0, container.top, keyboard.top);
    let left = gap(1, container.left, keyboard.left);
    let bottom = gap(2, container.bottom, keyboard.bottom);
    let right = gap(3, container.right, keyboard.right);
    Rect::new(
        bounds.origin.x + left,
        bounds.origin.y + top,
        bounds.size.width - left - right,
        bounds.size.height - top - bottom,
    )
}

#[cfg(target_os = "ios")]
pub fn content_frame(content: &PlatformView, host: &PlatformView) -> Rect {
    let safe = safe_area_rect(host);
    if extends_into_bands(content) {
        safe.extended_through(safe, band_target(host, accumulated_mask(content)))
    } else {
        safe
    }
}

#[cfg(target_os = "macos")]
pub fn content_frame(content: &PlatformView, host: &PlatformView) -> Rect {
    if extends_into_bands(content) {
        view::bounds(host)
    } else {
        safe_area_rect(host)
    }
}
