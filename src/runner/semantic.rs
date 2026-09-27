//! The GPU-free semantic runtime: it builds and patches the retained widget
//! tree and emits the AccessKit tree — no device, no style, no layout, no
//! encode.
//!
//! Where [`HeadlessRuntime`](crate::HeadlessRuntime) is the presentation
//! runtime — a [`HydrolysisRenderer`](crate::HydrolysisRenderer) behind an
//! offscreen surface — `SemanticRuntime` holds only [`SemanticCore`], which by
//! type cannot reach GPU or theme state. Semantic test sessions
//! (`waterui-testing`'s `SemanticApp`) drive this type; pointer, hover and
//! drag interactions do not exist here because they dispatch through laid-out
//! bounds, so the only inputs are keyboard and IME events to the focused node
//! and accessibility actions addressed to nodes directly.

use super::executor::{DrainExecutorOnDrop, HeadlessMainThreadExecutor};
use super::*;
use crate::renderer::{MenuShortcutRegistry, SemanticCore, WindowId};
#[cfg(target_arch = "wasm32")]
use std::sync::Arc;

/// What one semantic pump produced: whether the retained tree was (re)built or
/// re-emitted this pump, the phase timings, and the resulting accessibility
/// tree update plus focused node when the `accessibility` feature is on.
#[derive(Debug)]
pub struct SemanticPumpResult {
    pub rebuilt: bool,
    pub profile: FrameProfile,
    #[cfg(feature = "accessibility")]
    pub tree_update: Option<AccessibilityTreeUpdate>,
    #[cfg(feature = "accessibility")]
    pub ui_focus: Option<accesskit::NodeId>,
}

/// One window in a [`SemanticRuntime`]: the platform [`Window`] (title,
/// state, content builder), the [`SemanticCore`] owning its retained tree,
/// and the input events queued for the next pump.
struct SemanticWindow {
    window: Window,
    core: SemanticCore,
    pending_events: VecDeque<InputEvent>,
    /// Set when state changed outside a pump (an accessibility action, a
    /// signal write from an event) so the next pump re-emits even when no
    /// reactive request reached the core.
    refresh_requested: bool,
}

impl SemanticWindow {
    fn new(window: Window, fonts: &FontCollection, window_id: WindowId) -> Self {
        let mut core = SemanticCore::new(Instant::now());
        core.set_window_id(window_id);
        seed_core(&mut core, fonts);
        #[cfg(feature = "accessibility")]
        {
            core.use_semantic_keyboard_activation();
            core.use_semantic_walk();
        }
        Self {
            window,
            core,
            pending_events: VecDeque::new(),
            refresh_requested: true,
        }
    }

    fn push_event(&mut self, event: InputEvent) {
        self.pending_events.push_back(event);
    }

    fn has_pending_events(&self) -> bool {
        !self.pending_events.is_empty()
    }
}

/// A pump-based runtime with no presentation: it keeps every window's
/// retained widget tree current — dispatch, reactive patching — and emits the
/// accessibility tree from it.
///
/// Construct with [`Self::new`]; pump with [`Self::pump`] or
/// [`Self::pump_at`]. A pump that finds no pending work is cheap and returns
/// `rebuilt: false` with no tree update.
pub struct SemanticRuntime {
    env: Environment,
    window: SemanticWindow,
    pending_window_queue: Rc<RefCell<Vec<Window>>>,
    popup_windows: Vec<SemanticWindow>,
    /// The application's font collection: popup windows' cores are seeded
    /// from it so they shape with the same faces as the main window.
    fonts: FontCollection,
    local_executor: HeadlessMainThreadExecutor,
    /// Declared last so it drops after the runtime state above: consumes any
    /// still-queued spawned work while this thread's locals are intact, so no
    /// runnable is ever dropped during thread-local teardown.
    _executor_teardown: DrainExecutorOnDrop,
}

impl SemanticRuntime {
    /// Creates a semantic runtime over `content` mounted in `env`.
    ///
    /// `width`/`height` set the window's declared frame — window state is
    /// semantic (a view can read `window.frame`), but the emitted tree never
    /// carries geometry, so the size affects no accessibility output.
    #[must_use]
    pub fn new(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
    ) -> Self {
        // A native app loads `resources/fonts` from the filesystem; a browser
        // page has no synchronous resource directory to scan, so the semantic
        // runtime there shapes with the default collection alone.
        #[cfg(not(target_arch = "wasm32"))]
        return Self::on_env(env, content, width, height, native_resource_fonts);
        #[cfg(target_arch = "wasm32")]
        Self::on_env(env, content, width, height, parley::FontContext::new)
    }

