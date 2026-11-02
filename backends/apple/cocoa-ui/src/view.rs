//! The platform's base view and the free functions on it.
//!
//! `NSView` and `UIView` are never compiled together, so the base is an
//! alias, not a trait: code written against [`PlatformView`] has no `#[cfg]`.
//!
//! # Safety
//!
//! The `unsafe` here calls `AppKit`/`UIKit`'s own hierarchy and frame
//! accessors on views the caller guarantees are alive; all are ordinary
//! main-thread calls.

use objc2::rc::Retained;
use objc2_foundation::NSObjectProtocol;

use crate::PlatformView;
use crate::geometry::{Point, Rect};

/// The view's immediate subviews, in back-to-front order.
#[must_use]
pub fn subviews(view: &PlatformView) -> Vec<Retained<PlatformView>> {
    view.subviews().to_vec()
}

/// The window `view` is attached to, if any.
#[must_use]
#[cfg(target_os = "macos")]
pub fn window(view: &PlatformView) -> Option<Retained<objc2_app_kit::NSWindow>> {
    view.window()
}

/// The window `view` is attached to, if any.
#[must_use]
#[cfg(target_os = "ios")]
pub fn window(view: &PlatformView) -> Option<Retained<objc2_ui_kit::UIWindow>> {
    view.window()
}

/// Marks `view` with a stable accessibility identifier, the string a test or
/// an internal contract can find the view by. Pass `None` to clear it.
pub fn set_accessibility_identifier(view: &PlatformView, identifier: Option<&str>) {
    let identifier = identifier.map(objc2_foundation::NSString::from_str);
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSAccessibility;
        view.setAccessibilityIdentifier(identifier.as_deref());
    }
    #[cfg(target_os = "ios")]
    {
        use objc2_ui_kit::UIAccessibilityIdentification;
        view.setAccessibilityIdentifier(identifier.as_deref());
    }
}

/// The accessibility identifier set on `view`, if any.
#[must_use]
pub fn accessibility_identifier(view: &PlatformView) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSAccessibility;
        view.accessibilityIdentifier().map(|s| s.to_string())
    }
    #[cfg(target_os = "ios")]
    {
        use objc2_ui_kit::UIAccessibilityIdentification;
        view.accessibilityIdentifier()
            .map(|identifier| identifier.to_string())
    }
}

/// The tag (`UIView.tag`) used to marker-test a view without retaining a
/// subclass reference. `NSView` has no tag of its own; identity checks there
/// use the accessibility identifier.
#[must_use]
#[cfg(target_os = "ios")]
pub fn tag(view: &PlatformView) -> isize {
    view.tag()
}

/// Sets `view`'s tag; see [`tag`].
#[cfg(target_os = "ios")]
pub fn set_tag(view: &PlatformView, tag: isize) {
    view.setTag(tag);
}

/// The view's Objective-C class name.
#[must_use]
pub fn class_name(view: &PlatformView) -> &'static str {
    view.class().name().to_str().unwrap_or_default()
}

/// Converts `point` from `view`'s coordinate space into `to`'s.
#[must_use]
pub fn convert_point(view: &PlatformView, point: Point, to: &PlatformView) -> Point {
    view.convertPoint_toView(point.into(), Some(to)).into()
}

/// A +1 reference to `view` as its base class: any `NSView`/`UIView`
/// subclass, including the kit's own classes.
///
/// The caller keeps its own object alive; this retains the base view.
///
/// # Panics
///
/// `view` retains to a non-null object; unreachable for a live view.
#[must_use]
pub fn retain_base<V: AsRef<PlatformView> + ?Sized>(view: &V) -> Retained<PlatformView> {
    // SAFETY: `view.as_ref()` is a live `PlatformView` for the call; the
    // returned `Retained` owns the new +1.
    unsafe { Retained::retain(std::ptr::from_ref(view.as_ref()).cast_mut()) }
        .expect("a live view cannot retain to null")
}

/// The frame in the superview's coordinate space.
#[must_use]
pub fn frame(view: &PlatformView) -> Rect {
    view.frame().into()
}

/// The alignment rectangle `view`'s frame maps to.
///
/// This is the edge Auto Layout pins when it solves the view's anchors. A
/// control's frame can overshoot it: a `UISwitch` frame runs 2 pt past its
/// pinned trailing edge.
#[must_use]
pub fn alignment_frame(view: &PlatformView) -> Rect {
    #[cfg(target_os = "ios")]
    {
        view.alignmentRectForFrame(view.frame()).into()
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: `view` is a live `NSView`; `alignmentRectForFrame:` is a
        // documented `NSView` method the `objc2-app-kit` bindings do not
        // generate.
        let rect: objc2_core_foundation::CGRect =
            unsafe { objc2::msg_send![view, alignmentRectForFrame: view.frame()] };
        rect.into()
    }
}

/// The bounds in the view's own coordinate space.
#[must_use]
pub fn bounds(view: &PlatformView) -> Rect {
    view.bounds().into()
}

/// Whether `view`'s y axis runs top-down — `NSView.isFlipped` on `AppKit`.
#[cfg(target_os = "macos")]
#[must_use]
pub fn is_flipped(view: &PlatformView) -> bool {
    view.isFlipped()
}

