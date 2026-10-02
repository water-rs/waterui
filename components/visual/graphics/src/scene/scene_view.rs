use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use core::fmt;

use cherenkov::Recorder;
use nami::Signal;
use nami::watcher::BoxWatcherGuard;

use waterui_core::layout::{ProposalSize, Size, StretchAxis};
use waterui_core::{AnyView, Environment, Native, NativeView, View};

use crate::input::SurfaceInputEvent;
use crate::scene::resources::RecordingResources;

/// Environment marker: render `SceneView` directly in the backend scene.
#[derive(Debug, Clone, Copy, Default)]
pub struct SceneViewMergeToParent;

/// Callback used by scene content to request another frame.
pub type SceneInvalidator = Rc<dyn Fn()>;

/// Asks for a frame whenever `signal` changes, for as long as the returned
/// guard lives.
///
/// The one way scene content follows a signal it draws from. This is scene
/// invalidation, not a subtree rebuild: the content instance and whatever it
/// caches survive the change, and the next `build_scene` reads the new value.
/// Keep the guard beside the invalidator and drop both when the invalidator
/// is cleared, which is what stopping the frames means.
pub fn invalidate_on_change<S: Signal>(
    invalidator: &SceneInvalidator,
    signal: &S,
) -> BoxWatcherGuard {
    let invalidator = Rc::clone(invalidator);
    Box::new(signal.watch(move |_| invalidator()))
}

/// Object-safe scene producer for `SceneView`.
pub trait SceneContent: 'static {
    /// Records this content's drawing into `recorder`, registering and naming
    /// through `resources` whatever engine resource the drawing uses.
    ///
    /// The recorder is the engine's live recording target: constant operands
    /// freeze into the [`Content`](cherenkov::Content) it finishes, and
    /// `nami` signals handed to it stay bound, so a colour or transform the
    /// content draws from a signal animates without another call here.
    ///
    /// `width` and `height` are the box the content is drawing into, in
    /// logical points — the same space every layout contract on this trait
    /// measures in.
    ///
    /// # Resources
    ///
    /// A recorder only names fonts, images and shader paints by id; the
    /// engine that draws the recording owns the registrations behind those
    /// ids. `resources` is this recording's share of that engine's
    /// registration table, and it arrives with every call — rather than once
    /// at mount — because the moment a content first needs a resource is the
    /// moment it first records it: here, on whichever frame that is. A canvas
    /// whose closure reaches for a new font on its tenth frame, or an image
    /// whose source signal delivers a new picture, registers it in the call
    /// that draws it and records it straight away. Content never holds on to
    /// the engine: `resources` is borrowed for the duration of the call.
    ///
    /// Registering returns a [`Registered`] handle, which content keeps for
    /// as long as it goes on drawing the resource; asking again for a source
    /// it still holds returns the same registration without a new upload.
    /// A new registration is a round trip to the render thread and blocks
    /// this call — and so the host's frame — until the engine has the
    /// resource; see the blocking contract on
    /// [`SceneResources`](crate::resources::SceneResources#blocking).
    /// The id to record comes from [`RecordingResources::name`], which holds
    /// the registration for this recording. It is the only way to get an id
    /// from a handle, so name the resource in every call that draws it rather
    /// than keeping an id from an earlier one.
    ///
    /// Because the recording holds what it names, content drops a handle as
    /// soon as it stops drawing the resource, in the very call that records
    /// the drawing without it. The recording still installed keeps the
    /// resource until the host has installed the one that replaces it, so the
    /// release never reaches a recording that can still be drawn — whether the
    /// host renders before installing this call's recording or discards it.
    ///
    /// A host calls this with the [`RecordingResources`] of the recording
    /// `recorder` records into, and keeps that recording's
    /// [`HeldResources`](crate::resources::HeldResources) for as long as the
    /// recording is installed.
    ///
    /// Returns true when the content requires another frame to be rendered.
    ///
    /// [`Registered`]: crate::resources::Registered
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> bool;

    /// Reconstructs this content for a newly created engine generation.
    ///
    /// A `SceneContent` may own registrations, a Cherenkov recording, or
    /// another object whose identity belongs to the engine that created it.
    /// Those values cannot be carried into a replacement engine. Recovery
    /// clears or rebuilds this content's engine-bound state while retaining
    /// its semantic/source state for the next [`build_scene`](Self::build_scene)
    /// call. The host invokes this before it records the first scene on the
    /// replacement engine, and then supplies that call's `RecordingResources`
    /// as usual.
    ///
    /// Content that owns no engine-bound state must implement this explicitly
    /// as an empty reset.
    /// Content that caches a [`Registered`](crate::scene::resources::Registered),
    /// a [`PictureRecording`](crate::picture::PictureRecording), or another
    /// generation-bound value must preserve its semantic inputs while
    /// rebuilding that value.
    fn rebuild_for_engine(&mut self);

    /// Installs an invalidation callback that content can trigger from signal watchers.
    ///
    /// The host installs one when it mounts the content and clears it with
    /// `None` when it unmounts it, so this is also where content starts and
    /// stops frame sources of its own, such as an animation clock.
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

    /// What this drawing is named, for a screen reader.
    ///
    /// A scene reaches the screen as anonymous fills and glyph runs, so the node
    /// a backend emits for the leaf is the only place its content can be
    /// announced at all, and the backend has nothing to read it from but this. A
    /// chart answers with its title; content that is decoration, or that cannot
    /// name itself honestly, answers `None` — the default — and the node stays
    /// unnamed.
    ///
    /// This is the name the content *offers*, not the name it imposes: the
    /// application's own `.a11y_label(…)` wins over it wherever both exist,
    /// because the application knows what the drawing is for and the content
    /// only knows what it drew. It is read on every emission rather than once,
    /// so content whose drawing follows a signal answers with what it currently
    /// draws.
    ///
    /// The label is the node's *name*; what the drawing actually says — a
    /// formula's spoken mathematics, a chart's plotted summary — belongs to
    /// [`SceneContent::accessibility_value`], which the label never replaces.
    fn accessibility_label(&self) -> Option<String> {
        None
    }

    /// What this drawing says, for a screen reader.
    ///
    /// The value channel: the semantic content the drawing carries, announced
    /// beside the label rather than underneath it. A formula answers with its
    /// spoken mathematics, a chart with what it plots; content with nothing to
    /// say answers `None` — the default.
    ///
    /// Because the value is a separate channel from the label, it survives an
    /// application-supplied `.a11y_label(…)`: a formula labelled `"Euler's
    /// identity"` keeps speaking `e raised to i pi plus one equals zero`
    /// instead of falling silent. An application's own `.a11y_value(…)` wins
    /// over the content's answer wherever both exist. Like the label, it is
    /// read on every emission, so content whose drawing follows a signal
    /// answers with what it currently draws.
    fn accessibility_value(&self) -> Option<String> {
        None
    }

    /// Whether this content handles its own keyboard, IME, pointer and scroll
    /// input.
    ///
    /// Content that is interactive in itself — a terminal, a text editor, a
    /// game board — returns `true`, and whichever realization draws it then
    /// routes the events landing on it to [`SceneContent::input`]: a backend
    /// that merges the scene into its own tree registers the content as an
    /// input target, and the `GpuContentView` realization forwards its surface's
    /// events. Content that only draws — the common case — leaves this
    /// `false`, claims no focus, and every event keeps going to the widgets
    /// around it.
    ///
    /// Read when the content is placed, so the answer is a property of the
    /// content rather than of its current state.
    fn wants_input_events(&self) -> bool {
        false
    }

    /// Handles one input event.
    ///
    /// Only called when [`SceneContent::wants_input_events`] returns `true`.
    /// Every position is logical and local to the content: its own top-left is
    /// `(0, 0)`, in the same space [`SceneContent::build_scene`] draws in. See
    /// [`SurfaceInputEvent`] for the vocabulary.
    ///
    /// An event that changes what the content draws is followed by a call to
    /// the invalidator from [`SceneContent::set_invalidator`]: delivering an
    /// event does not itself schedule a frame.
    fn input(&mut self, event: &SurfaceInputEvent) {
        let _ = event;
    }

    /// Where this content's text caret is, in logical content-local
    /// coordinates.
    ///
    /// Backends place the input-method candidate window against it, so
    /// content that accepts composed text reports its caret. `None` — the
    /// default — means there is no caret to place the panel against.
    fn ime_caret(&self) -> Option<cherenkov::kurbo::Rect> {
        None
    }
}

