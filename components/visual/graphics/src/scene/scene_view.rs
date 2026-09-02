use alloc::boxed::Box;
use alloc::rc::Rc;
use core::fmt;

use waterui_core::layout::{ProposalSize, Size, StretchAxis};
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

    /// The size this drawing is naturally, in logical points.
    ///
    /// An image's pixel dimensions, an SVG document's `viewBox`, a barcode's
    /// module grid, a formula's typeset box: content that *is* a particular size
    /// answers with it, and layout uses it wherever nothing else settles the
    /// question. `None` — the default — means the drawing has no size of its own
    /// and takes whatever it is given, which is right for a full-bleed background,
    /// a shader, or a canvas whose author draws into the box they are handed.
    ///
    /// The answer is a whole size rather than a pair of independent axes because
    /// it is also the drawing's aspect ratio: when a container names one axis and
    /// leaves the other open, [`resolve_scene_proposal`] derives the open one from
    /// this ratio, which a per-axis answer could not express.
    ///
    /// It must be finite and positive on both axes; a drawing with no honest size
    /// answers `None` instead of a degenerate one.
    fn intrinsic_size(&self) -> Option<Size> {
        None
    }
}

/// Fills in the axes a proposal left open from scene content's intrinsic size.
///
/// This is the one rule every realization of a [`SceneView`] measures by — the
/// `GpuSurface` one, hydrolysis' retained tree, dew's display list — so a scene
/// cannot be sized differently depending on which backend drew it.
///
/// - Content with no intrinsic size is returned unchanged, so a scene that takes
///   whatever it is given keeps doing exactly that, and each caller keeps its own
///   fallback for the axes still left open.
/// - Both axes named: the proposal stands. A scene fills a box it was given a box
///   for, which is what `StretchAxis::Both` promises.
/// - Neither axis named: the natural size, which is the whole point of the hook.
/// - Exactly one axis named: that axis stands and the other follows the natural
///   aspect ratio — an image `.resizable()` under aspect fit, sized by the axis its
///   container actually constrained.
///
/// A named axis that is not finite (`f32::INFINITY` is how a container asks for a
/// maximum) cannot scale anything, so the open axis falls back to its natural
/// extent rather than becoming infinite too.
///
/// # Panics
///
/// Panics when `intrinsic` is not finite and positive on both axes: a
/// [`SceneContent`] that claims a degenerate natural size would otherwise push
/// `NaN` geometry into layout, which surfaces far away from the content that
/// produced it.
#[must_use]
pub fn resolve_scene_proposal(intrinsic: Option<Size>, proposal: ProposalSize) -> ProposalSize {
    let Some(natural) = intrinsic else {
        return proposal;
    };
    assert!(
        natural.width.is_finite()
            && natural.height.is_finite()
            && natural.width > 0.0
            && natural.height > 0.0,
        "SceneContent::intrinsic_size must be finite and positive, got {}x{}",
        natural.width,
        natural.height
    );

    match (proposal.width, proposal.height) {
        (Some(_), Some(_)) => proposal,
        (None, None) => ProposalSize::new(natural.width, natural.height),
        (Some(width), None) => {
            ProposalSize::new(width, scale_across(width, natural.width, natural.height))
        }
        (None, Some(height)) => {
            ProposalSize::new(scale_across(height, natural.height, natural.width), height)
        }
    }
}

/// The extent across the named axis, at the scale that axis was named at.
fn scale_across(named: f32, natural_along: f32, natural_across: f32) -> f32 {
    if named.is_finite() {
        natural_across * (named / natural_along)
    } else {
        natural_across
    }
}

