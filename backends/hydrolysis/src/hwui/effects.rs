//! Native effect lowering: a `FilteredView`'s portable description becomes a
//! chain of Android `RenderEffect`s — `createBlurEffect` stages plus one
//! linear-light matrix shader per contiguous run of colour links. The Kotlin
//! factory rebuilds the chain on every definition, so a parameter animation
//! is a `DefineEffect` payload, not a re-record or remount.
//!
//! Colour links are sampled in extended linear Display P3 — the filtrate
//! working space — and conjugated before upload (`P3→sRGB · M · sRGB→P3`)
//! because the platform shader's `toLinearSrgb`/`fromLinearSrgb` pair bounds
//! the matrix to sRGB primaries. Registration validates the whole chain
//! before allocating an id: an unsupported link or API level rejects without
//! consuming a registration.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use waterui_graphics::filter_view::{self as fv, AnyEffect};
use waterui_graphics::filtrate::{
    AnimatedTarget, AnimationTrack, ColorFilter, ColorStage, FilterLink, Placed, SpatialStage,
    StageCollector, WatchGuard, filters,
    space::{P3_LUMA, P3_TO_SRGB, SRGB_TO_P3},
};

use super::HwuiError;
use super::buffer::{CommandBuffer, Field};
use super::protocol::{Op, effect_kind};
use super::resources::{Dropped, Kind, SharedRegistry};

/// The redraw callback a watch installed on a reactive effect parameter
/// fires: the host passes the surface's content-redraw handle.
pub type EffectWake = Arc<dyn Fn() + Send + Sync>;

/// One colour link of the native plan: its params' base in the flattened
/// array and the function producing its row-major 4x4 matrix.
struct ColorLink {
    param_base: usize,
    matrix: fn(&[f32]) -> [[f32; 4]; 4],
}

/// A native chain step.
enum PlanStep {
    /// `createBlurEffect(radius, radius)`; `param` indexes `radius`.
    Blur { param: usize },
    /// Contiguous colour links composed into one matrix stage.
    ColorRun(Vec<ColorLink>),
}

/// The fixed native shape of a chain, shared across samples.
struct Plan(Vec<PlanStep>);

/// The most stages a `DefineEffect` record's u16 word count carries:
/// `2 + 13 * count` words must fit `u16::MAX`.
const MAX_EFFECT_STAGES: usize = (u16::MAX as usize - 2) / 13;

/// One stage of the wire payload: kind + 12 floats.
#[derive(Clone, PartialEq)]
struct EffectStage {
    kind: u32,
    values: [f32; 12],
}

/// A live registration's sampled state.
struct EffectEntry {
    /// The mounted `AnyEffect`, kept so its descriptions and signals stay
    /// valid for the entry's life.
    _source: AnyEffect,
    /// The param watches: dropping them ends wakes.
    _guards: Vec<WatchGuard>,
    plan: Plan,
    /// The declared stage names sampled errors carry.
    names: Vec<&'static str>,
    tracks: Vec<AnimationTrack>,
    events: Receiver<(usize, AnimatedTarget)>,
    /// The same `Sender` the signal callbacks hold; tests inject targets
    /// onto the real queue through it.
    #[cfg(test)]
    test_tx: Sender<(usize, AnimatedTarget)>,
    /// The last payload emitted; `None` while the initial definition is
    /// still queued in `pending`.
    emitted: Option<Vec<EffectStage>>,
}

/// The registry-side effect state: the encoder and the resource owner share
/// `Rc` clones of it.
#[derive(Clone)]
pub struct Effects(Rc<RefCell<EffectStore>>);

impl std::fmt::Debug for Effects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effects").finish_non_exhaustive()
    }
}

struct EffectStore {
    registry: SharedRegistry,
    api_level: u32,
    /// Initial definitions not yet emitted; an entry here outlives its
    /// registration, so a register+drop still defines before it releases.
    pending: Vec<(u32, Vec<EffectStage>)>,
    entries: BTreeMap<u32, EffectEntry>,
    last: Option<Instant>,
}

/// The registration a `FilteredView` mount retains; its drop releases the
/// effect after the tree stops naming it.
pub struct EffectRegistration {
    id: FilterId,
    wire: u32,
    store: Rc<RefCell<EffectStore>>,
    dropped: Dropped,
}

impl crate::renderer::mount::target::FilterRegistration for EffectRegistration {
    fn filter_id(&self) -> FilterId {
        self.id
    }
}

impl Drop for EffectRegistration {
    fn drop(&mut self) {
        self.store.borrow_mut().entries.remove(&self.wire);
        self.dropped.push(Kind::Effect, self.wire);
    }
}

impl Effects {
    /// Effect state sharing `registry`'s tables, on a device at `api_level`.
    pub(crate) fn new(registry: SharedRegistry, api_level: u32) -> Self {
        Self(Rc::new(RefCell::new(EffectStore {
            registry,
            api_level,
            pending: Vec::new(),
            entries: BTreeMap::new(),
            last: None,
        })))
    }

