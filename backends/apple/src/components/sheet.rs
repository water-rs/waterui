//! The `sheet` metadata: a native window-modal sheet — an `NSWindow` sheet on
//! `AppKit`, a `UISheetPresentationController` / `UIViewController` modal sheet
//! on `UIKit` — presented while `is_presented` reads `true`
//! (water-rs/waterui#659).
//!
//! The leaf is a transparent container around its wrapped child. One sheet is
//! presented per window at a time via [`SheetQueue`]; a second sheet presented
//! while one is open waits in presentation order until the first ends.

use alloc::rc::{Rc, Weak};
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, SheetWindow, window_of};
use cocoa_ui::geometry::Rect as KitRect;
#[cfg(target_os = "macos")]
use cocoa_ui::geometry::Size as KitSize;
#[cfg(target_os = "macos")]
use cocoa_ui::objc2_app_kit::NSWindow as PlatformWindow;
#[cfg(target_os = "ios")]
use cocoa_ui::objc2_ui_kit::UIWindow as PlatformWindow;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HostView, SheetController, SheetDetentSpec};
use cocoa_ui::{MainThreadMarker, PlatformView, Retained, view};
use waterui::reactive::{Binding, Signal};
use waterui::sheet::{Sheet, SheetDetent};
use waterui_backend_core::Environment;
use waterui_core::Metadata;
use waterui_core::extract::State as EnvState;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, StretchAxis, SubView, ViewDimensions,
};

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext, Renderer};
use crate::primary_content;
use crate::proposal::{self, Placement};

#[cfg(target_os = "macos")]
const MACOS_SHEET_MIN_WIDTH: f64 = 320.0;
#[cfg(target_os = "macos")]
const MACOS_SHEET_MAX_WIDTH: f64 = 640.0;
#[cfg(target_os = "macos")]
const MACOS_SHEET_WIDTH_FRACTION: f64 = 0.85;
#[cfg(target_os = "macos")]
const MACOS_SHEET_MIN_HEIGHT: f64 = 200.0;

#[cfg(target_os = "macos")]
struct ActivePresentation {
    sheet_window: SheetWindow,
    host: Retained<HostView>,
    _mounted: Mounted,
}

#[cfg(target_os = "ios")]
struct ActivePresentation {
    controller: SheetController,
    host: Retained<HostView>,
    _mounted: Mounted,
}

struct SheetState {
    mtm: MainThreadMarker,
    host: Retained<HostView>,
    queue: Rc<SheetQueue>,
    child: Mounted,
    env: Environment,
    renderer: Renderer,
    sheet: Sheet,
    is_presented: Binding<bool>,
    presentation: RefCell<Option<ActivePresentation>>,
}

impl fmt::Debug for SheetState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SheetState").finish_non_exhaustive()
    }
}

struct QueuedSheet {
    window: Retained<PlatformWindow>,
    state: Weak<SheetState>,
}

/// Every window's presented sheets in presentation order: the first entry for a
/// window is on screen and later ones wait until earlier entries dismiss.
#[derive(Default)]
pub struct SheetQueue {
    entries: RefCell<Vec<QueuedSheet>>,
}

impl fmt::Debug for SheetQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SheetQueue").finish_non_exhaustive()
    }
}

impl SheetQueue {
    fn request(&self, state: &Rc<SheetState>) {
        let Some(window) = view::window(&state.host) else {
            return;
        };
        let front = {
            let mut entries = self.entries.borrow_mut();
            if entries
                .iter()
                .any(|entry| Weak::as_ptr(&entry.state) == Rc::as_ptr(state))
            {
                return;
            }
            let front = !entries
                .iter()
                .any(|entry| Retained::as_ptr(&entry.window) == Retained::as_ptr(&window));
            entries.push(QueuedSheet {
                window,
                state: Rc::downgrade(state),
            });
            front
        };
        if front {
            present(state);
        }
    }

    fn withdraw(&self, state: &Rc<SheetState>) {
        if let Some(presentation) = state.presentation.borrow_mut().take() {
            dismiss_presentation(state, &presentation);
            return;
        }
        let mut entries = self.entries.borrow_mut();
        let Some(index) = entries
            .iter()
            .position(|entry| Weak::as_ptr(&entry.state) == Rc::as_ptr(state))
        else {
            return;
        };
        let window = Retained::as_ptr(&entries[index].window);
        let in_front = entries
            .iter()
            .position(|entry| Retained::as_ptr(&entry.window) == window)
            == Some(index);
        if !in_front {
            entries.remove(index);
        }
    }

