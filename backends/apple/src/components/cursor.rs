//! The `cursor` metadata: `Metadata<Cursor>` wrapped around a child.
//!
//! Mirrors `WuiCursor`. On `AppKit` a tracking area
//! (`mouseEnteredAndExited` + `cursorUpdate` + `activeInKeyWindow` +
//! `inVisibleRect`) applies the style on entry and the arrow on exit; the
//! style watcher re-applies while the pointer is inside. On `UIKit` a
//! `UIPointerInteraction` lifts the view's region into a rounded-rect
//! pointer shape.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::Rect;
use cocoa_ui::view;
use waterui::cursor::Cursor;
#[cfg(target_os = "macos")]
use waterui::cursor::CursorStyle;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{self as appkit, HostView};
#[cfg(target_os = "macos")]
use cocoa_ui::pointer::{PointerEvent, PointerEvents};

#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{self as uikit, HostView};

/// The leaf's live state: the mounted child.
struct CursorState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for CursorState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CursorState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: the content's answers verbatim.
struct CursorSubView {
    /// The leaf's state.
    state: Rc<RefCell<CursorState>>,
}

impl core::fmt::Debug for CursorSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CursorSubView").finish_non_exhaustive()
    }
}

impl SubView for CursorSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.borrow().child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.borrow().child.layout().priority()
    }
}

/// The semantic style as a kit `Cursor` — the enums line up.
#[cfg(target_os = "macos")]
const fn kit_style(style: CursorStyle) -> appkit::Cursor {
    match style {
        CursorStyle::PointingHand => appkit::Cursor::PointingHand,
        CursorStyle::IBeam => appkit::Cursor::IBeam,
        CursorStyle::Crosshair => appkit::Cursor::Crosshair,
        CursorStyle::OpenHand => appkit::Cursor::OpenHand,
        CursorStyle::ClosedHand => appkit::Cursor::ClosedHand,
        CursorStyle::NotAllowed => appkit::Cursor::NotAllowed,
        CursorStyle::ResizeLeft => appkit::Cursor::ResizeLeft,
        CursorStyle::ResizeRight => appkit::Cursor::ResizeRight,
        CursorStyle::ResizeUp => appkit::Cursor::ResizeUp,
        CursorStyle::ResizeDown => appkit::Cursor::ResizeDown,
        CursorStyle::ResizeLeftRight => appkit::Cursor::ResizeLeftRight,
        CursorStyle::ResizeUpDown => appkit::Cursor::ResizeUpDown,
        CursorStyle::Move => appkit::Cursor::Move,
        CursorStyle::Wait => appkit::Cursor::Wait,
        CursorStyle::Copy => appkit::Cursor::Copy,
        _ => appkit::Cursor::Arrow,
    }
}

/// Installs the `cursor` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Cursor>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(RefCell::new(CursorState { child: mounted }));

        // The content always fills the wrapper.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                let state = state.borrow();
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });

        // `setPlacementProposal` forwards to the content.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            move |selected| {
                let state = state.borrow();
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            &*host,
            CursorSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);

        #[cfg(target_os = "macos")]
        {
            use core::cell::Cell;
            let current = Rc::new(Cell::new(CursorStyle::Arrow));
            // Enter applies the style, exit restores the arrow; a style
            // change while the pointer is inside re-applies immediately —
            // `updateTrackingAreas` keeps the same area as `bounds` moves.
            host.set_pointer_handler(
                PointerEvents::ENTERED
                    .union(PointerEvents::EXITED)
                    .union(PointerEvents::CURSOR_UPDATE),
                {
                    let current = Rc::clone(&current);
                    move |_, event| {
                        match event {
                            PointerEvent::Entered | PointerEvent::CursorUpdate => {
                                kit_style(current.get()).set();
                            }
                            PointerEvent::Exited => {
                                appkit::Cursor::Arrow.set();
                            }
                            PointerEvent::Moved(_) => {}
                        }
                        false
                    }
                },
            );
            leaf.bind(&metadata.value.style, {
                move |style| {
                    current.set(style);
                    if host.is_pointer_inside() {
                        kit_style(style).set();
                    }
                }
            });
        }

        #[cfg(target_os = "ios")]
        {
            // `UIPointerInteraction` gives the lifted region a
            // rounded-rect pointer shape; the style signal does not
            // change it — `UIKit` has no cursor vocabulary.
            leaf.keep(uikit::PointerInteraction::rounded_rect(&host));
            let _ = &metadata.value.style;
        }

        leaf.keep(state);
        leaf
    });
}