/// Fills in the axes a proposal left open from scene content's intrinsic size.
///
/// This is the one rule every realization of a [`SceneView`] measures by — the
/// `GpuContentView` one, hydrolysis' retained tree, dew's display list — so a scene
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
/// An axis named `f32::INFINITY` is not a box: it is how a container asks for a
/// maximum, and the maximum of content that is a size is that size. It is
/// treated as open, so a row probing a scene's largest extent gets the natural
/// one back and the scene keeps the `StretchAxis::None` promise of
/// [`scene_stretch_axis`] instead of swallowing the row's leftover space.
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

    let width = proposal.width.filter(|width| width.is_finite());
    let height = proposal.height.filter(|height| height.is_finite());
    match (width, height) {
        (Some(width), Some(height)) => ProposalSize::new(width, height),
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
    natural_across * (named / natural_along)
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

/// A view that mounts [`SceneContent`] on the backend's engine layer tree.
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

    /// What the wrapped content is named, for a screen reader.
    ///
    /// See [`SceneContent::accessibility_label`]; a backend emitting the leaf's
    /// semantic node offers this as the node's default name.
    #[must_use]
    pub fn accessibility_label(&self) -> Option<String> {
        self.content.accessibility_label()
    }

    /// What the wrapped content says about itself, for a screen reader.
    ///
    /// See [`SceneContent::accessibility_value`]; a backend emitting the leaf's
    /// semantic node offers this as the node's default value.
    #[must_use]
    pub fn accessibility_value(&self) -> Option<String> {
        self.content.accessibility_value()
    }

    /// Takes ownership of the wrapped scene content.
    #[must_use]
    pub fn into_content(self) -> Box<dyn SceneContent> {
        self.content
    }

    /// Replaces this view's content with its source-preserving reconstruction
    /// for a newly created engine generation.
    ///
    /// The returned view keeps the same layout and accessibility contract;
    /// only engine-bound scene state is reconstructed. The host must call this
    /// before the first recording made with the replacement engine.
    #[must_use]
    pub fn rebuild_for_engine(mut self) -> Self {
        self.content.rebuild_for_engine();
        self
    }
}