    fn finished(&self, state: &Weak<SheetState>) {
        let next = {
            let mut entries = self.entries.borrow_mut();
            let Some(index) = entries
                .iter()
                .position(|entry| Weak::ptr_eq(&entry.state, state))
            else {
                return;
            };
            let window = entries.remove(index).window;
            loop {
                let Some(index) = entries
                    .iter()
                    .position(|entry| Retained::as_ptr(&entry.window) == Retained::as_ptr(&window))
                else {
                    break None;
                };
                match entries[index].state.upgrade() {
                    Some(next) if next.is_presented.snapshot() => break Some(next),
                    _ => {
                        entries.remove(index);
                    }
                }
            }
        };
        if let Some(next) = next {
            present(&next);
        }
    }
}

struct SheetSubView {
    state: Rc<SheetState>,
}

impl fmt::Debug for SheetSubView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SheetSubView").finish_non_exhaustive()
    }
}

impl SubView for SheetSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        self.state.child.layout().resolve_extent(bounds, proposal)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.child.layout().is_empty()
    }
}

fn sheet_content_env(state: &SheetState) -> Environment {
    state.env.extending(EnvState(state.sheet.dismiss_handle()))
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "sheet detent contract resolves f32 layout points against AppKit f64 window dimensions"
)]
fn resolve_macos_sheet_size(parent: &PlatformWindow, detent: SheetDetent) -> KitSize {
    let parent_frame = parent.contentRectForFrameRect(parent.frame());
    let parent_width = parent_frame.size.width.max(MACOS_SHEET_MIN_WIDTH);
    let parent_height = parent_frame.size.height.max(MACOS_SHEET_MIN_HEIGHT);
    match detent {
        SheetDetent::FullScreen => KitSize::new(parent_width, parent_height),
        detent => {
            let width = (parent_width * MACOS_SHEET_WIDTH_FRACTION).clamp(
                MACOS_SHEET_MIN_WIDTH.min(parent_width),
                MACOS_SHEET_MAX_WIDTH.min(parent_width),
            );
            let height = f64::from(detent.resolve_height(parent_height as f32))
                .clamp(MACOS_SHEET_MIN_HEIGHT.min(parent_height), parent_height);
            KitSize::new(width, height)
        }
    }
}

#[cfg(target_os = "macos")]
fn present(state: &Rc<SheetState>) {
    let Some(window) = window_of(&state.host) else {
        tracing::error!("a sheet's host is no longer in a window; the sheet is not presented");
        state.queue.finished(&Rc::downgrade(state));
        return;
    };
    let size = resolve_macos_sheet_size(&window, state.sheet.primary_detent());
    let sheet_host = HostView::new(state.mtm, KitRect::new(0.0, 0.0, size.width, size.height));
    let sheet_host_view: &PlatformView = &sheet_host;
    let content_view = Metadata::new(state.sheet.build_content(), sheet_content_env(state));
    let mounted = state.renderer.render(content_view).mount(sheet_host_view);
    view::set_translates_autoresizing(mounted.view(), true);
    let content_child_view = view::retain_base(mounted.view());
    view::set_frame(&content_child_view, view::bounds(sheet_host_view));
    sheet_host.set_layout_handler(move |host| {
        let bounds = view::bounds(host);
        view::set_frame(&content_child_view, bounds);
        let rect = proposal::layout_rect(bounds);
        proposal::deliver(
            &content_child_view,
            Placement::platform_hosted(ProposalSize::new(Some(rect.width()), Some(rect.height()))),
        );
    });

    let sheet_window = SheetWindow::new(state.mtm, size);
    sheet_window.set_content(sheet_host_view);
    sheet_window.set_on_cancel({
        let state = Rc::downgrade(state);
        move || {
            if let Some(state) = state.upgrade() {
                state.sheet.run_user_dismiss(&state.env);
            }
        }
    });

    let weak = Rc::downgrade(state);
    let queue = Rc::clone(&state.queue);
    sheet_window.begin_sheet(&window, move || {
        if let Some(state) = weak.upgrade()
            && let Some(presentation) = state.presentation.borrow_mut().take()
        {
            presentation.sheet_window.clear_on_cancel();
            presentation.host.clear_handlers();
        }
        queue.finished(&weak);
    });

    *state.presentation.borrow_mut() = Some(ActivePresentation {
        sheet_window,
        host: sheet_host,
        _mounted: mounted,
    });
}

#[cfg(target_os = "macos")]
fn dismiss_presentation(_state: &Rc<SheetState>, presentation: &ActivePresentation) {
    presentation.sheet_window.clear_on_cancel();
    presentation.host.clear_handlers();
    presentation.sheet_window.dismiss();
}

#[cfg(target_os = "ios")]
fn map_detents(detents: &[SheetDetent]) -> Vec<SheetDetentSpec> {
    detents
        .iter()
        .map(|&detent| match detent {
            SheetDetent::Medium => SheetDetentSpec::Medium,
            SheetDetent::Large => SheetDetentSpec::Large,
            SheetDetent::Fraction(fraction) => SheetDetentSpec::Fraction(f64::from(fraction)),
            SheetDetent::Height(height) => SheetDetentSpec::Height(f64::from(height)),
            SheetDetent::FullScreen => SheetDetentSpec::FullScreen,
        })
        .collect()
}