    /// Creates a semantic runtime for WaterUI test hosts: the same runtime
    /// [`Self::new`] builds, but text shapes with the bundled deterministic
    /// fonts so caret and selection behaviour is identical on every runner.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn new_for_tests(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
    ) -> Self {
        Self::on_env(
            env,
            content,
            width,
            height,
            super::fonts::deterministic_test_fonts,
        )
    }

    fn on_env(
        env: Environment,
        content: AnyViewBuilder<AnyView>,
        width: u32,
        height: u32,
        build_fonts: fn() -> parley::FontContext,
    ) -> Self {
        // The inspector endpoint is a TCP server a browser page cannot host —
        // on wasm32 the executor installs alone, with no probe to report to.
        #[cfg(not(target_arch = "wasm32"))]
        let inspector = init_main_thread_executors();
        #[cfg(not(target_arch = "wasm32"))]
        let inspector_probe = inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::runtime_probe);
        #[cfg(target_arch = "wasm32")]
        let inspector_probe: Option<Arc<dyn waterui::task::RuntimeProbe>> = {
            init_global_executor();
            None
        };
        let mut env = env.extending(waterui_graphics::SceneViewMergeToParent);
        #[cfg(not(target_arch = "wasm32"))]
        waterui::inspector::install(&mut env, inspector);
        let pending_window_queue = Rc::new(RefCell::new(Vec::new()));
        install_native_component_hooks(&mut env);
        install_headless_window_managers(&mut env, Rc::clone(&pending_window_queue));
        env.insert(HydrolysisTextContextMenuMode::Overlay);
        // Framework tokens only: the semantic runtime has no `Style`, so no
        // style-package tokens ever install — widget structure, roles, labels
        // and actions do not depend on one.
        crate::theme::install_theme_tokens(&mut env, None);
        let fonts = FontCollection::new(build_fonts());
        fonts.clone().install(&mut env);

        // Semantic test binaries have no platform runner to install a tracing
        // subscriber; honor `RUST_LOG` here so they stay debuggable.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .try_init();
        let local_executor = HeadlessMainThreadExecutor::thread_shared();
        // A semantic host paces frames itself, so the executor budgets at the
        // headless rate.
        let _ = try_init_local_executor(waterui::task::monitored_local_executor_with_probes(
            local_executor.clone(),
            waterui::task::RefreshRate::HEADLESS,
            inspector_probe,
        ));

        let content_builder = content.clone();
        let window = Window::new(
            "",
            waterui_core::binding(waterui::window::WindowState::Normal),
            move || content_builder.build(),
        );
        window.frame.set(waterui_core::layout::Rect::new(
            waterui_core::layout::Point::zero(),
            waterui_core::layout::Size::new(width.max(1) as f32, height.max(1) as f32),
        ));

        let window_id = env
            .get::<MenuShortcutRegistry>()
            .expect("install_headless_window_managers seeds MenuShortcutRegistry")
            .mint_window_id();
        Self {
            env,
            window: SemanticWindow::new(window, &fonts, window_id),
            pending_window_queue,
            popup_windows: Vec::new(),
            fonts,
            _executor_teardown: DrainExecutorOnDrop(local_executor.clone()),
            local_executor,
        }
    }

    /// Queues an input event for the next pump.
    ///
    /// Only keyboard and IME input has a semantic target — the focused node —
    /// so those dispatch on the pump; pointer, scroll and gesture events are
    /// geometry-routed and are dropped by the semantic pump (a test that
    /// needs them drives the rendered [`crate::HeadlessRuntime`]).
    pub fn push_input_event(&mut self, event: InputEvent) {
        self.window.push_event(event);
    }

    /// Requests a re-emit on the next pump, as a platform's redraw request
    /// would.
    pub fn request_redraw(&mut self) {
        self.window.refresh_requested = true;
    }

    /// Performs an accessibility action against the merged tree.
    ///
    /// Popup-window node ids are shifted into their own stride in the merged
    /// update (see [`Self::take_merged_accessibility_tree_update`]), so a
    /// request whose target lands in a popup's range demuxes back to that
    /// window's core — the action targets (a menu item's activation, a picker
    /// row's selection) live there. Returns whether the action changed state,
    /// in which case the next pump re-emits.
    #[cfg(feature = "accessibility")]
    pub fn perform_accessibility_action(&mut self, request: AccessibilityActionRequest) -> bool {
        /// The same id range [`Self::take_merged_accessibility_tree_update`]
        /// assigns each popup.
        const WINDOW_ID_STRIDE: u64 = 1 << 32;

        let target = request.target_node.0;
        let (window, request) = if target >= WINDOW_ID_STRIDE {
            let index = target / WINDOW_ID_STRIDE - 1;
            let popup = self
                .popup_windows
                .get_mut(index as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "hydrolysis semantic runtime: accessibility action {:?} targets closed popup \
                         node {target}",
                        request.action
                    )
                });
            let mut request = request;
            request.target_node = accesskit::NodeId(target % WINDOW_ID_STRIDE);
            (popup, request)
        } else {
            (&mut self.window, request)
        };
        let action_env = self.env.extending(semantic_window_origin(window));
        let changed = window
            .core
            .handle_accessibility_action(request, &action_env);
        if changed {
            window.refresh_requested = true;
        }
        changed
    }

    /// Whether the runtime is quiescent: no queued input, no spawned work
    /// awaiting a drain, no pending popup mounts, no re-emit request, and no
    /// core-scheduled semantic work (patches, rebuilds, animations, gesture
    /// deadlines, gliding scrolls) in this window or any popup.
    ///
    /// Visual-only repaint requests (caret blink) do not count: they never
    /// move semantic state.
    #[must_use]
    pub fn is_settled(&self) -> bool {
        !self.window.has_pending_events()
            && !self.window.refresh_requested
            && !self.local_executor.has_pending()
            && self.pending_window_queue.borrow().is_empty()
            && !self.window.core.has_scheduled_semantic_work()
            && self.popup_windows.iter().all(|popup| {
                !popup.has_pending_events()
                    && !popup.refresh_requested
                    && !popup.core.has_scheduled_semantic_work()
            })
    }

    /// Whether a state change has been requested but not yet emitted, so the
    /// accessibility tree the last pump produced is stale.
    #[must_use]
    pub fn has_pending_semantic_update(&self) -> bool {
        self.window.refresh_requested
            || self.window.core.has_pending_semantic_update()
            || self
                .popup_windows
                .iter()
                .any(|popup| popup.refresh_requested || popup.core.has_pending_semantic_update())
    }

    /// Drops UI focus, if any text input holds it.
    #[cfg(feature = "accessibility")]
    pub fn clear_ui_focus(&mut self) -> bool {
        let changed = self.window.core.clear_ui_focus();
        if changed {
            self.window.refresh_requested = true;
        }
        changed
    }

    /// The accessibility node id of the focused text input, if one is focused.
    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn focused_ui_node(&self) -> Option<accesskit::NodeId> {
        self.window.core.focused_ui_node()
    }

    /// Pumps one semantic frame at the current instant.
    pub fn pump(&mut self) -> SemanticPumpResult {
        self.pump_at(Instant::now())
    }

    /// Pumps one semantic frame at `at`.
    ///
    /// Drains spawned work, applies queued input, advances animations and
    /// gesture deadlines to `at`, mounts pending popup windows, then — per
    /// window — builds the retained tree on the first pump or patches and
    /// re-emits it on later ones. The returned [`SemanticPumpResult`] carries
    /// the merged accessibility tree update of every open window.
    pub fn pump_at(&mut self, at: Instant) -> SemanticPumpResult {
        let frame_started_at = Instant::now();
        let executor_before_started_at = Instant::now();
        let drained_before = self.local_executor.drain();
        let executor_before = executor_before_started_at.elapsed();
        let input_started_at = Instant::now();
        let _ = handle_semantic_input_events(&mut self.window, &self.env);
        let input = input_started_at.elapsed();
        let animation_started_at = Instant::now();
        advance_semantic_window(&mut self.window, &self.env, at);
        let animation = animation_started_at.elapsed();
        self.mount_pending_popup_windows();
        for popup in &mut self.popup_windows {
            let _ = handle_semantic_input_events(popup, &self.env);
            advance_semantic_window(popup, &self.env, at);
        }
        // A popup whose state flipped `Closed` — its menu group dismissed it
        // or a close request arrived — leaves the tree. Flag the main window
        // so the merged update re-emits without it.
        if self
            .popup_windows
            .iter()
            .any(|popup| popup.window.state.snapshot() == waterui::window::WindowState::Closed)
        {
            self.popup_windows.retain(|popup| {
                popup.window.state.snapshot() != waterui::window::WindowState::Closed
            });
            self.window.refresh_requested = true;
        }
        let rebuilt = pump_semantic_window(&mut self.window, &self.env);
        for popup in &mut self.popup_windows {
            let _ = pump_semantic_window(popup, &self.env);
        }
        let executor_after_started_at = Instant::now();
        let drained_after = self.local_executor.drain();
        let executor_after = executor_after_started_at.elapsed();

        SemanticPumpResult {
            rebuilt: rebuilt || drained_before || drained_after,
            profile: FrameProfile {
                phases: FramePhases {
                    executor_before,
                    input,
                    animation,
                    executor_after,
                    ..FramePhases::default()
                },
                ..FrameProfile::default()
            }
            .with_total(frame_started_at.elapsed()),
            #[cfg(feature = "accessibility")]
            tree_update: self.take_merged_accessibility_tree_update(),
            #[cfg(feature = "accessibility")]
            ui_focus: self.window.core.focused_ui_node(),
        }
    }

    fn mount_pending_popup_windows(&mut self) {
        let pending = self
            .pending_window_queue
            .borrow_mut()
            .drain(..)
            .collect::<Vec<_>>();
        for window in pending {
            let fonts = self.fonts.clone();
            let window_id = self
                .env
                .get::<MenuShortcutRegistry>()
                .expect("install_headless_window_managers seeds MenuShortcutRegistry")
                .mint_window_id();
            self.popup_windows
                .push(SemanticWindow::new(window, &fonts, window_id));
        }
    }

    /// The accessibility tree of every window this application has open,
    /// merged by [`SemanticCore::take_merged_accessibility_tree_update`]:
    /// each popup's ids are shifted into their own range and its root attached
    /// to the main root so the result is one tree — the same merge the
    /// rendered [`HeadlessRuntime`](crate::HeadlessRuntime) applies.
    #[cfg(feature = "accessibility")]
    fn take_merged_accessibility_tree_update(&mut self) -> Option<AccessibilityTreeUpdate> {
        self.window.core.take_merged_accessibility_tree_update(
            self.popup_windows.iter_mut().map(|popup| &mut popup.core),
        )
    }
}

