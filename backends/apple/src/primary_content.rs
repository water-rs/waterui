//! The primary-content forward every transparent single-child wrapper
//! installs.
//!
//! `WuiPrimaryContentProviding`: the window's safe-area question
//! (`wuiHandlesSafeArea`) and the scroll-surface search
//! (`wuiScrollSurface`) descend a wrapper's primary content. A wrapper that
//! mounts one content view on a plain `HostView` and draws no chrome of its
//! own is transparent — it answers both questions with its child, so a
//! scroll surface under it keeps managing its own safe area and reaches the
//! screen edges instead of being framed inside the wrapper's safe-area
//! rect.
//!
//! A wrapper that IS a content boundary does not forward: a fixed
//! container, a stack, or a chrome surface (material, glass, a control's
//! own chrome) owns the safe-area answer and frames its children inside
//! itself.

#[cfg(feature = "dynamic")]
use cocoa_ui::Retained;
use cocoa_ui::{PlatformView, view};

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// Forwards `host`'s primary content to `child`: call after mounting the
/// wrapper's content view.
///
/// The forward retains `child` — the mounted leaf keeps its usual owner.
pub fn forward(host: &HostView, child: &PlatformView) {
    let child = view::retain_base(child);
    host.set_primary_content_handler(move |_host| Some(child.clone()));
}

/// Forwards `host`'s primary content to the view `current` returns at call
/// time: the variant for a wrapper whose mounted child changes over the
/// leaf's life (`dynamic`, a navigation page root).
#[cfg(any(feature = "dynamic", feature = "navigation"))]
pub fn forward_current(
    host: &HostView,
    current: impl Fn(&HostView) -> Option<Retained<PlatformView>> + 'static,
) {
    host.set_primary_content_handler(current);
}

#[cfg(all(target_os = "ios", any(feature = "list", feature = "navigation")))]
/// The pane's declared scrolling surface: the view itself when it is
/// a `UIScrollView`, else the host's declared scroll-surface
/// candidates in stacking order, else a transparent wrapper's
/// primary content, else — at a view whose owning controller is a
/// `UINavigationController` — the pushed page controller's root view,
/// where the declared chain resumes. Undeclared subviews —
/// decorations, internal `UIKit` chrome — are never considered.
pub fn declared_scroll_view(
    view: &cocoa_ui::PlatformView,
) -> Option<cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIScrollView>> {
    use cocoa_ui::objc2_ui_kit::{UINavigationController, UIScrollView};
    use cocoa_ui::uikit::view_controller;
    if let Some(scroll) = view.downcast_ref::<UIScrollView>() {
        return Some(scroll.into());
    }
    for candidate in view::scroll_surface_candidates(view) {
        if let Some(found) = declared_scroll_view(&candidate) {
            return Some(found);
        }
    }
    if let Some(primary) = view::primary_content(view)
        && let Some(found) = declared_scroll_view(&primary)
    {
        return Some(found);
    }
    let controller = view_controller::owning_controller(view)
        .and_then(|controller| controller.downcast::<UINavigationController>().ok())?;
    declared_scroll_view(controller.topViewController()?.view()?.as_ref())
}
