//! Snippets: parsed, ABI-checked WGSL functions.

use std::collections::{HashMap, HashSet};

use naga::{
    Block, Expression, Function, FunctionArgument, Handle, ImageClass, ImageDimension, ImageQuery,
    Module, ScalarKind, Statement, Type, TypeInner, VectorSize,
    valid::{Capabilities, ValidationFlags, Validator},
};

use crate::{
    abi::{self, FoldBlocker, Param, ParamType, Precision, SamplerFilter, SnippetKind},
    errors::SnippetError,
    rewrite::{map_expression, read_block},
};

/// Which variant of a snippet a source is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Variant {
    /// The precision of colour values.
    pub precision: Precision,
    /// Whether the source uses subgroup operations.
    pub subgroups: bool,
}

impl Variant {
    /// The required base variant: `f32`, no subgroup operations.
    pub const BASE: Self = Self {
        precision: Precision::F32,
        subgroups: false,
    };

    pub(crate) fn capabilities(self) -> Capabilities {
        let mut capabilities = Capabilities::empty();
        if self.precision == Precision::F16 {
            capabilities |= Capabilities::SHADER_FLOAT16;
        }
        if self.subgroups {
            capabilities |= Capabilities::SUBGROUP;
        }
        capabilities
    }

    fn label(self) -> String {
        let precision = precision_name(self.precision);
        if self.subgroups {
            format!("{precision}, subgroups")
        } else {
            precision.to_owned()
        }
    }
}

/// The WGSL sources of one snippet.
#[derive(Debug, Clone)]
pub struct SnippetSource<'a> {
    name: &'a str,
    kind: SnippetKind,
    base: &'a str,
    others: Vec<(Variant, &'a str)>,
    libraries: Vec<LibrarySource<'a>>,
}

impl<'a> SnippetSource<'a> {
    /// A snippet with its required `f32` source.
    #[must_use]
    pub const fn new(name: &'a str, kind: SnippetKind, f32_source: &'a str) -> Self {
        Self {
            name,
            kind,
            base: f32_source,
            others: Vec::new(),
            libraries: Vec::new(),
        }
    }

    /// Declares another variant. Declaring [`Variant::BASE`] replaces the
    /// `f32` source; declaring any variant twice keeps the later source.
    #[must_use]
    pub fn variant(mut self, variant: Variant, source: &'a str) -> Self {
        if variant == Variant::BASE {
            self.base = source;
        } else {
            self.others.retain(|(existing, _)| *existing != variant);
            self.others.push((variant, source));
        }
        self
    }

    /// Registers a library the snippet may call by function name.
    ///
    /// A library is a WGSL source of plain helper functions — no `apply`,
    /// no globals, no entry points. The snippet's source may call the
    /// library's functions as if they were declared in it; the composer
    /// imports each referenced helper once per composed module, deduped by
    /// library identity.
    ///
    /// The library follows the same precision rule as the snippet: if the
    /// snippet declares an `f16` variant, the library needs one too.
    #[must_use]
    pub fn library(mut self, library: LibrarySource<'a>) -> Self {
        self.libraries.push(library);
        self
    }
}

/// The WGSL sources of one library of helper functions.
///
/// A library is not a stage: it defines no `apply` and carries no ABI. Its
/// functions keep their own names in a composed module, so a function name
/// may be defined by at most one registered library — and never by a
/// snippet that shares the module.
#[derive(Debug, Clone)]
pub struct LibrarySource<'a> {
    /// The library's identity name.
    pub name: &'a str,
    /// The `f32` source.
    pub f32: &'a str,
    /// The `f16` source. A snippet registering the library declares an
    /// `f16` variant only if the library has one.
    pub f16: Option<&'a str>,
}

impl<'a> LibrarySource<'a> {
    /// A library with its required `f32` source.
    #[must_use]
    pub const fn new(name: &'a str, f32: &'a str) -> Self {
        Self {
            name,
            f32,
            f16: None,
        }
    }

    /// Declares the library's `f16` source.
    #[must_use]
    pub const fn f16(mut self, source: &'a str) -> Self {
        self.f16 = Some(source);
        self
    }
}

/// One precision of a parsed library.
#[derive(Debug, Clone)]
pub struct LibraryVariant {
    /// The source with its `enable`/`requires`/`diagnostic` directives
    /// blanked, for concatenation into a snippet's combined source.
    pub source: String,
    /// The standalone parsed module — the canonical definitions the
    /// composer imports from.
    pub module: Module,
    /// The function names the variant defines.
    pub functions: Vec<String>,
}

/// A parsed, checked library.
#[derive(Debug, Clone)]
pub struct ParsedLibrary {
    name: String,
    f32: LibraryVariant,
    f16: Option<LibraryVariant>,
}

impl ParsedLibrary {
    fn parse(source: &LibrarySource<'_>) -> Result<Self, SnippetError> {
        Ok(Self {
            name: source.name.to_owned(),
            f32: parse_library(source.name, "f32", source.f32, Capabilities::empty())?,
            f16: source
                .f16
                .map(|wgsl| parse_library(source.name, "f16", wgsl, Capabilities::SHADER_FLOAT16))
                .transpose()?,
        })
    }