/// Whether `view`'s y axis runs top-down — `UIKit`'s coordinate space is
/// natively top-left, so `true`.
#[cfg(target_os = "ios")]
#[must_use]
pub const fn is_flipped(_view: &PlatformView) -> bool {
    true
}

/// Moves and resizes `view` in its superview's coordinate space.
pub fn set_frame(view: &PlatformView, frame: Rect) {
    view.setFrame(frame.into());
}

/// Sets `view`'s bounds — its frame in its own coordinate space.
///
/// Writing bounds instead of frame keeps the center fixed, which a
/// transform pivot depends on.
pub fn set_bounds(view: &PlatformView, bounds: Rect) {
    view.setBounds(bounds.into());
}

/// Moves `view` so its bounds' midpoint lands on `center` — a `UIView`
/// property; `NSView` positions through its frame.
#[cfg(target_os = "ios")]
pub fn set_center(view: &PlatformView, center: Point) {
    view.setCenter(center.into());
}

/// Writes `view`'s 2D transform — `UIView.transform`. On `AppKit` the layer
/// owns the transform instead; see [`crate::core_animation`].
#[cfg(target_os = "ios")]
pub fn set_transform(view: &PlatformView, transform: objc2_core_foundation::CGAffineTransform) {
    view.setTransform(transform);
}

/// Tells the nearest ancestor that captures rendered content — one
/// answering `renderedContentDidInvalidate` — that `view`'s appearance
/// changed, walking the superview chain until one answers.
///
/// A wrapper that changes how its content looks without a relayout (a
/// transform, an alpha, a decoration) calls this so a cached capture — a
/// filter or snapshot — re-renders.
pub fn invalidate_captured_rendering(view: &PlatformView) {
    let selector = objc2::sel!(renderedContentDidInvalidate);
    let mut ancestor = superview(view);
    while let Some(current) = ancestor {
        if current.respondsToSelector(selector) {
            // SAFETY: `respondsToSelector` proved the method exists; it
            // takes no arguments and returns void by the protocol's
            // contract.
            unsafe {
                let _: () = objc2::msg_send![&*current, renderedContentDidInvalidate];
            }
            return;
        }
        ancestor = superview(&current);
    }
}

/// Adds `child` above `parent`'s existing subviews.
pub fn add_subview(parent: &PlatformView, child: &PlatformView) {
    parent.addSubview(child);
}

/// Adds `child` below `parent`'s existing subviews.
pub fn add_subview_at_bottom(parent: &PlatformView, child: &PlatformView) {
    #[cfg(target_os = "macos")]
    parent.addSubview_positioned_relativeTo(
        child,
        objc2_app_kit::NSWindowOrderingMode::Below,
        None,
    );
    #[cfg(target_os = "ios")]
    parent.insertSubview_atIndex(child, 0);
}

/// Detaches `view` from its superview; does nothing when it has none.
pub fn remove_from_superview(view: &PlatformView) {
    view.removeFromSuperview();
}

/// True when a strict ancestor of `view` is a retired `HostView`.
/// Walks `superview` to the root, so the cost is O(depth).
#[must_use]
pub fn has_retired_ancestor(view: &PlatformView) -> bool {
    #[cfg(target_os = "macos")]
    use crate::appkit::HostView;
    #[cfg(target_os = "ios")]
    use crate::uikit::HostView;

    let mut ancestor = superview(view);
    while let Some(current) = ancestor {
        if current
            .downcast_ref::<HostView>()
            .is_some_and(HostView::is_retired)
        {
            return true;
        }
        ancestor = superview(&current);
    }
    false
}

/// Shows or hides `view` without detaching it.
pub fn set_hidden(view: &PlatformView, hidden: bool) {
    view.setHidden(hidden);
}

/// Whether `view` is inside a window's hierarchy right now.
#[must_use]
pub fn has_window(view: &PlatformView) -> bool {
    view.window().is_some()
}

/// `view`'s bounds in its window's coordinate space.
///
/// On macOS the result keeps `AppKit`'s bottom-left origin; a top-left
/// consumer mirrors it against the content area's height.
#[must_use]
pub fn bounds_in_window(view: &PlatformView) -> Rect {
    view.convertRect_toView(view.bounds(), None).into()
}

/// Lets `view` resize with its superview on both axes.
///
/// The autoresizing mask `UIViewAutoresizing.flexibleWidth |
/// .flexibleHeight`, the same flag pair `NSView` spells `ViewWidthSizable |
/// ViewHeightSizable`.
pub fn set_autoresizing_flexible_size(view: &PlatformView) {
    #[cfg(target_os = "macos")]
    view.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    #[cfg(target_os = "ios")]
    view.setAutoresizingMask(
        objc2_ui_kit::UIViewAutoresizing::FlexibleWidth
            | objc2_ui_kit::UIViewAutoresizing::FlexibleHeight,
    );
}

/// Whether `view` posts `NSViewFrameDidChangeNotification` on every frame
/// change — off by default, so a caller watching the notification opts in.
#[cfg(target_os = "macos")]
pub fn set_posts_frame_changed(view: &PlatformView, enabled: bool) {
    view.setPostsFrameChangedNotifications(enabled);
}

