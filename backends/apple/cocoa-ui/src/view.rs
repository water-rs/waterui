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

/// The bounds in the view's own coordinate space.
#[must_use]
pub fn bounds(view: &PlatformView) -> Rect {
    view.bounds().into()
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

/// Detaches `view` from its superview; does nothing when it has none.
pub fn remove_from_superview(view: &PlatformView) {
    view.removeFromSuperview();
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

/// Makes `parent`'s subviews exactly `ordered`, in that z-order, reusing the
/// subview instances already attached.
#[cfg(target_os = "macos")]
pub fn reconcile_subviews(parent: &PlatformView, ordered: &[Retained<PlatformView>]) {
    use objc2_foundation::NSArray;
    parent.setSubviews(&NSArray::from_retained_slice(ordered));
}

/// Makes `parent`'s subviews exactly `ordered`, in that z-order, reusing the
/// subview instances already attached.
#[cfg(target_os = "ios")]
pub fn reconcile_subviews(parent: &PlatformView, ordered: &[Retained<PlatformView>]) {
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
