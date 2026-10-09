//! Backdrop materials: chrome that samples what lies behind it.
//!
//! A widget theme draws a chrome into a [`Recorder`](crate::Recorder). A
//! chrome that is a backdrop surface — glass, frosted panels, a tab bar
//! over scrolling content — declares it with
//! [`Recorder::backdrop_material`](crate::Recorder::backdrop_material):
//! its live shape, the registered [`MaterialShader`] its effect runs, the
//! [`CaptureClass`] whose capture parameters its group takes, and its live
//! [`MaterialEffect`].
//!
//! A material is compositing, not drawing. It never enters a
//! [`DisplayList`](crate::DisplayList) or a [`Picture`](crate::Picture),
//! and no engine receives one. A recording that may hold materials is
//! opened with [`Content::record_layered`](crate::Content::record_layered),
//! which returns it split at each material as [`LayeredContent`]: the
//! content below the first material, then one [`MaterialRun`] per
//! material with the content recorded above it. The recording's host
//! realizes each material as a backdrop member layer at the material's
//! paint position, binding the layer's clip to the material's shape and
//! its backdrop sample to the material's effect, so a signal change in
//! either reaches the member with no re-recording.
//!
//! The keys are the theme's own and are engine-independent. A theme
//! declares what they mean once, in a [`MaterialRegistry`]; the realizing
//! host maps them to each engine's handles.
//!
//! **Groups.** A material's backdrop group — one capture and one filter
//! chain shared by its members — is keyed by the [`MaterialScope`] the
//! recording was opened under, the material's capture class, and the
//! compositing canvas the host installs the member in. Outside every
//! scope ([`MaterialScope::SOLO`]), or for a class whose grouping is
//! [`MaterialGrouping::Solo`], every member is a group of its own.
//! Members in different compositing canvases cannot share a capture.
//! Every capture parameter comes from the class, so the members of one
//! group agree on them by construction. A group inside a scope samples
//! the backdrop as it stood where the scope paints, so no member of the
//! scope — in any class — sees another; a group of its own samples it as
//! it stood at its member.

use std::num::NonZeroU64;

use rustc_hash::FxHashMap;

use crate::backdrop::{BackdropShaderSource, CaptureLevels, CaptureScale};
use crate::record::{Content, SharedLive};
use crate::shape::ShapeData;

/// The key a widget theme chooses for one of its backdrop shaders.
///
/// The theme registers the shader's source under it in a
/// [`MaterialRegistry`]; a material names it in
/// [`Recorder::backdrop_material`](crate::Recorder::backdrop_material).
/// It is engine-independent: the realizing host registers the source with
/// every engine it attaches and maps the key to that engine's handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialShader(u32);

impl MaterialShader {
    /// The key with raw value `raw`.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The key's raw value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// The key a widget theme chooses for one of its capture classes: a set of
/// backdrop-group parameters ([`MaterialCapture`]) its materials share.
///
/// The theme registers the class's parameters under it in a
/// [`MaterialRegistry`]; a material names it in
/// [`Recorder::backdrop_material`](crate::Recorder::backdrop_material).
/// Materials of different classes never share a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CaptureClass(u32);

impl CaptureClass {
    /// The key with raw value `raw`.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The key's raw value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// A capture class's backdrop-group parameters. They are theme tokens:
/// the theme fixes them at registration, and every group of the class is
/// created with them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialCapture {
    /// The resolution the class's groups capture at.
    pub scale: CaptureScale,
    /// How many capture levels the class's group pyramids keep
    /// ([`CaptureLevels`]): the pyramid a `backdrop_sample_level` shader
    /// reads.
    pub levels: CaptureLevels,
    /// How the class's members form groups inside a material scope.
    pub grouping: MaterialGrouping,
}

/// A union field's smoothing width in the group's local **logical**
/// pixels: finite and positive.
///
/// The realizing host converts it to device pixels at the display scale
/// the group is built for — surfacing a non-finite device value there —
/// so the value states its unit in the type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnionSmoothing(f32);

/// Why a [`UnionSmoothing`] is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UnionSmoothingError {
    /// NaN or infinite.
    #[error("the union smoothing is not finite")]
    NonFinite,
    /// Zero or negative — a union field needs a positive width.
    #[error("the union smoothing must be positive")]
    OutOfRange,
}