/// Tells `view` and every ancestor that its size may have changed.
///
/// Invalidates intrinsic content size and marks each for layout. Call from a
/// watcher after any imperative change that can alter what the view measures
/// to — a text change, a font change, a swapped child.
pub fn invalidate_layout(view: &PlatformView) {
    view.invalidateIntrinsicContentSize();
    set_needs_layout(view);
    let mut parent = superview(view);
    while let Some(current) = parent {
        current.invalidateIntrinsicContentSize();
        set_needs_layout(&current);
        parent = superview(&current);
    }
}

#[cfg(target_os = "macos")]
fn set_needs_layout(view: &PlatformView) {
    view.setNeedsLayout(true);
}

#[cfg(target_os = "ios")]
fn set_needs_layout(view: &PlatformView) {
    view.setNeedsLayout();
}

/// The view's superview, if attached.
#[must_use]
#[cfg(target_os = "macos")]
pub fn superview(view: &PlatformView) -> Option<Retained<PlatformView>> {
    // SAFETY: superview walking is a main-thread read of the view hierarchy.
    unsafe { view.superview() }
}

/// The view's superview, if attached.
#[must_use]
#[cfg(target_os = "ios")]
pub fn superview(view: &PlatformView) -> Option<Retained<PlatformView>> {
    view.superview()
}

/// The view's backing layer — `UIView` always has one; an `NSView` gains one
/// the first time `wantsLayer` is set, which this does when nil.
#[must_use]
pub fn layer(view: &PlatformView) -> Option<Retained<objc2_quartz_core::CALayer>> {
    #[cfg(target_os = "macos")]
    {
        view.setWantsLayer(true);
        view.layer()
    }
    #[cfg(target_os = "ios")]
    {
        Some(view.layer())
    }
}

/// Whether `view` is hidden.
#[must_use]
pub fn is_hidden(view: &PlatformView) -> bool {
    view.isHidden()
}

/// Whether `view` or any ancestor is hidden — `isHiddenOrHasHiddenAncestor`.
#[must_use]
pub fn is_hidden_in_hierarchy(view: &PlatformView) -> bool {
    #[cfg(target_os = "macos")]
    {
        view.isHiddenOrHasHiddenAncestor()
    }
    #[cfg(target_os = "ios")]
    {
        let mut current: Option<Retained<PlatformView>> = Some(Retained::from(view));
        while let Some(candidate) = current {
            if candidate.isHidden() {
                return true;
            }
            current = candidate.superview();
        }
        false
    }
}

/// `hasVisibleAncestry` — this view and every ancestor visible: neither
/// hidden nor fully transparent.
///
/// A hidden or alpha-0 view ancestor parks the surface. On `UIKit` the
/// terminal ancestor is the `UIWindow` itself (a `UIView`): a *hidden*
/// window still rejects, but its alpha is outside content visibility —
/// an ordered, unrevealed window holds `alpha = 0` until first paint and
/// must not reject the way a transparent view ancestor does. `AppKit`
/// windows sit outside the `NSView` chain, so behavior is unchanged.
#[must_use]
pub fn has_visible_ancestry(view: &PlatformView) -> bool {
    let mut node = Some(retain_base(view));
    while let Some(current) = node {
        if is_hidden(&current) || (!is_window_view(&current) && alpha(&current) <= 0.0) {
            return false;
        }
        node = superview(&current);
    }
    true
}

/// Whether `view` is the window itself. Only `UIKit` keeps the window in
/// the view-ancestor chain (`UIWindow` is a `UIView`); `AppKit` windows
/// never appear there.
#[cfg(target_os = "ios")]
fn is_window_view(view: &PlatformView) -> bool {
    use objc2::ClassType;
    view.isKindOfClass(objc2_ui_kit::UIWindow::class())
}
#[cfg(not(target_os = "ios"))]
const fn is_window_view(_view: &PlatformView) -> bool {
    false
}

/// `view`'s opacity (`alphaValue`/`alpha`).
#[must_use]
pub fn alpha(view: &PlatformView) -> f64 {
    #[cfg(target_os = "macos")]
    {
        view.alphaValue()
    }
    #[cfg(target_os = "ios")]
    {
        view.alpha()
    }
}

/// Converts `rect` from `view`'s coordinate space into `to`'s (`None` means
/// the window's base space).
#[must_use]
pub fn convert_rect(view: &PlatformView, rect: Rect, to: Option<&PlatformView>) -> Rect {
    view.convertRect_toView(rect.into(), to).into()
}

/// The size `view` prefers when unconstrained (`fittingSize` on `AppKit`,
/// `sizeThatFits` of an unbounded proposal is meaningless on `UIKit` where
/// callers pass the proposal through).
#[must_use]
#[cfg(target_os = "macos")]
pub fn fitting_size(view: &PlatformView) -> crate::geometry::Size {
    view.fittingSize().into()
}

/// The size `view` compresses to under Auto Layout — `UIKit`'s analogue
/// of `AppKit`'s `fittingSize`.
#[must_use]
#[cfg(target_os = "ios")]
pub fn fitting_size(view: &PlatformView) -> crate::geometry::Size {
    view.systemLayoutSizeFittingSize(objc2_core_foundation::CGSize::new(0.0, 0.0))
        .into()
}

