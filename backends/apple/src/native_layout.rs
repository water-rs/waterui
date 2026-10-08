//! Native safe-area policy shared by roots and transparent containers.
//!
//! The safe area has two regions on each edge — the *container* region
//! (system bars, cutouts, the home indicator) and the *keyboard* region
//! (the software keyboard). A view is laid out clear of every region it
//! does not ignore; a fill — a view whose painted surface is a color, a
//! gradient or a material — extends past its frame into the bands it
//! touches; a scroll surface extends under the bands and insets its
//! content by them. `docs/layout-spec.md` §7.1 is the normative contract.
//!
//! `UIKit` reports the container region through `safeAreaInsets`. The
//! keyboard region is geometric: the window's `KeyboardRegion` object —
//! one per `UIWindow`, shared by every host — tracks the keyboard's
//! frame from `UIKit`'s keyboard notifications, and the inset a view
//! measures is the depth of that rect inside the view's own window
//! frame, on the edges the rect covers. A scroll surface owns the
//! safe-area contract for its whole subtree, so inside one the regions
//! read zero.
//!
//! Ignored regions accumulate down the view tree as a mask — bits 0–3 the
//! container `Edges` mask, bits 4–7 the keyboard `Edges` mask — that a
//! `cocoaUiIgnoredSafeAreaEdges` ignorer declares. An ignorer's declaration
//! applies on an edge only while its laid-out frame touches the boundary
//! the regions released above it leave — the §7.1 "touches" test — so an
//! ignorer nested inside a subtree that never reached the edge releases
//! nothing.

use cocoa_ui::objc2_foundation::NSObjectProtocol;
use cocoa_ui::{PlatformView, Rect, view};

#[cfg(target_os = "ios")]
use alloc::vec::Vec;
#[cfg(target_os = "ios")]
use cocoa_ui::Retained;
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
    if view::is_scroll_surface(view) {
        return true;
    }
    if view.respondsToSelector(objc2::sel!(cocoaUiManagesSafeArea)) {
        // SAFETY: cocoa-ui's host classes declare this selector as a boolean query.
        unsafe { objc2::msg_send![view, cocoaUiManagesSafeArea] }
    } else {
        false
    }
}

/// Whether `view` is a fill — it declares `cocoaUiIsFill` itself, or a
/// transparent wrapper forwards the declaration through its
/// `cocoaUiPrimaryContent` chain: `Color.opacity(..)` in a background
/// slot stays a fill without the wrapper copying the bit.
#[cfg(target_os = "ios")]
pub fn is_fill(view: &PlatformView) -> bool {
    if view.respondsToSelector(objc2::sel!(cocoaUiIsFill))
        // SAFETY: cocoa-ui's classes declare this selector as a boolean query.
        && unsafe { objc2::msg_send![view, cocoaUiIsFill] }
    {
        return true;
    }
    view::primary_content(view).is_some_and(|child| is_fill(&child))
}

/// Whether `view` is an ignore-safe-area wrapper — it declared through
/// `cocoaUiIgnoredSafeAreaEdges` with the mark bit set.
#[cfg(target_os = "ios")]
fn is_ignorer(view: &PlatformView) -> bool {
    declared_mask(view) & IGNORER_MARK != 0
}

/// The mask `view` itself declares through `cocoaUiIgnoredSafeAreaEdges`,
/// mark bit included.
#[cfg(target_os = "ios")]
fn declared_mask(view: &PlatformView) -> u16 {
    if view.respondsToSelector(objc2::sel!(cocoaUiIgnoredSafeAreaEdges)) {
        // SAFETY: cocoa-ui declares this selector as an unsigned-short
        // edge mask — bits 0–3 container, bits 4–7 keyboard, bit 8 the
        // ignorer mark.
        unsafe { objc2::msg_send![view, cocoaUiIgnoredSafeAreaEdges] }
    } else {
        0
    }
}

/// `view`'s bounds in its window's coordinate space.
#[cfg(target_os = "ios")]
fn window_frame(view: &PlatformView) -> CGRect {
    view.convertRect_toView(view.bounds(), None)
}

