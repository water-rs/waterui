//! One `UIKit` window scene: its platform objects, and the session mounted
//! into them once the process's GPU context exists.
//!
//! `UIKit` asks for the scene's window while it connects, so the window, its
//! root controller and its input view are built then, with the display link
//! parked and the accessibility adapter attached before the view first
//! shows. The session — runtime window, renderer, environment — mounts later
//! if the GPU context is still opening, and the scene renders nothing until
//! it does.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cocoa_ui::Retained;
use cocoa_ui::notification::NotificationObserver;
use cocoa_ui::objc2_ui_kit::UIViewAutoresizing;
use cocoa_ui::uikit::input_view::InputView;
use cocoa_ui::uikit::{
    ColorSchemeObservation, DisplayScaleObservation, KeyboardBand, ViewController, Window,
    WindowScene, window_root,
};
use nami::Signal as _;
use waterui::theme::ColorScheme;
use waterui_layout::padding::EdgeInsets;

use super::super::accessibility::AccessibilityBridge;
use super::super::session::{AppleExecutor, AppleSession};
use super::super::surface::{AppleViewHandle, AppleViewSurface};
use super::super::text_input::{DocumentClient, ImeShared};
use super::Process;
use super::host::{SceneShared, UiKitHostWindow};
use crate::platform::{PlatformWindow as _, WindowKeyboardArea, WindowSafeArea};
use crate::renderer::{FontFamilyResolution, HydrolysisRenderer};
use crate::runner::{RenderDiagnosticsConfig, install_view_renderer};
use crate::text::ParleyEngine;

/// A connected scene.
///
/// Field order is teardown order: the session (renderer, surface) goes
/// first, then everything `UIKit` still calls into.
pub(super) struct UiKitScene {
    session: RefCell<Option<AppleSession<UiKitHostWindow>>>,
    /// Handed to the session when it mounts.
    accessibility: RefCell<Option<AccessibilityBridge>>,
    pub(super) shared: Rc<SceneShared>,
    session_id: String,
    mounted: Cell<bool>,
    _keyboard: NotificationObserver,
    _appearance: ColorSchemeObservation,
    _scale: DisplayScaleObservation,
    controller: Retained<ViewController>,
}

/// The framework scheme for the one `cocoa-ui` reports.
pub(super) const fn framework_scheme(scheme: cocoa_ui::ColorScheme) -> ColorScheme {
    match scheme {
        cocoa_ui::ColorScheme::Light => ColorScheme::Light,
        cocoa_ui::ColorScheme::Dark => ColorScheme::Dark,
    }
}

impl UiKitScene {
    /// Builds the platform objects for a scene `UIKit` is connecting, and the
    /// window they sit in.
    pub(super) fn connect(
        scene: &WindowScene,
        color_scheme: &nami::Binding<ColorScheme>,
    ) -> (Window, Rc<Self>) {
        let mtm = scene.main_thread();
        let window = Window::new(scene);
        let controller = ViewController::new(mtm, window_root(mtm));
        let view = InputView::new(mtm);
        view.setAutoresizingMask(
            UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight,
        );
        view.setFrame(controller.host_view().bounds());
        controller.host_view().add_subview(&view);
        window.set_root_view_controller(&controller);

        let keyboard_band = KeyboardBand::attach(mtm, &view);
        // A scene connects in the background and is told when it enters the
        // foreground; nothing draws before that.
        let shared = SceneShared::new(
            mtm,
            scene.clone(),
            Retained::clone(&view),
            keyboard_band,
            false,
        );
        let request_frame = frame_requester(&shared);

        view.set_event_handler({
            let shared = Rc::downgrade(&shared);
            move |event| {
                if let Some(shared) = shared.upgrade() {
                    shared.push_surface_event(event);
                }
            }
        });
        view.set_layout_handler({
            let request_frame = Rc::clone(&request_frame);
            move || request_frame()
        });
        view.set_safe_area_handler({
            let request_frame = Rc::clone(&request_frame);
            move |_| request_frame()
        });

        // The adapter subclasses the view, which must happen before the view
        // is first shown.
        let accessibility = AccessibilityBridge::new(&view, &request_frame);
        window.make_key_and_visible();
        window.layout_if_needed();

        let appearance = controller.observe_color_scheme({
            let color_scheme = color_scheme.clone();
            move |scheme| {
                let scheme = framework_scheme(scheme);
                if color_scheme.snapshot() != scheme {
                    color_scheme.set(scheme);
                }
            }
        });
        let scale = controller.observe_display_scale({
            let request_frame = Rc::clone(&request_frame);
            move |_| request_frame()
        });
        let keyboard = shared.observe_keyboard();

        let scene = Rc::new(Self {
            session: RefCell::new(None),
            accessibility: RefCell::new(Some(accessibility)),
            shared,
            session_id: scene.session_id(),
            mounted: Cell::new(false),
            _keyboard: keyboard,
            _appearance: appearance,
            _scale: scale,
            controller,
        });
        (window, scene)
    }

