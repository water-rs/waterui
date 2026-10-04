//! From a filter's stages to the reference program: one composition and the
//! passes that run it.
//!
//! The composer returns a normalized form whose piece boundaries are the
//! possible materialization points. The reference executor takes the plain
//! alternative everywhere: every piece is one pass, every spatial stage is
//! materialized, and no colour prefix is folded into a spatial stage's
//! samples.

extern crate alloc;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use std::collections::HashMap;

use cherenkov_shader::{
    ComposeOptions, LibrarySource, ParamType, ParamValue, Piece, SamplerFilter, Segment, Snippet,
    SnippetKind, SnippetSource, Stage, compose,
    naga::{Module, valid::Capabilities},
};
use filtrate_core::{
    AuxSource, ColorStage, Filter, OperatingSpace, ParamArray, ParamSource, Placed, ShapeInput,
    SpatialStage, StageCollector,
    kind::{FilterKind, Kind},
};

use crate::effect::EffectSetupError;

/// Converts premultiplied linear Display P3 to premultiplied sRGB, before a
/// stage that operates in sRGB.
const TO_SRGB: ColorStage = ColorStage {
    name: "to_srgb",
    source: include_str!("../shaders/space/to_srgb.wgsl"),
    params: &[],
    space: OperatingSpace::Working,
};

/// Converts back to premultiplied linear Display P3 after it.
const FROM_SRGB: ColorStage = ColorStage {
    name: "from_srgb",
    source: include_str!("../shaders/space/from_srgb.wgsl"),
    params: &[],
    space: OperatingSpace::Working,
};

/// The shared WGSL helper modules every snippet may call into. A library is
/// a source of plain functions, not a stage; the composer imports each
/// referenced function once into the composed module.
const LIBRARIES: &[LibrarySource<'static>] = &[
    LibrarySource::new("hsl", include_str!("../shaders/lib/hsl.wgsl")),
    LibrarySource::new("sampling", include_str!("../shaders/lib/sampling.wgsl")),
    LibrarySource::new("rotate", include_str!("../shaders/lib/rotate.wgsl")),
];

/// A stage as the filter reported it.
enum Declared {
    Color(Placed<ColorStage>),
    Spatial(Placed<SpatialStage>),
}

impl Declared {
    const fn space(&self) -> OperatingSpace {
        match self {
            Self::Color(placed) => placed.stage.space,
            Self::Spatial(placed) => placed.stage.space,
        }
    }
}

struct Collector(Vec<Declared>);

impl StageCollector for Collector {
    fn color(&mut self, stage: Placed<ColorStage>) {
        self.0.push(Declared::Color(stage));
    }

    fn spatial(&mut self, stage: Placed<SpatialStage>) {
        self.0.push(Declared::Spatial(stage));
    }
}

/// What a stage's auxiliary image argument binds, within the whole chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageAux {
    /// Image `n` of the chain's flattened images.
    Image(usize),
    /// Image `n` of the chain's flattened images, which must be a
    /// caller-provided GPU texture.
    Texture(usize),
    /// The input of stage `n` of the expanded chain.
    StageInput(usize),
}

/// One stage of the chain the executor composes: the filter's stages with
/// the operating-space conversions inserted around them.
struct Expanded {
    kind: SnippetKind,
    name: &'static str,
    source: &'static str,
    params: &'static [ParamSource],
    param_base: usize,
    shape: Option<ShapeInput>,
    aux: Vec<StageAux>,
}

impl Expanded {
    const fn color(placed: Placed<ColorStage>) -> Self {
        Self {
            kind: SnippetKind::Color,
            name: placed.stage.name,
            source: placed.stage.source,
            params: placed.stage.params,
            param_base: placed.param_base,
            shape: None,
            aux: Vec::new(),
        }
    }
}

/// What a pass's auxiliary image argument binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PassAux {
    /// Image `n` of the chain's flattened images.
    Image(usize),
    /// Image `n` of the chain's flattened images, which must be a
    /// caller-provided GPU texture.
    Texture(usize),
    /// The texture pass `n` reads as its input.
    PassInput(usize),
}