/// The keyboard's frame in the view's window coordinates — the frame the
/// window's `KeyboardRegion` holds (`keyboard::window_keyboard` reads it).
/// `CGRect::ZERO` only while the view is outside any window, where there
/// is no keyboard.
#[cfg(target_os = "ios")]
fn keyboard_rect(view: &PlatformView) -> CGRect {
    cocoa_ui::uikit::keyboard::window_keyboard(view).map_or(CGRect::ZERO, |(frame, _)| frame)
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

/// The chrome bands covering `view`'s subtree: per edge, the inner
/// position of the deepest band an ancestor reserves past the window's
/// container region — a tab or navigation bar's band, which `UIKit`
/// already reports as the deeper `safeAreaInsets` of the hosted content's
/// own view. On a covered edge nothing inside touches, releases or
/// extends (§7.1 "its content touches no edge where a bar sits").
#[cfg(target_os = "ios")]
#[derive(Clone, Copy, Default)]
struct Covered {
    /// The covered edges, in the `Edges::mask` bit order.
    mask: u16,
    /// The band's inner edge in window space, per covered edge.
    pos: [f64; 4],
}

/// `mask` with both region bits set on every covered edge — the release
/// veto a covered band applies to the whole subtree.
#[cfg(target_os = "ios")]
const fn covered_release_mask(mask: u16) -> u16 {
    mask * 0x11
}

/// The band `candidate` covers inside its subtree on each edge — a view
/// whose `safeAreaInsets` on an edge run deeper than the window's
/// container inset hosts a chrome band there (the tab bar's band inside
/// `UITabBarController`'s pane, the navigation bar's band inside a
/// pushed page); everything inside is covered by it.
#[cfg(target_os = "ios")]
fn cover_contribution(covered: &mut Covered, candidate: &PlatformView, context: &WindowContext) {
    let insets: EdgeInsets = candidate.safeAreaInsets().into();
    let frame = window_frame(candidate);
    for edge in 0..4u16 {
        let depth = depth_at(&insets, edge);
        if depth <= depth_at(&context.container, edge) + 0.5 {
            continue;
        }
        // The band's inner edge: the candidate's own safe boundary on
        // this edge, in window space — the deepest one wins.
        let pos = match edge {
            0 => frame.origin.y + insets.top,
            1 => frame.origin.x + insets.left,
            2 => frame.origin.y + frame.size.height - insets.bottom,
            _ => frame.origin.x + frame.size.width - insets.right,
        };
        let bit = 1 << edge;
        covered.pos[edge as usize] = if covered.mask & bit == 0 {
            pos
        } else {
            match edge {
                0 | 1 => covered.pos[edge as usize].max(pos),
                _ => covered.pos[edge as usize].min(pos),
            }
        };
        covered.mask |= bit;
    }
}

/// The covered bands `view` and its ancestors reserve past the window's
/// container region — inside a scroll surface there is no chrome reach.
#[cfg(target_os = "ios")]
fn covered_bands(view: &PlatformView, context: &WindowContext) -> Covered {
    let mut covered = Covered::default();
    if view::inside_scroll_surface(view) {
        return covered;
    }
    let mut current: Option<Retained<PlatformView>> = Some(Retained::from(view));
    while let Some(candidate) = current {
        cover_contribution(&mut covered, &candidate, context);
        current = view::superview(&candidate);
    }
    covered
}

/// What the boundary math needs from the window `view` lives in.
#[cfg(target_os = "ios")]
struct WindowContext {
    /// The window's own bounds.
    window: CGRect,
    /// The container region's depth on the window's edges.
    container: EdgeInsets,
    /// The keyboard region's depth on the window's edges.
    keyboard: EdgeInsets,
}

#[cfg(target_os = "ios")]
impl WindowContext {
    /// The two regions' depths at window level for `view` — no window means
    /// no regions: a detached view's own frame stands in for the window.
    fn of(view: &PlatformView) -> Self {
        let Some(window) = view::window(view) else {
            return Self {
                window: window_frame(view),
                container: EdgeInsets::ZERO,
                keyboard: EdgeInsets::ZERO,
            };
        };
        let window: &PlatformView = &window;
        Self {
            window: window.bounds(),
            // The container region is the window's own `safeAreaInsets` —
            // the bands system bars, cutouts and the home indicator
            // reserve; a root controller's `additionalSafeAreaInsets` are
            // the app's layout choice, not region geometry.
            container: EdgeInsets::from(window.safeAreaInsets()),
            // `keyboard_rect` resolves the window from the view it is
            // given: a `UIWindow` is inside no window, so the lookup must
            // run on `view` itself.
            keyboard: band_depths(window.bounds(), keyboard_rect(view)),
        }
    }

    /// The boundary position on `edge` — 0 top, 1 leading, 2 bottom,
    /// 3 trailing, the `Edges::mask` order — for a subtree that has
    /// released `mask`: the window's edge pushed in by the deepest region
    /// `mask` does not name, then clamped inside `covered`'s band on the
    /// edges a chrome ancestor covers.
    fn boundary(&self, mask: u16, edge: u16, covered: &Covered) -> f64 {
        let unreleased = |bit| {
            let c = if mask_marks(mask, 0, bit) {
                0.0
            } else {
                depth_at(&self.container, bit)
            };
            let k = if mask_marks(mask, 1, bit) {
                0.0
            } else {
                depth_at(&self.keyboard, bit)
            };
            c.max(k)
        };
        let mut position = match edge {
            0 => self.window.origin.y + unreleased(0),
            1 => self.window.origin.x + unreleased(1),
            2 => self.window.origin.y + self.window.size.height - unreleased(2),
            _ => self.window.origin.x + self.window.size.width - unreleased(3),
        };
        if covered.mask & (1 << edge) != 0 {
            position = match edge {
                0 | 1 => position.max(covered.pos[edge as usize]),
                _ => position.min(covered.pos[edge as usize]),
            };
        }
        position
    }

    /// The boundary rect for `mask` in `view`'s coordinate space: the
    /// window shrunk by each edge's unreleased depth and covered band,
    /// origin-shifted by `view`'s position in the window.
    fn boundary_rect(&self, view: &PlatformView, mask: u16, covered: &Covered) -> Rect {
        let frame = window_frame(view);
        Rect::new(
            self.boundary(mask, 1, covered) - frame.origin.x,
            self.boundary(mask, 0, covered) - frame.origin.y,
            self.boundary(mask, 3, covered) - self.boundary(mask, 1, covered),
            self.boundary(mask, 2, covered) - self.boundary(mask, 0, covered),
        )
    }

    /// The window's bounds in `view`'s coordinate space — the outermost
    /// extension target.
    fn window_rect(&self, view: &PlatformView) -> Rect {
        let frame = window_frame(view);
        Rect::new(
            -frame.origin.x,
            -frame.origin.y,
            self.window.size.width,
            self.window.size.height,
        )
    }
}

/// The `Edges::mask`-ordered depth at `bit` — 0 top, 1 leading, 2 bottom,
/// 3 trailing.
#[cfg(target_os = "ios")]
const fn depth_at(insets: &EdgeInsets, bit: u16) -> f64 {
    match bit {
        0 => insets.top,
        1 => insets.left,
        2 => insets.bottom,
        _ => insets.right,
    }
}

/// `Edges`-bit helpers on a region mask: whether region `region` (0 =
/// container, 1 = keyboard) is marked on the bit `bit` (the `Edges::mask`
/// order — top, leading, bottom, trailing).
#[cfg(target_os = "ios")]
const fn mask_marks(mask: u16, region: u16, bit: u16) -> bool {
    mask & (1 << (region * 4 + bit)) != 0
}

/// Whether `frame`'s `edge` reaches `boundary`, within `tolerance` — the
/// §7.1 "touches" test on one edge, run in window coordinates.
#[cfg(target_os = "ios")]
fn edge_reaches(frame: CGRect, boundary: f64, edge: u16, tolerance: f64) -> bool {
    match edge {
        0 => frame.origin.y <= boundary + tolerance,
        1 => frame.origin.x <= boundary + tolerance,
        2 => frame.origin.y + frame.size.height >= boundary - tolerance,
        _ => frame.origin.x + frame.size.width >= boundary - tolerance,
    }
}

/// The ignorer chain `view` hangs under: `(declared mask, window frame)`
/// pairs outermost first, stopping before a scroll surface — a surface
/// owns the safe-area contract for its subtree, so nothing inside it
/// accumulates.
#[cfg(target_os = "ios")]
fn ignorer_chain(view: &PlatformView) -> Vec<(u16, CGRect)> {
    let mut chain = Vec::new();
    let mut ancestor = view::superview(view);
    while let Some(current) = ancestor {
        if view::is_scroll_surface(&current) {
            break;
        }
        let mask = declared_mask(&current);
        if mask & IGNORER_MARK != 0 {
            chain.push((mask & !IGNORER_MARK, window_frame(&current)));
        }
        ancestor = view::superview(&current);
    }
    chain.reverse();
    chain
}

/// The released set of a `(mask, frame)` chain, folded outermost in: each
/// ignorer's declaration applies on the edges whose laid-out frame touches
/// the boundary the regions released above it leave, vetoed on the
/// subject's covered edges.
#[cfg(target_os = "ios")]
fn fold_released(
    chain: &[(u16, CGRect)],
    context: &WindowContext,
    covered: &Covered,
    tolerance: f64,
) -> u16 {
    let veto = covered_release_mask(covered.mask);
    let mut released = 0u16;
    for &(mask, frame) in chain {
        for edge in 0..4 {
            let bits = mask & (0x11 << edge) & !veto;
            if bits != 0
                && edge_reaches(
                    frame,
                    context.boundary(released, edge, covered),
                    edge,
                    tolerance,
                )
            {
                released |= bits;
            }
        }
    }
    released
}

/// Half a physical pixel on `view`'s display — the distance at which a
/// laid-out frame counts as ending on a boundary (§7.1 "touches").
fn touch_tolerance(view: &PlatformView) -> f64 {
    0.5 / view::backing_scale_factor(view)
}

pub fn manages_safe_area(view: &PlatformView) -> bool {
    if owns_safe_area(view) {
        return true;
    }
    view::primary_content(view).is_some_and(|child| manages_safe_area(&child))
}

/// The region context one host's layout pass works in: the window's
/// region depths, the chrome bands covering the host's subtree and the
/// ignorer chain the host's children fold over — computed once at the
/// top of the pass, then handed to every child the pass places instead
/// of walking ancestors per child.
#[cfg(target_os = "ios")]
pub struct LayoutContext<'a> {
    /// The host the pass belongs to — the children it places mount
    /// directly on it.
    host: &'a PlatformView,
    /// Whether the host sits inside a scroll surface — inside one,
    /// nothing touches an edge and every region reads zero.
    inside_scroll: bool,
    /// The window-level region depths.
    window: WindowContext,
    /// The chrome bands covering the host's subtree — the host's own
    /// bands included: a deeper `safeAreaInsets` on an edge is a band
    /// for everything placed inside.
    covered: Covered,
    /// The ignorer chain the host's children fold over: the host's
    /// ignorer ancestors plus the host's own declaration when it is one,
    /// outermost first, each with its laid-out window frame.
    chain: Vec<(u16, CGRect)>,
    /// The fold result for the host itself — what `context_mask` was.
    released: u16,
    /// The sub-pixel touch slack for the host's display.
    tolerance: f64,
}

