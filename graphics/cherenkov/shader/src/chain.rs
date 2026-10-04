//! Composing a chain of snippets into one module.

use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    ops::Range,
};

use naga::{
    BinaryOperator, Expression, Function, FunctionArgument, Handle, Literal, MathFunction, Module,
    Scalar, ScalarKind, StructMember, Type, TypeInner, VectorSize,
    compact::{KeepUnused, compact},
    valid::{Capabilities, ModuleInfo, ValidationFlags, Validator},
};

use crate::{
    abi::{self, FoldBlocker, ParamType, ParamValue, Precision, SamplerFilter, SnippetKind},
    builder::FunctionBuilder,
    errors::ComposeError,
    import::{GENERATED, Importer, identifier},
    parse::{self, ArgSlots, ParsedLibrary, SampleCount, Snippet, Variant},
    rewrite,
};

/// One stage of a chain: a snippet and the parameters specialized to constants.
#[derive(Debug, Clone)]
pub struct Stage<'s> {
    snippet: &'s Snippet,
    constants: Vec<(String, ParamValue)>,
}

impl<'s> Stage<'s> {
    /// A stage whose parameters are all dynamic (read from the uniform block).
    #[must_use]
    pub const fn new(snippet: &'s Snippet) -> Self {
        Self {
            snippet,
            constants: Vec::new(),
        }
    }

    /// Specializes a parameter to a constant. It is removed from the uniform
    /// block and becomes a literal at the call.
    #[must_use]
    pub fn constant(mut self, param: &str, value: impl Into<ParamValue>) -> Self {
        self.constants.push((param.to_owned(), value.into()));
        self
    }
}

/// What to compose for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComposeOptions {
    /// The precision of the colour values segments take and return. Each
    /// stage uses its variant of this precision when declared.
    pub precision: Precision,
    /// Use the subgroup variant of each stage that declares one.
    pub subgroups: bool,
}

impl Default for ComposeOptions {
    fn default() -> Self {
        Self {
            precision: Precision::F32,
            subgroups: false,
        }
    }
}

/// One argument of a segment function, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SegmentArg {
    /// The colour, `vec4` at the composition's precision.
    Color,
    /// The input texture, `texture_2d<f32>`.
    Input,
    /// The input sampler.
    InputSampler,
    /// The sample coordinate, `vec2<f32>`.
    Uv,
    /// The segment's uniform block (see [`Segment::uniform`]).
    Params,
    /// The clip shape, `texture_2d<f32>`.
    Shape,
    /// Auxiliary image `n`, `texture_2d<f32>`.
    Aux(u32),
    /// The working-space constants.
    WorkingSpace,
}

/// One dynamic parameter in a segment's uniform block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniformMember {
    /// The stage index in the chain.
    pub stage: usize,
    /// The parameter name.
    pub param: String,
    /// The parameter type.
    pub ty: ParamType,
    /// The byte offset in the block.
    pub offset: u32,
}

/// The uniform block of a segment, laid out by WGSL uniform rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UniformLayout {
    /// Spatial segments only: the byte offset of the `size` member, the
    /// `vec2<f32>` carrying the segment's input extent in pixels. It is
    /// the block's first member, always at offset 0.
    pub input_size: Option<u32>,
    /// The dynamic parameters, in offset order.
    pub members: Vec<UniformMember>,
    /// The block size in bytes; zero when every parameter is constant.
    pub size: u32,
}

/// One generated function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// The function's name in the module.
    pub function: String,
    /// The chain stages it applies.
    pub stages: Range<usize>,
    /// The variant each stage used, in stage order.
    pub variants: Vec<Variant>,
    /// Its arguments, in order.
    pub args: Vec<SegmentArg>,
    /// Its uniform block.
    pub uniform: UniformLayout,
}

/// The cost parameters of folding a colour prefix into a spatial stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FoldCost {
    /// Computed expressions in the prefix, applied once per sample.
    pub prefix_ops: u32,
    /// Samples the spatial stage takes per output pixel.
    pub samples: SampleCount,
}

/// A spatial segment with the preceding colour piece folded into its samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folded {
    /// The folded function. It replaces the preceding colour piece and the
    /// plain spatial segment.
    pub segment: Segment,
    /// What folding costs.
    pub cost: FoldCost,
    /// The filter mode the snippet declares for `SegmentArg::InputSampler`.
    /// The executor must honor it when binding the folded program's sampler
    /// — for a sampled stage it is [`SamplerFilter::Point`], the contract
    /// that makes folding equivalent.
    pub sampler: SamplerFilter,
}