#[cfg(target_os = "ios")]
fn present(state: &Rc<SheetState>) {
    let sheet_host = HostView::new(state.mtm, KitRect::ZERO);
    let sheet_host_view: &PlatformView = &sheet_host;
    let content_view = Metadata::new(state.sheet.build_content(), sheet_content_env(state));
    let mounted = state.renderer.render(content_view).mount(sheet_host_view);
    view::set_translates_autoresizing(mounted.view(), true);
    let content_child_view = view::retain_base(mounted.view());
    sheet_host.set_layout_handler(move |host| {
        let bounds = view::bounds(host);
        view::set_frame(&content_child_view, bounds);
        let rect = proposal::layout_rect(bounds);
        proposal::deliver(
            &content_child_view,
            Placement::platform_hosted(ProposalSize::new(Some(rect.width()), Some(rect.height()))),
        );
    });

    let detents = map_detents(state.sheet.configured_detents());
    let weak = Rc::downgrade(state);
    let queue = Rc::clone(&state.queue);
    let controller = SheetController::new(
        state.mtm,
        sheet_host.clone(),
        &detents,
        state.sheet.is_dismissible(),
        move || {
            if let Some(state) = weak.upgrade() {
                if let Some(presentation) = state.presentation.borrow_mut().take() {
                    presentation.controller.clear_callbacks();
                    presentation.host.clear_handlers();
                }
                state.sheet.run_user_dismiss(&state.env);
            }
            queue.finished(&weak);
        },
    );

    if !controller.present(&state.host) {
        controller.clear_callbacks();
        sheet_host.clear_handlers();
        tracing::error!("a sheet's host has no window or root view controller; not presented");
        state.queue.finished(&Rc::downgrade(state));
        return;
    }

    *state.presentation.borrow_mut() = Some(ActivePresentation {
        controller,
        host: sheet_host,
        _mounted: mounted,
    });
}

#[cfg(target_os = "ios")]
fn dismiss_presentation(state: &Rc<SheetState>, presentation: &ActivePresentation) {
    presentation.controller.clear_callbacks();
    presentation.host.clear_handlers();
    let weak = Rc::downgrade(state);
    let queue = Rc::clone(&state.queue);
    presentation.controller.dismiss(move || {
        queue.finished(&weak);
    });
}

fn attachment_changed(state: &Rc<SheetState>) {
    if view::has_window(&state.host) {
        if state.is_presented.snapshot() {
            state.queue.request(state);
        }
    } else {
        if state.is_presented.snapshot() {
            state.is_presented.set(false);
        }
        state.queue.withdraw(state);
    }
}

/// Installs the shared [`SheetQueue`] on the root `env`.
pub fn install(env: &mut Environment) {
    env.insert(Rc::new(SheetQueue::default()));
}

/// Realizes `Metadata<Sheet, Held>` as a transparent wrapper whose attached
/// modal sheet follows `is_presented`.
#[must_use]
pub fn realize_sheet_metadata(
    metadata: Metadata<Sheet, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let Metadata {
        content,
        value: sheet,
    } = metadata;
    let host = HostView::new(ctx.mtm(), KitRect::ZERO);
    let host_view: &PlatformView = &host;
    let mounted = ctx.realize(content).mount(host_view);
    view::set_translates_autoresizing(mounted.view(), true);
    primary_content::forward(&host, mounted.view());

    let state = Rc::new(SheetState {
        mtm: ctx.mtm(),
        host: host.clone(),
        queue: Rc::clone(
            ctx.env()
                .get::<Rc<SheetQueue>>()
                .expect("`dispatch::install` gives the root environment its sheet queue"),
        ),
        child: mounted,
        env: ctx.env().clone(),
        renderer: ctx.renderer(),
        is_presented: sheet.is_presented().clone(),
        sheet,
        presentation: RefCell::new(None),
    });

    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host| {
            view::set_frame(state.child.view(), view::bounds(host));
        }
    });
    host.set_window_handler({
        let state = Rc::downgrade(&state);
        move |_| {
            if let Some(state) = state.upgrade() {
                attachment_changed(&state);
            }
        }
    });
    let sink_guard = proposal::register_sink(host_view, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.child.view(), selected);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        SheetSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(sink_guard);
    leaf.keep(Rc::clone(&state));

    leaf.watch(&state.is_presented, {
        let state = Rc::downgrade(&state);
        move |ctx| {
            let Some(state) = state.upgrade() else {
                return;
            };
            if *ctx.value() {
                state.queue.request(&state);
            } else {
                state.queue.withdraw(&state);
            }
        }
    });

    if state.is_presented.snapshot() {
        state.queue.request(&state);
    }

    leaf
}