#[cfg(target_os = "ios")]
impl<'a> LayoutContext<'a> {
    /// Computes the region context for `host`'s upcoming layout pass.
    pub fn of(host: &'a PlatformView) -> Self {
        let inside_scroll = view::inside_scroll_surface(host);
        let window = WindowContext::of(host);
        let covered = covered_bands(host, &window);
        let mut chain = ignorer_chain(host);
        let own = declared_mask(host);
        if own & IGNORER_MARK != 0 {
            chain.push((own & !IGNORER_MARK, window_frame(host)));
        }
        let tolerance = touch_tolerance(host);
        let released = fold_released(&chain, &window, &covered, tolerance);
        Self {
            host,
            inside_scroll,
            window,
            covered,
            chain,
            released,
            tolerance,
        }
    }

    /// The bands one `child` adds on top of the host's own — the
    /// subject-side half of the covered-edge veto.
    fn child_covered(&self, child: &PlatformView) -> Covered {
        let mut covered = self.covered;
        cover_contribution(&mut covered, child, &self.window);
        covered
    }

    /// The regions the ancestors plus `child`'s own declaration release
    /// — unconditional on `child`'s declaration because the extension
    /// itself decides whether the frame touched the boundary — vetoed
    /// on the covered edges either way.
    fn accumulated(child: &PlatformView, covered: &Covered, released: u16) -> u16 {
        (released | (declared_mask(child) & !IGNORER_MARK)) & !covered_release_mask(covered.mask)
    }

