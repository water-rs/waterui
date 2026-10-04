//! IR rewrites: mapping expression handles, folding a colour prefix into
//! every sample a spatial function takes of its input, and substituting the
//! implementation of filtered samples.

use std::collections::{HashMap, HashSet};

use naga::{
    Block, CooperativeData, Expression, Function, FunctionArgument, GatherMode, Handle, ImageQuery,
    Literal, Module, Range, SampleLevel, Statement, SwitchCase, Type,
};

use crate::{abi::ParamType, import::GENERATED, parse};

/// Rebuilds `expression` with every expression operand passed through `map`.
/// Handles into other arenas (types, constants, functions) are kept.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per naga expression variant; the match stays exhaustive so a new variant fails to compile"
)]
pub fn map_expression(
    expression: &Expression,
    mut map: impl FnMut(Handle<Expression>) -> Handle<Expression>,
) -> Expression {
    match *expression {
        Expression::Literal(_)
        | Expression::Constant(_)
        | Expression::Override(_)
        | Expression::ZeroValue(_)
        | Expression::FunctionArgument(_)
        | Expression::GlobalVariable(_)
        | Expression::LocalVariable(_)
        | Expression::CallResult(_)
        | Expression::AtomicResult { .. }
        | Expression::WorkGroupUniformLoadResult { .. }
        | Expression::RayQueryProceedResult
        | Expression::SubgroupBallotResult
        | Expression::SubgroupOperationResult { .. } => expression.clone(),
        Expression::Compose { ty, ref components } => Expression::Compose {
            ty,
            components: components.iter().map(|&component| map(component)).collect(),
        },
        Expression::Access { base, index } => Expression::Access {
            base: map(base),
            index: map(index),
        },
        Expression::AccessIndex { base, index } => Expression::AccessIndex {
            base: map(base),
            index,
        },
        Expression::Splat { size, value } => Expression::Splat {
            size,
            value: map(value),
        },
        Expression::Swizzle {
            size,
            vector,
            pattern,
        } => Expression::Swizzle {
            size,
            vector: map(vector),
            pattern,
        },
        Expression::Load { pointer } => Expression::Load {
            pointer: map(pointer),
        },
        Expression::ImageSample {
            image,
            sampler,
            gather,
            coordinate,
            array_index,
            offset,
            level,
            depth_ref,
            clamp_to_edge,
        } => {
            let level = match level {
                SampleLevel::Auto => SampleLevel::Auto,
                SampleLevel::Zero => SampleLevel::Zero,
                SampleLevel::Exact(value) => SampleLevel::Exact(map(value)),
                SampleLevel::Bias(value) => SampleLevel::Bias(map(value)),
                SampleLevel::Gradient { x, y } => SampleLevel::Gradient {
                    x: map(x),
                    y: map(y),
                },
            };
            Expression::ImageSample {
                image: map(image),
                sampler: map(sampler),
                gather,
                coordinate: map(coordinate),
                array_index: array_index.map(&mut map),
                offset: offset.map(&mut map),
                level,
                depth_ref: depth_ref.map(&mut map),
                clamp_to_edge,
            }
        }
        Expression::ImageLoad {
            image,
            coordinate,
            array_index,
            sample,
            level,
        } => Expression::ImageLoad {
            image: map(image),
            coordinate: map(coordinate),
            array_index: array_index.map(&mut map),
            sample: sample.map(&mut map),
            level: level.map(&mut map),
        },
        Expression::ImageQuery { image, query } => Expression::ImageQuery {
            image: map(image),
            query: match query {
                ImageQuery::Size { level } => ImageQuery::Size {
                    level: level.map(&mut map),
                },
                other => other,
            },
        },
        Expression::Unary { op, expr } => Expression::Unary {
            op,
            expr: map(expr),
        },
        Expression::Binary { op, left, right } => Expression::Binary {
            op,
            left: map(left),
            right: map(right),
        },
        Expression::Select {
            condition,
            accept,
            reject,
        } => Expression::Select {
            condition: map(condition),
            accept: map(accept),
            reject: map(reject),
        },
        Expression::Derivative { axis, ctrl, expr } => Expression::Derivative {
            axis,
            ctrl,
            expr: map(expr),
        },
        Expression::Relational { fun, argument } => Expression::Relational {
            fun,
            argument: map(argument),
        },
        Expression::Math {
            fun,
            arg,
            arg1,
            arg2,
            arg3,
        } => Expression::Math {
            fun,
            arg: map(arg),
            arg1: arg1.map(&mut map),
            arg2: arg2.map(&mut map),
            arg3: arg3.map(&mut map),
        },
        Expression::As {
            expr,
            kind,
            convert,
        } => Expression::As {
            expr: map(expr),
            kind,
            convert,
        },
        Expression::ArrayLength(array) => Expression::ArrayLength(map(array)),
        Expression::RayQueryVertexPositions { query, committed } => {
            Expression::RayQueryVertexPositions {
                query: map(query),
                committed,
            }
        }
        Expression::RayQueryGetIntersection { query, committed } => {
            Expression::RayQueryGetIntersection {
                query: map(query),
                committed,
            }
        }
        Expression::CooperativeLoad {
            columns,
            rows,
            role,
            data,
        } => Expression::CooperativeLoad {
            columns,
            rows,
            role,
            data: CooperativeData {
                pointer: map(data.pointer),
                stride: map(data.stride),
                row_major: data.row_major,
            },
        },
        Expression::CooperativeMultiplyAdd { a, b, c } => Expression::CooperativeMultiplyAdd {
            a: map(a),
            b: map(b),
            c: map(c),
        },
    }
}