/// The window's origin as an environment value — the `input_env` the rendered
/// runner builds around each event. Kept identical so key dispatch resolves
/// the same environment in both runtimes.
fn semantic_window_origin(window: &SemanticWindow) -> HydrolysisWindowOrigin {
    HydrolysisWindowOrigin {
        x: window.window.frame.snapshot().x(),
        y: window.window.frame.snapshot().y(),
    }
}

/// Applies one window's queued input events. Keyboard and IME events dispatch
/// to the focused node through the core's key/text paths; geometry-routed
/// events have no semantic target and are dropped.
fn handle_semantic_input_events(window: &mut SemanticWindow, env: &Environment) -> bool {
    let mut should_close = window.window.state.snapshot() == waterui::window::WindowState::Closed;
    let events: Vec<InputEvent> = window.pending_events.drain(..).collect();
    // Same ordered IME keystroke ownership as the rendered runner
    // (`ime::ime_owned_events` tracks the composition through the batch).
    let ime_owned = ime::ime_owned_events(&events, window.core.ime_composition_active());
    for (event, ime_owned) in events.into_iter().zip(ime_owned) {
        let changed = match event {
            InputEvent::CloseRequested => {
                window
                    .window
                    .state
                    .set(waterui::window::WindowState::Closed);
                should_close = true;
                true
            }
            InputEvent::Moved { x, y } => {
                let frame = window.window.frame.snapshot();
                window.window.frame.set(waterui_core::layout::Rect::new(
                    waterui_core::layout::Point::new(x, y),
                    *frame.size(),
                ));
                false
            }
            InputEvent::Resize { width, height } => {
                let frame = window.window.frame.snapshot();
                window.window.frame.set(waterui_core::layout::Rect::new(
                    frame.origin(),
                    waterui_core::layout::Size::new(width as f32, height as f32),
                ));
                false
            }
            InputEvent::TextInput { text } => {
                !ime_owned && window.core.handle_text_input(text.as_str())
            }
            InputEvent::Key {
                key,
                physical_code,
                state: KeyState::Pressed,
                modifiers,
                ..
            } => {
                if ime_owned {
                    // Same swallowed-press tracking as the rendered runner:
                    // the release can arrive in a later batch, after the
                    // commit that ended the composition.
                    window.core.swallow_ime_key_press(physical_code);
                    false
                } else {
                    let key_env = env.extending(semantic_window_origin(window));
                    window.core.handle_key_with_env(&key, modifiers, &key_env)
                }
            }
            InputEvent::Key {
                key,
                physical_code,
                state: KeyState::Released,
                ..
            } => {
                let key_env = env.extending(semantic_window_origin(window));
                !window.core.take_ime_swallowed_release(physical_code)
                    && !ime_owned
                    && window.core.handle_key_release_with_env(&key, &key_env)
            }
            InputEvent::ImePreedit { text, caret } => {
                window.core.handle_ime_preedit(text.as_str(), caret)
            }
            InputEvent::ImeCommit { text } => window.core.handle_ime_commit(text.as_str()),
            InputEvent::ImeDisabled => window.core.handle_ime_disabled(),
            InputEvent::Focused(focused) => window.core.handle_window_focused(focused),
            InputEvent::KeyboardCancel => window.core.cancel_keyboard_press(),
            InputEvent::ModifiersChanged(modifiers) => {
                // Modifier state is input context, not geometry: keep it so
                // semantic actions (e.g. a row's Select click) observe the
                // same held modifiers a rendered pump would.
                window.core.update_embedded_modifiers(modifiers);
                false
            }
            geometric => {
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    event = ?geometric,
                    "semantic pump dropped geometry-routed input event"
                );
                false
            }
        };
        if changed {
            window.refresh_requested = true;
        }
    }
    should_close
}

