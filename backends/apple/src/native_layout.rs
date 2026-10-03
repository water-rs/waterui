//! Native safe-area policy shared by roots and transparent containers.

use cocoa_ui::objc2_foundation::NSObjectProtocol;
use cocoa_ui::{PlatformView, Rect, view};

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

pub fn manages_safe_area(view: &PlatformView) -> bool {
    if owns_safe_area(view) {
        return true;
    }
    view::primary_content(view).is_some_and(|child| manages_safe_area(&child))
}

#[cfg(target_os = "macos")]
pub fn safe_area_rect(view: &PlatformView) -> Rect {
    view.safeAreaRect().into()
}

#[cfg(target_os = "ios")]
pub fn safe_area_rect(view: &PlatformView) -> Rect {
    let mut insets = view.safeAreaInsets();
    let mut ancestor = Some(view::retain_base(view));
    while let Some(current) = ancestor {
        let ignored: isize = if current.respondsToSelector(objc2::sel!(cocoaUiIgnoredSafeAreaEdges))
        {
            // SAFETY: cocoa-ui declares this selector as an NSInteger edge mask.
            unsafe { objc2::msg_send![&*current, cocoaUiIgnoredSafeAreaEdges] }
        } else {
            0
        };
        if ignored & 0x10 != 0 {
            if ignored & 1 != 0 {
                insets.top = 0.0;
            }
            if ignored & 2 != 0 {
                insets.left = 0.0;
            }
            if ignored & 4 != 0 {
                insets.bottom = 0.0;
            }
            if ignored & 8 != 0 {
                insets.right = 0.0;
            }
        } else if owns_safe_area(&current) {
            break;
        }
        ancestor = view::superview(&current);
    }
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

pub fn content_frame(content: &PlatformView, host: &PlatformView) -> Rect {
    if manages_safe_area(content) {
        view::bounds(host)
    } else {
        safe_area_rect(host)
    }
}
