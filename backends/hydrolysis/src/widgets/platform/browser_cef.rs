use std::cell::RefCell;
use std::rc::Rc;

use waterui_browser_cef::{CefPageHandle, CefViewport, gpu_view_with_input};
use waterui_core::Environment;
use waterui_graphics::gpu_surface::GpuSurface;

use crate::renderer::{
    EmbeddedGpuSurfaceRuntime, GpuSurfaceSource, HydrolysisRenderer, transformed_rect,
};

pub(crate) struct CefSurfaceRenderState {
    gpu: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>,
    viewport: CefViewport,
}

#[cfg(not(test))]
pub(crate) fn install_runtime(env: &mut Environment) {
    let runtime = env
        .get::<waterui_browser_cef::CefRuntime>()
        .cloned()
        .unwrap_or_else(|| {
            let runtime = waterui_browser_cef::CefRuntime::initialize(
                waterui_browser_cef::CefRuntimeConfiguration::packaged(),
            );
            env.insert(runtime.clone());
            runtime
        });
    #[cfg(hydrolysis_cef_webview)]
    env.insert(runtime.webview_controller());
    #[cfg(feature = "chromium")]
    env.insert(runtime.chromium_controller());
}

impl CefSurfaceRenderState {
    pub(crate) fn new(page: CefPageHandle, env: &Environment) -> Self {
        let viewport = CefViewport::new();
        // The CEF view takes its own input: it reports `wants_input_events`,
        // so the renderer routes what lands on this layer straight into the
        // engine crate's adapter and Hydrolysis owns no Chromium semantics.
        let surface = GpuSurface::new(gpu_view_with_input(page, viewport.clone()));
        Self {
            gpu: Rc::new(RefCell::new(EmbeddedGpuSurfaceRuntime::new(surface, env))),
            viewport,
        }
    }

    pub(crate) fn prebuild(&self, renderer: &mut HydrolysisRenderer) {
        renderer.register_node_gpu_surface(Rc::clone(&self.gpu));
    }

    pub(crate) fn render(
        &self,
        renderer: &mut HydrolysisRenderer,
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) {
        self.viewport
            .set_scale(transform.determinant().abs().sqrt());
        renderer.register_gpu_surface_input_target(bounds, hit_transform, Rc::clone(&self.gpu));
        renderer.push_gpu_surface_layer(
            GpuSurfaceSource::Owned(Rc::clone(&self.gpu)),
            transform,
            bounds,
            transformed_rect(hit_transform, bounds),
        );
    }
}
