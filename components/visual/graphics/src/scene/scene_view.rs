use alloc::boxed::Box;
use alloc::rc::Rc;
use core::fmt;

use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View};

#[cfg(feature = "gpu")]
use crate::gpu_surface::GpuSurface;
#[cfg(feature = "gpu")]
use crate::scene::scene_surface::SceneSurfaceRenderer;
use crate::scene2d::Scene2D;

/// Environment marker: render `SceneView` directly in the backend scene.
#[derive(Debug, Clone, Copy, Default)]
pub struct SceneViewMergeToParent;

/// Callback used by scene content to request another frame.
pub type SceneInvalidator = Rc<dyn Fn()>;

/// Object-safe scene producer for `SceneView`.
pub trait SceneContent: 'static {
    /// Build commands into the provided scene.
    ///
    /// Returns true when the content requires another frame to be rendered.
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool;

    /// Installs an invalidation callback that content can trigger from signal watchers.
    fn set_invalidator(&mut self, _invalidator: Option<SceneInvalidator>) {}
}

/// A view that renders scene content either directly (backend) or via `GpuSurface`.
pub struct SceneView {
    content: Box<dyn SceneContent>,
}

impl fmt::Debug for SceneView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneView").finish_non_exhaustive()
    }
}

impl SceneView {
    /// Creates a scene view from object-safe scene content.
    #[must_use]
    pub fn new<C: SceneContent>(content: C) -> Self {
        Self {
            content: Box::new(content),
        }
    }

    /// Returns mutable access to the inner scene content.
    #[must_use]
    pub fn content_mut(&mut self) -> &mut dyn SceneContent {
        &mut *self.content
    }

    /// Takes ownership of the wrapped scene content.
    #[must_use]
    pub fn into_content(self) -> Box<dyn SceneContent> {
        self.content
    }

    /// Converts this scene directly into a GPU surface.
    ///
    /// This is primarily useful for offscreen rendering and visual tests. Normal
    /// view composition should return `SceneView` so a self-drawn backend can
    /// merge its commands directly into the parent scene.
    #[cfg(feature = "gpu")]
    #[must_use]
    pub fn into_gpu_surface(self) -> GpuSurface {
        GpuSurface::new(SceneSurfaceRenderer::new(self.content))
    }
}

impl NativeView for SceneView {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl View for SceneView {
    fn body(self, env: &Environment) -> impl View {
        if env.get::<SceneViewMergeToParent>().is_some() {
            return AnyView::new(Native::new(self));
        }
        #[cfg(feature = "gpu")]
        {
            AnyView::new(self.into_gpu_surface())
        }
        // Without a GPU surface to fall back on there is nowhere left to draw:
        // a scene either merges into a backend's own scene or rasterizes into a
        // surface of its own, and neither is available here.
        #[cfg(not(feature = "gpu"))]
        {
            panic!(
                "a SceneView has no way to render: the backend did not install \
                 `SceneViewMergeToParent`, and `waterui-graphics` was built \
                 without the `gpu` feature that provides the GpuSurface path"
            );
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}