/// The size `view` prefers for `proposed` points.
#[must_use]
#[cfg(target_os = "ios")]
pub fn size_that_fits(
    view: &PlatformView,
    proposed: crate::geometry::Size,
) -> crate::geometry::Size {
    view.sizeThatFits(proposed.into()).into()
}

/// Marks `view` and its subtree as needing layout, then flushes it.
#[cfg(target_os = "ios")]
pub fn layout_immediately(view: &PlatformView) {
    view.setNeedsLayout();
    view.layoutIfNeeded();
}

/// Marks `view` and its subtree as needing display, then draws it.
///
/// `UIView` has no `displayIfNeeded` — that method is `CALayer`'s, so the
/// display pass is flushed on the layer that backs the view.
#[cfg(target_os = "ios")]
pub fn display_immediately(view: &PlatformView) {
    view.setNeedsDisplay();
    view.layer().displayIfNeeded();
}

/// Performs any pending layout on `view`'s layer tree.
#[cfg(target_os = "ios")]
pub fn layout_layer_immediately(view: &PlatformView) {
    view.layer().layoutIfNeeded();
}

/// Performs any pending layout on `view`'s tree.
#[cfg(target_os = "macos")]
pub fn layout_immediately(view: &PlatformView) {
    view.setNeedsLayout(true);
    view.layoutSubtreeIfNeeded();
}

/// Performs any pending layout on `view`'s layer tree, after ensuring the
/// view is layer-backed.
#[cfg(target_os = "macos")]
pub fn layout_layer_immediately(view: &PlatformView) {
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.layoutIfNeeded();
    }
}

/// Performs any pending display on `view`'s tree, after ensuring the view is
/// layer-backed.
#[cfg(target_os = "macos")]
pub fn display_immediately(view: &PlatformView) {
    view.setWantsLayer(true);
    view.setNeedsDisplay(true);
    view.displayIfNeeded();
}

/// Ensures `view` and every descendant is layer-backed.
#[cfg(target_os = "macos")]
pub fn ensure_layer_backed(view: &PlatformView) {
    view.setWantsLayer(true);
    for subview in &view.subviews() {
        ensure_layer_backed(&subview);
    }
}

/// Prepares `view` for an off-screen layer capture: everything
/// layer-backed, laid out, and displayed.
#[cfg(target_os = "macos")]
pub fn prepare_for_capture(view: &PlatformView) {
    ensure_layer_backed(view);
    layout_immediately(view);
    layout_layer_immediately(view);
    display_immediately(view);
}

/// Prepares `view` for an off-screen layer capture: pending layout and
/// display applied.
#[cfg(target_os = "ios")]
pub fn prepare_for_capture(view: &PlatformView) {
    layout_immediately(view);
    layout_layer_immediately(view);
    display_immediately(view);
}

/// The scroll view `view` sits inside, when any — `AppKit`'s
/// `enclosingScrollView`.
#[must_use]
#[cfg(target_os = "macos")]
pub fn enclosing_scroll_view(view: &PlatformView) -> Option<Retained<objc2_app_kit::NSScrollView>> {
    view.enclosingScrollView()
}

/// The image representation `view` prepares for `cache_display_in`.
#[must_use]
#[cfg(target_os = "macos")]
pub fn bitmap_rep_for_caching_display(
    view: &PlatformView,
) -> Option<Retained<objc2_app_kit::NSBitmapImageRep>> {
    view.bitmapImageRepForCachingDisplayInRect(view.bounds())
}

/// Draws `view`'s tree into its cached bitmap rep.
#[cfg(target_os = "macos")]
pub fn cache_display(view: &PlatformView, rep: &objc2_app_kit::NSBitmapImageRep) {
    view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), rep);
}

/// The display-scale factor `view`'s window renders at, `1.0` detached.
#[must_use]
pub fn backing_scale_factor(view: &PlatformView) -> f64 {
    crate::view::window(view).map_or(1.0, |window| {
        #[cfg(target_os = "macos")]
        {
            window.backingScaleFactor()
        }
        #[cfg(target_os = "ios")]
        {
            window.screen().scale()
        }
    })
}

/// What pixel alignment discarded from a view's layout origin: the layout
/// origin minus the written origin, in points, each component within half
/// a device pixel.
///
/// A view's children are laid out against its layout origin, but they
/// land in the coordinate space of its written, aligned origin. Aligning a
/// child under its parent's offset puts the child where the parent's
/// layout placed it, so each view's written edges are its absolute layout
/// edges rounded once — the discarded fraction never accumulates down the
/// tree. A view the platform places, such as a window's content root,
/// starts at [`SubpixelOffset::ZERO`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SubpixelOffset {
    x: f64,
    y: f64,
}

impl SubpixelOffset {
    /// No discarded fraction: a view written exactly at its layout origin.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
}

/// A frame aligned to the device-pixel grid and the offset its alignment
/// left, which the view's own children align under.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelAlignedFrame {
    /// The frame to write, in the superview's coordinate space.
    pub frame: Rect,
    /// What the alignment discarded from the layout origin.
    pub offset: SubpixelOffset,
}