    #[must_use]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The library's variant at `precision`, `None` when it has none.
    #[must_use]
    pub const fn variant(&self, precision: Precision) -> Option<&LibraryVariant> {
        match precision {
            Precision::F32 => Some(&self.f32),
            Precision::F16 => self.f16.as_ref(),
        }
    }

    /// Whether `other` is the same library under the same name.
    #[must_use]
    pub(crate) fn same_library(&self, other: &Self) -> bool {
        self.f32.source == other.f32.source
            && self.f16.as_ref().map(|variant| &*variant.source)
                == other.f16.as_ref().map(|variant| &*variant.source)
    }
}

/// Parses and checks one library source.
fn parse_library(
    name: &str,
    label: &str,
    wgsl: &str,
    capabilities: Capabilities,
) -> Result<LibraryVariant, SnippetError> {
    let library_error = |reason: String| SnippetError::Library {
        name: name.to_owned(),
        variant: label.to_owned(),
        reason,
    };
    // The directives stay in place for the library's own parse — an f16
    // library needs its `enable f16;` to check — and are blanked only in
    // `source`, the text a snippet's combined source concatenates.
    let source = strip_directives(wgsl);
    let module = naga::front::wgsl::parse_str(wgsl)
        .map_err(|error| library_error(error.emit_to_string(wgsl)))?;
    Validator::new(ValidationFlags::all(), capabilities)
        .validate(&module)
        .map_err(|error| library_error(error.emit_to_string(wgsl)))?;
    check_module(&module, Variant::BASE)
        .map_err(|construct| library_error(format!("libraries may not use {construct}")))?;
    if let Some(name) = module.functions.iter().find_map(|(_, function)| {
        match function.name.as_deref() {
            // `apply` is a stage entry point; `main` collides with the
            // executor's own entry point.
            Some(name @ ("apply" | "main")) => Some(name),
            _ => None,
        }
    }) {
        return Err(library_error(format!("a library defines no `{name}`")));
    }
    Ok(LibraryVariant {
        source,
        functions: module
            .functions
            .iter()
            .filter_map(|(_, function)| function.name.clone())
            .collect(),
        module,
    })
}

/// Blanks the lines carrying `enable`, `requires` and `diagnostic`
/// directives, keeping line numbers so diagnostics still line up. A
/// directive is the only meaningful content of its line here: one whose
/// `;` follows the directive keyword on the same line.
fn strip_directives(wgsl: &str) -> String {
    let mut out = String::with_capacity(wgsl.len());
    for line in wgsl.split_inclusive('\n') {
        let stripped = line.trim_start();
        let directive = stripped.starts_with("enable ")
            || stripped.starts_with("requires ")
            || stripped.starts_with("diagnostic");
        if directive {
            if line.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

/// The names of every `fn` declaration in `wgsl` — a text scan for
/// collision checks; the parser still decides what parses.
fn wgsl_function_names(wgsl: &str) -> HashSet<String> {
    // Strip `//` line comments; WGSL has no block comments.
    let uncommented = wgsl
        .split('\n')
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut names = HashSet::new();
    let mut words = uncommented
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty());
    while let Some(word) = words.next() {
        if word == "fn"
            && let Some(name) = words.next()
        {
            names.insert(name.to_owned());
        }
    }
    names
}

/// How many times a spatial snippet samples its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleCount {
    /// A fixed number of samples per output pixel.
    Static(u32),
    /// The number depends on control flow (a loop) or on a helper function.
    Dynamic,
}

/// The ABI a snippet declares, identical across its variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Abi {
    pub kind: SnippetKind,
    pub params: Vec<Param>,
    pub working_space: bool,
    pub shape: bool,
    pub aux: u32,
    /// Spatial snippets: the filter mode `input_sampler` declares.
    pub sampler: Option<SamplerFilter>,
}

/// Where each optional ABI argument sits in the `apply` signature.
#[derive(Debug, Clone, Default)]
pub struct ArgSlots {
    pub params: Option<usize>,
    pub working_space: Option<usize>,
    pub shape: Option<usize>,
    pub aux: Vec<usize>,
}

/// One parsed variant.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub module: Module,
    pub apply: Handle<Function>,
    pub slots: ArgSlots,
    /// Spatial snippets: how often `input` is sampled.
    pub samples: SampleCount,
    /// Spatial snippets: why folding a colour prefix into `apply`'s accesses
    /// of `input` is not equivalent; `None` when the stage is foldable.
    pub not_foldable: Option<FoldBlocker>,
    /// The number of computed expressions in `apply` and every helper it
    /// calls, transitively — a proxy for the ALU cost of one invocation.
    pub ops: u32,
}

/// A parsed, ABI-checked snippet with all its declared variants.
#[derive(Debug, Clone)]
pub struct Snippet {
    name: String,
    abi: Abi,
    base: Parsed,
    others: Vec<(Variant, Parsed)>,
    libraries: Vec<ParsedLibrary>,
    /// Every function name the snippet's own sources define, in any
    /// variant.
    own_functions: HashSet<String>,
}