    /// Registers `source`'s chain and returns the mount's handle.
    ///
    /// Every link must map to a supported native stage before the id is
    /// allocated; `wake` is the redraw the param watches fire.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Unsupported`] for a GPU effect, an output-size override
    /// or a link no native stage covers; [`HwuiError::RequiresApi`] for a
    /// blur below API 31 or a colour stage below API 33.
    pub(crate) fn register(
        &self,
        source: AnyEffect,
        wake: EffectWake,
    ) -> Result<EffectRegistration, HwuiError> {
        let Some(description) = source.description() else {
            return Err(unsupported(
                "renders a GPU effect with no portable description",
            ));
        };
        if source.output_size().is_some() {
            return Err(unsupported("overrides its filter's output size"));
        }
        let names = stage_names(&description);
        let plan = plan(&description, &names)?;
        if plan.0.len() > MAX_EFFECT_STAGES {
            return Err(unsupported(format!(
                "an effect chain of {} stages exceeds the {MAX_EFFECT_STAGES}-stage record limit",
                plan.0.len()
            )));
        }
        let api = self.0.borrow().api_level;
        // The whole chain must fit the device before the id is allocated.
        for step in &plan.0 {
            match step {
                PlanStep::Blur { .. } if api < 31 => {
                    return Err(requires_api("a blur filter", 31, api));
                }
                PlanStep::ColorRun(_) if api < 33 => {
                    return Err(requires_api("a colour filter", 33, api));
                }
                _ => {}
            }
        }
        let params = description.params();
        if params.iter().any(|p| !p.is_finite()) {
            return Err(unsupported(format!(
                "has a non-finite filter parameter; the chain declares {names:?}"
            )));
        }
        // Every fallible check precedes the id's allocation and the
        // subscriptions: a rejected registration leaves nothing behind.
        let tracks: Vec<AnimationTrack> = params.iter().map(|&v| AnimationTrack::new(v)).collect();
        let stages = sample(&plan, &tracks, &names)?;
        let mut store = self.0.borrow_mut();
        let wire = store.registry.lock().acquire(Kind::Effect)?;
        let dropped = store.registry.dropped();
        let (tx, rx) = channel::<(usize, AnimatedTarget)>();
        let mut guards = Vec::new();
        description.visit_signals(|signal| {
            let index = signal.index();
            let tx = Sender::clone(&tx);
            let wake = Arc::clone(&wake);
            guards.push(signal.watch_animated(move |target| {
                if tx.send((index, target)).is_ok() {
                    wake();
                }
            }));
        });
        store.entries.insert(
            wire,
            EffectEntry {
                _source: source,
                _guards: guards,
                plan,
                names: names.clone(),
                tracks,
                events: rx,
                #[cfg(test)]
                test_tx: tx,
                emitted: None,
            },
        );
        store.pending.push((wire, stages));
        drop(wake);
        Ok(EffectRegistration {
            id: FilterId::new(u64::from(wire)),
            wire,
            store: Rc::clone(&self.0),
            dropped,
        })
    }

    /// Emits queued initial definitions and resampled payloads into
    /// `buffer`; `true` while any parameter animation remains active.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Encoding`] on a non-finite sample or an oversized
    /// chain, or [`HwuiError::Unsupported`] when a registration later
    /// declares an output size.
    pub(crate) fn encode(
        &self,
        time: Instant,
        buffer: &mut CommandBuffer,
    ) -> Result<bool, HwuiError> {
        let mut store = self.0.borrow_mut();
        let delta = store
            .last
            .replace(time)
            .map_or(Duration::ZERO, |prev| time.duration_since(prev));
        // Initial definitions first, even for entries already released —
        // the device must never release an id it has not defined.
        for (wire, stages) in std::mem::take(&mut store.pending) {
            emit_effect(buffer, wire, &stages)?;
            if let Some(entry) = store.entries.get_mut(&wire) {
                entry.emitted = Some(stages);
            }
        }
        let mut animating = false;
        for (&wire, entry) in &mut store.entries {
            // Advance past targets; a snap arriving below still cancels the
            // in-flight one before `animating` is read.
            for track in &mut entry.tracks {
                track.advance(delta);
            }
            while let Ok((index, target)) = entry.events.try_recv() {
                entry.tracks[index].set_target(target.value, target.interpolator);
            }
            for track in &mut entry.tracks {
                animating |= track.is_active();
            }
            let stages = sample(&entry.plan, &entry.tracks, &entry.names)?;
            if entry.emitted.as_ref() != Some(&stages) {
                emit_effect(buffer, wire, &stages)?;
                entry.emitted = Some(stages);
            }
        }
        Ok(animating)
    }
}

type Instant = cherenkov_record::Instant;
type FilterId = cherenkov_record::FilterId;

/// `unsupported`'s mount site names no layer yet — the id is 0.
fn unsupported(what: impl Into<String>) -> HwuiError {
    HwuiError::Unsupported {
        layer: 0,
        what: what.into(),
    }
}

const fn requires_api(what: &'static str, needs: u32, device: u32) -> HwuiError {
    HwuiError::RequiresApi {
        layer: 0,
        what,
        needs,
        device,
    }
}

