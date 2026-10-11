//! The `()` leaf: `Native<()>` rendered as an invisible zero-size view.
//!
//! Mirrors `WuiEmpty`: the unit view draws nothing, measures to `.zero`, and
//! answers `is_empty` so a stack treats it as a non-member (no slot, no
//! spacing). It must be claimed unconditionally — the Rust walk expands a
//! bare `()` into `Native<()>`, which the fallback's registry keys under
//! `()`, not `Native<()>` — so leaving it unclaimed drops it into
//! `body()`, where the panic propagates out of `render` and the packaging
//! profile (`panic = "abort"`) turns that into a process abort.

use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The zero-size `SubView` every empty leaf answers.
struct EmptySubView;

impl SubView for EmptySubView {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(0.0, 0.0))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }

    fn is_empty(&self) -> bool {
        true
    }
}

/// Installs the `()` handler on the dispatcher: `Native<()>` maps to a
/// hidden, zero-size view that never participates in layout.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<()>(|(), ctx| {
        let view = HostView::new(ctx.mtm(), cocoa_ui::Rect::ZERO);
        cocoa_ui::view::set_hidden(&view, true);
        NativeLeaf::new(&*view, EmptySubView)
    });
}