/// `frame` — a rect in the layout space of `view`'s superview, whose own
/// alignment discarded `parent` — aligned to the device-pixel grid of
/// `view`'s window ([`backing_scale_factor`], `1.0` detached).
///
/// The superview's written origin sits on the grid — this alignment or the
/// platform placed it there — so `frame` lies at `frame + parent` in the
/// superview's coordinate space, and rounding there rounds the absolute
/// layout edges. Each min and max edge rounds to the nearest device pixel
/// on its own — never origin and size separately — which keeps adjacent
/// siblings flush. A tie rounds toward the larger coordinate: rightward in
/// x and, in the top-down space every placed frame lives in, toward the
/// screen bottom, as `SwiftUI` rounds.
///
/// The written size never falls short of the layout size: when the
/// rounded edges span fewer whole pixels than the size covers, the max
/// edge moves out to cover it. Independent rounding alone can lose a
/// pixel — a size on the grid whose f32 value sits just below it, framed
/// at a half-pixel tie, rounds its min edge up and its max edge down — and
/// a leaf framed smaller than it measured, such as text, wraps or clips.
/// `AppKit`'s
/// `backingAlignedRect` is not used: it rounds in bottom-up backing space,
/// so its ties go toward the screen top, and it rounds each frame in its
/// superview's already-aligned space.
#[must_use]
pub fn pixel_aligned_frame(
    view: &PlatformView,
    frame: Rect,
    parent: SubpixelOffset,
) -> PixelAlignedFrame {
    align_to_pixel_grid(frame, parent, backing_scale_factor(view))
}

/// Writes [`pixel_aligned_frame`] as `view`'s frame, for a placement that
/// delivers nothing to the view and so discards the offset the alignment
/// left.
pub fn set_frame_pixel_aligned(view: &PlatformView, frame: Rect, parent: SubpixelOffset) {
    set_frame(view, pixel_aligned_frame(view, frame, parent).frame);
}

/// [`pixel_aligned_frame`]'s arithmetic, at device scale `scale`.
fn align_to_pixel_grid(frame: Rect, parent: SubpixelOffset, scale: f64) -> PixelAlignedFrame {
    let min_x = frame.origin.x + parent.x;
    let min_y = frame.origin.y + parent.y;
    let (pixel_min_x, pixel_max_x) = align_edges(min_x, frame.size.width, scale);
    let (pixel_min_y, pixel_max_y) = align_edges(min_y, frame.size.height, scale);
    let aligned_min_x = pixel_min_x / scale;
    let aligned_min_y = pixel_min_y / scale;
    PixelAlignedFrame {
        frame: Rect::new(
            aligned_min_x,
            aligned_min_y,
            (pixel_max_x - pixel_min_x) / scale,
            (pixel_max_y - pixel_min_y) / scale,
        ),
        offset: SubpixelOffset {
            x: min_x - aligned_min_x,
            y: min_y - aligned_min_y,
        },
    }
}

/// The min and max edges, in whole device pixels, of the extent `length`
/// points long starting at `min`: each edge rounded to the nearest pixel,
/// a tie toward the larger value, and the max edge moved out when the two
/// span fewer pixels than [`covering_pixels`] of `length`.
fn align_edges(min: f64, length: f64, scale: f64) -> (f64, f64) {
    let pixel_min = nearest_pixel(min * scale);
    let pixel_max =
        nearest_pixel((min + length) * scale).max(pixel_min + covering_pixels(length * scale));
    (pixel_min, pixel_max)
}

/// `pixels` rounded to the nearest whole pixel, a tie toward the larger
/// value.
fn nearest_pixel(pixels: f64) -> f64 {
    (pixels + 0.5).floor()
}

/// The fewest whole pixels that cover an extent `pixels` long.
///
/// Layout sizes are f32, so a size on the pixel grid — a text leaf measures
/// whole pixels — can arrive a few f32 ulps past it; within f32 precision
/// it covers that whole number, not one more.
fn covering_pixels(pixels: f64) -> f64 {
    pixels.mul_add(-f64::from(f32::EPSILON), pixels).ceil()
}

/// Removes `view` and everything inside it from the accessibility tree.
#[cfg(target_os = "macos")]
pub fn hide_from_accessibility(view: &PlatformView) {
    use objc2_app_kit::NSAccessibility;
    use objc2_foundation::NSArray;
    view.setAccessibilityElement(false);
    // SAFETY: installing an empty children array on a live view is the
    // documented way to strip its accessibility subtree.
    unsafe { view.setAccessibilityChildren(Some(&NSArray::new())) };
}

/// Removes `view` and everything inside it from the accessibility tree.
#[cfg(target_os = "ios")]
pub fn hide_from_accessibility(view: &PlatformView) {
    use objc2_ui_kit::NSObjectUIAccessibility;
    view.setAccessibilityElementsHidden(true, objc2::MainThreadMarker::from(view));
}

/// Whether the view lays out right-to-left for its current content.
#[must_use]
#[cfg(target_os = "macos")]
pub fn is_right_to_left(view: &PlatformView) -> bool {
    use objc2_app_kit::NSUserInterfaceLayoutDirection;
    view.userInterfaceLayoutDirection() == NSUserInterfaceLayoutDirection::RightToLeft
}