/// Builds a copy of the spatial function `spatial` in which every sample of
/// argument `input` is passed through `prefix` before use — including samples
/// `input` reaches through helpers it is passed to.
///
/// The fold is transitive: a callee that receives `input` is cloned once per
/// combination of argument positions `input` binds to, with its own samples
/// folded the same way, and the call is retargeted. A folded callee gains
/// `extra` as trailing arguments when it calls `prefix`, directly or through
/// a folded callee of its own; `spatial`'s copy always gains `extra`, which
/// are passed to `prefix` after the sampled colour, in order.
///
/// `prefix` is a function `fn(color, extra...) -> color` — the colour piece
/// of the chain as a function of one sample.
///
/// # Panics
///
/// Panics when a sample of `input` cannot be folded — parse analysis gates
/// the fold, so a failure is a composer defect.
pub fn fold_prefix(
    module: &mut Module,
    spatial: Handle<Function>,
    input: u32,
    prefix: Handle<Function>,
    extra: &[FunctionArgument],
    name: String,
) -> Function {
    let names: HashSet<String> = module
        .functions
        .iter()
        .filter_map(|(_, function)| function.name.clone())
        .collect();
    let mut fold = Fold {
        module,
        prefix,
        extra,
        functions: HashMap::new(),
        in_progress: HashSet::new(),
        names,
    };
    let source = fold.module.functions[spatial].clone();
    let inputs =
        vec![parse::argument_expression(&source, input).expect("a spatial stage declares `input`")];
    let (replaced, call_fns, call_results) = fold
        .calls(&source, &inputs)
        .expect("folding analysis passed");
    fold.rebuild(
        &source,
        &inputs,
        &replaced,
        &call_fns,
        &call_results,
        true,
        Some(name),
    )
}

/// Folded callees: `(callee, bound input positions)` to the folded clone and
/// whether it takes the trailing `extra` arguments.
type FoldFns = HashMap<Key, (Handle<Function>, bool)>;

/// A fold in progress: clones are memoized per callee and bound argument
/// positions.
struct Fold<'a> {
    /// The module new functions are appended to.
    module: &'a mut Module,
    /// The prefix applied after every sample.
    prefix: Handle<Function>,
    /// The arguments a folded clone gains as trailing arguments when it
    /// reaches a sample — `prefix` takes them after the sampled colour.
    extra: &'a [FunctionArgument],
    /// `(function, bound input positions)` to the folded clone and whether
    /// it takes `extra`, or `None` when it cannot be folded.
    functions: HashMap<Key, Option<(Handle<Function>, bool)>>,
    /// Keys currently being folded — a call cycle cannot be folded.
    in_progress: HashSet<Key>,
    /// Function names already taken, for the clones' unique names.
    names: HashSet<String>,
}