impl UnionSmoothing {
    /// Validates `px` as a logical union-smoothing width.
    ///
    /// # Errors
    /// [`UnionSmoothingError`] for a non-finite or non-positive `px`.
    pub const fn new(px: f32) -> Result<Self, UnionSmoothingError> {
        if !px.is_finite() {
            return Err(UnionSmoothingError::NonFinite);
        }
        if px <= 0.0 {
            return Err(UnionSmoothingError::OutOfRange);
        }
        Ok(Self(px))
    }

    /// The logical-pixel value.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// A member composite's outer extent in the group's local **logical**
/// pixels: finite and non-negative.
///
/// The realizing host converts it to device pixels at the display scale
/// the group is built for and binds it as the member's
/// [`BackdropOuter`](crate::BackdropOuter), surfacing a non-finite device
/// value there — so the value states its unit in the type.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OuterExtent(f32);

/// Why an [`OuterExtent`] is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OuterExtentError {
    /// NaN or infinite.
    #[error("the outer extent is not finite")]
    NonFinite,
    /// Negative — an extent reaches outward or not at all.
    #[error("the outer extent must be non-negative")]
    Negative,
}

impl OuterExtent {
    /// The zero extent: the clip's coverage.
    pub const ZERO: Self = Self(0.0);

    /// Validates `px` as a logical outer extent.
    ///
    /// # Errors
    /// [`OuterExtentError`] for a non-finite or negative `px`.
    pub const fn new(px: f32) -> Result<Self, OuterExtentError> {
        if !px.is_finite() {
            return Err(OuterExtentError::NonFinite);
        }
        if px < 0.0 {
            return Err(OuterExtentError::Negative);
        }
        Ok(Self(px))
    }

    /// The logical-pixel value.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// How a capture class's members form groups inside a
/// [`MaterialScope`]: the scope a view subtree declares to say that its
/// materials belong together.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MaterialGrouping {
    /// The class ignores scopes: every member is a group of its own, with
    /// its own capture.
    Solo,
    /// The class's members within one scope and one compositing canvas
    /// share one group: one capture of what lies behind the scope, and
    /// one filter chain. No member of the scope — of this class or
    /// another — sees another.
    Shared,
    /// The class's members group like [`Shared`](Self::Shared) and
    /// additionally sample one union field: the smooth minimum of the
    /// members' signed distances with `smoothing` as the field's width
    /// ([`UnionSmoothing`], a validated logical-pixel value — the device
    /// [`BackdropUnion`]). The realizing host converts it to device
    /// pixels at the display scale the group is built for, surfacing
    /// [`BackdropUnionError`] for a non-finite device value.
    ///
    /// [`BackdropUnion`]: crate::BackdropUnion
    /// [`BackdropUnionError`]: crate::BackdropUnionError
    Union {
        /// The union field's width in the group's local logical pixels
        /// ([`BackdropUnion`]'s smoothing).
        ///
        /// [`BackdropUnion`]: crate::BackdropUnion
        smoothing: UnionSmoothing,
    },
}

/// A material's live per-member parameters, applied in the member's
/// composite against its group's shared capture.
///
/// Bound as a signal, a change reaches the realized member with no
/// re-recording.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialEffect {
    uniforms: Vec<f32>,
    /// The member composite's outer extent in the group's local logical
    /// pixels; the realizing host converts it to device pixels at the
    /// display scale the group is built for and binds it as the member's
    /// [`BackdropOuter`]. [`OuterExtent::ZERO`] keeps the clip's coverage.
    ///
    /// [`BackdropOuter`]: crate::BackdropOuter
    outer: OuterExtent,
}

impl MaterialEffect {
    /// The most uniforms a backdrop shader receives: its sixteen
    /// `vec4` parameters.
    pub const MAX_UNIFORMS: usize = 64;