/// Advances one window's time-driven state to `now`: the frame clock, armed
/// gesture deadlines, smoothed scrolls and animations — everything a rendered
/// advance does that does not need a scene. Reactive patch and rebuild
/// requests raised along the way become the window's refresh flag.
fn advance_semantic_window(window: &mut SemanticWindow, env: &Environment, now: Instant) {
    window.core.set_frame_instant(now);
    if window.core.handle_gesture_tick(now, env) {
        window.refresh_requested = true;
    }
    if window.core.tick_smooth_scrolls(now) {
        window.refresh_requested = true;
    }
    let _animations_active = window.core.advance_animations();
    if window.core.take_patch_request() {
        window.refresh_requested = true;
    }
    if window.core.take_rebuild_request() || window.core.take_next_frame_rebuild_request() {
        window.refresh_requested = true;
    }
}

/// One pump of a window's semantic pass: builds the retained tree from the
/// window's `body()` when none exists, otherwise patches and re-emits it when
/// work is pending. Returns whether the tree was emitted this pump.
fn pump_semantic_window(window: &mut SemanticWindow, env: &Environment) -> bool {
    // The rendered pump subscribes to the window's frame/state signals every
    // frame (`render_window_with_capture`): the semantic pump holds the same
    // subscriptions so a `frame`/`state` change re-emits here too, and so their
    // watch guards roll over through `signal_watches` in the same teardown
    // order the renderer releases them in (water-rs/waterui#1213).
    let _ = window.core.read_signal(&window.window.frame);
    let _ = window.core.read_signal(&window.window.state);
    #[cfg(feature = "accessibility")]
    window
        .core
        .set_accessibility_root_label(window.window.title.snapshot().as_str());

    if window.core.take_rebuild_request() {
        window.refresh_requested = true;
    }
    let work_pending = window.refresh_requested
        || window.core.has_patch_request()
        || window.core.take_redraw_request();
    if !work_pending {
        return false;
    }
    if window.core.has_render_tree() {
        assert!(
            window.core.flush_window_semantics(env),
            "hydrolysis semantic runtime: retained window tree vanished during pump"
        );
        window.refresh_requested = false;
        return true;
    }
    let content = window.window.build_content();
    window.core.capture_window_semantics(content, env);
    window.refresh_requested = false;
    true
}
#[cfg(all(test, feature = "accessibility"))]
mod tests {
    //! End-to-end semantic-runtime tests: the retained tree emits the
    //! accesskit tree with no GPU, no style, no layout and no encode, and
    //! accessibility actions drive widget state directly.
    //!
    //! These cannot run until `waterui-testing` (a dev-dependency every test
    //! target links) carries the `waterui#1125` driver side; the identical
    //! flow is verified against the public API in an external scratch crate.