impl Fold<'_> {
    /// The fold of `function` when `inputs`' argument positions bind to the
    /// caller's `input` — `(function, false)` when nothing changes, or `None`
    /// when it cannot be folded.
    ///
    /// The flag in the result tells whether the clone takes the trailing
    /// `extra` arguments, which every bound call then supplies from its own.
    fn fold(
        &mut self,
        function: Handle<Function>,
        inputs: &[u32],
    ) -> Option<(Handle<Function>, bool)> {
        let mut positions = inputs.to_vec();
        positions.sort_unstable();
        let key = (function, positions);
        if let Some(&folded) = self.functions.get(&key) {
            return folded;
        }
        // WGSL admits no recursion; bail rather than looping on a cycle.
        if !self.in_progress.insert(key.clone()) {
            return None;
        }
        let folded = self.try_fold(function, &key.1);
        self.in_progress.remove(&key);
        self.functions.insert(key, folded);
        folded
    }

    fn try_fold(
        &mut self,
        function: Handle<Function>,
        inputs: &[u32],
    ) -> Option<(Handle<Function>, bool)> {
        let source = self.module.functions[function].clone();
        let inputs: Vec<Handle<Expression>> = inputs
            .iter()
            .map(|&arg| parse::argument_expression(&source, arg))
            .collect::<Option<_>>()?;

        let (replaced, call_fns, call_results) = self.calls(&source, &inputs)?;
        // A clone that folds samples, or retargets a call into a clone that
        // takes `extra`, takes `extra` itself.
        let needs_extra =
            !replaced.is_empty() || call_fns.values().any(|&(_, needs_extra)| needs_extra);
        if replaced.is_empty()
            && call_fns
                .iter()
                .all(|(&(callee, _), &(call, _))| callee == call)
        {
            // Nothing samples `input` and no call needed retargeting.
            return Some((function, false));
        }
        let rebuilt = self.rebuild(
            &source,
            &inputs,
            &replaced,
            &call_fns,
            &call_results,
            needs_extra,
            None,
        );
        Some((
            self.module.functions.append(rebuilt, GENERATED),
            needs_extra,
        ))
    }

    /// The samples `function`'s copies fold and the calls `input` flows
    /// into, folded: the replaced sample expressions, the `(callee, bound)`
    /// retargeting map, and `CallResult` expressions to folded callees.
    fn calls(&mut self, source: &Function, inputs: &[Handle<Expression>]) -> Option<FoldSites> {
        let replaced: Vec<Handle<Expression>> = source
            .expressions
            .iter()
            .filter(|(_, expression)| {
                inputs
                    .iter()
                    .any(|&input| parse::samples_input(expression, input))
            })
            .map(|(handle, _)| handle)
            .collect();
        let mut call_fns: FoldFns = HashMap::new();
        let mut call_results: HashMap<Handle<Expression>, Handle<Function>> = HashMap::new();
        for call in bound_calls(source, inputs)? {
            let folded = self.fold(call.function, &call.bound)?;
            call_fns.insert((call.function, call.bound), folded);
            if let Some(result) = call.result {
                call_results.insert(result, folded.0);
            }
        }
        Some((replaced, call_fns, call_results))
    }

    /// Clones `source`: every replaced sample is passed through `prefix`,
    /// every bound call is retargeted to its folded callee. When
    /// `append_extra` the clone gains `extra` as trailing arguments — always
    /// for `spatial`'s copy, when needed for a callee — and passes them to
    /// `prefix` and to folded callees that take them.
    #[expect(
        clippy::too_many_arguments,
        reason = "the rebuild needs every analysis result; bundling them would only rename the list"
    )]
    fn rebuild(
        &mut self,
        source: &Function,
        inputs: &[Handle<Expression>],
        replaced: &[Handle<Expression>],
        call_fns: &FoldFns,
        call_results: &HashMap<Handle<Expression>, Handle<Function>>,
        append_extra: bool,
        name: Option<String>,
    ) -> Function {
        let replaced: HashSet<Handle<Expression>> = replaced.iter().copied().collect();
        let mut rebuilt = Function {
            name: Some(name.unwrap_or_else(|| self.name_for(source))),
            arguments: source.arguments.clone(),
            result: source.result.clone(),
            local_variables: source.local_variables.clone(),
            diagnostic_filter_leaf: source.diagnostic_filter_leaf,
            ..Function::default()
        };

        let mut copier = FoldRebuild {
            map: Vec::with_capacity(source.expressions.len()),
            replaced: HashMap::new(),
            prefix: self.prefix,
            inputs,
            extra: Vec::new(),
            call_fns,
        };
        for (handle, expression) in source.expressions.iter() {
            let copy = match *expression {
                Expression::CallResult(_) => call_results.get(&handle).map_or_else(
                    || expression.clone(),
                    |&target| Expression::CallResult(target),
                ),
                _ => map_expression(expression, |operand| copier.operand(operand)),
            };
            let new = rebuilt
                .expressions
                .append(copy, source.expressions.get_span(handle));
            copier.map.push(new);
            if replaced.contains(&handle) {
                // The sample itself is copied and emitted; the `prefix`
                // call's `CallResult` takes its place for every use.
                let call = rebuilt
                    .expressions
                    .append(Expression::CallResult(self.prefix), GENERATED);
                copier.replaced.insert(handle, call);
            }
        }
        if append_extra {
            let first_extra =
                u32::try_from(rebuilt.arguments.len()).expect("argument count fits in u32");
            rebuilt.arguments.extend_from_slice(self.extra);
            copier.extra = (0..self.extra.len())
                .map(|offset| {
                    let index =
                        first_extra + u32::try_from(offset).expect("argument count fits in u32");
                    rebuilt
                        .expressions
                        .append(Expression::FunctionArgument(index), GENERATED)
                })
                .collect();
        }

        for (_, local) in rebuilt.local_variables.iter_mut() {
            local.init = local.init.map(|init| copier.operand(init));
        }
        for (old, name) in &source.named_expressions {
            rebuilt
                .named_expressions
                .insert(copier.map[old.index()], name.clone());
        }
        rebuilt.body = copier.block(&source.body);
        rebuilt
    }

    /// A unique name for `source`'s clone.
    fn name_for(&mut self, source: &Function) -> String {
        let base = format!("{}_folded", source.name.as_deref().unwrap_or("function"));
        let mut name = base.clone();
        let mut suffix = 0;
        while self.names.contains(&name) {
            suffix += 1;
            name = format!("{base}_{suffix}");
        }
        self.names.insert(name.clone());
        name
    }
}

/// Statement rebuilding for the fold: a folded sample keeps its place in the
/// emit range, followed by the `prefix` call whose `CallResult` every use of
/// the sample remaps to, and calls `input` flows through are retargeted to
/// folded callees.
struct FoldRebuild<'a> {
    /// Every source expression to its copy.
    map: Vec<Handle<Expression>>,
    /// Source sample expression to the `prefix` call's `CallResult`.
    replaced: HashMap<Handle<Expression>, Handle<Expression>>,
    /// The prefix applied after every sample.
    prefix: Handle<Function>,
    /// The clone's own `input` argument expressions, for call retargeting.
    inputs: &'a [Handle<Expression>],
    /// The clone's own `extra` argument expressions, passed to `prefix` and
    /// to folded callees that take them — empty when the clone gains none.
    extra: Vec<Handle<Expression>>,
    /// `(callee, bound input positions)` to the folded callee and whether it
    /// takes `extra`.
    call_fns: &'a FoldFns,
}