/// Every declared stage's name, for link errors.
fn stage_names(description: &fv::FilterDescription<'_>) -> Vec<&'static str> {
    struct Names(Vec<&'static str>);
    impl StageCollector for Names {
        fn color(&mut self, stage: Placed<ColorStage>) {
            self.0.push(stage.stage.name);
        }
        fn spatial(&mut self, stage: Placed<SpatialStage>) {
            self.0.push(stage.stage.name);
        }
    }
    let mut names = Names(Vec::new());
    description.collect_stages(&mut names);
    names.0
}

/// The whole chain as native steps, or the first failure found.
fn plan(
    description: &fv::FilterDescription<'_>,
    names: &[&'static str],
) -> Result<Plan, HwuiError> {
    let mut steps = Vec::new();
    let mut run: Vec<ColorLink> = Vec::new();
    let mut index = 0usize;
    let mut failure: Option<HwuiError> = None;
    description.visit_links(|link| {
        let i = index;
        index += 1;
        match map_link(&link) {
            Some(Mapped::Color(m)) => run.push(ColorLink {
                param_base: link.param_base,
                matrix: m,
            }),
            Some(Mapped::Blur) => {
                if !run.is_empty() {
                    steps.push(PlanStep::ColorRun(std::mem::take(&mut run)));
                }
                steps.push(PlanStep::Blur {
                    param: link.param_base,
                });
            }
            None => {
                if failure.is_none() {
                    failure = Some(unsupported(format!(
                        "has no native stage for filter link {i}; the chain declares {names:?}"
                    )));
                }
            }
        }
    });
    if let Some(failure) = failure {
        return Err(failure);
    }
    if !run.is_empty() {
        steps.push(PlanStep::ColorRun(run));
    }
    Ok(Plan(steps))
}

enum Mapped {
    Color(fn(&[f32]) -> [[f32; 4]; 4]),
    Blur,
}

/// `link`'s native step: each colour mapping names a concrete `LINEAR`
/// filter so a semantic change fails compilation, not the device.
fn map_link(link: &FilterLink<'_>) -> Option<Mapped> {
    macro_rules! color {
        ($ty:ty, $f:expr) => {{
            const { assert!(<$ty as ColorFilter>::LINEAR) };
            return Some(Mapped::Color($f));
        }};
    }
    if link.downcast_ref::<fv::Brightness>().is_some()
        || link.downcast_ref::<filters::Brightness<f32>>().is_some()
    {
        color!(filters::Brightness<f32>, brightness);
    }
    if link.downcast_ref::<fv::Contrast>().is_some()
        || link.downcast_ref::<filters::Contrast<f32>>().is_some()
    {
        color!(filters::Contrast<f32>, contrast);
    }
    if link.downcast_ref::<fv::Exposure>().is_some()
        || link.downcast_ref::<filters::Exposure<f32>>().is_some()
    {
        color!(filters::Exposure<f32>, exposure);
    }
    if link.downcast_ref::<fv::Saturation>().is_some()
        || link.downcast_ref::<filters::Saturation<f32>>().is_some()
    {
        color!(filters::Saturation<f32>, saturation);
    }
    if link.downcast_ref::<fv::Grayscale>().is_some()
        || link.downcast_ref::<filters::Grayscale<f32>>().is_some()
    {
        color!(filters::Grayscale<f32>, grayscale);
    }
    if link.downcast_ref::<fv::Sepia>().is_some()
        || link.downcast_ref::<filters::Sepia<f32>>().is_some()
    {
        color!(filters::Sepia<f32>, sepia);
    }
    if link.downcast_ref::<fv::HueRotation>().is_some()
        || link.downcast_ref::<filters::HueRotation<f32>>().is_some()
    {
        color!(filters::HueRotation<f32>, hue_rotation);
    }
    if link.downcast_ref::<fv::Invert>().is_some() {
        color!(filters::Invert, invert);
    }
    if link.downcast_ref::<fv::ColorMatrix>().is_some()
        || link
            .downcast_ref::<filters::ColorMatrix<fv::Reactive>>()
            .is_some()
    {
        color!(filters::ColorMatrix<f32>, color_matrix);
    }
    if link
        .downcast_ref::<filters::WhitePoint<fv::Reactive, fv::Reactive, fv::Reactive>>()
        .is_some()
        || link
            .downcast_ref::<filters::WhitePoint<f32, f32, f32>>()
            .is_some()
    {
        color!(filters::WhitePoint<f32, f32, f32>, white_point);
    }
    if link.downcast_ref::<filters::PhotoEffectMono>().is_some() {
        color!(filters::PhotoEffectMono, photo_effect_mono);
    }
    if link.downcast_ref::<filters::PhotoEffectTonal>().is_some() {
        color!(filters::PhotoEffectTonal, photo_effect_tonal);
    }
    if link.downcast_ref::<fv::Blur>().is_some()
        || link.downcast_ref::<filters::Blur<f32>>().is_some()
    {
        return Some(Mapped::Blur);
    }
    None
}

// ----- The matrix formulas, from the stages' WGSL. -----

const fn identity() -> [[f32; 4]; 4] {
    [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]
}

const fn brightness(p: &[f32]) -> [[f32; 4]; 4] {
    let mut m = identity();
    m[0][3] = p[0];
    m[1][3] = p[0];
    m[2][3] = p[0];
    m
}

fn contrast(p: &[f32]) -> [[f32; 4]; 4] {
    let bias = 0.5 * (1.0 - p[0]);
    let mut m = [[0f32; 4]; 4];
    for (r, row) in m.iter_mut().enumerate().take(3) {
        row[r] = p[0];
        row[3] = bias;
    }
    m[3][3] = 1.0;
    m
}

fn exposure(p: &[f32]) -> [[f32; 4]; 4] {
    let g = p[0].exp2();
    let mut m = identity();
    m[0][0] = g;
    m[1][1] = g;
    m[2][2] = g;
    m
}

fn saturation(p: &[f32]) -> [[f32; 4]; 4] {
    let (s, i) = (p[0], 1.0 - p[0]);
    let l = P3_LUMA;
    [
        [i.mul_add(l[0], s), i * l[1], i * l[2], 0.],
        [i * l[0], i.mul_add(l[1], s), i * l[2], 0.],
        [i * l[0], i * l[1], i.mul_add(l[2], s), 0.],
        [0., 0., 0., 1.],
    ]
}

fn grayscale(p: &[f32]) -> [[f32; 4]; 4] {
    let (s, i) = (p[0], 1.0 - p[0]);
    let l = P3_LUMA;
    [
        [s.mul_add(l[0], i), s * l[1], s * l[2], 0.],
        [s * l[0], s.mul_add(l[1], i), s * l[2], 0.],
        [s * l[0], s * l[1], s.mul_add(l[2], i), 0.],
        [0., 0., 0., 1.],
    ]
}

fn sepia(p: &[f32]) -> [[f32; 4]; 4] {
    const S: [[f32; 3]; 3] = [
        [0.393, 0.769, 0.189],
        [0.349, 0.686, 0.168],
        [0.272, 0.534, 0.131],
    ];
    let (s, i) = (p[0], 1.0 - p[0]);
    let mut m = [[0f32; 4]; 4];
    for (r, srow) in S.iter().enumerate() {
        for (c, &v) in srow.iter().enumerate() {
            m[r][c] = i.mul_add(f32::from(u8::from(r == c)), s * v);
        }
    }
    m[3][3] = 1.0;
    m
}

const fn invert(_: &[f32]) -> [[f32; 4]; 4] {
    [
        [-1., 0., 0., 1.],
        [0., -1., 0., 1.],
        [0., 0., -1., 1.],
        [0., 0., 0., 1.],
    ]
}

fn color_matrix(p: &[f32]) -> [[f32; 4]; 4] {
    let mut m = identity();
    for (r, row) in m.iter_mut().enumerate().take(3) {
        for (c, cell) in row.iter_mut().enumerate().take(4) {
            *cell = p[r * 4 + c];
        }
    }
    m
}

fn white_point(p: &[f32]) -> [[f32; 4]; 4] {
    let mut m = identity();
    for (r, row) in m.iter_mut().enumerate().take(3) {
        row[r] = 1.0 / p[r].max(0.0001);
    }
    m
}

const fn photo_effect_mono(_: &[f32]) -> [[f32; 4]; 4] {
    let l = P3_LUMA;
    [
        [l[0], l[1], l[2], 0.],
        [l[0], l[1], l[2], 0.],
        [l[0], l[1], l[2], 0.],
        [0., 0., 0., 1.],
    ]
}

const fn photo_effect_tonal(_: &[f32]) -> [[f32; 4]; 4] {
    let l = P3_LUMA;
    [
        [0.6f32.mul_add(l[0], 0.4), 0.6 * l[1], 0.6 * l[2], 0.],
        [0.6 * l[0], 0.6f32.mul_add(l[1], 0.4), 0.6 * l[2], 0.],
        [0.6 * l[0], 0.6 * l[1], 0.6f32.mul_add(l[2], 0.4), 0.],
        [0., 0., 0., 1.],
    ]
}

fn hue_rotation(p: &[f32]) -> [[f32; 4]; 4] {
    let (c, s) = {
        let (s, c) = p[0].to_radians().sin_cos();
        (c, s)
    };
    [
        [
            c.mul_add(0.787, s.mul_add(-0.213, 0.213)),
            c.mul_add(-0.715, s.mul_add(-0.715, 0.715)),
            s.mul_add(0.928, c.mul_add(-0.072, 0.072)),
            0.,
        ],
        [
            s.mul_add(0.143, c.mul_add(-0.213, 0.213)),
            c.mul_add(0.285, s.mul_add(0.140, 0.715)),
            c.mul_add(-0.072, s.mul_add(-0.283, 0.072)),
            0.,
        ],
        [
            c.mul_add(-0.213, s.mul_add(-0.787, 0.213)),
            s.mul_add(0.715, c.mul_add(-0.715, 0.715)),
            c.mul_add(0.928, s.mul_add(0.072, 0.072)),
            0.,
        ],
        [0., 0., 0., 1.],
    ]
}

// ----- Row-major 4x4 composition and the working-space conjugation. -----

fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut m = [[0f32; 4]; 4];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = a[r][0].mul_add(
                b[0][c],
                a[r][1].mul_add(b[1][c], a[r][2].mul_add(b[2][c], a[r][3] * b[3][c])),
            );
        }
    }
    m
}