impl Snippet {
    /// Parses and checks every declared variant of a snippet.
    ///
    /// # Errors
    ///
    /// Returns an error when a source does not parse or validate, breaks the
    /// ABI, uses a construct snippets may not use, declares a different ABI
    /// than the `f32` source, or declares a function name a registered
    /// library also defines.
    pub fn parse(source: &SnippetSource<'_>) -> Result<Self, SnippetError> {
        let libraries = source
            .libraries
            .iter()
            .map(ParsedLibrary::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let reader = Reader {
            name: source.name,
            kind: source.kind,
            libraries: &libraries,
        };
        let (abi, base) = reader.variant(Variant::BASE, source.base)?;
        let mut others = Vec::with_capacity(source.others.len());
        for &(variant, wgsl) in &source.others {
            let (variant_abi, parsed) = reader.variant(variant, wgsl)?;
            if variant_abi != abi {
                return Err(SnippetError::VariantMismatch {
                    name: source.name.to_owned(),
                    variant: variant.label(),
                });
            }
            others.push((variant, parsed));
        }
        let mut own_functions = wgsl_function_names(source.base);
        for &(_, wgsl) in &source.others {
            own_functions.extend(wgsl_function_names(wgsl));
        }
        Ok(Self {
            name: source.name.to_owned(),
            abi,
            base,
            others,
            libraries,
            own_functions,
        })
    }

    /// The snippet name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the snippet does to pixels.
    #[must_use]
    pub const fn kind(&self) -> SnippetKind {
        self.abi.kind
    }

    /// The declared parameters, in `Params` member order.
    #[must_use]
    pub fn params(&self) -> &[Param] {
        &self.abi.params
    }

    /// Whether the snippet takes the working-space constants.
    #[must_use]
    pub const fn needs_working_space(&self) -> bool {
        self.abi.working_space
    }

    /// Whether the snippet takes the clip shape as input.
    #[must_use]
    pub const fn needs_shape(&self) -> bool {
        self.abi.shape
    }

    /// The number of auxiliary images the snippet takes.
    #[must_use]
    pub const fn aux_images(&self) -> u32 {
        self.abi.aux
    }

    /// The filter mode a spatial snippet declares for `input_sampler`
    /// (`None` for colour snippets).
    #[must_use]
    pub const fn sampler_filter(&self) -> Option<SamplerFilter> {
        self.abi.sampler
    }

    /// The declared variants, the base first.
    pub fn variants(&self) -> impl Iterator<Item = Variant> + '_ {
        core::iter::once(Variant::BASE).chain(self.others.iter().map(|(variant, _)| *variant))
    }

    pub(crate) const fn abi(&self) -> &Abi {
        &self.abi
    }

    /// The libraries the snippet registered.
    pub(crate) fn libraries(&self) -> &[ParsedLibrary] {
        &self.libraries
    }

    /// The function names the snippet's own sources define.
    #[must_use]
    pub const fn own_functions(&self) -> &HashSet<String> {
        &self.own_functions
    }

    /// The best declared variant for a request: the exact variant when
    /// declared, otherwise without subgroups, otherwise at `f32`.
    pub(crate) fn select(&self, want: Variant) -> (Variant, &Parsed) {
        let candidates = [
            want,
            Variant {
                subgroups: false,
                ..want
            },
            Variant {
                precision: Precision::F32,
                ..want
            },
        ];
        candidates
            .iter()
            .find_map(|candidate| {
                self.others
                    .iter()
                    .find(|(variant, _)| variant == candidate)
                    .map(|(variant, parsed)| (*variant, parsed))
            })
            .unwrap_or((Variant::BASE, &self.base))
    }
}

/// Parses the variants of one snippet.
struct Reader<'a> {
    name: &'a str,
    kind: SnippetKind,
    libraries: &'a [ParsedLibrary],
}

