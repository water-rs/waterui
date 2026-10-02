//! A user shader as a view: a Cherenkov shader paint filling the view.
//!
//! The fragment is WGSL against the engine's prelude — `uniforms.time`,
//! `uniforms.resolution`, a `uv` in `[0, 1]` and `@fragment fn main` — with
//! user uniforms passed as a flat `f32` list that follows a signal.
//!
//! Whether the shader animates follows a signal too: while it is `true` the
//! engine re-renders the shader every frame so `uniforms.time` advances, and
//! while it is `false` the shader renders once per change of its uniforms or
//! size and the engine stays idle.

extern crate alloc;

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt;

use cherenkov::kurbo::Rect;
use cherenkov::{Draw, Recorder, Shader, ShaderPaint, ShaderSource};
use nami::watcher::BoxWatcherGuard;
use nami::{Computed, Signal, SignalExt};
use waterui_core::layout::StretchAxis;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, View};

use crate::scene::resources::{RecordingResources, Registered};
use crate::scene_view::{SceneContent, SceneInvalidator, SceneView, invalidate_on_change};

/// A view painted by a WGSL fragment shader.
///
/// # Layout Behavior
///
/// Stretches on both axes; constrain it with `.frame()`.
pub struct ShaderPaintView {
    fragment: Cow<'static, str>,
    animated: Computed<bool>,
    uniforms: Computed<Vec<f32>>,
}

impl fmt::Debug for ShaderPaintView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShaderPaintView").finish_non_exhaustive()
    }
}

impl ShaderPaintView {
    /// A static shader from its fragment body.
    #[must_use]
    pub fn new(fragment: impl Into<Cow<'static, str>>) -> Self {
        Self::from_source(ShaderSource::wgsl(fragment))
    }

    /// A shader from a prepared source, animated when the source says so.
    #[must_use]
    pub fn from_source(source: ShaderSource) -> Self {
        Self {
            fragment: source.source,
            animated: nami::constant(source.animated).computed(),
            uniforms: nami::constant(Vec::new()).computed(),
        }
    }

    /// Whether the shader re-renders every frame so `uniforms.time` advances,
    /// following a signal.
    ///
    /// A change re-records the view's scene with the matching shader; the
    /// view itself is not rebuilt.
    #[must_use]
    pub fn animated(mut self, animated: impl IntoComputed<bool>) -> Self {
        self.animated = animated.into_computed().distinct().computed();
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
            fragment: self.fragment,
            animated: self.animated,
            uniforms: self.uniforms,
            shader: None,
            animation_watch: None,
        })
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

struct ShaderContent {
    fragment: Cow<'static, str>,
    animated: Computed<bool>,
    uniforms: Computed<Vec<f32>>,
    /// The registration drawn, and whether it is the animated source.
    shader: Option<(bool, Registered<Shader>)>,
    /// Re-records the scene when `animated` changes, while mounted.
    animation_watch: Option<BoxWatcherGuard>,
}

impl SceneContent for ShaderContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> bool {
        let animated = self.animated.snapshot();
        let (_, shader) = match self.shader.take() {
            Some((registered, shader)) if registered == animated => {
                self.shader.insert((animated, shader))
            }
            // The other source's registration, if any, is dropped here: in
            // the call that stops drawing it.
            _ => {
                let source = ShaderSource {
                    source: self.fragment.clone(),
                    animated,
                };
                let shader = resources
                    .shader(source)
                    .unwrap_or_else(|error| panic!("shader paint: {error}"));
                self.shader.insert((animated, shader))
            }
        };
        let id = resources.name(shader);
        let paint = self.uniforms.map(move |uniforms| ShaderPaint {
            shader: id,
            uniforms,
        });
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        recorder.fill(bounds, paint);
        false
    }

    fn rebuild_for_engine(self: Box<Self>) -> Box<dyn SceneContent> {
        let Self {
            fragment,
            animated,
            uniforms,
            shader: _,
            animation_watch: _,
        } = *self;
        Box::new(Self {
            fragment,
            animated,
            uniforms,
            shader: None,
            animation_watch: None,
        })
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.animation_watch =
            invalidator.map(|invalidator| invalidate_on_change(&invalidator, &self.animated));
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
    use alloc::rc::Rc;
    use core::cell::Cell;

    use cherenkov::testing::Event;
    use nami::Binding;
    use waterui_core::AnyView;

    use crate::scene::resources::tests::Mount;

    const FRAGMENT: &str = "@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(uv, 0.0, 1.0); }";

    #[test]
    fn a_shader_view_is_a_stretching_scene_view() {
        let view = ShaderPaintView::new(FRAGMENT);
        assert_eq!(view.stretch_axis(), StretchAxis::Both);
        let body = AnyView::new(view.body(&Environment::new()));
        assert!(body.downcast::<SceneView>().is_ok());
    }

    fn shader_events(events: &[Event]) -> (usize, usize) {
        events
            .iter()
            .fold((0, 0), |(added, removed), event| match event {
                Event::AddShader(_) => (added + 1, removed),
                Event::RemoveShader(_) => (added, removed + 1),
                _ => (added, removed),
            })
    }

    /// Turning animation on or off re-records the scene with the other
    /// source — the engine only re-renders an animated shader every frame —
    /// and lets the source it stopped drawing go.
    #[test]
    fn toggling_animation_swaps_the_registered_source() {
        let animated = Binding::container(false);
        let view = ShaderPaintView::new(FRAGMENT).animated(animated.clone());
        let scene = AnyView::new(view.body(&Environment::new()))
            .downcast::<SceneView>()
            .expect("a shader view is a scene view");
        let mut content = scene.into_content();
        let invalidated = Rc::new(Cell::new(0));
        let count = Rc::clone(&invalidated);
        content.set_invalidator(Some(Rc::new(move || count.set(count.get() + 1))));

        let mount = Mount::new();
        let first = mount.frame(&mut *content);
        assert_eq!(shader_events(&first.events), (1, 0));

        animated.set(true);
        assert_eq!(
            invalidated.get(),
            1,
            "a new animation state asks for a frame"
        );
        animated.set(true);
        assert_eq!(invalidated.get(), 1, "an unchanged state does not");

        let second = mount.frame(&mut *content);
        assert_eq!(
            shader_events(&second.events),
            (1, 1),
            "the animated source replaces the static one"
        );
        let third = mount.frame(&mut *content);
        assert_eq!(shader_events(&third.events), (0, 0));

        content.set_invalidator(None);
        animated.set(false);
        assert_eq!(invalidated.get(), 1, "an unmounted shader stops asking");
    }
}