    /// An effect with `uniforms` in the shader's declared order.
    ///
    /// # Panics
    /// Panics, naming the violated rule, unless there are at most
    /// [`MAX_UNIFORMS`](Self::MAX_UNIFORMS) uniforms and each is finite.
    #[must_use]
    pub fn new(uniforms: Vec<f32>) -> Self {
        assert!(
            uniforms.len() <= Self::MAX_UNIFORMS,
            "a backdrop material effect carries {} uniforms; a backdrop shader takes at most {}",
            uniforms.len(),
            Self::MAX_UNIFORMS,
        );
        if let Some(index) = uniforms.iter().position(|u| !u.is_finite()) {
            panic!(
                "backdrop material uniform {index} is {}; every uniform must be finite",
                uniforms[index]
            );
        }
        Self {
            uniforms,
            outer: OuterExtent::ZERO,
        }
    }

    /// The member composite's outer extent — `extent` logical pixels
    /// beyond the field's edge, validated by [`OuterExtent`] — converted
    /// to device pixels at the group's build display scale by the
    /// realizing host and bound as the member's
    /// [`BackdropOuter`]. [`OuterExtent::ZERO`] — the default — keeps the
    /// clip's coverage.
    ///
    /// [`BackdropOuter`]: crate::BackdropOuter
    #[must_use]
    pub fn outer(self, extent: OuterExtent) -> Self {
        Self {
            uniforms: self.uniforms,
            outer: extent,
        }
    }

    /// The uniforms the effect was made with, in the shader's declared
    /// order: at most [`MAX_UNIFORMS`](Self::MAX_UNIFORMS), each finite.
    #[must_use]
    pub fn uniforms(&self) -> &[f32] {
        &self.uniforms
    }

    /// The outer extent the effect was made with, in the group's local
    /// logical pixels: [`OuterExtent::ZERO`] unless
    /// [`outer`](Self::outer) set it.
    #[must_use]
    pub const fn outer_extent(&self) -> OuterExtent {
        self.outer
    }
}

nami_core::impl_constant!(MaterialEffect);

/// The material scope a layered recording is opened under: the nearest
/// enclosing view subtree that groups its materials, or
/// [`MaterialScope::SOLO`] for none.
///
/// The recording's host supplies it, never the drawing code: a widget
/// theme draws one chrome and cannot know which of its neighbours belong
/// with it. Two scopes are the same group scope exactly when they are
/// equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialScope(Option<NonZeroU64>);

impl MaterialScope {
    /// No enclosing scope: every material recorded under it is a group of
    /// its own.
    pub const SOLO: Self = Self(None);

    /// The scope the host identifies by `id`. The host keeps ids unique
    /// among the scopes that are live at once.
    #[must_use]
    pub const fn new(id: NonZeroU64) -> Self {
        Self(Some(id))
    }

    /// The scope's id; `None` for [`MaterialScope::SOLO`].
    #[must_use]
    pub const fn id(self) -> Option<NonZeroU64> {
        self.0
    }

    /// Whether this is [`MaterialScope::SOLO`].
    #[must_use]
    pub const fn is_solo(self) -> bool {
        self.0.is_none()
    }
}

/// A backdrop material as a layered recording returns it, for its host to
/// realize as a backdrop member layer.
///
/// The shape and the effect are [`SharedLive`]: the host binds the member's
/// clip to the shape and its backdrop sample to the effect (through
/// [`SharedLive::map`]), so a signal change reaches the member with no
/// re-recording. Each binding starts from the signal's value at the bind —
/// [`SharedLive::rebound`] — so a group rebuild never restarts stale.
#[derive(Clone, Debug)]
pub struct BackdropMaterial {
    shape: SharedLive<ShapeData>,
    shader: MaterialShader,
    capture: CaptureClass,
    effect: SharedLive<MaterialEffect>,
    scope: MaterialScope,
}

impl BackdropMaterial {
    pub(crate) const fn new(
        shape: SharedLive<ShapeData>,
        shader: MaterialShader,
        capture: CaptureClass,
        effect: SharedLive<MaterialEffect>,
        scope: MaterialScope,
    ) -> Self {
        Self {
            shape,
            shader,
            capture,
            effect,
            scope,
        }
    }