/// Whether the view lays out right-to-left for its current content and
/// trait environment.
#[must_use]
#[cfg(target_os = "ios")]
pub fn is_right_to_left(view: &PlatformView) -> bool {
    use objc2_ui_kit::UIUserInterfaceLayoutDirection;
    PlatformView::userInterfaceLayoutDirectionForSemanticContentAttribute(
        view.semanticContentAttribute(),
        objc2::MainThreadMarker::from(view),
    ) == UIUserInterfaceLayoutDirection::RightToLeft
}

/// Fades `view` — and everything it contains — toward transparent, `1.0`
/// being fully opaque.
pub fn set_alpha(view: &PlatformView, alpha: f64) {
    #[cfg(target_os = "macos")]
    view.setAlphaValue(alpha);
    #[cfg(target_os = "ios")]
    view.setAlpha(alpha);
}

/// Declares `view` an accessibility element and gives it `label` (and, on
/// `AppKit`, a tooltip of the same text). Pass text already stripped of
/// bidirectional controls.
pub fn set_accessibility_label(view: &PlatformView, label: &str) {
    let label = objc2_foundation::NSString::from_str(label);
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSAccessibility;
        view.setAccessibilityElement(true);
        view.setAccessibilityLabel(Some(&label));
        view.setToolTip(Some(&label));
    }
    #[cfg(target_os = "ios")]
    {
        use objc2_ui_kit::NSObjectUIAccessibility;
        let mtm = objc2::MainThreadMarker::from(view);
        view.setIsAccessibilityElement(true, mtm);
        view.setAccessibilityLabel(Some(&label), mtm);
    }
}

/// Whether `view` clips its subviews to its bounds — `UIView`'s
/// `clipsToBounds`, a `UIKit`-only property.
#[cfg(target_os = "ios")]
pub fn set_clips_to_bounds(view: &PlatformView, clips: bool) {
    view.setClipsToBounds(clips);
}

/// The primary content `view` exposes through `cocoaUiPrimaryContent`.
///
/// The child a kit host view surfaces for chrome like list cells and scroll
/// surfaces; `None` when `view` does not answer the selector.
#[must_use]
pub fn primary_content(view: &PlatformView) -> Option<Retained<PlatformView>> {
    if view.respondsToSelector(objc2::sel!(cocoaUiPrimaryContent)) {
        // SAFETY: every kit class implementing `cocoaUiPrimaryContent`
        // declares it `-> Option<Retained<PlatformView>>`.
        unsafe { objc2::msg_send![view, cocoaUiPrimaryContent] }
    } else {
        None
    }
}

/// Whether `view` declares itself a kit scroll surface through
/// `cocoaUiIsScrollSurface`.
///
/// The `ScrollView` and `TableView` classes answer `true`; a foreign
/// `UIScrollView` such as `UITextView` does not answer it, so a view
/// nested inside one keeps its own region contract.
#[cfg(target_os = "ios")]
#[must_use]
pub fn is_scroll_surface(view: &PlatformView) -> bool {
    view.respondsToSelector(objc2::sel!(cocoaUiIsScrollSurface))
        // SAFETY: kit classes declaring `cocoaUiIsScrollSurface` declare
        // it `-> bool`.
        && unsafe { objc2::msg_send![view, cocoaUiIsScrollSurface] }
}

/// Whether a kit scroll surface sits above `view`.
///
/// This is the ancestor walk the keyboard-region marking refuses to
/// descend: a scroll surface's subtree sees no keyboard region — the
/// surface already moved its content clear — so nothing under it is a
/// region reader, and a surface nested inside another neither insets
/// nor scrolls for the keyboard itself (the field clears once, through
/// the innermost surface that sees the band). A foreign `UIScrollView`
/// — a `UITextView` — does not answer `cocoaUiIsScrollSurface` and does
/// not count.
#[cfg(target_os = "ios")]
#[must_use]
pub fn inside_scroll_surface(view: &PlatformView) -> bool {
    let mut ancestor = superview(view);
    while let Some(current) = ancestor {
        if is_scroll_surface(&current) {
            return true;
        }
        ancestor = superview(&current);
    }
    false
}

/// The scroll-surface candidates `view` declares through
/// `cocoaUiScrollSurfaceCandidates`.
///
/// The children a kit host view reports for the scroll-surface search, in
/// stacking order; empty when `view` does not answer the selector.
#[must_use]
pub fn scroll_surface_candidates(view: &PlatformView) -> Vec<Retained<PlatformView>> {
    if view.respondsToSelector(objc2::sel!(cocoaUiScrollSurfaceCandidates)) {
        // SAFETY: every kit class implementing
        // `cocoaUiScrollSurfaceCandidates` declares it
        // `-> Retained<NSArray<PlatformView>>`.
        let candidates: Retained<objc2_foundation::NSArray<PlatformView>> =
            unsafe { objc2::msg_send![view, cocoaUiScrollSurfaceCandidates] };
        candidates.to_vec()
    } else {
        Vec::new()
    }
}

/// Whether `view` sizes itself by its frame rather than by Auto Layout
/// constraints — `true` for views a layout container positions manually.
pub fn set_translates_autoresizing(view: &PlatformView, enabled: bool) {
    view.setTranslatesAutoresizingMaskIntoConstraints(enabled);
}