    use super::*;
    use accesskit::{Action, ActionRequest, Node as AccessibilityNode, NodeId, Role, TreeId};
    use nami::Binding;
    use waterui::ViewExt as _;
    use waterui_controls::button::button;
    use waterui_controls::menu::{CommandExt, Menu, MenuItem};
    use waterui_core::handler::AnyViewBuilder;
    use waterui_layout::stack::vstack;
    use waterui_text::text;

    fn semantic_environment() -> Environment {
        let mut env = Environment::new();
        crate::testing::install_theme(&mut env);
        crate::localization::install(&mut env);
        env
    }

    fn find_by_label<'a>(
        update: &'a AccessibilityTreeUpdate,
        role: Role,
        label: &str,
    ) -> Option<(NodeId, &'a AccessibilityNode)> {
        update.nodes.iter().find_map(|(id, node)| {
            (node.role() == role && node.label().is_some_and(|text| text == label))
                .then_some((*id, node))
        })
    }

    fn click(runtime: &mut SemanticRuntime, target: NodeId) -> bool {
        runtime.perform_accessibility_action(ActionRequest {
            action: Action::Click,
            target_node: target,
            target_tree: TreeId::ROOT,
            data: None,
        })
    }

    /// Pumps until the runtime settles and returns the last tree update it
    /// emitted — `None` when nothing changed since the previous emit (a
    /// settled pump produces no update).
    fn pump_until_settled(runtime: &mut SemanticRuntime) -> Option<AccessibilityTreeUpdate> {
        let mut last = None;
        for _ in 0..64 {
            let result = runtime.pump();
            if let Some(update) = result.tree_update {
                last = Some(update);
            }
            if runtime.is_settled() {
                break;
            }
        }
        assert!(
            !runtime.has_pending_semantic_update(),
            "semantic runtime never settled"
        );
        last
    }

    #[test]
    fn semantic_pump_emits_widgets_and_click_activates() {
        let fired = Binding::container(false);
        let fired_for_button = fired.clone();
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            let fired = fired_for_button.clone();
            AnyView::new(vstack((
                text("hello semantic"),
                button("Tap").action(move || fired.set(true)),
            )))
        });
        let mut runtime = SemanticRuntime::new(semantic_environment(), builder, 800, 600);

        let update =
            pump_until_settled(&mut runtime).expect("the initial pump emitted no tree update");
        let (tap, tap_node) = find_by_label(&update, Role::Button, "Tap")
            .expect("Tap button missing from the semantic tree");
        assert!(
            tap_node.supports_action(Action::Click),
            "the Tap button must advertise Click"
        );
        assert!(
            update.nodes.iter().any(|(_, node)| {
                node.role() == Role::Label && node.label().is_some_and(|v| v == "hello semantic")
            }),
            "the text view must emit a Label node"
        );

        assert!(click(&mut runtime, tap), "Tap click changed nothing");
        assert!(fired.snapshot(), "the button action did not fire");
    }

    #[test]
    fn semantic_menu_popup_emits_items_and_dismisses() {
        let fired = Binding::container(String::new());
        let fired_for_menu = fired.clone();
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            let save = fired_for_menu.clone();
            AnyView::new(vstack((Menu::new(
                "File",
                vec![
                    MenuItem::Command("Save".action(move || save.set("save".to_string()))),
                    MenuItem::Divider,
                    MenuItem::Command("Quit".action(|| {})),
                ],
            ),)))
        });
        let mut runtime = SemanticRuntime::new(semantic_environment(), builder, 800, 600);

        let update =
            pump_until_settled(&mut runtime).expect("the initial pump emitted no tree update");
        let (menu, menu_node) = find_by_label(&update, Role::Button, "File")
            .expect("the File menu trigger is missing from the semantic tree");
        assert!(menu_node.supports_action(Action::Click));
        assert!(
            find_by_label(&update, Role::Button, "Save").is_none(),
            "menu items must not appear before the menu opens"
        );

        // Activation mounts the popup as a semantic window: its items land in
        // the merged tree at root level.
        assert!(click(&mut runtime, menu), "menu activation changed nothing");
        let update =
            pump_until_settled(&mut runtime).expect("opening the menu emitted no tree update");
        let (save, _) = find_by_label(&update, Role::Button, "Save")
            .expect("Save menu item missing after the menu opened");
        assert!(
            find_by_label(&update, Role::Button, "Quit").is_some(),
            "Quit menu item missing after the menu opened"
        );

        // The item's action lives in the popup window's own core — the merged
        // id demuxes to it — and its `close_all` dismisses the whole group.
        assert!(click(&mut runtime, save), "Save click changed nothing");
        assert_eq!(
            fired.snapshot().as_str(),
            "save",
            "menu item action did not fire"
        );
        let update =
            pump_until_settled(&mut runtime).expect("dismissing the menu emitted no tree update");
        assert!(
            find_by_label(&update, Role::Button, "Save").is_none(),
            "a dismissed menu must leave the merged tree"
        );
    }

    /// water-rs/hydrolysis#140: a popup runs in the environment of the view
    /// that opened it, so `.state(&store)` on an ancestor reaches the item
    /// action's extractors.
    #[test]
    fn semantic_menu_item_action_reads_state_inherited_from_the_opening_view() {
        #[waterui::prelude::state]
        #[derive(Clone)]
        struct Store {
            hits: Binding<u32>,
        }

        let store = Store {
            hits: Binding::container(0),
        };
        let store_for_view = store.clone();
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            let store = store_for_view.clone();
            AnyView::new(
                vstack((Menu::new(
                    "File",
                    vec![MenuItem::Command("Bump".action(|store: Store| {
                        store.hits.set(store.hits.snapshot() + 1);
                    }))],
                ),))
                .state(&store),
            )
        });
        let mut runtime = SemanticRuntime::new(semantic_environment(), builder, 800, 600);

        let update =
            pump_until_settled(&mut runtime).expect("the initial pump emitted no tree update");
        let (menu, _) = find_by_label(&update, Role::Button, "File")
            .expect("the File menu trigger is missing from the semantic tree");
        assert!(click(&mut runtime, menu), "menu activation changed nothing");

        let update =
            pump_until_settled(&mut runtime).expect("opening the menu emitted no tree update");
        let (bump, _) = find_by_label(&update, Role::Button, "Bump")
            .expect("Bump menu item missing after the menu opened");
        assert!(click(&mut runtime, bump), "Bump click changed nothing");
        assert_eq!(
            store.hits.snapshot(),
            1,
            "the item action did not reach the injected store"
        );
    }

    #[test]
    fn semantic_text_field_focuses_and_edits() {
        let value = Binding::container(waterui_core::Str::default());
        let value_for_field = value.clone();
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(vstack((waterui_controls::text_field::field(
                "Name",
                &value_for_field,
            ),)))
        });
        let mut runtime = SemanticRuntime::new(semantic_environment(), builder, 800, 600);

        let update =
            pump_until_settled(&mut runtime).expect("the initial pump emitted no tree update");
        let (field, _) = update
            .nodes
            .iter()
            .find(|(_, node)| node.role() == Role::TextInput)
            .map(|(id, node)| (*id, node))
            .expect("the text field is missing from the semantic tree");

        // Focus routes the input target; key events then edit through it —
        // all with zero geometry.
        assert!(
            runtime.perform_accessibility_action(ActionRequest {
                action: Action::Focus,
                target_node: field,
                target_tree: TreeId::ROOT,
                data: None,
            }),
            "focusing the text field changed nothing"
        );
        let _ = pump_until_settled(&mut runtime).expect("focusing emitted no tree update");
        assert_eq!(
            runtime.focused_ui_node(),
            Some(field),
            "the text field did not take UI focus"
        );

        for ch in "Jo".chars() {
            runtime.push_input_event(InputEvent::TextInput {
                text: ch.to_string(),
            });
        }
        let _ = pump_until_settled(&mut runtime).expect("text input emitted no tree update");
        assert_eq!(
            value.snapshot().to_string().as_str(),
            "Jo",
            "text input did not edit the field"
        );
    }
}