impl Reader<'_> {
    fn variant(&self, variant: Variant, wgsl: &str) -> Result<(Abi, Parsed), SnippetError> {
        let label = variant.label();
        let abi_error = |reason: String| SnippetError::Abi {
            name: self.name.to_owned(),
            variant: label.clone(),
            reason,
        };
        self.check_libraries(variant, &label, &wgsl_function_names(wgsl))?;
        // The snippet's calls to a library's functions parse as ordinary
        // calls once the library's source is appended — naga resolves
        // forward references, so order does not matter.
        let combined = self
            .libraries
            .iter()
            // The same library registered twice contributes its source once.
            .fold(
                (wgsl.to_owned(), Vec::new()),
                |(mut source, mut appended): (String, Vec<&ParsedLibrary>), library| {
                    if appended
                        .iter()
                        .any(|seen| seen.name() == library.name() && seen.same_library(library))
                    {
                        return (source, appended);
                    }
                    appended.push(library);
                    source.push('\n');
                    source.push_str(
                        &library
                            .variant(variant.precision)
                            .expect("checked above")
                            .source,
                    );
                    (source, appended)
                },
            )
            .0;
        let module =
            naga::front::wgsl::parse_str(&combined).map_err(|error| SnippetError::Parse {
                name: self.name.to_owned(),
                variant: label.clone(),
                message: error.emit_to_string(&combined),
            })?;
        Validator::new(ValidationFlags::all(), variant.capabilities())
            .validate(&module)
            .map_err(|error| SnippetError::Invalid {
                name: self.name.to_owned(),
                variant: label.clone(),
                message: error.emit_to_string(&combined),
            })?;
        check_module(&module, variant).map_err(|construct| SnippetError::Unsupported {
            name: self.name.to_owned(),
            variant: label.clone(),
            construct,
        })?;

        let apply = module
            .functions
            .iter()
            .find(|(_, function)| function.name.as_deref() == Some("apply"))
            .map(|(handle, _)| handle)
            .ok_or_else(|| abi_error("defines no function named `apply`".to_owned()))?;
        let function = &module.functions[apply];
        let (required, filter) = self
            .check_required(&module, function, variant.precision)
            .map_err(abi_error)?;
        let (slots, params) = self
            .read_optional(&module, &function.arguments[required..], required)
            .map_err(abi_error)?;
        if self.kind == SnippetKind::Spatial {
            check_size_query(&module, apply).map_err(abi_error)?;
        }

        let (samples, not_foldable) = match self.kind {
            SnippetKind::Spatial => {
                count_samples(&module, apply, filter.expect("spatial declares one"))
            }
            SnippetKind::Color => (SampleCount::Static(0), None),
        };
        let ops = total_ops(&module, apply);

        let abi = Abi {
            kind: self.kind,
            params,
            working_space: slots.working_space.is_some(),
            shape: slots.shape.is_some(),
            aux: u32::try_from(slots.aux.len()).unwrap_or(u32::MAX),
            sampler: filter,
        };
        Ok((
            abi,
            Parsed {
                module,
                apply,
                slots,
                samples,
                not_foldable,
                ops,
            },
        ))
    }

    /// Every registered library must have a source at `variant`'s
    /// precision, and no library may define a name the snippet — or another
    /// library — defines: the combined source would not parse, and even
    /// otherwise the composer could not tell the copies apart.
    fn check_libraries(
        &self,
        variant: Variant,
        label: &str,
        own: &HashSet<String>,
    ) -> Result<(), SnippetError> {
        let abi_error = |reason: String| SnippetError::Abi {
            name: self.name.to_owned(),
            variant: label.to_owned(),
            reason,
        };
        for library in self.libraries {
            if library.variant(variant.precision).is_none() {
                return Err(abi_error(format!(
                    "variant {label} requires the {} source of library `{}`, which declares none",
                    precision_name(variant.precision),
                    library.name()
                )));
            }
        }
        for (first, a) in self.libraries.iter().enumerate() {
            let a_variant = a.variant(variant.precision).expect("checked above");
            for function in &a_variant.functions {
                if own.contains(function) {
                    return Err(abi_error(format!(
                        "`{function}` is defined by both the snippet and library `{}`",
                        a.name()
                    )));
                }
            }
            for b in &self.libraries[first + 1..] {
                // The same library registered twice is one library, not a
                // collision with itself.
                if a.name() == b.name() && a.same_library(b) {
                    continue;
                }
                let b_variant = b.variant(variant.precision).expect("checked above");
                if let Some(function) = b_variant
                    .functions
                    .iter()
                    .find(|function| a_variant.functions.contains(function))
                {
                    return Err(abi_error(format!(
                        "`{function}` is defined by both library `{}` and library `{}`",
                        a.name(),
                        b.name()
                    )));
                }
            }
        }
        Ok(())
    }
}

fn check_module(module: &Module, variant: Variant) -> Result<(), &'static str> {
    if !module.global_variables.is_empty() {
        return Err("global variables (inputs are function arguments)");
    }
    if !module.overrides.is_empty() {
        return Err("override declarations");
    }
    if !module.entry_points.is_empty() {
        return Err("entry points");
    }
    for (_, function) in module.functions.iter() {
        check_constructs(function, variant.subgroups)?;
    }
    Ok(())
}