/// Whether `view` receives touch events itself. `false` on a view laid over
/// an interactive control lets touches fall through to the control.
#[cfg(target_os = "ios")]
pub fn set_user_interaction_enabled(view: &PlatformView, enabled: bool) {
    view.setUserInteractionEnabled(enabled);
}

/// The color `view` draws behind its content — `UIView`'s `backgroundColor`;
/// `None` restores transparent.
#[cfg(target_os = "ios")]
pub fn set_background_color(view: &PlatformView, color: Option<&objc2_ui_kit::UIColor>) {
    view.setBackgroundColor(color);
}

/// The color `view` and the controls inside it tint with — `UIView`'s
/// `tintColor`, inherited down the view tree until a subview overrides it.
#[cfg(target_os = "ios")]
pub fn set_tint_color(view: &PlatformView, color: Option<&objc2_ui_kit::UIColor>) {
    // SAFETY: see the module safety note. `tintColor` takes nil.
    unsafe { view.setTintColor(color) };
}

/// Whether `view`'s layout margins get their own inset from the safe area.
///
/// `UIView`'s `insetsLayoutMarginsFromSafeArea`; a wrapper that manages the
/// safe area itself turns it off so the platform does not double-inset.
#[cfg(target_os = "ios")]
pub fn set_insets_layout_margins_from_safe_area(view: &PlatformView, insets: bool) {
    view.setInsetsLayoutMarginsFromSafeArea(insets);
}

/// Makes `parent`'s subviews exactly `ordered`, in that z-order.
///
/// Reuses the subview instances already attached. A hierarchy already in
/// that order is left untouched, so a caller may reconcile on every layout
/// pass.
pub fn reconcile_subviews(parent: &PlatformView, ordered: &[Retained<PlatformView>]) {
    let current = parent.subviews();
    if current.len() == ordered.len()
        && current
            .iter()
            .zip(ordered)
            .all(|(attached, wanted)| std::ptr::eq(&raw const *attached, &raw const **wanted))
    {
        return;
    }
    apply_subview_order(parent, ordered);
}

#[cfg(target_os = "macos")]
fn apply_subview_order(parent: &PlatformView, ordered: &[Retained<PlatformView>]) {
    use objc2_foundation::NSArray;
    parent.setSubviews(&NSArray::from_retained_slice(ordered));
}

#[cfg(target_os = "ios")]
fn apply_subview_order(parent: &PlatformView, ordered: &[Retained<PlatformView>]) {
    use std::ptr;

    for subview in parent.subviews().to_vec() {
        if !ordered
            .iter()
            .any(|wanted| ptr::eq(&raw const **wanted, &raw const *subview))
        {
            subview.removeFromSuperview();
        }
    }
    for (index, child) in ordered.iter().enumerate() {
        #[expect(
            clippy::cast_possible_wrap,
            reason = "a view hierarchy never reaches `NSInteger::MAX` subviews"
        )]
        parent.insertSubview_atIndex(child, index as isize);
    }
}
/// Pins `child`'s leading, trailing, top, and bottom edges to `parent`'s
/// with four equal-to-anchor constraints.
///
/// The caller is responsible for installing `child` as a subview of
/// `parent` first.
pub fn pin_edges(
    #[cfg_attr(target_os = "macos", allow(unused_variables))] mtm: crate::MainThreadMarker,
    parent: &PlatformView,
    child: &PlatformView,
) {
    #[cfg(target_os = "macos")]
    use objc2_app_kit::NSLayoutConstraint;
    use objc2_foundation::NSArray;
    #[cfg(target_os = "ios")]
    use objc2_ui_kit::NSLayoutConstraint;

    let constraints = NSArray::from_retained_slice(&[
        child
            .leadingAnchor()
            .constraintEqualToAnchor(&parent.leadingAnchor()),
        child
            .trailingAnchor()
            .constraintEqualToAnchor(&parent.trailingAnchor()),
        child
            .topAnchor()
            .constraintEqualToAnchor(&parent.topAnchor()),
        child
            .bottomAnchor()
            .constraintEqualToAnchor(&parent.bottomAnchor()),
    ]);
    #[cfg(target_os = "macos")]
    NSLayoutConstraint::activateConstraints(&constraints);
    #[cfg(target_os = "ios")]
    NSLayoutConstraint::activateConstraints(&constraints, mtm);
}

/// Applies the content-declared label and value — the label channel
/// controls whether the
/// view is an accessibility element; a value only ever marks it.
#[cfg(target_os = "macos")]
pub fn set_accessibility_content(view: &PlatformView, label: Option<&str>, value: Option<&str>) {
    use objc2_app_kit::NSAccessibility;
    use objc2_foundation::NSString;
    view.setAccessibilityElement(label.is_some() || value.is_some());
    view.setAccessibilityLabel(label.map(NSString::from_str).as_deref());
    // SAFETY: the setter is a plain property accessor on the main thread.
    let value = value.map(NSString::from_str);
    // SAFETY: the setter is a plain property accessor on the main thread; the
    // pointer is a live `NSString` while the call runs.
    unsafe {
        view.setAccessibilityValue(
            value
                .as_deref()
                .map(|value| &*std::ptr::from_ref(value).cast::<objc2::runtime::AnyObject>()),
        );
    };
}

