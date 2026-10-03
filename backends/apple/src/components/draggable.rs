//! The `draggable` metadata: `Metadata<Draggable>` wrapped around a child.
//!
//! Mirrors `WuiDraggable`: a transparent `HostView` container — measure,
//! stretch and placement all answer for the mounted child — that is a drag
//! source. The payload is read from its `Computed` when a drag begins and
//! travels two ways: platform-representable payloads (text, URL, files) are
//! written to the pasteboard so other applications can receive them, and an
//! `Rc` of the typed `DragPayload` travels in-process for same-process drop
//! destinations (iOS `localObject`, macOS the dragging source object). An
//! `InProcess` payload has no pasteboard representation — only a marker
//! type routes the drag.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

#[cfg(target_os = "macos")]
use cocoa_ui::Point;
use cocoa_ui::Rect;
use cocoa_ui::view;
#[cfg(target_os = "macos")]
use waterui::drag_drop::DragPayload;
use waterui::drag_drop::{Draggable, PlatformRepresentation};
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, drag_drop as kit};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HostView, drag_drop as kit};

/// Pasteboard type marking a drag whose payload stays inside the process —
/// the marker carries no data; a same-process destination reads the
/// `DragPayload` off the drag item (iOS) or the dragging source (macOS).
const IN_PROCESS_TYPE: &str = "dev.waterui.inProcessDragPayload";

/// The leaf's live state: the mounted child the layout face forwards to,
/// the draggable it reads payloads from, and where the pointer came down
/// while a macOS drag is pending.
struct DraggableLeafState {
    /// The mounted content.
    child: Mounted,
    /// The draggable whose `Computed` is snapshotted when a drag begins.
    draggable: Draggable,
    /// The window-space point the last `mouseDown:` reported; `None` while
    /// no drag is pending.
    #[cfg(target_os = "macos")]
    drag_origin: Option<Point>,
}

impl core::fmt::Debug for DraggableLeafState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DraggableLeafState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: the content's own answers everywhere.
struct DraggableSubView {
    /// The leaf's state.
    state: Rc<RefCell<DraggableLeafState>>,
}

impl core::fmt::Debug for DraggableSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DraggableSubView").finish_non_exhaustive()
    }
}

impl SubView for DraggableSubView {
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

#[cfg(target_os = "macos")]
/// One dragging item per payload representation, matching
/// `WuiDraggable.mouseDragged`: text and URL are single items, files are
/// one item per URL (like Finder), and an in-process payload only carries
/// the routing marker. Each item drags a snapshot of the view's bounds.
fn drag_items(host: &HostView, payload: &DragPayload) -> Vec<kit::DragItemSpec> {
    let frame = view::bounds(host);
    let image = kit::view_snapshot(host);
    match payload.platform_representation() {
        PlatformRepresentation::Text(text) => vec![kit::DragItemSpec {
            item: kit::text_item(text.as_str()),
            frame: frame.into(),
            image,
        }],
        PlatformRepresentation::Url(url) => vec![kit::DragItemSpec {
            item: kit::url_item(url.as_str()),
            frame: frame.into(),
            image,
        }],
        PlatformRepresentation::Files(files) => files
            .urls()
            .iter()
            .map(|url| kit::DragItemSpec {
                item: kit::file_url_item(url.as_str()),
                frame: frame.into(),
                image: image.clone(),
            })
            .collect(),
        PlatformRepresentation::InProcess => vec![kit::DragItemSpec {
            item: kit::marker_item(IN_PROCESS_TYPE),
            frame: frame.into(),
            image,
        }],
    }
}

/// Installs the `draggable` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Draggable>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(RefCell::new(DraggableLeafState {
            child: mounted,
            draggable: metadata.value,
            #[cfg(target_os = "macos")]
            drag_origin: None,
        }));

        // The content always fills the wrapper — `contentView.frame = bounds`.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                let state = state.borrow();
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });

        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            move |selected| {
                let state = state.borrow();
                proposal::deliver(state.child.view(), selected);
            }
        });

        #[cfg(target_os = "macos")]
        {
            // `mouseDown:` stores where the drag might start.
            host.set_mouse_down_handler({
                let state = Rc::clone(&state);
                move |_view, event| {
                    state.borrow_mut().drag_origin = Some(event.locationInWindow().into());
                }
            });
            // `mouseDragged:` beyond 3pt begins the dragging session.
            host.set_mouse_dragged_handler({
                let state = Rc::clone(&state);
                move |view, event| {
                    let origin = state.borrow().drag_origin;
                    let Some(origin) = origin else { return };
                    let current: Point = event.locationInWindow().into();
                    let distance = (current.x - origin.x).hypot(current.y - origin.y);
                    if distance <= 3.0 {
                        return;
                    }
                    state.borrow_mut().drag_origin = None;
                    let payload = state.borrow().draggable.payload();
                    let local: Rc<dyn core::any::Any> = Rc::new(payload.clone());
                    let items = drag_items(view, &payload);
                    let _session = kit::begin_drag(view, event, items, Some(local), || {});
                }
            });
        }

        #[cfg(target_os = "ios")]
        let drag_source = {
            let state = Rc::clone(&state);
            kit::drag_source(&host, move || {
                let payload = state.borrow().draggable.payload();
                let local: Rc<dyn core::any::Any> = Rc::new(payload.clone());
                let providers: Vec<_> = match payload.platform_representation() {
                    PlatformRepresentation::Text(text) => {
                        vec![kit::text_item_provider(text.as_str())]
                    }
                    PlatformRepresentation::Url(url) => {
                        vec![kit::url_item_provider(url.as_str())]
                    }
                    PlatformRepresentation::Files(files) => files
                        .urls()
                        .iter()
                        .map(|url| kit::url_item_provider(url.as_str()))
                        .collect(),
                    PlatformRepresentation::InProcess => {
                        vec![kit::marker_item_provider(IN_PROCESS_TYPE)]
                    }
                };
                providers
                    .into_iter()
                    .map(|provider| kit::DragItemSpec {
                        provider,
                        local: Some(Rc::clone(&local)),
                    })
                    .collect()
            })
        };

        let mut leaf = NativeLeaf::new(
            &*host,
            DraggableSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        #[cfg(target_os = "ios")]
        leaf.keep(drag_source);
        leaf
    });
}