/// One piece of a composed chain. Boundaries between pieces are the possible
/// materialization points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// Consecutive colour stages.
    Color(Segment),
    /// One spatial stage.
    Spatial {
        /// The spatial stage alone.
        plain: Segment,
        /// When a colour piece precedes it and the stage is foldable: the
        /// alternative that applies that piece to each access of `input`
        /// instead of materializing it.
        folded: Option<Folded>,
        /// For a stage that samples `input` through a filtering sampler: the
        /// same stage with those samples implemented by texel loads — the
        /// executor's alternative when the input's format has no hardware
        /// filtering. `None` for a point-sampled stage, and when the stage
        /// uses `input` in ways that cannot be substituted.
        ///
        /// Its [`Segment::args`] are identical to `plain`'s; the executor
        /// binds a non-filtering sampler and an unfilterable-float input to
        /// it.
        manual: Option<Box<Segment>>,
        /// The filter mode the snippet declares for `SegmentArg::InputSampler`;
        /// the executor must honor it for `plain` and `folded`.
        sampler: SamplerFilter,
        /// Why the stage cannot fold a colour prefix; `None` when it can.
        not_foldable: Option<FoldBlocker>,
    },
}

/// A composed, validated module and its normalized description.
#[derive(Debug, Clone)]
pub struct Composition {
    module: Module,
    info: ModuleInfo,
    capabilities: Capabilities,
    pieces: Vec<Piece>,
}

impl Composition {
    /// The module.
    #[must_use]
    pub const fn module(&self) -> &Module {
        &self.module
    }

    /// The validation result for [`Self::module`].
    #[must_use]
    pub const fn info(&self) -> &ModuleInfo {
        &self.info
    }

    /// The validator capabilities the module needs.
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// The pieces, in chain order.
    #[must_use]
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// The function a segment names.
    #[must_use]
    pub fn function(&self, segment: &Segment) -> Option<Handle<Function>> {
        self.module
            .functions
            .iter()
            .find(|(_, function)| function.name.as_deref() == Some(segment.function.as_str()))
            .map(|(handle, _)| handle)
    }

    /// The module, its capabilities and the pieces, for an executor that adds
    /// entry points and re-validates.
    pub fn into_parts(self) -> (Module, Capabilities, Vec<Piece>) {
        (self.module, self.capabilities, self.pieces)
    }
}