    /// The rect the host's subtree lays out inside: the host's bounds
    /// clipped to the boundary the regions it does not ignore leave —
    /// container, keyboard and covered chrome bands alike.
    #[must_use]
    pub fn safe_rect(&self) -> Rect {
        if self.inside_scroll {
            return view::bounds(self.host);
        }
        let rect = self
            .window
            .boundary_rect(self.host, self.released, &self.covered);
        let bounds = view::bounds(self.host);
        let x = bounds.origin.x.max(rect.origin.x);
        let y = bounds.origin.y.max(rect.origin.y);
        let width = (bounds.origin.x + bounds.size.width).min(rect.origin.x + rect.size.width) - x;
        let height =
            (bounds.origin.y + bounds.size.height).min(rect.origin.y + rect.size.height) - y;
        if width <= 0.0 || height <= 0.0 {
            Rect::new(bounds.origin.x, bounds.origin.y, 0.0, 0.0)
        } else {
            Rect::new(x, y, width, height)
        }
    }

    /// The frame `frame` extended by `child`'s extension rule: an
    /// ignorer reaches the deepest boundary its accumulated declaration
    /// releases; a scroll surface and a background-slot fill reach the
    /// window edge on the edges they touch; a safe-area manager reaches
    /// the host's bounds. On an edge no region releases, the target is
    /// the boundary itself, so an edge with no extension stays where it
    /// was laid out.
    #[must_use]
    pub fn extend_child(&self, child: &PlatformView, frame: Rect, fill_slot: bool) -> Rect {
        // Inside a scroll surface nothing touches an edge — the surface
        // owns its subtree's safe-area contract.
        if self.inside_scroll {
            return frame;
        }
        let covered = self.child_covered(child);
        let released = fold_released(&self.chain, &self.window, &covered, self.tolerance);
        let within = self.window.boundary_rect(self.host, released, &covered);
        if is_ignorer(child) {
            let target = self.window.boundary_rect(
                self.host,
                Self::accumulated(child, &covered, released),
                &covered,
            );
            frame.extended_through(within, target, self.tolerance)
        } else if view::is_scroll_surface(child) || (fill_slot && is_fill(child)) {
            frame.extended_through(within, self.window.window_rect(self.host), self.tolerance)
        } else if manages_safe_area(child) {
            frame.extended_through(within, view::bounds(self.host), self.tolerance)
        } else {
            frame
        }
    }