const fn extend(m: [[f32; 3]; 3]) -> [[f32; 4]; 4] {
    [
        [m[0][0], m[0][1], m[0][2], 0.],
        [m[1][0], m[1][1], m[1][2], 0.],
        [m[2][0], m[2][1], m[2][2], 0.],
        [0., 0., 0., 1.],
    ]
}

/// `m` conjugated from extended linear P3 into the shader's sRGB working
/// space, bias column included.
fn conjugate(m: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    mul(&mul(&extend(P3_TO_SRGB), m), &extend(SRGB_TO_P3))
}

/// The current sample as wire stages.
fn sample(
    plan: &Plan,
    tracks: &[AnimationTrack],
    names: &[&'static str],
) -> Result<Vec<EffectStage>, HwuiError> {
    let params: Vec<f32> = tracks.iter().map(AnimationTrack::value).collect();
    if params.iter().any(|p| !p.is_finite()) {
        return Err(unsupported(format!(
            "sampled a non-finite filter parameter; the chain declares {names:?}"
        )));
    }
    let mut stages = Vec::with_capacity(plan.0.len());
    for step in &plan.0 {
        match step {
            PlanStep::Blur { param } => {
                let radius = params[*param].max(0.0);
                let mut values = [0f32; 12];
                values[0] = radius;
                stages.push(EffectStage {
                    kind: effect_kind::BLUR,
                    values,
                });
            }
            PlanStep::ColorRun(links) => {
                // Links compose in application order: the next link's matrix
                // left-multiplies the accumulated product.
                let mut acc = identity();
                for link in links {
                    let m = (link.matrix)(&params[link.param_base..]);
                    acc = mul(&m, &acc);
                }
                let m = conjugate(&acc);
                if m.iter().flatten().any(|v| !v.is_finite()) {
                    return Err(unsupported(format!(
                        "composed a non-finite colour matrix; the chain declares {names:?}"
                    )));
                }
                let mut values = [0f32; 12];
                for (r, row) in m.iter().enumerate().take(3) {
                    values[r * 4..r * 4 + 4].copy_from_slice(row);
                }
                stages.push(EffectStage {
                    kind: effect_kind::LINEAR_COLOR,
                    values,
                });
            }
        }
    }
    Ok(stages)
}

/// One `DefineEffect` record: `effect, stage_count`, then per stage
/// `kind` + 12 floats — `2 + 13 * count` words.
fn emit_effect(
    buffer: &mut CommandBuffer,
    effect: u32,
    stages: &[EffectStage],
) -> Result<(), HwuiError> {
    if stages.len() > MAX_EFFECT_STAGES {
        return Err(HwuiError::Encoding {
            op: Op::DefineEffect.name(),
            reason: format!(
                "an effect chain of {} stages exceeds the {MAX_EFFECT_STAGES}-stage record limit",
                stages.len()
            ),
        });
    }
    let count = u32::try_from(stages.len()).expect("bounded by the check above");
    let mut fields = Vec::with_capacity(2 + 13 * stages.len());
    fields.push(Field::U("effect", effect));
    fields.push(Field::U("stages", count));
    for stage in stages {
        fields.push(Field::U("kind", stage.kind));
        for (i, &v) in stage.values.iter().enumerate() {
            fields.push(Field::F(VALUES[i], v));
        }
    }
    buffer.op(Op::DefineEffect, &fields)
}

const VALUES: [&str; 12] = [
    "v0", "v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8", "v9", "v10", "v11",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_order_does_not_commute() {
        // Brightness(.2) then Exposure(1): out = 2in + .4.
        let m = mul(&exposure(&[1.0]), &brightness(&[0.2]));
        assert!((m[0][3] - 0.4).abs() < 1e-6);
        assert_eq!(m[0][0], 2.0);
        // Exposure(1) then Brightness(.2): out = 2in + .2.
        let m = mul(&brightness(&[0.2]), &exposure(&[1.0]));
        assert!((m[0][3] - 0.2).abs() < 1e-6);
        assert_eq!(m[0][0], 2.0);
    }

    #[test]
    fn conjugation_preserves_alpha_and_identity_bias() {
        let m = conjugate(&identity());
        assert_eq!(m[3], [0.0, 0.0, 0.0, 1.0]);
        // The identity in P3 stays the identity in sRGB.
        for r in 0..3 {
            for c in 0..4 {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!((m[r][c] - want).abs() < 1e-6, "{m:?}");
            }
        }
    }

    #[test]
    fn blur_radius_is_clamped_nonnegative() {
        let plan = Plan(vec![PlanStep::Blur { param: 0 }]);
        let tracks = vec![AnimationTrack::new(-2.0)];
        let stages = sample(&plan, &tracks, &[]).unwrap();
        assert_eq!(stages[0].kind, effect_kind::BLUR);
        assert_eq!(stages[0].values[0], 0.0);
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    use waterui_graphics::filtrate::{FilterExt, Interpolator};

    fn fixture(
        api: u32,
    ) -> (
        SharedRegistry,
        Effects,
        std::sync::Arc<AtomicUsize>,
        EffectWake,
    ) {
        let registry = SharedRegistry::new();
        let effects = Effects::new(registry.clone(), api);
        let wakes = std::sync::Arc::new(AtomicUsize::new(0));
        let wake: EffectWake = {
            let wakes = std::sync::Arc::clone(&wakes);
            std::sync::Arc::new(move || {
                wakes.fetch_add(1, Ordering::SeqCst);
            })
        };
        (registry, effects, wakes, wake)
    }

    /// The wire id a registration's `FilterId` carries.
    fn wire_of(registration: &EffectRegistration) -> u64 {
        crate::renderer::mount::target::FilterRegistration::filter_id(registration).raw()
    }

    #[test]
    fn api_levels_gate_blur_and_colour_at_registration() {
        let blur = || AnyEffect::filter(filters::Blur(2.0f32));
        let color = || AnyEffect::filter(filters::Brightness(0.5f32));
        let (_, e30, _, w30) = fixture(30);
        assert!(matches!(
            e30.register(blur(), w30),
            Err(HwuiError::RequiresApi { needs: 31, .. })
        ));
        let (_, e31, _, w31) = fixture(31);
        assert!(e31.register(blur(), w31).is_ok());
        let (_, e32, _, w32) = fixture(32);
        assert!(matches!(
            e32.register(color(), w32),
            Err(HwuiError::RequiresApi { needs: 33, .. })
        ));
        let (_, e33, _, w33) = fixture(33);
        assert!(e33.register(color(), w33).is_ok());
    }

    #[test]
    fn unsupported_links_name_their_declared_stages() {
        let cases = [
            ("Gamma", AnyEffect::filter(filters::Gamma(0.5f32))),
            (
                "PhotoEffectNoir",
                AnyEffect::filter(filters::PhotoEffectNoir),
            ),
        ];
        for (name, source) in cases {
            let (_, effects, _, wake) = fixture(34);
            let error = effects.register(source, wake).map(|_| ()).unwrap_err();
            let message = error.to_string();
            assert!(message.contains(name), "{message}");
        }
    }

    #[test]
    fn a_late_unknown_link_rejects_before_any_id_allocates() {
        let (_registry, effects, _, wake) = fixture(34);
        // A supported link followed by an unsupported one: the whole
        // registration must fail before an id is consumed.
        let chain = AnyEffect::filter(filters::Brightness(0.2f32).then(filters::Gamma(0.5f32)));
        let error = effects
            .register(chain, Arc::clone(&wake))
            .map(|_| ())
            .unwrap_err();
        assert!(error.to_string().contains("Gamma"), "{error}");
        // Same table: the failure's id was never allocated.
        let ok = effects
            .register(AnyEffect::filter(filters::Brightness(0.2f32)), wake)
            .unwrap();
        assert_eq!(wire_of(&ok), 0, "the rejected chain left slot 0 free");
    }

    #[test]
    fn failed_registrations_consume_no_ids() {
        let (_, effects, _, wake) = fixture(34);
        // A finite parameter producing an infinite matrix fails at sample,
        // before acquire — like an unsupported stage, it leaves no id.
        let infinite = AnyEffect::filter(filters::Exposure(1000f32));
        assert!(effects.register(infinite, Arc::clone(&wake)).is_err());
        let nan = AnyEffect::filter(filters::Brightness(f32::NAN));
        assert!(effects.register(nan, Arc::clone(&wake)).is_err());
        let unsupported = AnyEffect::filter(filters::Gamma(0.5f32));
        assert!(effects.register(unsupported, Arc::clone(&wake)).is_err());
        let ok = effects
            .register(AnyEffect::filter(filters::Brightness(0.2f32)), wake)
            .unwrap();
        assert_eq!(
            wire_of(&ok),
            0,
            "failed mounts released no id into the table"
        );
    }

    #[test]
    fn the_stage_count_limit_is_the_wire_limit() {
        assert_eq!(MAX_EFFECT_STAGES, 5041);
        let stage = || EffectStage {
            kind: effect_kind::BLUR,
            values: [0.0; 12],
        };
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        let stages: Vec<EffectStage> = (0..5041).map(|_| stage()).collect();
        assert!(emit_effect(&mut buffer, 0, &stages).is_ok());
        let stages: Vec<EffectStage> = (0..5042).map(|_| stage()).collect();
        assert!(emit_effect(&mut buffer, 0, &stages).is_err());
    }

    #[expect(clippy::too_many_lines, reason = "a flat table of concrete cases")]
    #[test]
    fn every_supported_concrete_form_registers() {
        struct Case {
            name: &'static str,
            build: fn(&mut fv::ParamGuards) -> AnyEffect,
        }
        let cases = [
            Case {
                name: "blur f32",
                build: |_| AnyEffect::filter(filters::Blur(1.0f32)),
            },
            Case {
                name: "blur reactive",
                build: |g| AnyEffect::filter(filters::Blur(g.bind(nami::binding::<f32>(1.0f32)))),
            },
            Case {
                name: "brightness f32",
                build: |_| AnyEffect::filter(filters::Brightness(0.5f32)),
            },
            Case {
                name: "brightness reactive",
                build: |g| {
                    AnyEffect::filter(filters::Brightness(g.bind(nami::binding::<f32>(0.5f32))))
                },
            },
            Case {
                name: "contrast f32",
                build: |_| AnyEffect::filter(filters::Contrast(1.1f32)),
            },
            Case {
                name: "contrast reactive",
                build: |g| {
                    AnyEffect::filter(filters::Contrast(g.bind(nami::binding::<f32>(1.1f32))))
                },
            },
            Case {
                name: "exposure f32",
                build: |_| AnyEffect::filter(filters::Exposure(0.5f32)),
            },
            Case {
                name: "exposure reactive",
                build: |g| {
                    AnyEffect::filter(filters::Exposure(g.bind(nami::binding::<f32>(0.5f32))))
                },
            },
            Case {
                name: "saturation f32",
                build: |_| AnyEffect::filter(filters::Saturation(0.5f32)),
            },
            Case {
                name: "saturation reactive",
                build: |g| {
                    AnyEffect::filter(filters::Saturation(g.bind(nami::binding::<f32>(0.5f32))))
                },
            },
            Case {
                name: "grayscale f32",
                build: |_| AnyEffect::filter(filters::Grayscale(0.5f32)),
            },
            Case {
                name: "grayscale reactive",
                build: |g| {
                    AnyEffect::filter(filters::Grayscale(g.bind(nami::binding::<f32>(0.5f32))))
                },
            },
            Case {
                name: "sepia f32",
                build: |_| AnyEffect::filter(filters::Sepia(0.5f32)),
            },
            Case {
                name: "sepia reactive",
                build: |g| AnyEffect::filter(filters::Sepia(g.bind(nami::binding::<f32>(0.5f32)))),
            },
            Case {
                name: "hue_rotation f32",
                build: |_| AnyEffect::filter(filters::HueRotation(30.0f32)),
            },
            Case {
                name: "hue_rotation reactive",
                build: |g| {
                    AnyEffect::filter(filters::HueRotation(g.bind(nami::binding::<f32>(30.0f32))))
                },
            },
            Case {
                name: "invert",
                build: |_| AnyEffect::filter(filters::Invert),
            },
            Case {
                name: "invert reactive",
                build: |g| {
                    let _ = g;
                    AnyEffect::filter(filters::Invert)
                },
            },
            Case {
                name: "color_matrix f32",
                build: |_| AnyEffect::filter(filters::ColorMatrix([1.; 12])),
            },
            Case {
                name: "color_matrix reactive",
                build: |g| {
                    let m: [fv::Reactive; 12] =
                        std::array::from_fn(|_| g.bind(nami::binding::<f32>(1.0f32)));
                    AnyEffect::filter(filters::ColorMatrix(m))
                },
            },
            Case {
                name: "white_point reactive",
                build: |g| {
                    AnyEffect::filter(filters::WhitePoint(
                        g.bind(nami::binding::<f32>(0.9f32)),
                        g.bind(nami::binding::<f32>(0.9f32)),
                        g.bind(nami::binding::<f32>(0.9f32)),
                    ))
                },
            },
            Case {
                name: "white_point f32",
                build: |_| AnyEffect::filter(filters::WhitePoint(0.9f32, 0.9f32, 0.9f32)),
            },
            Case {
                name: "photo_effect_mono",
                build: |_| AnyEffect::filter(filters::PhotoEffectMono),
            },
            Case {
                name: "photo_effect_tonal",
                build: |_| AnyEffect::filter(filters::PhotoEffectTonal),
            },
        ];
        for case in cases {
            let (_registry, effects, _, wake) = fixture(34);
            let mut guards = fv::ParamGuards::default();
            let registration = effects.register((case.build)(&mut guards), wake);
            let registration =
                registration.unwrap_or_else(|error| panic!("{}: {error}", case.name));
            let mut buffer = CommandBuffer::new();
            buffer.begin_frame();
            effects.encode(Instant::now(), &mut buffer).unwrap();
            let log = buffer.take_log();
            assert!(
                log.iter().any(|line| line.starts_with("DefineEffect")),
                "{}: {log:?}",
                case.name
            );
            let defines = log
                .iter()
                .filter(|line| line.starts_with("DefineEffect"))
                .count();
            assert_eq!(defines, 1);
            drop(registration);
        }
    }

    #[test]
    fn a_bound_parameter_update_wakes_and_redefines() {
        let binding = nami::binding::<f32>(0.2f32);
        let mut guards = fv::ParamGuards::default();
        let source = AnyEffect::filter(filters::Brightness(guards.bind(binding.clone())));
        let (_registry, effects, wakes, wake) = fixture(34);
        let registration = effects.register(source, wake).unwrap();
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        effects.encode(Instant::now(), &mut buffer).unwrap();
        buffer.take_log();
        binding.set(0.9f32);
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            1,
            "the watch fired the redraw"
        );
        effects.encode(Instant::now(), &mut buffer).unwrap();
        let log = buffer.take_log();
        assert_eq!(
            log.iter().filter(|l| l.starts_with("DefineEffect")).count(),
            1,
            "{log:?}"
        );
        // An unchanged frame emits nothing.
        effects.encode(Instant::now(), &mut buffer).unwrap();
        let log = buffer.take_log();
        assert!(
            !log.iter().any(|l| l.starts_with("DefineEffect")),
            "{log:?}"
        );
        // The registration's drop ends the subscription.
        drop(registration);
        binding.set(0.4f32);
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "no wake after drop");
    }

    #[test]
    fn register_and_drop_before_the_first_frame_still_defines_first() {
        let (registry, effects, _, wake) = fixture(34);
        let registration = effects
            .register(
                AnyEffect::filter(filters::Brightness(0.5f32)),
                Arc::clone(&wake),
            )
            .unwrap();
        let wire = u32::try_from(wire_of(&registration)).unwrap();
        drop(registration);
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        effects.encode(Instant::now(), &mut buffer).unwrap();
        registry.drain_releases(&mut buffer).unwrap();
        let log = buffer.take_log();
        let define = log.iter().position(|l| l.starts_with("DefineEffect"));
        let release = log.iter().position(|l| l.starts_with("ReleaseEffect"));
        assert!(define.is_some(), "{log:?}");
        assert!(release.is_some(), "{log:?}");
        assert!(
            define.unwrap() < release.unwrap(),
            "the definition precedes the release: {log:?}"
        );
        // The released wire is not recycled in the same frame: a fresh
        // registration gets the next slot.
        let next = effects
            .register(AnyEffect::filter(filters::Brightness(0.5f32)), wake)
            .unwrap();
        assert_ne!(
            u32::try_from(wire_of(&next)).unwrap(),
            wire,
            "wire reused before the release drained"
        );
    }

    struct Linear;
    impl Interpolator for Linear {
        fn duration(&self) -> Duration {
            Duration::from_secs(1)
        }
        fn interpolate(&self, from: f32, to: f32, elapsed: Duration) -> f32 {
            (to - from).mul_add((elapsed.as_secs_f32() / 1.0).min(1.0), from)
        }
    }

    #[test]
    fn parameter_animations_sample_deterministically() {
        use waterui_graphics::filtrate::AnimatedTarget;
        let (_registry, effects, _, wake) = fixture(34);
        let registration = effects
            .register(AnyEffect::filter(filters::Brightness(0.0f32)), wake)
            .unwrap();
        let wire = u32::try_from(wire_of(&registration)).unwrap();
        let send = |effects: &Effects, value: f32, interpolator: Option<Box<dyn Interpolator>>| {
            effects
                .0
                .borrow_mut()
                .entries
                .get_mut(&wire)
                .unwrap()
                .test_tx
                .send((
                    0,
                    AnimatedTarget {
                        value,
                        interpolator,
                    },
                ))
                .unwrap();
        };
        let define = |buffer: &mut CommandBuffer| {
            buffer
                .take_log()
                .into_iter()
                .find(|l| l.starts_with("DefineEffect"))
                .unwrap_or_default()
        };
        let t0 = Instant::now();
        let mut buffer = CommandBuffer::new();
        buffer.begin_frame();
        // Initial definition: the seed value.
        assert!(!effects.encode(t0, &mut buffer).unwrap());
        buffer.take_log();
        // A one-second animated target travels the real event queue; the
        // callback side of it wakes the frame (asserted on the binding path
        // in `a_bound_parameter_update_wakes_and_redefines`).
        send(&effects, 1.0, Some(Box::new(Linear)));
        // The event drains after advance: this frame installs the target
        // (the sampled payload is still the seed value), the next advance
        // moves the animation. `animating` is already true.
        let t1 = t0 + Duration::from_millis(500);
        assert!(
            effects.encode(t1, &mut buffer).unwrap(),
            "the new target is animating"
        );
        let t2 = t0 + Duration::from_secs(1);
        assert!(
            effects.encode(t2, &mut buffer).unwrap(),
            "midpoint is animating"
        );
        assert!(
            define(&mut buffer).contains("v3=3f000000"),
            "midpoint bias .5"
        );
        let t3 = t0 + Duration::from_millis(1500);
        assert!(
            !effects.encode(t3, &mut buffer).unwrap(),
            "the completed animation is not active"
        );
        assert!(
            define(&mut buffer).contains("v3=3f800000"),
            "final bias 1.0"
        );
        // Idle afterwards: no payload, not animating.
        assert!(
            !effects
                .encode(t3 + Duration::from_secs(1), &mut buffer)
                .unwrap()
        );
        assert_eq!(buffer.take_log(), Vec::<String>::new());
        // Install another animation before snapping it on a later frame.
        // Advancing that still-active track must not leave the frame's
        // animation flag set after the queued snap cancels it.
        send(&effects, 5.0, Some(Box::new(Linear)));
        let restart = t3 + Duration::from_secs(2);
        assert!(effects.encode(restart, &mut buffer).unwrap());
        buffer.take_log();
        send(&effects, 2.0, None);
        assert!(
            !effects
                .encode(restart + Duration::from_millis(100), &mut buffer)
                .unwrap(),
            "a snapped update is not animating this frame"
        );
        assert!(define(&mut buffer).contains("v3=40000000"), "snap bias 2.0");
        drop(registration);
    }

    #[test]
    fn an_output_size_override_rejects_at_registration() {
        let view = fv::FilteredView {
            content: waterui_core::AnyView::new(()),
            effect: AnyEffect::filter(filters::Brightness(0.5f32)),
            guards: fv::ParamGuards::default(),
        }
        .output_size(waterui_graphics::filter_view::OutputSize::Fixed {
            width: 4,
            height: 4,
        });
        let fv::FilteredView { effect, guards, .. } = view;
        assert!(effect.output_size().is_some());
        let (_registry, effects, _, wake) = fixture(34);
        assert!(effects.register(effect, Arc::clone(&wake)).is_err());
        drop(guards);
        // The rejection consumed no id: the next registration gets slot 0.
        let ok = effects
            .register(AnyEffect::filter(filters::Brightness(0.5f32)), wake)
            .unwrap();
        assert_eq!(wire_of(&ok), 0);
    }

    #[test]
    fn a_custom_stage_is_named_in_its_rejection() {
        // A filter type nobody mapped: its declared stage name reaches the
        // error rather than a guess at the type name.
        static STAGE: waterui_graphics::filtrate::ColorStage =
            waterui_graphics::filtrate::ColorStage {
                name: "custom_unsupported",
                source: "",
                params: &[],
                space: waterui_graphics::filtrate::OperatingSpace::Working,
            };
        struct Custom;
        impl waterui_graphics::filtrate::Filter for Custom {
            type Kind = waterui_graphics::filtrate::kind::Color;
            type Params = [f32; 0];
            fn params(&self) -> Self::Params {
                []
            }
            fn collect_stages<C: waterui_graphics::filtrate::StageCollector>(
                &self,
                collector: &mut C,
            ) {
                collector.color(waterui_graphics::filtrate::Placed::new(&STAGE));
            }
        }
        impl waterui_graphics::filtrate::ColorFilter for Custom {
            const LINEAR: bool = true;
        }
        let (_, effects, _, wake) = fixture(34);
        let error = effects
            .register(AnyEffect::filter(Custom), wake)
            .map(|_| ())
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("custom_unsupported"), "{message}");
    }

    #[test]
    fn colour_blur_colour_stays_three_steps() {
        let run = |base: usize| {
            PlanStep::ColorRun(vec![ColorLink {
                param_base: base,
                matrix: brightness,
            }])
        };
        let plan = Plan(vec![run(0), PlanStep::Blur { param: 0 }, run(1)]);
        let tracks = vec![AnimationTrack::new(0.5), AnimationTrack::new(0.25)];
        let stages = sample(&plan, &tracks, &[]).unwrap();
        assert_eq!(
            stages.iter().map(|s| s.kind).collect::<Vec<_>>(),
            vec![
                effect_kind::LINEAR_COLOR,
                effect_kind::BLUR,
                effect_kind::LINEAR_COLOR
            ]
        );
    }
}