/// Which axes a scene claims from its container, given its intrinsic size.
///
/// Content with no size of its own takes whatever it is offered, exactly as it
/// always has. Content that *is* a size is content-sized and claims no leftover
/// space: an icon in a row must not eat the row, and a container that wants it
/// bigger says so with a frame, which [`resolve_scene_proposal`] then honours.
/// This is the rule `waterui-image` already measures its own surfaces by.
#[must_use]
pub const fn scene_stretch_axis(intrinsic: Option<Size>) -> StretchAxis {
    if intrinsic.is_some() {
        StretchAxis::None
    } else {
        StretchAxis::Both
    }
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

    /// The natural size of the wrapped content, if it has one.
    ///
    /// See [`SceneContent::intrinsic_size`]; backends measuring a `SceneView` as a
    /// native leaf feed this to [`resolve_scene_proposal`].
    #[must_use]
    pub fn intrinsic_size(&self) -> Option<Size> {
        self.content.intrinsic_size()
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
        scene_stretch_axis(self.intrinsic_size())
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
        scene_stretch_axis(self.intrinsic_size())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NativeView, ProposalSize, SceneContent, SceneView, Size, StretchAxis,
        resolve_scene_proposal, scene_stretch_axis,
    };
    use crate::scene2d::Scene2D;

    /// Content that is naturally 100x200 — twice as tall as it is wide.
    struct Tall;

    impl SceneContent for Tall {
        fn build_scene(&mut self, _scene: &mut dyn Scene2D, _width: f32, _height: f32) -> bool {
            false
        }

        fn intrinsic_size(&self) -> Option<Size> {
            Some(Size::new(100.0, 200.0))
        }
    }

    /// Content with no size of its own, which is the trait's default.
    struct Sizeless;

    impl SceneContent for Sizeless {
        fn build_scene(&mut self, _scene: &mut dyn Scene2D, _width: f32, _height: f32) -> bool {
            false
        }
    }

    #[test]
    fn content_defaults_to_no_intrinsic_size() {
        assert_eq!(Sizeless.intrinsic_size(), None);
        assert_eq!(
            SceneView::new(Sizeless).intrinsic_size(),
            None,
            "a view must report exactly what its content reports"
        );
        assert_eq!(
            SceneView::new(Tall).intrinsic_size(),
            Some(Size::new(100.0, 200.0))
        );
    }

    #[test]
    fn sizeless_content_is_proposed_unchanged() {
        // Every axis, open or named, reaches the caller's own fallback untouched.
        for proposal in [
            ProposalSize::UNSPECIFIED,
            ProposalSize::ZERO,
            ProposalSize::INFINITY,
            ProposalSize::new(Some(80.0), None),
            ProposalSize::new(None, Some(40.0)),
        ] {
            assert_eq!(resolve_scene_proposal(None, proposal), proposal);
        }
    }

    #[test]
    fn an_open_proposal_resolves_to_the_natural_size() {
        assert_eq!(
            resolve_scene_proposal(Tall.intrinsic_size(), ProposalSize::UNSPECIFIED),
            ProposalSize::new(100.0, 200.0)
        );
    }

    #[test]
    fn a_named_proposal_stands_on_both_axes() {
        let named = ProposalSize::new(320.0, 40.0);
        assert_eq!(
            resolve_scene_proposal(Tall.intrinsic_size(), named),
            named,
            "content given a box fills it, however far that is from its natural size"
        );
        assert_eq!(
            resolve_scene_proposal(Tall.intrinsic_size(), ProposalSize::ZERO),
            ProposalSize::ZERO,
            "a minimum-size probe must still be answerable with zero"
        );
    }

    #[test]
    fn one_named_axis_drives_the_other_by_aspect_ratio() {
        assert_eq!(
            resolve_scene_proposal(Tall.intrinsic_size(), ProposalSize::new(Some(200.0), None)),
            ProposalSize::new(200.0, 400.0),
            "twice the natural width is twice the natural height"
        );
        assert_eq!(
            resolve_scene_proposal(Tall.intrinsic_size(), ProposalSize::new(None, Some(50.0))),
            ProposalSize::new(25.0, 50.0),
            "a quarter of the natural height is a quarter of the natural width"
        );
    }

    #[test]
    fn an_unbounded_named_axis_leaves_the_other_natural() {
        // `INFINITY` is how a container asks for a maximum; nothing can be scaled
        // by it, so the open axis stays the size the content actually is.
        assert_eq!(
            resolve_scene_proposal(
                Tall.intrinsic_size(),
                ProposalSize::new(Some(f32::INFINITY), None)
            ),
            ProposalSize::new(f32::INFINITY, 200.0)
        );
    }

    #[test]
    #[should_panic(expected = "must be finite and positive")]
    fn a_degenerate_natural_size_is_rejected() {
        let _ = resolve_scene_proposal(Some(Size::new(0.0, 10.0)), ProposalSize::UNSPECIFIED);
    }

    #[test]
    fn only_sizeless_content_claims_leftover_space() {
        assert_eq!(scene_stretch_axis(None), StretchAxis::Both);
        assert_eq!(
            scene_stretch_axis(Tall.intrinsic_size()),
            StretchAxis::None,
            "an icon in a row must not eat the row"
        );
        assert_eq!(
            NativeView::stretch_axis(&SceneView::new(Tall)),
            StretchAxis::None
        );
        assert_eq!(
            NativeView::stretch_axis(&SceneView::new(Sizeless)),
            StretchAxis::Both
        );
    }
}