impl FoldRebuild<'_> {
    /// Where a use of `old` points after the rewrite.
    fn operand(&self, old: Handle<Expression>) -> Handle<Expression> {
        self.replaced
            .get(&old)
            .copied()
            .unwrap_or_else(|| self.map[old.index()])
    }

    /// Where the definition of `old` lives after the rewrite.
    fn defined(&self, old: Handle<Expression>) -> Handle<Expression> {
        self.map[old.index()]
    }

    fn block(&self, block: &Block) -> Block {
        let mut out = Block::with_capacity(block.len());
        for (statement, span) in block.span_iter() {
            match statement {
                Statement::Emit(range) => self.emit(range, &mut out, *span),
                other => out.push(self.statement(other), *span),
            }
        }
        out
    }

    /// Re-emits `range`, splitting it after every folded sample to call the
    /// prefix on that sample.
    fn emit(&self, range: &Range<Expression>, out: &mut Block, span: naga::Span) {
        let mut run: Option<(Handle<Expression>, Handle<Expression>)> = None;
        for old in range.clone() {
            let new = self.defined(old);
            run = Some(run.map_or((new, new), |(first, _)| (first, new)));
            if let Some(&call) = self.replaced.get(&old) {
                let (first, last) = run.take().expect("the run holds the sample");
                out.push(Statement::Emit(Range::new_from_bounds(first, last)), span);
                let mut arguments = Vec::with_capacity(1 + self.extra.len());
                arguments.push(new);
                arguments.extend_from_slice(&self.extra);
                out.push(
                    Statement::Call {
                        function: self.prefix,
                        arguments,
                        result: Some(call),
                    },
                    GENERATED,
                );
            }
        }
        if let Some((first, last)) = run {
            out.push(Statement::Emit(Range::new_from_bounds(first, last)), span);
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per naga statement variant; the match stays exhaustive so a new variant fails to compile"
    )]
    fn statement(&self, statement: &Statement) -> Statement {
        let op = |handle: Handle<Expression>| self.operand(handle);
        let block = |block: &Block| self.block(block);
        match *statement {
            Statement::Emit(_) => unreachable!("emits are rebuilt by `FoldRebuild::emit`"),
            Statement::Block(ref inner) => Statement::Block(block(inner)),
            Statement::If {
                condition,
                ref accept,
                ref reject,
            } => Statement::If {
                condition: op(condition),
                accept: block(accept),
                reject: block(reject),
            },
            Statement::Switch {
                selector,
                ref cases,
            } => Statement::Switch {
                selector: op(selector),
                cases: cases
                    .iter()
                    .map(|case| SwitchCase {
                        value: case.value,
                        body: block(&case.body),
                        fall_through: case.fall_through,
                    })
                    .collect(),
            },
            Statement::Loop {
                ref body,
                ref continuing,
                break_if,
            } => Statement::Loop {
                body: block(body),
                continuing: block(continuing),
                break_if: break_if.map(op),
            },
            Statement::Break => Statement::Break,
            Statement::Continue => Statement::Continue,
            Statement::Return { value } => Statement::Return {
                value: value.map(op),
            },
            Statement::Store { pointer, value } => Statement::Store {
                pointer: op(pointer),
                value: op(value),
            },
            Statement::Call {
                function,
                ref arguments,
                result,
            } => {
                let mut bound: Vec<u32> = arguments
                    .iter()
                    .enumerate()
                    .filter(|&(_, argument)| self.inputs.contains(argument))
                    .map(|(index, _)| u32::try_from(index).expect("argument index fits in u32"))
                    .collect();
                let mut arguments: Vec<Handle<Expression>> =
                    arguments.iter().map(|&argument| op(argument)).collect();
                let function = if bound.is_empty() {
                    function
                } else {
                    bound.sort_unstable();
                    let &(target, needs_extra) = self
                        .call_fns
                        .get(&(function, bound))
                        .expect("every bound call was folded");
                    if needs_extra {
                        // The clone took `extra` for exactly this.
                        arguments.extend_from_slice(&self.extra);
                    }
                    target
                };
                Statement::Call {
                    function,
                    arguments,
                    result: result.map(|result| self.defined(result)),
                }
            }
            Statement::SubgroupBallot { result, predicate } => Statement::SubgroupBallot {
                result: self.defined(result),
                predicate: predicate.map(op),
            },
            Statement::SubgroupGather {
                mode,
                argument,
                result,
            } => Statement::SubgroupGather {
                mode: gather_mode(mode, op),
                argument: op(argument),
                result: self.defined(result),
            },
            Statement::SubgroupCollectiveOperation {
                op: operation,
                collective_op,
                argument,
                result,
            } => Statement::SubgroupCollectiveOperation {
                op: operation,
                collective_op,
                argument: op(argument),
                result: self.defined(result),
            },
            Statement::Kill
            | Statement::ControlBarrier(_)
            | Statement::MemoryBarrier(_)
            | Statement::ImageStore { .. }
            | Statement::Atomic { .. }
            | Statement::ImageAtomic { .. }
            | Statement::WorkGroupUniformLoad { .. }
            | Statement::RayQuery { .. }
            | Statement::RayPipelineFunction(_)
            | Statement::CooperativeStore { .. } => {
                unreachable!("snippet parsing rejects {statement:?}")
            }
        }
    }
}

fn gather_mode(
    mode: GatherMode,
    op: impl Fn(Handle<Expression>) -> Handle<Expression>,
) -> GatherMode {
    match mode {
        GatherMode::BroadcastFirst => GatherMode::BroadcastFirst,
        GatherMode::Broadcast(index) => GatherMode::Broadcast(op(index)),
        GatherMode::Shuffle(index) => GatherMode::Shuffle(op(index)),
        GatherMode::ShuffleDown(delta) => GatherMode::ShuffleDown(op(delta)),
        GatherMode::ShuffleUp(delta) => GatherMode::ShuffleUp(op(delta)),
        GatherMode::ShuffleXor(mask) => GatherMode::ShuffleXor(op(mask)),
        GatherMode::QuadBroadcast(index) => GatherMode::QuadBroadcast(op(index)),
        GatherMode::QuadSwap(direction) => GatherMode::QuadSwap(direction),
    }
}

// ============================================================================
// Sample substitution: `textureSampleLevel(input, input_sampler, uv)` runs as
// a manual bilinear of texel loads.
// ============================================================================