impl Reader<'_> {
    /// Checks the required leading arguments and the result. Returns the
    /// number of required arguments and, for spatial snippets, the filter
    /// mode the sampler argument declares.
    ///
    /// The sampler argument declares its filter mode by name:
    /// `input_point_sampler` requires the executor to bind a nearest (point)
    /// sampler, `input_sampler` allows a filtering one.
    fn check_required(
        &self,
        module: &Module,
        function: &Function,
        precision: Precision,
    ) -> Result<(usize, Option<SamplerFilter>), String> {
        let colour = colour_type(precision);
        let uv = ParamType::Vec2.inner();
        let texture = texture_2d();
        // (argument index, name, type); the spatial sampler argument is
        // checked separately because its name carries the filter mode.
        let required: Vec<(usize, &str, &TypeInner)> = match self.kind {
            SnippetKind::Color => vec![(0, "color", &colour)],
            SnippetKind::Spatial => vec![(0, "input", &texture), (2, "uv", &uv), (3, "size", &uv)],
        };
        let count = required.last().map_or(0, |(index, ..)| index + 1);
        if function.arguments.len() < count {
            return Err(format!(
                "`apply` takes {} arguments; the {:?} ABI requires at least {}",
                function.arguments.len(),
                self.kind,
                count
            ));
        }
        for (index, name, expected) in &required {
            let argument = &function.arguments[*index];
            if argument.name.as_deref() != Some(*name)
                || module.types[argument.ty].inner != **expected
            {
                return Err(format!("argument {index} must be `{name}` of the ABI type"));
            }
        }
        let sampler = if self.kind == SnippetKind::Spatial {
            let argument = &function.arguments[1];
            let filter = match argument.name.as_deref() {
                Some("input_point_sampler") => SamplerFilter::Point,
                Some("input_sampler") => SamplerFilter::Filtered,
                other => {
                    return Err(format!(
                        "argument 1 must be `input_point_sampler` (nearest sampling) or `input_sampler` (filtering allowed), found `{}`",
                        other.unwrap_or("?")
                    ));
                }
            };
            if module.types[argument.ty].inner != (TypeInner::Sampler { comparison: false }) {
                return Err("argument 1 must be a `sampler`".to_owned());
            }
            Some(filter)
        } else {
            None
        };
        let result_ok = function
            .result
            .as_ref()
            .is_some_and(|result| module.types[result.ty].inner == colour);
        if !result_ok {
            return Err(format!(
                "`apply` must return vec4<{}>",
                precision_name(precision)
            ));
        }
        Ok((count, sampler))
    }

    /// Reads the optional arguments after the required ones.
    fn read_optional(
        &self,
        module: &Module,
        arguments: &[FunctionArgument],
        first: usize,
    ) -> Result<(ArgSlots, Vec<Param>), String> {
        let mut slots = ArgSlots::default();
        let mut params = Vec::new();
        for (offset, argument) in arguments.iter().enumerate() {
            let slot = first + offset;
            let inner = &module.types[argument.ty].inner;
            let spatial = self.kind == SnippetKind::Spatial;
            match argument.name.as_deref() {
                Some("params") if slots.params.is_none() => {
                    params = read_params(module, inner)?;
                    slots.params = Some(slot);
                }
                Some("space") if slots.working_space.is_none() => {
                    if !abi::is_working_space(module, argument.ty) {
                        return Err(format!(
                            "`space` must be a struct exactly equivalent to `{}` — same members, offsets and span; the name is free",
                            abi::WORKING_SPACE_WGSL
                        ));
                    }
                    slots.working_space = Some(slot);
                }
                Some("shape") if spatial && slots.shape.is_none() => {
                    if !is_texture_2d(inner) {
                        return Err("`shape` must be texture_2d<f32>".to_owned());
                    }
                    slots.shape = Some(slot);
                }
                Some(other) if spatial && other.starts_with("aux") => {
                    let expected = format!("aux{}", slots.aux.len());
                    if other != expected || !is_texture_2d(inner) {
                        return Err(format!(
                            "auxiliary images are `aux0`, `aux1`, … of type texture_2d<f32>, in order; found `{other}`, expected `{expected}`"
                        ));
                    }
                    slots.aux.push(slot);
                }
                other => {
                    return Err(format!("unexpected argument `{}`", other.unwrap_or("?")));
                }
            }
        }
        Ok((slots, params))
    }
}

fn read_params(module: &Module, inner: &TypeInner) -> Result<Vec<Param>, String> {
    let TypeInner::Struct { members, .. } = inner else {
        return Err("`params` must be a struct".to_owned());
    };
    members
        .iter()
        .map(|member| {
            let name = member
                .name
                .clone()
                .ok_or_else(|| "parameters must be named".to_owned())?;
            let ty = ParamType::from_inner(&module.types[member.ty].inner).ok_or_else(|| {
                format!("parameter `{name}` must be f32, vec2<f32>, vec3<f32> or vec4<f32>")
            })?;
            Ok(Param { name, ty })
        })
        .collect()
}

pub const fn colour_type(precision: Precision) -> TypeInner {
    TypeInner::Vector {
        size: VectorSize::Quad,
        scalar: precision.scalar(),
    }
}

pub fn colour_type_handle(module: &mut Module, precision: Precision) -> Handle<Type> {
    module.types.insert(
        Type {
            name: None,
            inner: colour_type(precision),
        },
        naga::Span::UNDEFINED,
    )
}

const fn precision_name(precision: Precision) -> &'static str {
    match precision {
        Precision::F32 => "f32",
        Precision::F16 => "f16",
    }
}

pub const fn texture_2d() -> TypeInner {
    TypeInner::Image {
        dim: ImageDimension::D2,
        arrayed: false,
        class: ImageClass::Sampled {
            kind: ScalarKind::Float,
            multi: false,
        },
    }
}

pub fn is_texture_2d(inner: &TypeInner) -> bool {
    *inner == texture_2d()
}