    /// The frame the host hands its single content view: the bounds when
    /// the content manages its own safe area — an ignorer's bounds still
    /// released only as far as its accumulated declaration reaches — the
    /// safe-area rect otherwise. Inside a scroll surface the surface owns
    /// the subtree's safe-area contract, so the content frame is the
    /// plain `safe_rect` — the bounds — with no region release.
    #[must_use]
    pub fn content_frame(&self, content: &PlatformView) -> Rect {
        if self.inside_scroll {
            return self.safe_rect();
        }
        if is_ignorer(content) {
            let covered = self.child_covered(content);
            let released = fold_released(&self.chain, &self.window, &covered, self.tolerance);
            self.safe_rect().extended_through(
                self.window.boundary_rect(self.host, released, &covered),
                self.window.boundary_rect(
                    self.host,
                    Self::accumulated(content, &covered, released),
                    &covered,
                ),
                self.tolerance,
            )
        } else if manages_safe_area(content) {
            view::bounds(self.host)
        } else {
            self.safe_rect()
        }
    }
}

/// The region context one host's layout pass works in. `AppKit` keeps
/// the container region only, so the context carries the host alone.
#[cfg(target_os = "macos")]
pub struct LayoutContext<'a> {
    host: &'a PlatformView,
}

#[cfg(target_os = "macos")]
impl<'a> LayoutContext<'a> {
    /// Computes the region context for `host`'s upcoming layout pass.
    pub const fn of(host: &'a PlatformView) -> Self {
        Self { host }
    }

    /// The rect the host's subtree lays out inside — `AppKit`'s
    /// `safeAreaRect`.
    #[must_use]
    pub fn safe_rect(&self) -> Rect {
        self.host.safeAreaRect().into()
    }

    /// `AppKit` keeps the container region only: a safe-area manager
    /// extends to the host's bounds on the edges it touches.
    #[must_use]
    pub fn extend_child(&self, child: &PlatformView, frame: Rect, _fill_slot: bool) -> Rect {
        if manages_safe_area(child) {
            frame.extended_through(
                self.safe_rect(),
                view::bounds(self.host),
                touch_tolerance(self.host),
            )
        } else {
            frame
        }
    }

    /// The frame the host hands its single content view: the bounds when
    /// the content manages its own safe area, the safe-area rect
    /// otherwise.
    #[must_use]
    pub fn content_frame(&self, content: &PlatformView) -> Rect {
        if manages_safe_area(content) {
            view::bounds(self.host)
        } else {
            self.safe_rect()
        }
    }
}