/// Replaces `function`'s filtered samples of `input` by calls to `manual`.
///
/// `input` is the index of the texture argument samples are taken of,
/// `sampler` — when `Some` — the index of the sampler argument those samples
/// must go through, and `size` the index of the argument carrying `input`'s
/// extent in pixels. `manual` is a function `fn(texture_2d<f32>, vec2<f32>,
/// vec2<f32>) -> vec4<f32>` implementing a filtered sample out of texel
/// loads; its third argument is the extent.
///
/// The substitution is transitive: a callee that receives `input` is cloned
/// once per combination of argument positions `input` binds to, with its own
/// samples replaced, and the call is retargeted. A substituted callee gains
/// a trailing `size` argument its callers supply from their own `size`.
///
/// Returns the function to call instead of `function` — `function` itself
/// when nothing needed substituting — or `None` when a use of `input` has no
/// manual equivalent: a gather, a depth reference, an array index or offset,
/// a level other than mip 0, a sampler that is not the function's own, a
/// store or atomic, or a call into a function that cannot itself be
/// substituted.
pub fn substitute_samples(
    module: &mut Module,
    function: Handle<Function>,
    input: u32,
    sampler: Option<u32>,
    size: u32,
    manual: Handle<Function>,
) -> Option<Handle<Function>> {
    let names: HashSet<String> = module
        .functions
        .iter()
        .filter_map(|(_, function)| function.name.clone())
        .collect();
    let mut substitution = Substitution {
        module,
        manual,
        functions: HashMap::new(),
        in_progress: HashSet::new(),
        names,
    };
    substitution
        .substitute(function, &[input], sampler, Some(size))
        .map(|(function, _)| function)
}

/// A substitution key: a callee and the sorted argument positions the
/// caller's `input` binds to.
type Key = (Handle<Function>, Vec<u32>);

/// Substituted callees: `(callee, bound positions)` to the substitute and
/// whether it takes a trailing `size` argument.
type CallFns = HashMap<Key, (Handle<Function>, bool)>;

/// One call `input` flows into, found while scanning a function's body.
struct BoundCall {
    /// The callee.
    function: Handle<Function>,
    /// The callee argument positions `input` binds to, sorted.
    bound: Vec<u32>,
    /// The call's `CallResult` expression, when the call returns a value.
    result: Option<Handle<Expression>>,
}

/// A substitution in progress: clones are memoized per callee and bound
/// argument positions.
struct Substitution<'a> {
    /// The module new functions are appended to.
    module: &'a mut Module,
    /// The manual sample implementation calls are retargeted to.
    manual: Handle<Function>,
    /// `(function, argument positions `input` binds to)` to the function to
    /// call instead — and whether it takes a trailing `size` argument —
    /// or `None` when it cannot be substituted.
    functions: HashMap<Key, Option<(Handle<Function>, bool)>>,
    /// Keys currently being substituted — a cycle is not substitutable.
    in_progress: HashSet<Key>,
    /// Function names already taken, for the clones' unique names.
    names: HashSet<String>,
}

/// `(replaced samples, callee retargets, CallResult remaps)` — the sites
/// one function's fold rewrites.
type FoldSites = (
    Vec<Handle<Expression>>,
    FoldFns,
    HashMap<Handle<Expression>, Handle<Function>>,
);

/// What one replaced sample expression becomes: the manual call's
/// `CallResult`, and the source operands the call takes.
#[derive(Clone, Copy)]
struct SubstitutedSample {
    /// The `CallResult` of the call to `manual` that replaces the sample.
    result: Handle<Expression>,
    /// The sample's image operand — the `input` expression.
    image: Handle<Expression>,
    /// The sample's coordinate operand.
    coordinate: Handle<Expression>,
}