/// Rejects every construct outside the snippet contract.
fn check_constructs(function: &Function, subgroups: bool) -> Result<(), &'static str> {
    for (_, expression) in function.expressions.iter() {
        match *expression {
            Expression::AtomicResult { .. } => return Err("atomics"),
            Expression::WorkGroupUniformLoadResult { .. } => return Err("workgroup memory"),
            Expression::ArrayLength(_) => return Err("runtime-sized arrays"),
            Expression::RayQueryVertexPositions { .. }
            | Expression::RayQueryProceedResult
            | Expression::RayQueryGetIntersection { .. } => return Err("ray queries"),
            Expression::CooperativeLoad { .. } | Expression::CooperativeMultiplyAdd { .. } => {
                return Err("cooperative matrices");
            }
            Expression::SubgroupBallotResult | Expression::SubgroupOperationResult { .. }
                if !subgroups =>
            {
                return Err("subgroup operations outside a subgroup variant");
            }
            _ => {}
        }
    }
    check_block(&function.body, subgroups)
}

fn check_block(block: &Block, subgroups: bool) -> Result<(), &'static str> {
    for statement in block {
        match statement {
            Statement::Block(inner) => check_block(inner, subgroups)?,
            Statement::If { accept, reject, .. } => {
                check_block(accept, subgroups)?;
                check_block(reject, subgroups)?;
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    check_block(&case.body, subgroups)?;
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                check_block(body, subgroups)?;
                check_block(continuing, subgroups)?;
            }
            Statement::Kill => return Err("discard"),
            Statement::ControlBarrier(_) | Statement::MemoryBarrier(_) => return Err("barriers"),
            Statement::ImageStore { .. } => return Err("image stores"),
            Statement::Atomic { .. } | Statement::ImageAtomic { .. } => return Err("atomics"),
            Statement::WorkGroupUniformLoad { .. } => return Err("workgroup memory"),
            Statement::RayQuery { .. } | Statement::RayPipelineFunction(_) => {
                return Err("ray tracing");
            }
            Statement::CooperativeStore { .. } => return Err("cooperative matrices"),
            Statement::SubgroupBallot { .. }
            | Statement::SubgroupGather { .. }
            | Statement::SubgroupCollectiveOperation { .. }
                if !subgroups =>
            {
                return Err("subgroup operations outside a subgroup variant");
            }
            Statement::Emit(_)
            | Statement::Break
            | Statement::Continue
            | Statement::Return { .. }
            | Statement::Store { .. }
            | Statement::Call { .. }
            | Statement::SubgroupBallot { .. }
            | Statement::SubgroupGather { .. }
            | Statement::SubgroupCollectiveOperation { .. } => {}
        }
    }
    Ok(())
}

/// The expression that reads argument `index`, if the function reads it.
pub fn argument_expression(function: &Function, index: u32) -> Option<Handle<Expression>> {
    function
        .expressions
        .iter()
        .find(|(_, expression)| **expression == Expression::FunctionArgument(index))
        .map(|(handle, _)| handle)
}

/// Rejects `textureDimensions` on the stage `input` — argument 3, `size`,
/// carries the extent now. The check is transitive: a helper that queries a
/// texture argument bound to `input` at the call is a size query of `input`
/// all the same. Auxiliary images and `shape` keep their own queries.
fn check_size_query(module: &Module, apply: Handle<Function>) -> Result<(), String> {
    let mut queue: Vec<(Handle<Function>, Vec<u32>)> = vec![(apply, vec![0])];
    let mut seen = HashSet::new();
    while let Some((handle, positions)) = queue.pop() {
        if !seen.insert((handle, positions.clone())) {
            continue;
        }
        let function = &module.functions[handle];
        let inputs: Vec<Handle<Expression>> = positions
            .iter()
            .filter_map(|&position| argument_expression(function, position))
            .collect();
        for (_, expression) in function.expressions.iter() {
            if let Expression::ImageQuery {
                image,
                query: ImageQuery::Size { .. },
            } = *expression
                && inputs.contains(&image)
            {
                return Err(
                    "`textureDimensions` may not read `input`'s extent; the `size` argument carries it"
                        .to_owned(),
                );
            }
        }
        read_block(&function.body, &mut |statement| {
            if let Statement::Call {
                function: callee,
                ref arguments,
                ..
            } = *statement
            {
                let bound: Vec<u32> = arguments
                    .iter()
                    .enumerate()
                    .filter(|&(_, argument)| inputs.contains(argument))
                    .map(|(index, _)| u32::try_from(index).expect("argument index fits in u32"))
                    .collect();
                if !bound.is_empty() {
                    queue.push((callee, bound));
                }
            }
        });
    }
    Ok(())
}