/// Applies the content-declared label and value.
///
/// The label channel
/// controls whether the view is an accessibility element; a value only ever
/// marks it.
///
/// # Panics
///
/// If not called on the main thread.
#[cfg(target_os = "ios")]
pub fn set_accessibility_content(view: &PlatformView, label: Option<&str>, value: Option<&str>) {
    use objc2_foundation::NSString;
    use objc2_ui_kit::NSObjectUIAccessibility;
    let mtm = objc2::MainThreadMarker::new().expect("main thread");
    view.setAccessibilityLabel(label.map(NSString::from_str).as_deref(), mtm);
    let value = value.map(NSString::from_str);
    view.setAccessibilityValue(
        value.as_deref(),
        objc2::MainThreadMarker::new().expect("main thread"),
    );
    if label.is_some() || value.is_some() {
        view.setIsAccessibilityElement(true, mtm);
    }
}

/// An associated-object storage key derived from a selector name.
///
/// Registering the same string yields the same key at every call, so a
/// platform-side compatibility layer using the same name shares the
/// association.
#[must_use]
pub fn association_key(name: &core::ffi::CStr) -> *const core::ffi::c_void {
    let selector = objc2::runtime::Sel::register(name);
    // SAFETY: `Sel` is `repr(transparent)` over the selector pointer the
    // associated-object API expects as the key.
    unsafe { core::mem::transmute::<objc2::runtime::Sel, *const core::ffi::c_void>(selector) }
}

#[cfg(test)]
mod tests {
    use super::{Rect, SubpixelOffset, align_to_pixel_grid};

    #[test]
    fn pixel_ties_round_rightward_and_toward_the_screen_bottom() {
        let at_1x = align_to_pixel_grid(Rect::new(2.5, 7.5, 10.0, 10.0), SubpixelOffset::ZERO, 1.0);
        assert_eq!(at_1x.frame, Rect::new(3.0, 8.0, 10.0, 10.0));
        let at_2x = align_to_pixel_grid(Rect::new(0.25, 0.75, 1.0, 1.0), SubpixelOffset::ZERO, 2.0);
        assert_eq!(at_2x.frame, Rect::new(0.5, 1.0, 1.0, 1.0));
    }

    #[test]
    fn nested_alignment_rounds_absolute_layout_edges_once() {
        // Layout puts the parent at y 10.5 and the child 0.5 below it: the
        // child's absolute layout edge is 11.0, which needs no rounding.
        let parent =
            align_to_pixel_grid(Rect::new(0.0, 10.5, 40.0, 20.0), SubpixelOffset::ZERO, 1.0);
        assert_eq!(parent.frame, Rect::new(0.0, 11.0, 40.0, 20.0));
        let child = align_to_pixel_grid(Rect::new(0.0, 0.5, 16.0, 16.0), parent.offset, 1.0);
        assert_eq!(parent.frame.origin.y + child.frame.origin.y, 11.0);
        assert_eq!(child.frame.size.height, 16.0);
        // A grandchild a further 0.5 below rounds its absolute 11.5 once.
        let grandchild = align_to_pixel_grid(Rect::new(0.0, 0.5, 8.0, 8.0), child.offset, 1.0);
        assert_eq!(
            parent.frame.origin.y + child.frame.origin.y + grandchild.frame.origin.y,
            12.0
        );
    }

    #[test]
    fn a_frame_on_a_half_pixel_tie_keeps_its_measured_width() {
        // A 367pt block centred in 402pt lands on the half pixel 17.5 at
        // 3x; text 1001 pixels wide sits 16pt inside it. The f32 width is
        // just under 1001 pixels, so the tied max edge would round down.
        let block =
            align_to_pixel_grid(Rect::new(17.5, 0.0, 367.0, 40.0), SubpixelOffset::ZERO, 3.0);
        let text_width = f64::from(1001.0_f32 / 3.0);
        let text = align_to_pixel_grid(Rect::new(16.0, 0.0, text_width, 20.0), block.offset, 3.0);
        assert_eq!((text.frame.size.width * 3.0).round(), 1001.0);
    }

    #[test]
    fn no_frame_is_written_smaller_than_its_layout_size() {
        for scale in [1.0, 2.0, 3.0] {
            for origin_pixels in 0..24_u8 {
                // Origins on, between and at ties of the pixel grid, under
                // a parent offset that is itself a tie.
                let origin = f64::from(origin_pixels) / 8.0;
                let parent = align_to_pixel_grid(
                    Rect::new(origin, origin, 50.0, 50.0),
                    SubpixelOffset::ZERO,
                    scale,
                );
                for size_pixels in 0..400_u16 {
                    let size = f64::from(f32::from(size_pixels) / 7.0);
                    let child = align_to_pixel_grid(
                        Rect::new(origin, origin, size, size),
                        parent.offset,
                        scale,
                    );
                    let written = child.frame.size.width * scale;
                    let covered = (size * scale).mul_add(-f64::from(f32::EPSILON), size * scale);
                    assert!(written >= covered, "{size} at {origin} under 1/{scale}");
                    assert_eq!(written, child.frame.size.height * scale);
                    // The max edge moves out by at most the one pixel the
                    // tie or the rounding lost.
                    assert!(written < size.mul_add(scale, 1.0) + 1e-9);
                }
            }
        }
    }
}