impl Substitution<'_> {
    /// The substitute of `function` when `inputs`' argument positions bind to
    /// the caller's `input` — `(function, false)` when nothing changes, or
    /// `None` when it cannot be substituted.
    ///
    /// `size` is the index of the argument carrying `input`'s extent when
    /// `function` already declares one (`apply`); for a callee the extent
    /// arrives as a trailing argument appended to the clone when it needs
    /// one — the returned flag — and supplied by every bound call.
    fn substitute(
        &mut self,
        function: Handle<Function>,
        inputs: &[u32],
        sampler: Option<u32>,
        size: Option<u32>,
    ) -> Option<(Handle<Function>, bool)> {
        let mut positions = inputs.to_vec();
        positions.sort_unstable();
        let key = (function, positions);
        if let Some(&substituted) = self.functions.get(&key) {
            return substituted;
        }
        // WGSL admits no recursion; bail rather than looping on a cycle.
        if !self.in_progress.insert(key.clone()) {
            return None;
        }
        let substituted = self.try_substitute(function, &key.1, sampler, size);
        self.in_progress.remove(&key);
        self.functions.insert(key, substituted);
        substituted
    }

    /// Clones `function` with its substitutable samples of `inputs` replaced;
    /// `sampler` is the declared sampler argument a sample must use (`None`
    /// for a callee, whose sampler arguments all derive from the caller's).
    ///
    /// The flag in the result tells whether the clone takes a trailing
    /// `size` argument — `Some` in `size` means the argument already exists
    /// and no clone gains one.
    fn try_substitute(
        &mut self,
        function: Handle<Function>,
        inputs: &[u32],
        sampler: Option<u32>,
        size: Option<u32>,
    ) -> Option<(Handle<Function>, bool)> {
        let source = self.module.functions[function].clone();
        let inputs: Vec<Handle<Expression>> = inputs
            .iter()
            .map(|&arg| parse::argument_expression(&source, arg))
            .collect::<Option<_>>()?;
        let sampler = match sampler {
            Some(arg) => Some(parse::argument_expression(&source, arg)?),
            None => None,
        };
        let size = match size {
            Some(arg) => Some(parse::argument_expression(&source, arg)?),
            None => None,
        };

        let replaced = substitutable_samples(&source, &inputs, sampler)?;
        let calls = bound_calls(&source, &inputs)?;

        // `(callee, bound positions)` to the substituted callee.
        let mut call_fns: CallFns = HashMap::new();
        // A call's `CallResult` expression to its substituted callee.
        let mut call_results: HashMap<Handle<Expression>, Handle<Function>> = HashMap::new();
        for call in calls {
            let substituted = self.substitute(call.function, &call.bound, None, None)?;
            call_fns.insert((call.function, call.bound), substituted);
            if let Some(result) = call.result {
                call_results.insert(result, substituted.0);
            }
        }
        // A clone that replaces samples, or retargets a call into a clone
        // that gained a `size` argument, passes its own `size` along.
        let needs_size =
            !replaced.is_empty() || call_fns.values().any(|&(_, needs_size)| needs_size);
        if replaced.is_empty()
            && call_fns
                .iter()
                .all(|(&(callee, _), &(call, _))| callee == call)
        {
            // Nothing samples `input` and no call needed retargeting.
            return Some((function, false));
        }
        Some((
            self.rebuild(&source, &inputs, replaced, &call_fns, &call_results, size),
            needs_size && size.is_none(),
        ))
    }

    /// Clones `source` into the module: every replaced sample becomes a call
    /// to `manual`, every bound call is retargeted to its substituted callee.
    /// `size` is the source expression reading `input`'s extent; when the
    /// clone needs one and has none, it is appended as a trailing argument.
    fn rebuild(
        &mut self,
        source: &Function,
        inputs: &[Handle<Expression>],
        replaced: Vec<(Handle<Expression>, Handle<Expression>)>,
        call_fns: &CallFns,
        call_results: &HashMap<Handle<Expression>, Handle<Function>>,
        size: Option<Handle<Expression>>,
    ) -> Handle<Function> {
        // As the caller decided: a clone that replaces samples, or retargets
        // a call into a clone that gained a `size` argument, gets one too.
        let appended = size.is_none()
            && (!replaced.is_empty() || call_fns.values().any(|&(_, needs_size)| needs_size));
        let replaced: HashMap<Handle<Expression>, Handle<Expression>> =
            replaced.into_iter().collect();
        let mut rebuilt = Function {
            name: Some(self.name_for(source)),
            arguments: source.arguments.clone(),
            result: source.result.clone(),
            local_variables: source.local_variables.clone(),
            diagnostic_filter_leaf: source.diagnostic_filter_leaf,
            ..Function::default()
        };

        // As in `fold_prefix`: `map` indexes every source expression to its
        // copy, and `substituted` records a replaced sample's call along with
        // the operands the call takes.
        let mut substitute = Substitute {
            map: Vec::with_capacity(source.expressions.len()),
            substituted: HashMap::new(),
            manual: self.manual,
            inputs,
            size: None,
            call_fns,
        };
        for (handle, expression) in source.expressions.iter() {
            if let Some(&coordinate) = replaced.get(&handle) {
                // The sample itself is never copied or emitted; the call's
                // `CallResult` takes its slot so uses remap to it.
                let call = rebuilt
                    .expressions
                    .append(Expression::CallResult(self.manual), GENERATED);
                let Expression::ImageSample { image, .. } = *expression else {
                    unreachable!("a substituted expression is an ImageSample")
                };
                substitute.map.push(call);
                substitute.substituted.insert(
                    handle,
                    SubstitutedSample {
                        result: call,
                        image,
                        coordinate,
                    },
                );
                continue;
            }
            let copy = match *expression {
                Expression::CallResult(_) => call_results.get(&handle).map_or_else(
                    || expression.clone(),
                    |&target| Expression::CallResult(target),
                ),
                _ => map_expression(expression, |operand| substitute.operand(operand)),
            };
            let new = rebuilt
                .expressions
                .append(copy, source.expressions.get_span(handle));
            substitute.map.push(new);
        }

        substitute.size = if let Some(size) = size {
            Some(substitute.defined(size))
        } else if appended {
            let index = u32::try_from(rebuilt.arguments.len()).expect("argument count fits in u32");
            rebuilt.arguments.push(FunctionArgument {
                name: Some("size".to_owned()),
                ty: vec2f_type(self.module),
                binding: None,
            });
            Some(
                rebuilt
                    .expressions
                    .append(Expression::FunctionArgument(index), GENERATED),
            )
        } else {
            None
        };

        for (_, local) in rebuilt.local_variables.iter_mut() {
            local.init = local.init.map(|init| substitute.operand(init));
        }
        for (old, name) in &source.named_expressions {
            rebuilt
                .named_expressions
                .insert(substitute.defined(*old), name.clone());
        }
        rebuilt.body = substitute.block(&source.body);

        self.module.functions.append(rebuilt, GENERATED)
    }

    /// A unique name for `source`'s clone.
    fn name_for(&mut self, source: &Function) -> String {
        let base = format!("{}_manual", source.name.as_deref().unwrap_or("function"));
        let mut name = base.clone();
        let mut suffix = 0;
        while self.names.contains(&name) {
            suffix += 1;
            name = format!("{base}_{suffix}");
        }
        self.names.insert(name.clone());
        name
    }
}