    pub(super) fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Mounts the process's window declaration into this scene.
    ///
    /// # Panics
    /// When the declaration was already mounted: one declaration backs one
    /// scene until the host can instantiate declarations.
    pub(super) fn mount(self: &Rc<Self>, process: &Process, executor: &AppleExecutor) {
        assert!(!self.mounted.replace(true), "a scene mounts once");
        let declaration = process.take_window();
        let shared = &self.shared;

        let safe_area = nami::binding(EdgeInsets::default());
        let keyboard_area = nami::binding(EdgeInsets::default());
        let mut env = process.env.clone();
        env.insert(WindowSafeArea(safe_area.clone()));
        env.insert(WindowKeyboardArea(keyboard_area.clone()));
        install_view_renderer(&mut env, &process.theme);

        let display_moved = Rc::new(Cell::new(false));
        let view_handle = AppleViewHandle::new(
            Retained::into_super(Retained::clone(&shared.view)),
            shared.max_frames_per_second(),
        );
        let surface = AppleViewSurface::new(
            process.gpu.clone(),
            view_handle,
            (1, 1),
            Rc::clone(&display_moved),
        );
        let mut platform = UiKitHostWindow::new(
            Rc::clone(shared),
            surface,
            display_moved,
            safe_area,
            keyboard_area,
            executor.poster(),
        );
        platform.apply_properties(&declaration);

        let mut renderer = HydrolysisRenderer::with_engine(
            Rc::clone(&process.theme),
            ParleyEngine::from_collection(&process.fonts, FontFamilyResolution::Lenient),
        );
        renderer.set_window_id(process.shortcuts.mint_window_id());
        renderer.set_window_closable(declaration.closable);
        let mut runtime = crate::runner::window::RuntimeWindow::new_gpu(
            declaration,
            platform,
            renderer,
            RenderDiagnosticsConfig::from_env(),
            &env,
        );
        // Mounted parked when the scene is not yet visible: the lifecycle
        // report that makes it visible unparks it.
        runtime.set_hidden(runtime.platform.is_occluded());
        let occluded = runtime.platform.is_occluded();
        crate::runner::apple::session::AppleHostWindow::frame_transaction(&runtime.platform)
            .sync_occlusion(occluded);

        let ime = ImeShared::new(shared.view.text_input_notifier(), frame_requester(shared));
        shared
            .view
            .set_text_input_client(Rc::new(DocumentClient(Rc::clone(&ime))));
        let accessibility = self
            .accessibility
            .borrow_mut()
            .take()
            .expect("a scene mounts once");
        self.session.replace(Some(AppleSession::new(
            env,
            runtime,
            executor.clone(),
            ime,
            accessibility,
            Rc::clone(&process.pending_window_queue),
        )));

        let this = Rc::downgrade(self);
        shared.start(Rc::new(move || {
            if let Some(scene) = this.upgrade() {
                scene.tick();
            }
        }));
        shared.request_frame();
    }

    /// One display-link tick: the frame transaction.
    fn tick(&self) {
        let outcome = {
            let mut session = self
                .session
                .try_borrow_mut()
                .expect("a frame tick never runs inside another use of the session");
            let Some(session) = session.as_mut() else {
                return;
            };
            session.on_frame()
        };
        self.shared.frame_done(&outcome);
    }

    /// The scene's visibility moved: re-derive the pump's occlusion and, on
    /// un-hiding, post the restore frame.
    fn visibility_changed(&self) {
        let mut session = self
            .session
            .try_borrow_mut()
            .expect("lifecycle callbacks never run inside a frame tick");
        if let Some(session) = session.as_mut() {
            let occluded = session.runtime.platform.is_occluded();
            crate::runner::apple::session::AppleHostWindow::frame_transaction(
                &session.runtime.platform,
            )
            .sync_occlusion(occluded);
            session.runtime.sync_occlusion_and_post_restore();
        }
    }

    pub(super) fn entered_foreground(&self) {
        self.shared.set_foreground(true);
        self.visibility_changed();
    }

    pub(super) fn entered_background(&self) {
        self.shared.set_foreground(false);
        self.visibility_changed();
    }

    pub(super) fn became_active(&self) {
        self.shared
            .push_event(crate::platform::InputEvent::Focused(true));
    }

    pub(super) fn resigned_active(&self) {
        self.shared
            .push_event(crate::platform::InputEvent::Focused(false));
    }

    /// The system reclaimed the scene: stop its clock and clear what `UIKit`
    /// still calls into; the scene's drop releases the rest.
    pub(super) fn disconnect(&self) {
        self.shared.stop();
        self.shared.view.clear_text_input_client();
        self.controller.host_view().clear_handlers();
    }
}

/// A closure that asks the scene for a frame, without owning it.
fn frame_requester(shared: &Rc<SceneShared>) -> Rc<dyn Fn()> {
    let shared = Rc::downgrade(shared);
    Rc::new(move || {
        if let Some(shared) = shared.upgrade() {
            shared.request_frame();
        }
    })
}