/// Where one dynamic member of a pass's uniform block takes its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct UniformSlot {
    /// Byte offset in the block.
    pub offset: u32,
    /// The first of the flattened filter parameters it reads.
    pub param: usize,
    /// How many consecutive parameters it reads.
    pub components: usize,
}

/// One pass: a segment of the composition.
#[derive(Debug, Clone)]
pub(super) struct PassPlan {
    /// The segment the pass's entry point calls.
    pub segment: Segment,
    /// The stage the segment applies, for diagnostics.
    pub name: &'static str,
    /// The sampler a spatial pass declares; `None` for a colour pass.
    pub sampler: Option<SamplerFilter>,
    /// The clip-shape representation the pass reads.
    pub shape: Option<ShapeInput>,
    /// What each auxiliary image argument binds, `aux0` first.
    pub aux: Vec<PassAux>,
    /// The dynamic members of the segment's uniform block.
    pub uniforms: Vec<UniformSlot>,
}

/// The composed program for a filter.
#[derive(Debug)]
pub(super) struct Plan {
    /// The composed module; each pass adds its entry point to a copy.
    pub module: Module,
    /// The validator capabilities the module needs.
    pub capabilities: Capabilities,
    /// The passes, in order.
    pub passes: Vec<PassPlan>,
}

/// A dynamic parameter: the composer stage and member name, and the
/// flattened filter parameters it reads.
type DynamicParams = HashMap<(usize, String), (usize, usize)>;

