//! A user shader as a view: a Cherenkov shader paint filling the view.
//!
//! The fragment is WGSL against the engine's prelude — `uniforms.time`,
//! `uniforms.resolution`, a `uv` in `[0, 1]` and `@fragment fn main` — with
//! user uniforms passed as a flat `f32` list that follows a signal.

extern crate alloc;

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt;

use cherenkov::kurbo::Rect;
use cherenkov::{Draw, Shader, ShaderPaint, ShaderSource};
use nami::{Computed, SignalExt};
use waterui_core::layout::StretchAxis;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, View};

use crate::scene::resources::Scene;
use crate::scene_view::{SceneContent, SceneView};

/// A view painted by a WGSL fragment shader.
///
/// # Layout Behavior
///
/// Stretches on both axes; constrain it with `.frame()`.
pub struct ShaderPaintView {
    source: ShaderSource,
    uniforms: Computed<Vec<f32>>,
}

impl fmt::Debug for ShaderPaintView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShaderPaintView")
            .field("animated", &self.source.animated)
            .finish_non_exhaustive()
    }
}

impl ShaderPaintView {
    /// A static shader from its fragment body.
    #[must_use]
    pub fn new(fragment: impl Into<Cow<'static, str>>) -> Self {
        Self::from_source(ShaderSource::wgsl(fragment))
    }

    /// A shader from a prepared source.
    #[must_use]
    pub fn from_source(source: ShaderSource) -> Self {
        Self {
            source,
            uniforms: nami::constant(Vec::new()).computed(),
        }
    }

    /// The bundled flowing gradient: noise-driven colour bands that drift
    /// with `uniforms.time`, entirely on the GPU.
    #[must_use]
    pub fn flowing_gradient() -> Self {
        Self::new(include_str!("shaders/flowing_gradient.wgsl")).animated()
    }

    /// Re-renders the shader every frame so `uniforms.time` advances.
    #[must_use]
    pub fn animated(mut self) -> Self {
        self.source = self.source.animated();
        self
    }

    /// The user uniforms the fragment reads, following a signal.
    #[must_use]
    pub fn uniforms(mut self, uniforms: impl IntoComputed<Vec<f32>>) -> Self {
        self.uniforms = uniforms.into_computed();
        self
    }
}

impl View for ShaderPaintView {
    fn body(self, _env: &Environment) -> impl View {
        SceneView::new(ShaderContent {
            source: self.source,
            uniforms: self.uniforms,
            shader: None,
        })
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

struct ShaderContent {
    source: ShaderSource,
    uniforms: Computed<Vec<f32>>,
    shader: Option<Shader>,
}

impl SceneContent for ShaderContent {
    fn record(&mut self, scene: &mut Scene<'_>) -> bool {
        let shader = if let Some(shader) = &self.shader {
            shader.clone()
        } else {
            let shader = scene
                .resources()
                .shader(self.source.clone())
                .unwrap_or_else(|error| panic!("shader paint: {error}"));
            self.shader = Some(shader.clone());
            shader
        };
        let id = shader.id();
        let paint = self.uniforms.map(move |uniforms| ShaderPaint {
            shader: id,
            uniforms,
        });
        let bounds = Rect::new(
            0.0,
            0.0,
            f64::from(scene.width()),
            f64::from(scene.height()),
        );
        scene.recorder().fill(bounds, paint);
        false
    }
}

/// A [`ShaderPaintView`] from a `.wgsl` file beside the calling source file.
#[macro_export]
macro_rules! shader {
    ($path:literal) => {
        $crate::shader_paint::ShaderPaintView::new(include_str!($path))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_core::AnyView;

    #[test]
    fn a_shader_view_is_a_stretching_scene_view() {
        let view = ShaderPaintView::new(
            "@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(uv, 0.0, 1.0); }",
        );
        assert_eq!(view.stretch_axis(), StretchAxis::Both);
        let body = AnyView::new(view.body(&Environment::new()));
        assert!(body.downcast::<SceneView>().is_ok());
    }
}