/// Counts how often `apply` accesses argument 0 (`input`), and whether
/// folding a colour prefix into every access is equivalent.
///
/// The count is transitive: `input` bound to a helper's argument is tracked
/// through the call, and the helper's own samples of it count toward the
/// total, as does a sample it makes of the declared sampler.
///
/// Folding is equivalent only when every access returns an exact texel of
/// `input`: an `ImageLoad`, or an `ImageSample` that does not gather, does
/// not compare a depth reference, and goes through the declared point
/// sampler. A filtered sample computes `prefix(lerp(a, b))` where
/// materializing first computes `lerp(prefix(a), prefix(b))`; a gather would
/// apply the prefix to a gathered component vector.
fn count_samples(
    module: &Module,
    apply: Handle<Function>,
    filter: SamplerFilter,
) -> (SampleCount, Option<FoldBlocker>) {
    let mut analysis = SampleAnalysis {
        module,
        filter,
        done: HashMap::new(),
        in_progress: HashSet::new(),
    };
    match analysis.count(apply, &[0], &[1]) {
        Some(Count {
            samples,
            dynamic,
            blocker,
            ..
        }) => {
            let count = if dynamic {
                SampleCount::Dynamic
            } else {
                SampleCount::Static(samples)
            };
            (count, blocker)
        }
        // WGSL admits no recursion, so a call cycle cannot arise; if it
        // did, folding could not follow `input` through it.
        None => (SampleCount::Dynamic, Some(FoldBlocker::UnknownUse)),
    }
}

/// One function's part of the transitive sample count.
#[derive(Clone, Copy)]
struct Count {
    /// Samples of `input` taken per call into the subtree.
    samples: u32,
    /// Whether the subtree accesses `input` at all.
    any: bool,
    /// Whether a sample may execute inside a loop.
    dynamic: bool,
    /// Why folding the subtree is not equivalent.
    blocker: Option<FoldBlocker>,
}

/// `(function, bound input positions, bound sampler positions)` — one
/// transitive count's key.
type Bound = (Handle<Function>, Vec<u32>, Vec<u32>);

/// A transitive sample count in progress, memoized per function and bound
/// argument positions.
struct SampleAnalysis<'m> {
    module: &'m Module,
    /// The filter mode `apply`'s declared sampler carries.
    filter: SamplerFilter,
    /// What each bound key counted.
    done: HashMap<Bound, Count>,
    /// Keys under analysis — a call cycle cannot be counted.
    in_progress: HashSet<Bound>,
}

impl<'m> SampleAnalysis<'m> {
    /// The count for `function` given the caller's `input` binds to argument
    /// positions `inputs` and its declared sampler to `samplers`.
    fn count(
        &mut self,
        function: Handle<Function>,
        inputs: &[u32],
        samplers: &[u32],
    ) -> Option<Count> {
        let mut bound_inputs = inputs.to_vec();
        bound_inputs.sort_unstable();
        let mut bound_samplers = samplers.to_vec();
        bound_samplers.sort_unstable();
        let key = (function, bound_inputs, bound_samplers);
        if let Some(&count) = self.done.get(&key) {
            return Some(count);
        }
        if !self.in_progress.insert(key.clone()) {
            return None;
        }
        let count = self.try_count(function, &key.1, &key.2);
        self.in_progress.remove(&key);
        self.done.insert(key, count);
        Some(count)
    }

    fn try_count(&mut self, handle: Handle<Function>, inputs: &[u32], samplers: &[u32]) -> Count {
        let function = &self.module.functions[handle];
        let input_exprs: Vec<Handle<Expression>> = inputs
            .iter()
            .filter_map(|&index| argument_expression(function, index))
            .collect();
        let sampler_exprs: Vec<Handle<Expression>> = samplers
            .iter()
            .filter_map(|&index| argument_expression(function, index))
            .collect();

        let mut count = Count {
            samples: 0,
            any: false,
            dynamic: false,
            blocker: function.expressions.iter().find_map(|(_, expression)| {
                access_blocker(expression, &input_exprs, &sampler_exprs, self.filter)
            }),
        };
        self.block(
            function,
            &function.body,
            &input_exprs,
            &sampler_exprs,
            false,
            &mut count,
        );
        count
    }