/// Composes a chain.
///
/// # Errors
///
/// Returns an error when the chain is empty, when a constant names an
/// unknown parameter, has the wrong type or is given twice, or, as a
/// composer defect, when the composed module does not validate.
///
/// # Panics
///
/// Panics when a spatial stage lacks the sampler declaration that
/// `Snippet::parse` requires — a composer defect.
pub fn compose(stages: &[Stage<'_>], options: ComposeOptions) -> Result<Composition, ComposeError> {
    if stages.is_empty() {
        return Err(ComposeError::EmptyChain);
    }
    let bindings = stages
        .iter()
        .enumerate()
        .map(|(index, stage)| resolve_constants(index, stage))
        .collect::<Result<Vec<_>, _>>()?;
    check_libraries(stages, options)?;

    let mut composer = Composer::new(options);
    for (index, stage) in stages.iter().enumerate() {
        composer.import(index, stage.snippet, bindings[index].clone());
    }

    let mut pieces: Vec<Piece> = Vec::new();
    let mut index = 0;
    while index < stages.len() {
        match stages[index].snippet.kind() {
            SnippetKind::Color => {
                let end = (index..stages.len())
                    .find(|&next| stages[next].snippet.kind() != SnippetKind::Color)
                    .unwrap_or(stages.len());
                pieces.push(Piece::Color(composer.color_segment(index..end)));
                index = end;
            }
            SnippetKind::Spatial => {
                let plain = composer.spatial_segment(index);
                let manual = composer.manual_segment(index);
                let sampler = composer.stages[index]
                    .sampler
                    .expect("a spatial stage declares a sampler");
                let not_foldable = composer.stages[index].not_foldable;
                let folded = match pieces.last() {
                    Some(Piece::Color(prefix)) if not_foldable.is_none() => {
                        Some(composer.folded_segment(prefix.stages.clone(), index))
                    }
                    _ => None,
                };
                pieces.push(Piece::Spatial {
                    plain,
                    folded,
                    manual: manual.map(Box::new),
                    sampler,
                    not_foldable,
                });
                index += 1;
            }
        }
    }

    composer.finish(pieces)
}

/// Rejects function-name collisions between the composition's registered
/// libraries — and between a library and a snippet — and two libraries
/// that share a name but are not the same library.
fn check_libraries(stages: &[Stage<'_>], options: ComposeOptions) -> Result<(), ComposeError> {
    let mut libraries: HashMap<&str, &ParsedLibrary> = HashMap::new();
    for stage in stages {
        for library in stage.snippet.libraries() {
            match libraries.entry(library.name()) {
                Entry::Occupied(registered) => {
                    if !registered.get().same_library(library) {
                        return Err(ComposeError::DuplicateLibrary {
                            name: library.name().to_owned(),
                        });
                    }
                }
                Entry::Vacant(slot) => {
                    slot.insert(library);
                }
            }
        }
    }
    // A library function carries its own name into the composed module, so
    // the name may come from only one origin — and only the variant each
    // stage actually imports can collide: an f16-only name never meets an
    // f32 name at f32.
    let want = Variant {
        precision: options.precision,
        subgroups: options.subgroups,
    };
    let mut owners: HashMap<&str, String> = HashMap::new();
    for stage in stages {
        let (variant, _) = stage.snippet.select(want);
        for library in stage.snippet.libraries() {
            let lib_variant = library
                .variant(variant.precision)
                .expect("snippet parsing checked the variant exists");
            let owner = format!("library `{}`", library.name());
            for function in &lib_variant.functions {
                match owners.entry(function.as_str()) {
                    Entry::Occupied(existing) => {
                        // Both precisions of one library are one origin.
                        if existing.get() != &owner {
                            return Err(ComposeError::LibraryConflict {
                                name: function.clone(),
                                first: existing.get().clone(),
                                second: owner,
                            });
                        }
                    }
                    Entry::Vacant(slot) => {
                        slot.insert(owner.clone());
                    }
                }
            }
        }
    }
    for stage in stages {
        let (variant, parsed) = stage.snippet.select(want);
        // The parsed module also holds the libraries' sources — exclude
        // them: a snippet's own function is what is left.
        let imported: HashSet<&str> = stage
            .snippet
            .libraries()
            .iter()
            .filter_map(|library| library.variant(variant.precision))
            .flat_map(|variant| variant.functions.iter().map(String::as_str))
            .collect();
        for (_, function) in parsed.module.functions.iter() {
            let Some(name) = function.name.as_deref() else {
                continue;
            };
            if imported.contains(name) {
                continue;
            }
            if let Some(owner) = owners.get(name) {
                return Err(ComposeError::LibraryConflict {
                    name: name.to_owned(),
                    first: owner.clone(),
                    second: format!("snippet `{}`", stage.snippet.name()),
                });
            }
        }
    }
    Ok(())
}

fn resolve_constants(
    index: usize,
    stage: &Stage<'_>,
) -> Result<Vec<Option<ParamValue>>, ComposeError> {
    let params = stage.snippet.params();
    let mut values = vec![None; params.len()];
    for (name, value) in &stage.constants {
        let slot = params
            .iter()
            .position(|param| param.name == *name)
            .ok_or_else(|| ComposeError::UnknownParam {
                stage: index,
                snippet: stage.snippet.name().to_owned(),
                param: name.clone(),
            })?;
        if params[slot].ty != value.ty() {
            return Err(ComposeError::ParamType {
                stage: index,
                snippet: stage.snippet.name().to_owned(),
                param: name.clone(),
                expected: params[slot].ty,
                found: value.ty(),
            });
        }
        if values[slot].replace(*value).is_some() {
            return Err(ComposeError::DuplicateConstant {
                stage: index,
                snippet: stage.snippet.name().to_owned(),
                param: name.clone(),
            });
        }
    }
    Ok(values)
}

/// A stage's snippet function, imported into the composed module.
#[derive(Debug, Clone)]
struct Imported {
    variant: Variant,
    apply: Handle<Function>,
    argument_count: usize,
    slots: ArgSlots,
    params: Vec<(String, ParamType)>,
    params_ty: Option<Handle<Type>>,
    constants: Vec<Option<ParamValue>>,
    shape: bool,
    aux: u32,
    working_space: bool,
    samples: SampleCount,
    not_foldable: Option<FoldBlocker>,
    sampler: Option<SamplerFilter>,
    ops: u32,
}

impl Imported {
    const fn precision(&self) -> Precision {
        self.variant.precision
    }
}

struct Composer<'a> {
    module: Module,
    options: ComposeOptions,
    capabilities: Capabilities,
    stages: Vec<Imported>,
    working_space: Handle<Type>,
    /// `(library name, precision, function name)` to the canonical import
    /// of that library function — the one copy every stage shares.
    library_functions: HashMap<(String, Precision, String), Handle<Function>>,
    /// `(library name, precision)` to the importer sharing that library's
    /// copies — one per composed module, so a callee reached through two
    /// imports lands once.
    library_importers: HashMap<(String, Precision), Importer<'a>>,
    /// The manual bilinear, built on first use — shared by every manual
    /// segment.
    bilinear: Option<Handle<Function>>,
    next_name: usize,
}

/// A segment's uniform block.
struct UniformBlock {
    layout: UniformLayout,
    ty: Option<Handle<Type>>,
    /// Stage and parameter index to the member index in the block.
    members: HashMap<(usize, usize), u32>,
}

/// The expressions a generated function reads its inputs from.
#[derive(Default, Clone)]
struct Inputs {
    params: Option<Handle<Expression>>,
    working_space: Option<Handle<Expression>>,
    shape: Option<Handle<Expression>>,
    aux: Vec<Handle<Expression>>,
    /// Spatial functions: `input`'s extent, read from the params block.
    size: Option<Handle<Expression>>,
    /// Stage and parameter index to the member index in the params block.
    members: HashMap<(usize, usize), u32>,
}

impl<'a> Composer<'a> {
    fn new(options: ComposeOptions) -> Self {
        let mut module = Module::default();
        let working_space = abi::insert_working_space(&mut module);
        Self {
            module,
            options,
            capabilities: Capabilities::empty(),
            stages: Vec::new(),
            working_space,
            library_functions: HashMap::new(),
            library_importers: HashMap::new(),
            bilinear: None,
            next_name: 0,
        }
    }

    /// The canonical import of `function` from `library` at `precision` —
    /// imported once, under its own name, however many stages call it. The
    /// library's importer is shared for the whole module, so every callee
    /// an import reaches is canonical too, not copied per call site.
    fn library_function(
        &mut self,
        library: &'a ParsedLibrary,
        precision: Precision,
        function: &str,
    ) -> Handle<Function> {
        let key = (library.name().to_owned(), precision, function.to_owned());
        if let Some(&handle) = self.library_functions.get(&key) {
            return handle;
        }
        let variant = library
            .variant(precision)
            .expect("snippet parsing checked the variant exists");
        let importer = self
            .library_importers
            .entry((library.name().to_owned(), precision))
            // The empty prefix keeps the functions' own names — they are the
            // one definition of each name in the module.
            .or_insert_with(|| Importer::new(&variant.module, ""));
        let local = variant
            .module
            .functions
            .iter()
            .find(|(_, candidate)| candidate.name.as_deref() == Some(function))
            .map(|(handle, _)| handle)
            .expect("the name came from the library's function list");
        let handle = importer.function(&mut self.module, local);
        for (source, mapped) in importer.imported_functions() {
            if let Some(name) = variant.module.functions[source].name.clone() {
                self.library_functions
                    .insert((library.name().to_owned(), precision, name), mapped);
            }
        }
        handle
    }

    fn import(&mut self, index: usize, snippet: &'a Snippet, constants: Vec<Option<ParamValue>>) {
        let want = Variant {
            precision: self.options.precision,
            subgroups: self.options.subgroups,
        };
        let (variant, parsed) = snippet.select(want);
        self.capabilities |= variant.capabilities();
        let mut importer = Importer::new(&parsed.module, &format!("s{index}_{}", snippet.name()));
        // The snippet's working-space block is exactly equivalent to the
        // composer's canonical one — parsing rejects it otherwise — so it
        // maps onto the canonical handle whatever name it carries.
        if let Some(slot) = parsed.slots.working_space {
            let declared = parsed.module.functions[parsed.apply].arguments[slot].ty;
            importer.alias_type(declared, self.working_space);
        }
        // Each library function `apply` reaches maps to the library's one
        // canonical import — however many stages call it, the composed
        // module holds exactly one copy.
        let reachable = parse::reachable_handles(&parsed.module, parsed.apply);
        for library in snippet.libraries() {
            let lib_variant = library
                .variant(variant.precision)
                .expect("snippet parsing checked the variant exists");
            for (local, function) in parsed.module.functions.iter() {
                let Some(name) = function.name.clone() else {
                    continue;
                };
                if reachable.contains(&local) && lib_variant.functions.contains(&name) {
                    let canonical = self.library_function(library, variant.precision, &name);
                    importer.alias_function(local, canonical);
                }
            }
        }
        let apply = importer.function(&mut self.module, parsed.apply);
        let function = &self.module.functions[apply];
        let params_ty = parsed.slots.params.map(|slot| function.arguments[slot].ty);
        let abi = snippet.abi();
        self.stages.push(Imported {
            variant,
            apply,
            argument_count: function.arguments.len(),
            slots: parsed.slots.clone(),
            params: abi
                .params
                .iter()
                .map(|param| (param.name.clone(), param.ty))
                .collect(),
            params_ty,
            constants,
            shape: abi.shape,
            aux: abi.aux,
            working_space: abi.working_space,
            samples: parsed.samples,
            not_foldable: parsed.not_foldable,
            sampler: abi.sampler,
            ops: parsed.ops,
        });
    }

    fn name(&mut self, kind: &str) -> String {
        let name = format!("cherenkov_{kind}_{}", self.next_name);
        self.next_name += 1;
        name
    }

    fn ty(&mut self, inner: TypeInner) -> Handle<Type> {
        self.module
            .types
            .insert(Type { name: None, inner }, GENERATED)
    }

    /// Lays out the dynamic parameters of `stages` and declares the block
    /// type. `input_size` — for spatial segments — prepends the `size`
    /// member, the `vec2<f32>` extent of the segment's input in pixels, at
    /// offset 0.
    fn uniform(&mut self, stages: &[usize], name: &str, input_size: bool) -> UniformBlock {
        let mut layout = UniformLayout::default();
        let mut members = Vec::new();
        let mut index = HashMap::new();
        let mut offset = 0u32;
        if input_size {
            members.push(StructMember {
                name: Some("size".to_owned()),
                ty: self.ty(ParamType::Vec2.inner()),
                binding: None,
                offset,
            });
            layout.input_size = Some(offset);
            offset += ParamType::Vec2.size();
        }
        for &stage in stages {
            for (param, (param_name, ty)) in
                self.stages[stage].params.clone().into_iter().enumerate()
            {
                if self.stages[stage].constants[param].is_some() {
                    continue;
                }
                offset = offset.next_multiple_of(ty.align());
                index.insert(
                    (stage, param),
                    u32::try_from(members.len()).expect("member count fits in u32"),
                );
                members.push(StructMember {
                    name: Some(format!("s{stage}_{}", identifier(&param_name))),
                    ty: self.ty(ty.inner()),
                    binding: None,
                    offset,
                });
                layout.members.push(UniformMember {
                    stage,
                    param: param_name,
                    ty,
                    offset,
                });
                offset += ty.size();
            }
        }
        if members.is_empty() {
            return UniformBlock {
                layout,
                ty: None,
                members: index,
            };
        }
        layout.size = offset.next_multiple_of(16);
        let ty = self.module.types.insert(
            Type {
                name: Some(format!("{name}_params")),
                inner: TypeInner::Struct {
                    members,
                    span: layout.size,
                },
            },
            GENERATED,
        );
        UniformBlock {
            layout,
            ty: Some(ty),
            members: index,
        }
    }

    /// The `Params` value a stage's `apply` takes, from constants and the block.
    fn stage_params(
        &mut self,
        builder: &mut FunctionBuilder,
        stage: usize,
        inputs: &Inputs,
    ) -> Option<Handle<Expression>> {
        let params_ty = self.stages[stage].params_ty?;
        let constants = self.stages[stage].constants.clone();
        let mut components = Vec::with_capacity(constants.len());
        for (param, constant) in constants.into_iter().enumerate() {
            components.push(self.param_component(builder, stage, param, constant, inputs));
        }
        Some(builder.expression(Expression::Compose {
            ty: params_ty,
            components,
        }))
    }

    /// One member of a stage's `Params`: its constant, or its block member.
    fn param_component(
        &mut self,
        builder: &mut FunctionBuilder,
        stage: usize,
        param: usize,
        constant: Option<ParamValue>,
        inputs: &Inputs,
    ) -> Handle<Expression> {
        if let Some(value) = constant {
            return self.constant(builder, value);
        }
        let base = inputs
            .params
            .expect("a dynamic parameter implies a params block");
        builder.expression(Expression::AccessIndex {
            base,
            index: inputs.members[&(stage, param)],
        })
    }

    fn constant(&mut self, builder: &mut FunctionBuilder, value: ParamValue) -> Handle<Expression> {
        if let ParamValue::F32(scalar) = value {
            return builder.f32(scalar);
        }
        let components = value
            .components()
            .iter()
            .map(|&component| builder.expression(Expression::Literal(Literal::F32(component))))
            .collect();
        let ty = self.ty(value.ty().inner());
        builder.expression(Expression::Compose { ty, components })
    }

    /// Arguments for a stage's `apply`, given the leading required ones.
    fn stage_arguments(
        &mut self,
        builder: &mut FunctionBuilder,
        stage: usize,
        required: &[Handle<Expression>],
        inputs: &Inputs,
    ) -> Vec<Handle<Expression>> {
        let params = self.stage_params(builder, stage, inputs);
        let imported = &self.stages[stage];
        let mut arguments: Vec<Option<Handle<Expression>>> = vec![None; imported.argument_count];
        for (slot, &value) in required.iter().enumerate() {
            arguments[slot] = Some(value);
        }
        let slots = &imported.slots;
        if let Some(slot) = slots.params {
            arguments[slot] = params;
        }
        if let Some(slot) = slots.working_space {
            arguments[slot] = inputs.working_space;
        }
        if let Some(slot) = slots.shape {
            arguments[slot] = inputs.shape;
        }
        for (aux, &slot) in slots.aux.iter().enumerate() {
            arguments[slot] = inputs.aux.get(aux).copied();
        }
        arguments
            .into_iter()
            .map(|argument| argument.expect("every `apply` argument is an ABI slot"))
            .collect()
    }

    /// Applies consecutive colour stages to `colour`.
    fn apply_colour(
        &mut self,
        builder: &mut FunctionBuilder,
        stages: Range<usize>,
        mut colour: Handle<Expression>,
        mut precision: Precision,
        inputs: &Inputs,
    ) -> (Handle<Expression>, Precision) {
        for stage in stages {
            let stage_precision = self.stages[stage].precision();
            colour = builder.convert(colour, precision, stage_precision);
            precision = stage_precision;
            let arguments = self.stage_arguments(builder, stage, &[colour], inputs);
            colour = builder.call(self.stages[stage].apply, arguments);
        }
        (colour, precision)
    }

    fn needs_working_space(&self, stages: &[usize]) -> bool {
        stages.iter().any(|&stage| self.stages[stage].working_space)
    }

    /// Declares the params block and working-space arguments, in that order.
    fn trailing_inputs(
        &self,
        builder: &mut FunctionBuilder,
        args: &mut Vec<SegmentArg>,
        block: &UniformBlock,
        working_space: bool,
        inputs: &mut Inputs,
    ) {
        if let Some(ty) = block.ty {
            inputs.params = Some(builder.argument("params", ty));
            args.push(SegmentArg::Params);
            // `size` is the block's first member when the layout carries one.
            if block.layout.input_size.is_some() {
                inputs.size = Some(builder.expression(Expression::AccessIndex {
                    base: inputs.params.expect("declared above"),
                    index: 0,
                }));
            }
        }
        inputs.members.clone_from(&block.members);
        if working_space {
            inputs.working_space = Some(builder.argument("space", self.working_space));
            args.push(SegmentArg::WorkingSpace);
        }
    }

    fn color_segment(&mut self, stages: Range<usize>) -> Segment {
        let name = self.name("segment");
        let indices: Vec<usize> = stages.clone().collect();
        let block = self.uniform(&indices, &name, false);
        let precision = self.options.precision;
        let colour_ty = parse::colour_type_handle(&mut self.module, precision);

        let mut builder = FunctionBuilder::new(name.clone());
        let mut args = vec![SegmentArg::Color];
        let colour = builder.argument("color", colour_ty);
        let mut inputs = Inputs::default();
        let working_space = self.needs_working_space(&indices);
        self.trailing_inputs(&mut builder, &mut args, &block, working_space, &mut inputs);
        let (result, result_precision) =
            self.apply_colour(&mut builder, stages.clone(), colour, precision, &inputs);
        let result = builder.convert(result, result_precision, precision);
        let function = builder.finish(result, colour_ty);
        self.module.functions.append(function, GENERATED);

        Segment {
            function: name,
            variants: indices
                .iter()
                .map(|&stage| self.stages[stage].variant)
                .collect(),
            stages,
            args,
            uniform: block.layout,
        }
    }

    /// Declares the spatial inputs (texture, sampler, coordinate) and the
    /// optional shape and auxiliary images.
    fn spatial_inputs(
        &mut self,
        builder: &mut FunctionBuilder,
        stage: usize,
        args: &mut Vec<SegmentArg>,
        inputs: &mut Inputs,
    ) -> [Handle<Expression>; 3] {
        let texture = self.ty(parse::texture_2d());
        let sampler = self.ty(TypeInner::Sampler { comparison: false });
        let uv = self.ty(ParamType::Vec2.inner());
        let sampler_name = match self.stages[stage].sampler {
            Some(SamplerFilter::Point) => "input_point_sampler",
            _ => "input_sampler",
        };
        let required = [
            builder.argument("input", texture),
            builder.argument(sampler_name, sampler),
            builder.argument("uv", uv),
        ];
        args.extend([SegmentArg::Input, SegmentArg::InputSampler, SegmentArg::Uv]);
        if self.stages[stage].shape {
            inputs.shape = Some(builder.argument("shape", texture));
            args.push(SegmentArg::Shape);
        }
        for aux in 0..self.stages[stage].aux {
            inputs
                .aux
                .push(builder.argument(&format!("aux{aux}"), texture));
            args.push(SegmentArg::Aux(aux));
        }
        required
    }

    fn spatial_segment(&mut self, stage: usize) -> Segment {
        let apply = self.stages[stage].apply;
        self.spatial_segment_with(stage, apply)
    }

    /// The stage's plain segment, wrapping `apply` — which is the stage's own
    /// `apply`, or its substituted copy for [`Piece::Spatial::manual`].
    fn spatial_segment_with(&mut self, stage: usize, apply: Handle<Function>) -> Segment {
        let name = self.name("segment");
        let block = self.uniform(&[stage], &name, true);
        let precision = self.options.precision;
        let colour_ty = parse::colour_type_handle(&mut self.module, precision);

        let mut builder = FunctionBuilder::new(name.clone());
        let mut args = Vec::new();
        let mut inputs = Inputs::default();
        let required = self.spatial_inputs(&mut builder, stage, &mut args, &mut inputs);
        let working_space = self.stages[stage].working_space;
        self.trailing_inputs(&mut builder, &mut args, &block, working_space, &mut inputs);
        let size = inputs.size.expect("a spatial segment declares `size`");
        let required = [required[0], required[1], required[2], size];
        let arguments = self.stage_arguments(&mut builder, stage, &required, &inputs);
        let result = builder.call(apply, arguments);
        let result = builder.convert(result, self.stages[stage].precision(), precision);
        let function = builder.finish(result, colour_ty);
        self.module.functions.append(function, GENERATED);

        Segment {
            function: name,
            stages: stage..stage + 1,
            variants: vec![self.stages[stage].variant],
            args,
            uniform: block.layout,
        }
    }

    /// The stage's segment with filtered samples of `input` implemented by
    /// texel loads — `None` for a point-sampled stage, and when its samples
    /// cannot be substituted.
    fn manual_segment(&mut self, stage: usize) -> Option<Segment> {
        if self.stages[stage].sampler != Some(SamplerFilter::Filtered) {
            return None;
        }
        let bilinear = self.manual_bilinear();
        let apply = rewrite::substitute_samples(
            &mut self.module,
            self.stages[stage].apply,
            0,
            Some(1),
            3,
            bilinear,
        )?;
        Some(self.spatial_segment_with(stage, apply))
    }

    /// The manual bilinear every substituted sample calls:
    /// `fn(input: texture_2d<f32>, uv: vec2<f32>, size: vec2<f32>) ->
    /// vec4<f32>` — edge-clamped texel loads with the same weights hardware
    /// filtering uses. `size` is `input`'s extent in pixels.
    fn manual_bilinear(&mut self) -> Handle<Function> {
        if let Some(bilinear) = self.bilinear {
            return bilinear;
        }
        let texture = self.ty(parse::texture_2d());
        let vec2f = self.ty(ParamType::Vec2.inner());
        let coord_ty = self.ty(TypeInner::Vector {
            size: VectorSize::Bi,
            scalar: Scalar::I32,
        });
        let colour = parse::colour_type_handle(&mut self.module, Precision::F32);

        let mut builder = FunctionBuilder::new(self.name("manual_bilinear"));
        let input = builder.argument("input", texture);
        let uv = builder.argument("uv", vec2f);
        let size = builder.argument("size", vec2f);
        let scaled = builder.expression(Expression::Binary {
            op: BinaryOperator::Multiply,
            left: uv,
            right: size,
        });
        let half = builder.f32(0.5);
        let halves = builder.expression(Expression::Compose {
            ty: vec2f,
            components: vec![half, half],
        });
        let position = builder.expression(Expression::Binary {
            op: BinaryOperator::Subtract,
            left: scaled,
            right: halves,
        });
        let base = builder.expression(Expression::Math {
            fun: MathFunction::Floor,
            arg: position,
            arg1: None,
            arg2: None,
            arg3: None,
        });
        let t = builder.expression(Expression::Binary {
            op: BinaryOperator::Subtract,
            left: position,
            right: base,
        });
        let base_i32 = builder.expression(Expression::As {
            expr: base,
            kind: ScalarKind::Sint,
            convert: Some(4),
        });
        // Pixel extents are whole numbers, so truncation loses nothing.
        let span = builder.expression(Expression::As {
            expr: size,
            kind: ScalarKind::Sint,
            convert: Some(4),
        });
        let zero = builder.expression(Expression::Literal(Literal::I32(0)));
        let lo = builder.expression(Expression::Compose {
            ty: coord_ty,
            components: vec![zero, zero],
        });
        let one = builder.expression(Expression::Literal(Literal::I32(1)));
        let ones = builder.expression(Expression::Compose {
            ty: coord_ty,
            components: vec![one, one],
        });
        let hi = builder.expression(Expression::Binary {
            op: BinaryOperator::Subtract,
            left: span,
            right: ones,
        });
        let level = builder.expression(Expression::Literal(Literal::I32(0)));
        let corners = BilinearCorners {
            input,
            base: base_i32,
            lo,
            hi,
            coord_ty,
            level,
        };
        let c00 = bilinear_corner(&mut builder, &corners, [0, 0]);
        let c10 = bilinear_corner(&mut builder, &corners, [1, 0]);
        let c01 = bilinear_corner(&mut builder, &corners, [0, 1]);
        let c11 = bilinear_corner(&mut builder, &corners, [1, 1]);
        let tx = builder.expression(Expression::AccessIndex { base: t, index: 0 });
        let ty = builder.expression(Expression::AccessIndex { base: t, index: 1 });
        let x0 = bilinear_mix(&mut builder, c00, c10, tx);
        let x1 = bilinear_mix(&mut builder, c01, c11, tx);
        let result = bilinear_mix(&mut builder, x0, x1, ty);
        let bilinear = self
            .module
            .functions
            .append(builder.finish(result, colour), GENERATED);
        self.bilinear = Some(bilinear);
        bilinear
    }

    fn folded_segment(&mut self, prefix: Range<usize>, spatial: usize) -> Folded {
        let name = self.name("folded");
        let mut indices: Vec<usize> = prefix.clone().collect();
        indices.push(spatial);
        let block = self.uniform(&indices, &name, true);
        let prefix_indices: Vec<usize> = prefix.clone().collect();
        let prefix_space = self.needs_working_space(&prefix_indices);

        // The prefix as a function of one sample: f32 in and out, since samples are f32.
        let prefix_name = self.name("prefix");
        let sample_ty = parse::colour_type_handle(&mut self.module, Precision::F32);
        let mut builder = FunctionBuilder::new(prefix_name);
        let mut prefix_args = Vec::new();
        let sample = builder.argument("color", sample_ty);
        let mut inputs = Inputs::default();
        self.trailing_inputs(
            &mut builder,
            &mut prefix_args,
            &block,
            prefix_space,
            &mut inputs,
        );
        let (result, result_precision) = self.apply_colour(
            &mut builder,
            prefix.clone(),
            sample,
            Precision::F32,
            &inputs,
        );
        let result = builder.convert(result, result_precision, Precision::F32);
        let prefix_function = self
            .module
            .functions
            .append(builder.finish(result, sample_ty), GENERATED);

        // The spatial apply with every sample of `input` passed through the prefix.
        let mut extra = Vec::new();
        if let Some(ty) = block.ty {
            extra.push(FunctionArgument {
                name: Some("prefix_params".to_owned()),
                ty,
                binding: None,
            });
        }
        if prefix_space {
            extra.push(FunctionArgument {
                name: Some("prefix_space".to_owned()),
                ty: self.working_space,
                binding: None,
            });
        }
        let spatial_apply = self.stages[spatial].apply;
        let folded_name = self.name("folded_apply");
        let folded_apply = rewrite::fold_prefix(
            &mut self.module,
            spatial_apply,
            0,
            prefix_function,
            &extra,
            folded_name,
        );
        let folded_apply = self.module.functions.append(folded_apply, GENERATED);

        // The segment: spatial inputs, one block for prefix and spatial, the working space.
        let precision = self.options.precision;
        let colour_ty = parse::colour_type_handle(&mut self.module, precision);
        let mut builder = FunctionBuilder::new(name.clone());
        let mut args = Vec::new();
        let mut inputs = Inputs::default();
        let required = self.spatial_inputs(&mut builder, spatial, &mut args, &mut inputs);
        let working_space = self.needs_working_space(&indices);
        self.trailing_inputs(&mut builder, &mut args, &block, working_space, &mut inputs);
        let size = inputs.size.expect("a spatial segment declares `size`");
        let required = [required[0], required[1], required[2], size];
        let mut arguments = self.stage_arguments(&mut builder, spatial, &required, &inputs);
        if block.ty.is_some() {
            arguments.push(inputs.params.expect("declared above"));
        }
        if prefix_space {
            arguments.push(inputs.working_space.expect("declared above"));
        }
        let result = builder.call(folded_apply, arguments);
        let result = builder.convert(result, self.stages[spatial].precision(), precision);
        self.module
            .functions
            .append(builder.finish(result, colour_ty), GENERATED);

        Folded {
            segment: Segment {
                function: name,
                stages: prefix.start..spatial + 1,
                variants: indices
                    .iter()
                    .map(|&stage| self.stages[stage].variant)
                    .collect(),
                args,
                uniform: block.layout,
            },
            cost: FoldCost {
                prefix_ops: prefix.map(|stage| self.stages[stage].ops).sum(),
                samples: self.stages[spatial].samples,
            },
            sampler: self.stages[spatial]
                .sampler
                .expect("a spatial stage declares a sampler"),
        }
    }

    fn finish(mut self, pieces: Vec<Piece>) -> Result<Composition, ComposeError> {
        let invalid = |error: naga::WithSpan<naga::valid::ValidationError>| {
            ComposeError::Invalid(format!("{error}: {:?}", error.as_inner()))
        };
        Validator::new(ValidationFlags::all(), self.capabilities)
            .validate(&self.module)
            .map_err(invalid)?;
        compact(&mut self.module, KeepUnused::Yes);
        let info = Validator::new(ValidationFlags::all(), self.capabilities)
            .validate(&self.module)
            .map_err(invalid)?;
        Ok(Composition {
            module: self.module,
            info,
            capabilities: self.capabilities,
            pieces,
        })
    }
}

/// The shared operands of the four bilinear corner loads.
struct BilinearCorners {
    /// The `input` texture.
    input: Handle<Expression>,
    /// `floor(position)` as `vec2<i32>`.
    base: Handle<Expression>,
    /// `vec2<i32>(0, 0)`.
    lo: Handle<Expression>,
    /// `size - 1` as `vec2<i32>`.
    hi: Handle<Expression>,
    /// The `vec2<i32>` type.
    coord_ty: Handle<Type>,
    /// The mip level literal `0`.
    level: Handle<Expression>,
}

/// One bilinear corner:
/// `textureLoad(input, clamp(base + offset, 0, size - 1), 0)`.
fn bilinear_corner(
    builder: &mut FunctionBuilder,
    corners: &BilinearCorners,
    [dx, dy]: [i32; 2],
) -> Handle<Expression> {
    let BilinearCorners {
        input,
        base,
        lo,
        hi,
        coord_ty,
        level,
    } = *corners;
    let dx = builder.expression(Expression::Literal(Literal::I32(dx)));
    let dy = builder.expression(Expression::Literal(Literal::I32(dy)));
    let offset = builder.expression(Expression::Compose {
        ty: coord_ty,
        components: vec![dx, dy],
    });
    let at = builder.expression(Expression::Binary {
        op: BinaryOperator::Add,
        left: base,
        right: offset,
    });
    let coordinate = builder.expression(Expression::Math {
        fun: MathFunction::Clamp,
        arg: at,
        arg1: Some(lo),
        arg2: Some(hi),
        arg3: None,
    });
    builder.expression(Expression::ImageLoad {
        image: input,
        coordinate,
        array_index: None,
        sample: None,
        level: Some(level),
    })
}

/// `mix(a, b, t)` on colour handles — the bilinear weights.
fn bilinear_mix(
    builder: &mut FunctionBuilder,
    a: Handle<Expression>,
    b: Handle<Expression>,
    t: Handle<Expression>,
) -> Handle<Expression> {
    builder.expression(Expression::Math {
        fun: MathFunction::Mix,
        arg: a,
        arg1: Some(b),
        arg2: Some(t),
        arg3: None,
    })
}
