//! The HWUI window: the session's `RenderNode` painter peer, mounting the
//! runtime's scene into the shared `HwuiSurface` and replaying each frame
//! through one JNI call.

use std::rc::Rc;

use crate::theme::WidgetTheme;
use jni::objects::JObject;
use waterui::Environment;
use waterui_graphics::draw::SurfaceId;

use super::jni::JniError;
use crate::hwui::{HwuiSurface, JniPainter};
use crate::platform::PlatformWindow as _;
use crate::renderer::HydrolysisRenderer;
use crate::renderer::mount::Mount;
use crate::renderer::mount::hwui::HwuiHost;
use crate::renderer::recording;
use crate::runner::window::{
    RuntimeWindow, SurfaceBackground, pump_window_scene, sync_platform_input,
};

use super::AndroidTextEngine;
use super::host::AndroidHostWindow;

/// The session's HWUI painter: the retained surface the mount commits, and
/// the JNI client each frame replays into.
pub(super) struct HwuiWindow {
    pub(super) surface: HwuiSurface,
    host: HwuiHost,
    mount: Mount<crate::hwui::HwuiTarget, AndroidTextEngine>,
    painter: JniPainter,
    /// Scratch for the frame's platform-view bindings — reused, never
    /// reallocated while the binding count fits.
    bindings: Vec<crate::hwui::PlatformViewBinding>,
}

impl HwuiWindow {
    /// The window peer of Kotlin's `HwuiPainter` at `peer`: creates the
    /// surface, the platform text engine over the painter's text provider,
    /// the resource tables over its sink, and the renderer + mount pair.
    ///
    /// # Errors
    ///
    /// A description when `peer` is null, any JNI lookup fails, or the
    /// device's API level cannot encode the root node.
    pub(super) fn new(
        env: &mut jni::Env<'_>,
        peer: &JObject<'_>,
        id: SurfaceId,
        theme: Rc<dyn WidgetTheme>,
        wake: Rc<dyn Fn()>,
    ) -> Result<(Self, HydrolysisRenderer<AndroidTextEngine>), JniError> {
        if peer.is_null() {
            return Err(JniError(
                "hydrolysis HWUI: nativeCreateSession got a null HwuiPainter".to_owned(),
            ));
        }
        let api = env
            .get_static_field(
                jni::jni_str!("android/os/Build$VERSION"),
                jni::jni_str!("SDK_INT"),
                jni::jni_sig!("I"),
            )?
            .i()?;
        let api = u32::try_from(api).expect("Build.VERSION.SDK_INT is never negative");
        let text_object = env
            .call_method(
                peer,
                jni::jni_str!("getText"),
                jni::jni_sig!("()Ldev/waterui/hydrolysis/hwui/HwuiTextProvider;"),
                &[],
            )?
            .l()?;
        let sink_object = env
            .call_method(
                peer,
                jni::jni_str!("getSink"),
                jni::jni_sig!("()Ldev/waterui/hydrolysis/hwui/RenderNodeSink;"),
                &[],
            )?
            .l()?;

        let surface =
            HwuiSurface::new(id, api, move || wake()).map_err(|e| JniError(e.to_string()))?;
        // The window starts parked: `set_host_attached` and metrics unhide
        // it the way the GPU band's surface attach does.
        surface.set_hidden(true);
        let provider = crate::hwui::text::JniTextProvider::new(env, &text_object)
            .map_err(|e| JniError(e.to_string()))?;
        let text = crate::text::HwuiTextEngine::new(
            std::sync::Arc::new(provider),
            surface.encoder().text_layouts().clone(),
            crate::text::FontFamilyResolution::Lenient,
        );
        let renderer = HydrolysisRenderer::with_engine(theme, text);
        let resources = Rc::new(
            crate::hwui::HwuiResources::over_jni(env, &sink_object, surface.encoder())
                .map_err(JniError)?,
        );
        let painter = JniPainter::new(env, peer).map_err(JniError)?;
        let (mount, host) = renderer.mount_hwui(&surface, resources);
        Ok((
            Self {
                surface,
                host,
                mount,
                painter,
                bindings: Vec::new(),
            },
            renderer,
        ))
    }

    /// One frame: layout and record the scene, commit it through the mount,
    /// encode and replay the frame, then sync input state back. Returns
    /// `true` — the frame reached the painter.
    ///
    /// # Errors
    ///
    /// A description when encoding or the JNI replay fails.
    pub(super) fn render(
        &mut self,
        runtime: &mut RuntimeWindow<AndroidHostWindow, AndroidTextEngine>,
        env: &Environment,
        drain: &mut dyn FnMut() -> bool,
    ) -> Result<bool, JniError> {
        runtime.platform.apply_properties(&runtime.window);
        let background = SurfaceBackground::of(&runtime.window, env);
        assert!(
            background.behind_level().is_none(),
            "hydrolysis HWUI: behind-window material is unsupported \
             (water-rs/waterui#1899)"
        );
        runtime.platform.set_transparent(background.transparent());
        runtime.renderer.set_window_backdrop(background.backdrop());
        pump_window_scene(runtime, env, drain);
        self.surface
            .shared()
            .borrow_mut()
            .set_clear(recording::working_color(background.clear(env)));
        let scale = runtime.platform.scale_factor();
        runtime
            .renderer
            .commit_mount(&mut self.mount, &self.host, scale);
        runtime.renderer.platform_view_bindings(&mut self.bindings);
        self.surface.set_platform_views(&self.bindings);
        let frame = self
            .surface
            .frame(cherenkov_record::Instant::now(), scale)
            .map_err(|e| JniError(e.to_string()))?;
        let animating = frame.content_animating() || frame.sampling().rate.is_some();
        self.painter
            .replay(frame.words(), frame.clear())
            .map_err(JniError)?;
        sync_platform_input(runtime);
        if runtime.renderer.take_redraw_request() || animating || self.surface.has_pending() {
            runtime.request_redraw();
        }
        runtime.presented_frames += 1;
        Ok(true)
    }
}