    fn block(
        &mut self,
        function: &'m Function,
        block: &'m Block,
        inputs: &[Handle<Expression>],
        samplers: &[Handle<Expression>],
        in_loop: bool,
        count: &mut Count,
    ) {
        for statement in block {
            match statement {
                Statement::Emit(range) => {
                    for handle in range.clone() {
                        if inputs
                            .iter()
                            .any(|&input| samples_input(&function.expressions[handle], input))
                        {
                            count.samples = count.samples.saturating_add(1);
                            count.any = true;
                            count.dynamic |= in_loop;
                        }
                    }
                }
                Statement::Call {
                    function: callee,
                    arguments,
                    ..
                } => {
                    let bound = |targets: &[Handle<Expression>]| -> Vec<u32> {
                        arguments
                            .iter()
                            .enumerate()
                            .filter(|&(_, argument)| targets.contains(argument))
                            .map(|(index, _)| {
                                u32::try_from(index).expect("argument index fits in u32")
                            })
                            .collect()
                    };
                    let bound_inputs = bound(inputs);
                    if bound_inputs.is_empty() {
                        continue;
                    }
                    let bound_samplers = bound(samplers);
                    match self.count(*callee, &bound_inputs, &bound_samplers) {
                        Some(child) => {
                            count.samples = count.samples.saturating_add(child.samples);
                            count.any |= child.any;
                            count.dynamic |= child.dynamic || (in_loop && child.any);
                            count.blocker = count.blocker.or(child.blocker);
                        }
                        None => count.blocker = Some(FoldBlocker::UnknownUse),
                    }
                }
                Statement::Block(inner) => {
                    self.block(function, inner, inputs, samplers, in_loop, count);
                }
                Statement::If { accept, reject, .. } => {
                    self.block(function, accept, inputs, samplers, in_loop, count);
                    self.block(function, reject, inputs, samplers, in_loop, count);
                }
                Statement::Switch { cases, .. } => {
                    for case in cases {
                        self.block(function, &case.body, inputs, samplers, in_loop, count);
                    }
                }
                Statement::Loop {
                    body, continuing, ..
                } => {
                    self.block(function, body, inputs, samplers, true, count);
                    self.block(function, continuing, inputs, samplers, true, count);
                }
                Statement::ImageStore { image, .. } | Statement::ImageAtomic { image, .. }
                    if inputs.contains(image) =>
                {
                    count.blocker = Some(FoldBlocker::UnknownUse);
                }
                _ => {}
            }
        }
    }
}

/// Why `expression`, if it accesses an `inputs` expression, makes folding
/// non-equivalent. `samplers` are the argument expressions the declared
/// point sampler binds to.
fn access_blocker(
    expression: &Expression,
    inputs: &[Handle<Expression>],
    samplers: &[Handle<Expression>],
    filter: SamplerFilter,
) -> Option<FoldBlocker> {
    match *expression {
        // `textureDimensions(input)` is rejected outright; any other query
        // of `input` is an access the folder cannot reproduce.
        Expression::ImageQuery { image, .. } if inputs.contains(&image) => {
            Some(FoldBlocker::UnknownUse)
        }
        Expression::ImageLoad { image, .. } if inputs.contains(&image) => None,
        Expression::ImageSample {
            image,
            gather,
            depth_ref,
            sampler: used,
            ..
        } if inputs.contains(&image) => {
            if gather.is_some() {
                Some(FoldBlocker::Gather)
            } else if depth_ref.is_some() {
                Some(FoldBlocker::DepthComparison)
            } else if !samplers.contains(&used) {
                Some(FoldBlocker::UnknownUse)
            } else if filter == SamplerFilter::Filtered {
                Some(FoldBlocker::FilteringSampler)
            } else {
                None
            }
        }
        ref other if inputs.iter().any(|&input| reads(other, input)) => {
            Some(FoldBlocker::UnknownUse)
        }
        _ => None,
    }
}

/// Whether `expression` reads `operand`, in any operand position.
pub fn reads(expression: &Expression, operand: Handle<Expression>) -> bool {
    let mut found = false;
    let _ = map_expression(expression, |handle| {
        found |= handle == operand;
        handle
    });
    found
}

/// Every function `apply` can reach through `Statement::Call`s,
/// transitively, including `apply` itself.
pub fn reachable_handles(module: &Module, apply: Handle<Function>) -> HashSet<Handle<Function>> {
    let mut seen = HashSet::from([apply]);
    let mut queue = vec![apply];
    while let Some(handle) = queue.pop() {
        collect_calls(&module.functions[handle].body, &mut queue);
        queue.retain(|next| seen.insert(*next));
    }
    seen
}

/// The number of computed expressions in `apply` and every function it
/// calls, transitively.
fn total_ops(module: &Module, apply: Handle<Function>) -> u32 {
    let mut ops = 0u32;
    let mut seen = HashSet::new();
    let mut queue = vec![apply];
    while let Some(handle) = queue.pop() {
        if !seen.insert(handle) {
            continue;
        }
        let function = &module.functions[handle];
        ops = ops.saturating_add(
            u32::try_from(
                function
                    .expressions
                    .iter()
                    .filter(|(_, expression)| !expression.needs_pre_emit())
                    .count(),
            )
            .unwrap_or(u32::MAX),
        );
        collect_calls(&function.body, &mut queue);
    }
    ops
}

pub fn collect_calls(block: &Block, calls: &mut Vec<Handle<Function>>) {
    for statement in block {
        match statement {
            Statement::Call { function, .. } => calls.push(*function),
            Statement::Block(inner) => collect_calls(inner, calls),
            Statement::If { accept, reject, .. } => {
                collect_calls(accept, calls);
                collect_calls(reject, calls);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    collect_calls(&case.body, calls);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                collect_calls(body, calls);
                collect_calls(continuing, calls);
            }
            _ => {}
        }
    }
}

/// Whether `expression` samples or loads the texture read by `input`.
pub fn samples_input(expression: &Expression, input: Handle<Expression>) -> bool {
    match *expression {
        Expression::ImageSample { image, .. } | Expression::ImageLoad { image, .. } => {
            image == input
        }
        _ => false,
    }
}