impl Plan {
    /// Collects, checks and composes `filter`'s stages.
    ///
    /// `input_filterable` is whether the input's format supports a filtering
    /// sampler, and `intermediate_filterable` whether the intermediate
    /// format does; a filtered sample of an unfilterable input runs the
    /// manual bilinear instead. `fold` takes the folded alternative wherever
    /// the composer offers one — tests compare it against the plain program.
    pub(super) fn new<F: Filter>(
        filter: &F,
        input_filterable: bool,
        intermediate_filterable: bool,
        fold: bool,
    ) -> Result<Self, EffectSetupError> {
        let mut collector = Collector(Vec::new());
        filter.collect_stages(&mut collector);
        let declared = collector.0;
        if declared.is_empty() {
            return Err(EffectSetupError::EmptyGraph);
        }
        if F::Kind::VALUE == FilterKind::Color
            && let Some(Declared::Spatial(placed)) = declared
                .iter()
                .find(|stage| matches!(stage, Declared::Spatial(_)))
        {
            return Err(mismatch(
                placed.stage.name,
                "a colour filter reported a spatial stage".to_string(),
            ));
        }

        let expanded = expand::<F>(&declared)?;
        let snippets = expanded
            .iter()
            .map(|stage| {
                let mut source = SnippetSource::new(stage.name, stage.kind, stage.source);
                for library in LIBRARIES {
                    source = source.library(library.clone());
                }
                Snippet::parse(&source)
                    .map_err(|error| EffectSetupError::Snippet(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut dynamic = DynamicParams::new();
        let stages = expanded
            .iter()
            .zip(&snippets)
            .enumerate()
            .map(|(index, (stage, snippet))| {
                bind_params(
                    index,
                    stage,
                    snippet,
                    <F::Params as ParamArray>::LEN,
                    &mut dynamic,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let composition = compose(&stages, ComposeOptions::default())
            .map_err(|error| EffectSetupError::Compose(error.to_string()))?;
        let (module, capabilities, pieces) = composition.into_parts();
        Ok(Self {
            module,
            capabilities,
            passes: passes(
                pieces,
                &expanded,
                &dynamic,
                input_filterable,
                intermediate_filterable,
                fold,
            ),
        })
    }
}

/// Checks a stage against its snippet and specializes its constants;
/// records its dynamic parameters in `dynamic`.
fn bind_params<'s>(
    index: usize,
    stage: &Expanded,
    snippet: &'s Snippet,
    param_count: usize,
    dynamic: &mut DynamicParams,
) -> Result<Stage<'s>, EffectSetupError> {
    check_inputs(stage, snippet)?;
    let mut composed = Stage::new(snippet);
    for (param, source) in snippet.params().iter().zip(stage.params) {
        let components = components(param.ty);
        match *source {
            ParamSource::Constant(values) => {
                let value = param_value(param.ty, values).ok_or_else(|| {
                    mismatch(
                        stage.name,
                        format!(
                            "parameter `{}` is {:?}, but its constant has {} components",
                            param.name,
                            param.ty,
                            values.len()
                        ),
                    )
                })?;
                composed = composed.constant(&param.name, value);
            }
            ParamSource::Param(offset) => {
                let first = stage.param_base + offset;
                if first + components > param_count {
                    return Err(mismatch(
                        stage.name,
                        format!(
                            "parameter `{}` reads filter parameters {first}..{}, but the filter has {param_count}",
                            param.name,
                            first + components
                        ),
                    ));
                }
                dynamic.insert((index, param.name.clone()), (first, components));
            }
        }
    }
    Ok(composed)
}

/// One pass per piece: the plain alternative — or the folded one, which
/// absorbs the colour prefix pass before it — and the manual one when the
/// pass's input format has no hardware filtering.
fn passes(
    pieces: Vec<Piece>,
    expanded: &[Expanded],
    dynamic: &DynamicParams,
    input_filterable: bool,
    intermediate_filterable: bool,
    fold: bool,
) -> Vec<PassPlan> {
    // A pass's input is the chain's input exactly when its segment starts
    // at the first stage.
    let filterable = |start: usize| {
        if start == 0 {
            input_filterable
        } else {
            intermediate_filterable
        }
    };
    let mut selected: Vec<(Segment, Option<SamplerFilter>)> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Color(segment) => selected.push((segment, None)),
            Piece::Spatial {
                plain,
                folded,
                manual,
                sampler,
                ..
            } => {
                // A filtered fold needs a filterable input, like any filtered
                // sample; a point-sampled fold works anywhere.
                let folded = match (fold, folded) {
                    (true, Some(folded))
                        if folded.sampler == SamplerFilter::Point
                            || filterable(folded.segment.stages.start) =>
                    {
                        Some(folded)
                    }
                    _ => None,
                };
                match (folded, filterable(plain.stages.start), manual) {
                    // The colour prefix runs inside the stage's samples:
                    // this pass replaces it.
                    (Some(folded), _, _) => {
                        selected.pop();
                        selected.push((folded.segment, Some(folded.sampler)));
                    }
                    // The input has no hardware filtering: texel loads
                    // implement the sample, bound with a point sampler.
                    (None, false, Some(manual)) => {
                        selected.push((*manual, Some(SamplerFilter::Point)));
                    }
                    (None, _, _) => selected.push((plain, Some(sampler))),
                }
            }
        }
    }

    let mut pass_of_stage = alloc::vec![0; expanded.len()];
    for (pass, (segment, _)) in selected.iter().enumerate() {
        for stage in segment.stages.clone() {
            pass_of_stage[stage] = pass;
        }
    }
    selected
        .into_iter()
        .map(|(segment, sampler)| {
            // A spatial piece is one stage — the last stage of its segment,
            // since a folded segment starts at its colour prefix; a colour
            // piece has no shape or auxiliary inputs.
            let (name, shape, aux) = match sampler {
                Some(_) => {
                    let stage = &expanded[segment.stages.end - 1];
                    let aux = stage
                        .aux
                        .iter()
                        .map(|aux| match *aux {
                            StageAux::Image(image) => PassAux::Image(image),
                            StageAux::Texture(image) => PassAux::Texture(image),
                            StageAux::StageInput(stage) => PassAux::PassInput(pass_of_stage[stage]),
                        })
                        .collect();
                    (stage.name, stage.shape, aux)
                }
                None => (expanded[segment.stages.start].name, None, Vec::new()),
            };
            let uniforms = segment
                .uniform
                .members
                .iter()
                .map(|member| {
                    let (param, components) = dynamic[&(member.stage, member.param.clone())];
                    UniformSlot {
                        offset: member.offset,
                        param,
                        components,
                    }
                })
                .collect();
            PassPlan {
                segment,
                name,
                sampler,
                shape,
                aux,
                uniforms,
            }
        })
        .collect()
}

/// Inserts the operating-space conversions and resolves auxiliary sources
/// against the whole chain.
fn expand<F: Filter>(declared: &[Declared]) -> Result<Vec<Expanded>, EffectSetupError> {
    let mut expanded = Vec::with_capacity(declared.len());
    let mut position = Vec::with_capacity(declared.len());
    for (index, stage) in declared.iter().enumerate() {
        let srgb = stage.space() == OperatingSpace::Srgb;
        if srgb {
            expanded.push(Expanded::color(Placed::new(&TO_SRGB)));
        }
        position.push(expanded.len());
        expanded.push(match stage {
            Declared::Color(placed) => Expanded::color(*placed),
            Declared::Spatial(placed) => {
                let aux = placed
                    .stage
                    .aux
                    .iter()
                    .map(|source| resolve_aux::<F>(declared, &position, index, placed, *source))
                    .collect::<Result<Vec<_>, _>>()?;
                Expanded {
                    kind: SnippetKind::Spatial,
                    name: placed.stage.name,
                    source: placed.stage.source,
                    params: placed.stage.params,
                    param_base: placed.param_base,
                    shape: placed.stage.shape,
                    aux,
                }
            }
        });
        if srgb {
            expanded.push(Expanded::color(Placed::new(&FROM_SRGB)));
        }
    }
    Ok(expanded)
}

fn resolve_aux<F: Filter>(
    declared: &[Declared],
    position: &[usize],
    index: usize,
    placed: &Placed<SpatialStage>,
    source: AuxSource,
) -> Result<StageAux, EffectSetupError> {
    match source {
        AuxSource::Image(image) | AuxSource::Texture(image) => {
            let image = placed.image_base + image;
            if image >= F::IMAGES {
                return Err(mismatch(
                    placed.stage.name,
                    format!("binds image {image}, but the filter provides {}", F::IMAGES),
                ));
            }
            Ok(match source {
                AuxSource::Texture(_) => StageAux::Texture(image),
                _ => StageAux::Image(image),
            })
        }
        AuxSource::PreviousStageInput => match index
            .checked_sub(1)
            .map(|previous| &declared[previous])
        {
            Some(Declared::Spatial(_)) => Ok(StageAux::StageInput(position[index - 1])),
            _ => Err(mismatch(
                placed.stage.name,
                "reads the previous stage's input, but the previous stage is not a spatial stage"
                    .to_string(),
            )),
        },
    }
}

/// Checks the declaration against the snippet's ABI.
fn check_inputs(stage: &Expanded, snippet: &Snippet) -> Result<(), EffectSetupError> {
    if stage.params.len() != snippet.params().len() {
        return Err(mismatch(
            stage.name,
            format!(
                "declares {} parameter sources, but the snippet's `Params` has {} members",
                stage.params.len(),
                snippet.params().len()
            ),
        ));
    }
    if stage.shape.is_some() != snippet.needs_shape() {
        return Err(mismatch(
            stage.name,
            format!(
                "declares shape input {:?}, but the snippet {} a `shape` argument",
                stage.shape,
                if snippet.needs_shape() {
                    "takes"
                } else {
                    "has no"
                }
            ),
        ));
    }
    let aux_images = usize::try_from(snippet.aux_images()).expect("aux image count fits in usize");
    if stage.aux.len() != aux_images {
        return Err(mismatch(
            stage.name,
            format!(
                "declares {} auxiliary images, but the snippet takes {}",
                stage.aux.len(),
                snippet.aux_images()
            ),
        ));
    }
    Ok(())
}

const fn components(ty: ParamType) -> usize {
    match ty {
        ParamType::F32 => 1,
        ParamType::Vec2 => 2,
        ParamType::Vec3 => 3,
        ParamType::Vec4 => 4,
    }
}

const fn param_value(ty: ParamType, values: &[f32]) -> Option<ParamValue> {
    Some(match (ty, values) {
        (ParamType::F32, &[x]) => ParamValue::F32(x),
        (ParamType::Vec2, &[x, y]) => ParamValue::Vec2([x, y]),
        (ParamType::Vec3, &[x, y, z]) => ParamValue::Vec3([x, y, z]),
        (ParamType::Vec4, &[x, y, z, w]) => ParamValue::Vec4([x, y, z, w]),
        _ => return None,
    })
}

const fn mismatch(stage: &'static str, reason: String) -> EffectSetupError {
    EffectSetupError::StageMismatch { stage, reason }
}