    /// The member's clip, in the recording's coordinates, and its later
    /// changes.
    #[must_use]
    pub const fn shape(&self) -> &SharedLive<ShapeData> {
        &self.shape
    }

    /// The registered backdrop shader the member's effect runs.
    #[must_use]
    pub const fn shader(&self) -> MaterialShader {
        self.shader
    }

    /// The capture class whose parameters the member's group takes.
    #[must_use]
    pub const fn capture(&self) -> CaptureClass {
        self.capture
    }

    /// The member's per-member parameters and their later changes.
    #[must_use]
    pub const fn effect(&self) -> &SharedLive<MaterialEffect> {
        &self.effect
    }

    /// The material scope the recording was opened under.
    #[must_use]
    pub const fn scope(&self) -> MaterialScope {
        self.scope
    }
}

/// One material of a layered recording and the content recorded after it,
/// which draws above it.
#[derive(Debug)]
pub struct MaterialRun {
    /// The material.
    pub material: BackdropMaterial,
    /// What was recorded after the material and before the next one: it
    /// draws above the material, and the next material samples it.
    pub above: Content,
}

/// A recording split at each backdrop material, as
/// [`Content::record_layered`](crate::Content::record_layered) returns it.
///
/// Paint order is `below`, then each run's material and its `above`
/// content in turn. Each part is a [`Content`] of its own: a signal used
/// by commands of one part updates slots of that part only.
#[derive(Debug)]
pub struct LayeredContent {
    /// What was recorded before the first material: it draws below every
    /// material, and the first material samples it.
    pub below: Content,
    /// Each material in recording order, with the content above it.
    pub runs: Vec<MaterialRun>,
}

/// The backdrop shaders and capture classes a widget theme declares, under
/// its own keys.
///
/// A theme fills one once, before any chrome is drawn. The contract a
/// realizing host follows: it registers every shader with each engine it
/// attaches, creates groups from the capture classes, and panics when it
/// realizes a material naming a key the registry lacks.
#[derive(Clone, Debug, Default)]
pub struct MaterialRegistry {
    shaders: FxHashMap<MaterialShader, BackdropShaderSource>,
    captures: FxHashMap<CaptureClass, MaterialCapture>,
}