impl NativeView for SceneView {
    fn stretch_axis(&self) -> StretchAxis {
        scene_stretch_axis(self.intrinsic_size())
    }
}

impl View for SceneView {
    fn body(self, _env: &Environment) -> impl View {
        // A scene draws on the engine its backend already owns: the native
        // leaf carries the content and the backend mounts it in its layer
        // tree. There is no second renderer to fall back on.
        AnyView::new(Native::new(self))
    }

    fn stretch_axis(&self) -> StretchAxis {
        scene_stretch_axis(self.intrinsic_size())
    }
}

#[cfg(test)]
mod tests {
    use cherenkov::{Draw, kurbo::Shape};

    use super::{
        NativeView, ProposalSize, Recorder, RecordingResources, SceneContent, SceneView, Size,
        StretchAxis, resolve_scene_proposal, scene_stretch_axis,
    };

    /// Content that is naturally 100x200 — twice as tall as it is wide.
    struct Tall;

    struct Rebuildable {
        value: u32,
    }

    impl SceneContent for Rebuildable {
        fn build_scene(
            &mut self,
            recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            width: f32,
            height: f32,
        ) -> bool {
            recorder.fill(
                cherenkov::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height))
                    .to_path(0.1),
                cherenkov::WorkingColor::BLACK,
            );
            false
        }

        fn intrinsic_size(&self) -> Option<Size> {
            Some(Size::new(self.value as f32, 1.0))
        }

        fn rebuild_for_engine(&mut self) {}
    }

    impl SceneContent for Tall {
        fn build_scene(
            &mut self,
            _recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            _width: f32,
            _height: f32,
        ) -> bool {
            false
        }

        fn intrinsic_size(&self) -> Option<Size> {
            Some(Size::new(100.0, 200.0))
        }

        fn rebuild_for_engine(&mut self) {}
    }

    /// Content with no size of its own, which is the trait's default.
    struct Sizeless;

    impl SceneContent for Sizeless {
        fn build_scene(
            &mut self,
            _recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            _width: f32,
            _height: f32,
        ) -> bool {
            false
        }

        fn rebuild_for_engine(&mut self) {}
    }

    /// Content that says what it draws, the way a formula or a chart does.
    struct Spoken;

    impl SceneContent for Spoken {
        fn build_scene(
            &mut self,
            _recorder: &mut Recorder,
            _resources: &mut RecordingResources<'_>,
            _width: f32,
            _height: f32,
        ) -> bool {
            false
        }

        fn accessibility_value(&self) -> Option<alloc::string::String> {
            Some("x squared plus one".into())
        }

        fn rebuild_for_engine(&mut self) {}
    }

    /// The view must forward exactly what its content offers a screen reader:
    /// the value channel survives, and quiet content is not given a name or a
    /// value it never had.
    #[test]
    fn the_view_carries_the_value_its_content_gives() {
        assert_eq!(
            SceneView::new(Spoken).accessibility_value(),
            Some("x squared plus one".into())
        );
        assert_eq!(
            SceneView::new(Spoken).accessibility_label(),
            None,
            "content that names nothing must not invent a name for itself"
        );
        assert_eq!(
            SceneView::new(Sizeless).accessibility_value(),
            None,
            "content with nothing to say must not invent a value either"
        );
    }

    #[test]
    fn reconstruction_replaces_engine_bound_content_and_preserves_semantic_state() {
        let view = SceneView::new(Rebuildable { value: 37 });
        let rebuilt = view.rebuild_for_engine();
        assert_eq!(
            rebuilt.intrinsic_size(),
            Some(Size::new(37.0, 1.0)),
            "reconstruction must carry semantic state into the new content"
        );
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
    fn an_unbounded_axis_is_a_maximum_probe_answered_by_the_natural_size() {
        // `INFINITY` is how a container asks for a maximum, and the most a scene
        // that is a size wants is that size: a stack ranking its children's
        // flexibility must not hear "as much as you have" from content that
        // declared `StretchAxis::None`.
        assert_eq!(
            resolve_scene_proposal(
                Tall.intrinsic_size(),
                ProposalSize::new(Some(f32::INFINITY), None)
            ),
            ProposalSize::new(100.0, 200.0)
        );
        assert_eq!(
            resolve_scene_proposal(Tall.intrinsic_size(), ProposalSize::INFINITY),
            ProposalSize::new(100.0, 200.0)
        );
        // The finite axis still drives the open one; the infinite one is open.
        assert_eq!(
            resolve_scene_proposal(
                Tall.intrinsic_size(),
                ProposalSize::new(Some(f32::INFINITY), Some(400.0))
            ),
            ProposalSize::new(200.0, 400.0),
            "a row of height 400 probing the maximum width gets the aspect-fit width"
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