/// Statement rebuilding for [`substitute_samples`], mirroring the fold's
/// [`Rebuild`]: the same `map`, but a replaced sample becomes a `Call` to the
/// manual bilinear at its position in the emit range, and calls are
/// retargeted to substituted callees.
struct Substitute<'a> {
    /// Every source expression to its copy; a replaced sample's entry is the
    /// call's `CallResult`.
    map: Vec<Handle<Expression>>,
    /// Source sample expression to its replacement.
    substituted: HashMap<Handle<Expression>, SubstitutedSample>,
    /// The manual bilinear function samples call.
    manual: Handle<Function>,
    /// The clone's own `input` argument expressions, for call retargeting.
    inputs: &'a [Handle<Expression>],
    /// The clone's own `size` expression — `input`'s extent — that manual
    /// calls and retargeted callees take, in the clone's own arena.
    size: Option<Handle<Expression>>,
    /// `(callee, bound argument positions)` to the substituted callee and
    /// whether the callee takes a trailing `size` argument.
    call_fns: &'a CallFns,
}

impl Substitute<'_> {
    /// Where a use of `old` points after the rewrite.
    fn operand(&self, old: Handle<Expression>) -> Handle<Expression> {
        self.substituted
            .get(&old)
            .map_or_else(|| self.map[old.index()], |substituted| substituted.result)
    }

    /// Where the definition of `old` lives after the rewrite.
    fn defined(&self, old: Handle<Expression>) -> Handle<Expression> {
        self.map[old.index()]
    }

    fn block(&self, block: &Block) -> Block {
        let mut out = Block::with_capacity(block.len());
        for (statement, span) in block.span_iter() {
            match statement {
                Statement::Emit(range) => self.emit(range, &mut out, *span),
                other => out.push(self.statement(other), *span),
            }
        }
        out
    }

    /// Re-emits `range`, splitting it at each replaced sample: the sample
    /// becomes a `Call` to `manual` on its image and coordinate operands.
    fn emit(&self, range: &Range<Expression>, out: &mut Block, span: naga::Span) {
        let mut run: Option<(Handle<Expression>, Handle<Expression>)> = None;
        for old in range.clone() {
            if let Some(&substituted) = self.substituted.get(&old) {
                if let Some((first, last)) = run.take() {
                    out.push(Statement::Emit(Range::new_from_bounds(first, last)), span);
                }
                out.push(
                    Statement::Call {
                        function: self.manual,
                        arguments: vec![
                            self.operand(substituted.image),
                            self.operand(substituted.coordinate),
                            self.size.expect("a substituted stage reads `size`"),
                        ],
                        result: Some(substituted.result),
                    },
                    GENERATED,
                );
            } else {
                let new = self.defined(old);
                run = Some(run.map_or((new, new), |(first, _)| (first, new)));
            }
        }
        if let Some((first, last)) = run {
            out.push(Statement::Emit(Range::new_from_bounds(first, last)), span);
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per naga statement variant; the match stays exhaustive so a new variant fails to compile"
    )]
    fn statement(&self, statement: &Statement) -> Statement {
        let op = |handle: Handle<Expression>| self.operand(handle);
        let block = |block: &Block| self.block(block);
        match *statement {
            Statement::Emit(_) => unreachable!("emits are rebuilt by `Substitute::emit`"),
            Statement::Block(ref inner) => Statement::Block(block(inner)),
            Statement::If {
                condition,
                ref accept,
                ref reject,
            } => Statement::If {
                condition: op(condition),
                accept: block(accept),
                reject: block(reject),
            },
            Statement::Switch {
                selector,
                ref cases,
            } => Statement::Switch {
                selector: op(selector),
                cases: cases
                    .iter()
                    .map(|case| SwitchCase {
                        value: case.value,
                        body: block(&case.body),
                        fall_through: case.fall_through,
                    })
                    .collect(),
            },
            Statement::Loop {
                ref body,
                ref continuing,
                break_if,
            } => Statement::Loop {
                body: block(body),
                continuing: block(continuing),
                break_if: break_if.map(op),
            },
            Statement::Break => Statement::Break,
            Statement::Continue => Statement::Continue,
            Statement::Return { value } => Statement::Return {
                value: value.map(op),
            },
            Statement::Store { pointer, value } => Statement::Store {
                pointer: op(pointer),
                value: op(value),
            },
            Statement::Call {
                function,
                ref arguments,
                result,
            } => {
                let mut bound: Vec<u32> = arguments
                    .iter()
                    .enumerate()
                    .filter(|&(_, argument)| self.inputs.contains(argument))
                    .map(|(index, _)| u32::try_from(index).expect("argument index fits in u32"))
                    .collect();
                let mut arguments: Vec<Handle<Expression>> =
                    arguments.iter().map(|&argument| op(argument)).collect();
                let function = if bound.is_empty() {
                    function
                } else {
                    bound.sort_unstable();
                    let (target, needs_size) = self.call_fns[&(function, bound)];
                    if needs_size {
                        arguments.push(self.size.expect("a substituted stage reads `size`"));
                    }
                    target
                };
                Statement::Call {
                    function,
                    arguments,
                    result: result.map(|result| self.defined(result)),
                }
            }
            Statement::SubgroupBallot { result, predicate } => Statement::SubgroupBallot {
                result: self.defined(result),
                predicate: predicate.map(op),
            },
            Statement::SubgroupGather {
                mode,
                argument,
                result,
            } => Statement::SubgroupGather {
                mode: gather_mode(mode, op),
                argument: op(argument),
                result: self.defined(result),
            },
            Statement::SubgroupCollectiveOperation {
                op: operation,
                collective_op,
                argument,
                result,
            } => Statement::SubgroupCollectiveOperation {
                op: operation,
                collective_op,
                argument: op(argument),
                result: self.defined(result),
            },
            Statement::Kill
            | Statement::ControlBarrier(_)
            | Statement::MemoryBarrier(_)
            | Statement::ImageStore { .. }
            | Statement::Atomic { .. }
            | Statement::ImageAtomic { .. }
            | Statement::WorkGroupUniformLoad { .. }
            | Statement::RayQuery { .. }
            | Statement::RayPipelineFunction(_)
            | Statement::CooperativeStore { .. } => {
                unreachable!("snippet parsing rejects {statement:?}")
            }
        }
    }
}