impl MaterialRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the backdrop shader `source` under `key`.
    ///
    /// # Panics
    /// Panics when `key` is already registered.
    pub fn register_shader(
        &mut self,
        key: MaterialShader,
        source: BackdropShaderSource,
    ) -> &mut Self {
        let previous = self.shaders.insert(key, source);
        assert!(
            previous.is_none(),
            "backdrop material shader {key:?} is registered twice"
        );
        self
    }

    /// Registers the capture class `capture` under `key`.
    ///
    /// # Panics
    /// Panics when `key` is already registered.
    pub fn register_capture_class(
        &mut self,
        key: CaptureClass,
        capture: MaterialCapture,
    ) -> &mut Self {
        let previous = self.captures.insert(key, capture);
        assert!(
            previous.is_none(),
            "backdrop material capture class {key:?} is registered twice"
        );
        self
    }

    /// The shader source registered under `key`.
    #[must_use]
    pub fn shader(&self, key: MaterialShader) -> Option<&BackdropShaderSource> {
        self.shaders.get(&key)
    }

    /// The capture parameters registered under `key`.
    #[must_use]
    pub fn capture_class(&self, key: CaptureClass) -> Option<&MaterialCapture> {
        self.captures.get(&key)
    }

    /// Every registered shader, in no particular order.
    pub fn shaders(&self) -> impl Iterator<Item = (MaterialShader, &BackdropShaderSource)> {
        self.shaders.iter().map(|(key, source)| (*key, source))
    }

    /// Every registered capture class, in no particular order.
    pub fn capture_classes(&self) -> impl Iterator<Item = (CaptureClass, &MaterialCapture)> {
        self.captures.iter().map(|(key, capture)| (*key, capture))
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shaders.is_empty() && self.captures.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use kurbo::{Affine, Circle, Rect};
    use nami::{SignalExt, binding};

    use super::*;
    use crate::display_list::{Command, Operand};
    use crate::record::{ContentChange, Draw};
    use crate::size::LayoutSize;
    use crate::style::Group;
    use crate::{Recorder, WorkingColor};

    const GLASS: MaterialShader = MaterialShader::new(1);
    const CHROME: CaptureClass = CaptureClass::new(7);

    fn white() -> WorkingColor {
        WorkingColor::new([1., 1., 1., 1.])
    }

    /// The display list a content first sends, as fill rects.
    fn fills(content: &mut Content) -> Vec<Rect> {
        let Some(ContentChange::Replace(picture)) = content.take_change() else {
            panic!("the first commit sends the whole list");
        };
        picture
            .display_list()
            .commands()
            .iter()
            .map(|command| match command {
                Command::Fill {
                    shape: ShapeData::Rect(rect),
                    ..
                } => *rect,
                other => panic!("only rect fills were recorded, found {other:?}"),
            })
            .collect()
    }

    fn material(c: &mut Recorder, x: f32) {
        let left = f64::from(x);
        c.backdrop_material(
            Rect::new(left, 0., left + 10., 10.),
            GLASS,
            CHROME,
            MaterialEffect::new(vec![x]),
        );
    }

    #[test]
    fn a_layered_recording_splits_at_each_material_and_routes_slots_per_run() {
        let (a, b, c) = (binding(1.0_f64), binding(2.0_f64), binding(3.0_f64));
        let rect = |x: f64| Rect::new(x, x, x + 1., x + 1.);
        let scope = MaterialScope::new(NonZeroU64::new(9).expect("non-zero"));
        let LayeredContent { mut below, runs } =
            Content::record_layered(&LayoutSize::new(), scope, |r| {
                r.fill(Rect::new(0., 0., 1., 1.), white());
                r.fill(a.clone().map(rect), white());
                material(r, 10.);
                r.fill(b.clone().map(rect), white());
                material(r, 20.);
                r.fill(Rect::new(5., 5., 6., 6.), white());
                r.fill(c.clone().map(rect), white());
            });
        let mut runs: Vec<_> = runs.into_iter().collect();
        assert_eq!(runs.len(), 2);
        for (run, x) in runs.iter().zip([10.0_f32, 20.0]) {
            let left = f64::from(x);
            assert_eq!(run.material.shader(), GLASS);
            assert_eq!(run.material.capture(), CHROME);
            assert_eq!(
                run.material.scope(),
                scope,
                "the host's scope reaches every material"
            );
            assert_eq!(
                *run.material.shape().value(),
                ShapeData::Rect(Rect::new(left, 0., left + 10., 10.))
            );
            assert_eq!(run.material.effect().value().uniforms(), [x]);
        }
        assert_eq!(fills(&mut below), [Rect::new(0., 0., 1., 1.), rect(1.)]);
        assert_eq!(fills(&mut runs[0].above), [rect(2.)]);
        assert_eq!(
            fills(&mut runs[1].above),
            [Rect::new(5., 5., 6., 6.), rect(3.)]
        );

        // Each signal updates the slot of its own part, at that part's
        // command index.
        b.set(4.);
        assert_eq!(below.take_change(), None);
        assert_eq!(runs[1].above.take_change(), None);
        let Some(ContentChange::Update(updates)) = runs[0].above.take_change() else {
            panic!("b's part updates");
        };
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].command, 0);
        assert_eq!(updates[0].value, Operand::Shape(ShapeData::Rect(rect(4.))));

        c.set(5.);
        a.set(6.);
        let Some(ContentChange::Update(updates)) = runs[1].above.take_change() else {
            panic!("c's part updates");
        };
        assert_eq!((updates.len(), updates[0].command), (1, 1));
        let Some(ContentChange::Update(updates)) = below.take_change() else {
            panic!("a's part updates");
        };
        assert_eq!((updates.len(), updates[0].command), (1, 1));
        assert_eq!(runs[0].above.take_change(), None);
    }

    #[test]
    fn a_layered_recording_without_materials_is_all_below() {
        let LayeredContent { below, runs } =
            Content::record_layered(&LayoutSize::new(), MaterialScope::SOLO, |r| {
                r.fill(Rect::new(0., 0., 1., 1.), white());
            });
        assert!(runs.is_empty());
        assert_eq!(below.len(), 1);
        assert!(MaterialScope::SOLO.is_solo());
        assert_eq!(MaterialScope::SOLO.id(), None);
    }

    #[test]
    fn material_signals_reach_their_live_and_no_slot() {
        let width = binding(10.0_f64);
        let gain = binding(1.0_f32);
        let LayeredContent { mut below, runs } =
            Content::record_layered(&LayoutSize::new(), MaterialScope::SOLO, |r| {
                r.fill(Rect::new(0., 0., 1., 1.), white());
                r.backdrop_material(
                    width.clone().map(|w| Rect::new(0., 0., w, 10.)),
                    GLASS,
                    CHROME,
                    gain.clone().map(|g| MaterialEffect::new(vec![g, 0.5])),
                );
            });
        let [
            MaterialRun {
                material,
                mut above,
            },
        ] = <[MaterialRun; 1]>::try_from(runs)
            .unwrap_or_else(|_| panic!("one material was recorded"));
        let _ = below.take_change();
        let _ = above.take_change();

        let (shape, effect) = (material.shape().clone(), material.effect().clone());
        let shapes = Rc::new(RefCell::new(Vec::new()));
        let effects = Rc::new(RefCell::new(Vec::new()));
        let (_, shape_guard) = shape.watch({
            let shapes = Rc::clone(&shapes);
            move |context| shapes.borrow_mut().push(context.into_value())
        });
        let (_, effect_guard) = effect.watch({
            let effects = Rc::clone(&effects);
            move |context| effects.borrow_mut().push(context.into_value())
        });

        width.set(20.);
        gain.set(2.);
        assert_eq!(
            *shapes.borrow(),
            [ShapeData::Rect(Rect::new(0., 0., 20., 10.))]
        );
        assert_eq!(*effects.borrow(), [MaterialEffect::new(vec![2., 0.5])]);
        assert_eq!(
            below.take_change(),
            None,
            "no slot of the recording updates"
        );
        assert_eq!(
            above.take_change(),
            None,
            "no slot of the recording updates"
        );
        drop((shape_guard, effect_guard));
    }

    #[test]
    fn a_material_effect_binds_and_tracks_its_signal() {
        let gain = binding(1.0_f32);
        let LayeredContent { runs, .. } =
            Content::record_layered(&LayoutSize::new(), MaterialScope::SOLO, |r| {
                r.backdrop_material(
                    Rect::new(0., 0., 5., 5.),
                    GLASS,
                    CHROME,
                    gain.clone().map(|g| MaterialEffect::new(vec![g])),
                );
            });
        let effect = runs
            .into_iter()
            .next()
            .expect("one run")
            .material
            .effect()
            .clone();
        gain.set(2.);
        assert_eq!(
            *effect.value(),
            MaterialEffect::new(vec![1.]),
            "the value when the `Live` was made — a single snapshot per value"
        );
        let rebound = effect.rebound();
        assert_eq!(
            *rebound.value(),
            MaterialEffect::new(vec![2.]),
            "a rebound `Live` reads the signal's current value"
        );
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (start, _guard) = rebound.watch({
            let seen = Rc::clone(&seen);
            move |context| seen.borrow_mut().push(context.into_value())
        });
        assert_eq!(
            start,
            MaterialEffect::new(vec![2.]),
            "the binding starts from the rebound value, not the recording-time one"
        );
        gain.set(3.);
        assert_eq!(
            *seen.borrow(),
            [MaterialEffect::new(vec![3.])],
            "changes after the bind arrive through the watch"
        );
    }

    #[test]
    #[should_panic(expected = "opened with `Content::record_layered`")]
    fn a_material_in_a_plain_recording_panics() {
        let _ = Content::record(&LayoutSize::new(), |r| material(r, 0.));
    }

    fn layered(body: impl FnOnce(&mut Recorder)) {
        let _ = Content::record_layered(&LayoutSize::new(), MaterialScope::SOLO, body);
    }

    #[test]
    #[should_panic(expected = "outside every clip, transform and group scope")]
    fn a_material_inside_a_clip_panics() {
        layered(|r| r.clip(Rect::new(0., 0., 5., 5.), |r| material(r, 0.)));
    }

    #[test]
    #[should_panic(expected = "outside every clip, transform and group scope")]
    fn a_material_inside_a_transform_panics() {
        layered(|r| r.transform(Affine::IDENTITY, |r| material(r, 0.)));
    }

    #[test]
    #[should_panic(expected = "outside every clip, transform and group scope")]
    fn a_material_inside_a_group_panics() {
        layered(|r| r.group(Group::default(), |r| material(r, 0.)));
    }

    #[test]
    fn a_material_after_a_closed_scope_is_top_level() {
        layered(|r| {
            r.clip(Rect::new(0., 0., 5., 5.), |r| {
                r.fill(Circle::new((1., 1.), 1.), white());
            });
            material(r, 0.);
        });
    }

    #[test]
    #[should_panic(expected = "a backdrop shader takes at most 64")]
    fn an_effect_with_more_than_64_uniforms_panics() {
        let _ = MaterialEffect::new(vec![0.; 65]);
    }

    #[test]
    #[should_panic(expected = "uniform 1 is NaN")]
    fn an_effect_with_a_non_finite_uniform_panics() {
        let _ = MaterialEffect::new(vec![0., f32::NAN]);
    }

    #[test]
    #[should_panic(expected = "uniform 0 is inf")]
    fn a_non_finite_uniform_change_panics_in_its_signal_map() {
        let gain = binding(1.0_f32);
        let LayeredContent { runs, .. } =
            Content::record_layered(&LayoutSize::new(), MaterialScope::SOLO, |r| {
                r.backdrop_material(
                    Rect::new(0., 0., 5., 5.),
                    GLASS,
                    CHROME,
                    gain.clone().map(|g| MaterialEffect::new(vec![g])),
                );
            });
        let effect = runs
            .into_iter()
            .next()
            .expect("one run")
            .material
            .effect()
            .clone();
        let (_, _guard) = effect.watch(|_| {});
        gain.set(f32::INFINITY);
    }

    #[test]
    #[should_panic(expected = "shader MaterialShader(1) is registered twice")]
    fn a_duplicate_shader_key_panics() {
        let source = || BackdropShaderSource::wgsl("");
        MaterialRegistry::new()
            .register_shader(GLASS, source())
            .register_shader(GLASS, source());
    }

    #[test]
    #[should_panic(expected = "capture class CaptureClass(7) is registered twice")]
    fn a_duplicate_capture_class_key_panics() {
        let capture = MaterialCapture {
            scale: CaptureScale::FULL,
            levels: CaptureLevels::ONE,
            grouping: MaterialGrouping::Shared,
        };
        MaterialRegistry::new()
            .register_capture_class(CHROME, capture)
            .register_capture_class(CHROME, capture);
    }

    #[test]
    fn a_registry_answers_for_its_keys() {
        let capture = MaterialCapture {
            scale: CaptureScale::new(0.25).expect("in range"),
            levels: CaptureLevels::new(3).expect("in range"),
            grouping: MaterialGrouping::Solo,
        };
        let mut registry = MaterialRegistry::new();
        assert!(registry.is_empty());
        registry
            .register_shader(GLASS, BackdropShaderSource::wgsl("src").reach(6.))
            .register_capture_class(CHROME, capture);
        assert!(!registry.is_empty());
        assert_eq!(registry.shader(GLASS).map(|s| s.reach), Some(6.));
        assert_eq!(
            registry.shader(MaterialShader::new(2)).map(|s| s.reach),
            None
        );
        assert_eq!(registry.capture_class(CHROME), Some(&capture));
        assert_eq!(registry.capture_class(CaptureClass::new(8)), None);
        assert_eq!(registry.shaders().count(), 1);
        assert_eq!(registry.capture_classes().count(), 1);
    }
}