/// The `ImageSample` expressions of `inputs` a manual call replaces, each
/// keyed to its coordinate operand — `None` when an expression has no
/// manual equivalent.
fn substitutable_samples(
    source: &Function,
    inputs: &[Handle<Expression>],
    sampler: Option<Handle<Expression>>,
) -> Option<Vec<(Handle<Expression>, Handle<Expression>)>> {
    let mut replaced = Vec::new();
    for (handle, expression) in source.expressions.iter() {
        match *expression {
            Expression::ImageSample {
                image,
                sampler: used,
                coordinate,
                gather,
                array_index,
                offset,
                level,
                depth_ref,
                clamp_to_edge: _,
            } if inputs.contains(&image) => {
                if gather.is_some()
                    || depth_ref.is_some()
                    || array_index.is_some()
                    || offset.is_some()
                    || !manual_level(source, level)
                    || !manual_sampler(source, used, sampler)
                {
                    return None;
                }
                replaced.push((handle, coordinate));
            }
            // Texel loads and queries reproduce as they are.
            Expression::ImageLoad { image, .. } | Expression::ImageQuery { image, .. }
                if inputs.contains(&image) => {}
            // Any other read of `input` has no manual equivalent.
            ref other if inputs.iter().any(|&input| parse::reads(other, input)) => {
                return None;
            }
            _ => {}
        }
    }
    Some(replaced)
}

/// The calls `input` flows into — `None` when `input` is stored to or
/// atomically accessed, which no manual equivalent covers.
fn bound_calls(source: &Function, inputs: &[Handle<Expression>]) -> Option<Vec<BoundCall>> {
    let mut calls = Vec::new();
    let mut stored = false;
    read_block(&source.body, &mut |statement| match *statement {
        Statement::Call {
            function: callee,
            ref arguments,
            result,
        } => {
            let bound: Vec<u32> = arguments
                .iter()
                .enumerate()
                .filter(|&(_, argument)| inputs.contains(argument))
                .map(|(index, _)| u32::try_from(index).expect("argument index fits in u32"))
                .collect();
            if !bound.is_empty() {
                calls.push(BoundCall {
                    function: callee,
                    bound,
                    result,
                });
            }
        }
        Statement::ImageStore { image, .. } | Statement::ImageAtomic { image, .. }
            if inputs.contains(&image) =>
        {
            stored = true;
        }
        _ => {}
    });
    if stored { None } else { Some(calls) }
}

/// Whether `level` resolves to mip 0 — the only level a manual sample
/// reproduces.
fn manual_level(function: &Function, level: SampleLevel) -> bool {
    match level {
        SampleLevel::Auto | SampleLevel::Zero => true,
        SampleLevel::Exact(value) => zero_level(function, value),
        SampleLevel::Bias(_) | SampleLevel::Gradient { .. } => false,
    }
}

/// Whether `used` is the function's own sampler argument: `declared` for the
/// stage's `apply`, or any sampler argument of a substituted helper — its
/// samplers all derive from `apply`'s.
fn manual_sampler(
    function: &Function,
    used: Handle<Expression>,
    declared: Option<Handle<Expression>>,
) -> bool {
    declared.map_or_else(
        || matches!(function.expressions[used], Expression::FunctionArgument(_)),
        |declared| used == declared,
    )
}

/// Whether `value` is the literal `0.0`.
fn zero_level(function: &Function, value: Handle<Expression>) -> bool {
    match function.expressions[value] {
        Expression::Literal(Literal::F32(v)) => v == 0.0,
        Expression::Literal(Literal::F16(v)) => v.to_bits() == 0,
        Expression::Literal(Literal::AbstractFloat(v)) => v == 0.0,
        _ => false,
    }
}

/// The module's `vec2<f32>` type handle, inserting it if absent.
fn vec2f_type(module: &mut Module) -> Handle<Type> {
    let existing = module
        .types
        .iter()
        .find(|(_, ty)| ty.inner == ParamType::Vec2.inner())
        .map(|(handle, _)| handle);
    existing.unwrap_or_else(|| {
        module.types.insert(
            Type {
                name: None,
                inner: ParamType::Vec2.inner(),
            },
            GENERATED,
        )
    })
}

/// Walks every statement of `block` and its nested blocks.
pub fn read_block(block: &Block, each: &mut impl FnMut(&Statement)) {
    for statement in block {
        each(statement);
        match statement {
            Statement::Block(inner) => read_block(inner, each),
            Statement::If { accept, reject, .. } => {
                read_block(accept, each);
                read_block(reject, each);
            }
            Statement::Switch { cases, .. } => {
                for case in cases {
                    read_block(&case.body, each);
                }
            }
            Statement::Loop {
                body, continuing, ..
            } => {
                read_block(body, each);
                read_block(continuing, each);
            }
            _ => {}
        }
    }
}
