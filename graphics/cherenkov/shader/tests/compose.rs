//! Composition against the sequential application of the same snippets, and
//! emission for every back end.

use std::collections::HashMap;

use cherenkov_shader::{
    ComposeError, ComposeOptions, Composition, FoldBlocker, Folded, LibrarySource, Piece,
    Precision, SampleCount, SamplerFilter, Segment, SegmentArg, Snippet, SnippetError, SnippetKind,
    SnippetSource, Stage, Variant, compose,
    eval::{Eval, Texture, Value, assert_close},
    msl,
    naga::{self, valid::Capabilities},
    spirv, validate, wgsl,
};

const BRIGHTNESS: &str = include_str!("snippets/brightness.wgsl");
const BRIGHTNESS_F16: &str = include_str!("snippets/brightness_f16.wgsl");
const BRIGHTNESS_SUBGROUPS: &str = include_str!("snippets/brightness_subgroups.wgsl");
const SATURATION: &str = include_str!("snippets/saturation.wgsl");
const CONTRAST: &str = include_str!("snippets/contrast.wgsl");
const INVERT: &str = include_str!("snippets/invert.wgsl");
const BLUR: &str = include_str!("snippets/blur.wgsl");
const KERNEL: &str = include_str!("snippets/kernel.wgsl");
const GATHER: &str = include_str!("snippets/gather.wgsl");
const MEASURE: &str = include_str!("snippets/measure.wgsl");
const MISSING_COLOR: &str = include_str!("snippets/missing_color.wgsl");

/// A colour stage that does not commute with interpolation, unlike the
/// affine test filters.
const SQUARE: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(color.rgb * color.rgb, color.a);
}";

/// Luma coefficients of linear Display P3 (the Y row of its RGB to XYZ matrix).
const LUMA: [f32; 3] = [0.228_974_6, 0.691_738_5, 0.079_286_9];
const PIVOT: [f32; 3] = [0.18, 0.18, 0.18];

fn parse(name: &str, kind: SnippetKind, source: &str) -> Snippet {
    Snippet::parse(&SnippetSource::new(name, kind, source)).expect("the test snippet is valid")
}

fn module(source: &str) -> naga::Module {
    naga::front::wgsl::parse_str(source).expect("the test snippet parses")
}

fn space() -> Value {
    Value::Struct(vec![Value::vec(&LUMA)])
}

/// The params block for a segment, from per-(stage, parameter) values. The
/// input extent is prepended for spatial segments; every test texture is
/// 16×16.
fn block(segment: &Segment, values: &HashMap<(usize, &str), Value>) -> Value {
    let mut members = Vec::new();
    if segment.uniform.input_size.is_some() {
        members.push(Value::vec(&[16.0, 16.0]));
    }
    members.extend(
        segment
            .uniform
            .members
            .iter()
            .map(|member| values[&(member.stage, member.param.as_str())].clone()),
    );
    Value::Struct(members)
}

fn only_colour(composition: &Composition) -> &Segment {
    match composition.pieces() {
        [Piece::Color(segment)] => segment,
        other => panic!("expected one colour piece, got {other:?}"),
    }
}

fn run(composition: &Composition, segment: &Segment, args: Vec<Value>) -> Value {
    let eval = Eval::new(composition.module());
    eval.call(eval.function(&segment.function), args)
}

fn apply(source: &str, args: Vec<Value>) -> Value {
    let module = module(source);
    let eval = Eval::new(&module);
    eval.call(eval.function("apply"), args)
}

#[test]
fn colour_chain_matches_sequential_application() {
    let snippets = [
        parse("brightness", SnippetKind::Color, BRIGHTNESS),
        parse("saturation", SnippetKind::Color, SATURATION),
        parse("contrast", SnippetKind::Color, CONTRAST),
        parse("invert", SnippetKind::Color, INVERT),
    ];
    let stages = [
        Stage::new(&snippets[0]),
        Stage::new(&snippets[1]),
        Stage::new(&snippets[2]).constant("pivot", PIVOT),
        Stage::new(&snippets[3]),
    ];
    let composition = compose(&stages, ComposeOptions::default()).expect("the chain composes");
    let segment = only_colour(&composition);
    assert_eq!(
        segment.args,
        [
            SegmentArg::Color,
            SegmentArg::Params,
            SegmentArg::WorkingSpace
        ]
    );

    let values = HashMap::from([
        ((0, "amount"), Value::Float(0.1)),
        ((1, "amount"), Value::Float(0.8)),
        ((2, "amount"), Value::Float(1.3)),
    ]);
    for colour in [
        [0.2, 0.4, 0.6, 1.0],
        [0.05, 0.3, 0.1, 0.5],
        [0.9, 0.9, 0.0, 0.8],
    ] {
        let composed = run(
            &composition,
            segment,
            vec![Value::vec(&colour), block(segment, &values), space()],
        );

        let mut expected = Value::vec(&colour);
        expected = apply(
            BRIGHTNESS,
            vec![expected, Value::Struct(vec![Value::Float(0.1)])],
        );
        expected = apply(
            SATURATION,
            vec![expected, Value::Struct(vec![Value::Float(0.8)]), space()],
        );
        expected = apply(
            CONTRAST,
            vec![
                expected,
                Value::Struct(vec![Value::Float(1.3), Value::vec(&PIVOT)]),
            ],
        );
        expected = apply(INVERT, vec![expected]);

        assert_close(&composed, &expected, 1e-6);
    }
}

#[test]
fn constant_parameters_leave_the_uniform_block() {
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let contrast = parse("contrast", SnippetKind::Color, CONTRAST);

    let dynamic = compose(
        &[Stage::new(&brightness), Stage::new(&contrast)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let dynamic_segment = only_colour(&dynamic);
    let names: Vec<_> = dynamic_segment
        .uniform
        .members
        .iter()
        .map(|member| (member.stage, member.param.as_str(), member.offset))
        .collect();
    assert_eq!(
        names,
        [(0, "amount", 0), (1, "amount", 4), (1, "pivot", 16)]
    );
    assert_eq!(dynamic_segment.uniform.size, 32);

    let specialized = compose(
        &[
            Stage::new(&brightness).constant("amount", 0.25),
            Stage::new(&contrast).constant("pivot", PIVOT),
        ],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let specialized_segment = only_colour(&specialized);
    let names: Vec<_> = specialized_segment
        .uniform
        .members
        .iter()
        .map(|member| (member.stage, member.param.as_str()))
        .collect();
    assert_eq!(names, [(1, "amount")]);
    assert_eq!(specialized_segment.uniform.size, 16);

    // Specializing changes where a value comes from, not the result.
    let colour = Value::vec(&[0.3, 0.2, 0.1, 1.0]);
    let from_block = run(
        &dynamic,
        dynamic_segment,
        vec![
            colour.clone(),
            block(
                dynamic_segment,
                &HashMap::from([
                    ((0, "amount"), Value::Float(0.25)),
                    ((1, "amount"), Value::Float(1.5)),
                    ((1, "pivot"), Value::vec(&PIVOT)),
                ]),
            ),
        ],
    );
    let from_constants = run(
        &specialized,
        specialized_segment,
        vec![
            colour,
            block(
                specialized_segment,
                &HashMap::from([((1, "amount"), Value::Float(1.5))]),
            ),
        ],
    );
    assert_close(&from_constants, &from_block, 1e-6);
}

#[test]
fn colour_prefix_folds_into_every_sample() {
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let kernel = parse("kernel", SnippetKind::Spatial, KERNEL);
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&kernel)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");

    let [
        Piece::Color(_),
        Piece::Spatial {
            plain,
            folded,
            manual: None,
            sampler,
            not_foldable,
        },
    ] = composition.pieces()
    else {
        panic!("expected a colour piece then a spatial piece");
    };
    assert_eq!(*sampler, SamplerFilter::Point);
    assert_eq!(*not_foldable, None);
    let folded = folded.as_ref().expect("the kernel reads only texels");
    assert_eq!(folded.sampler, SamplerFilter::Point);
    assert_eq!(folded.cost.samples, SampleCount::Static(3));
    assert!(folded.cost.prefix_ops > 0);
    assert_eq!(folded.segment.stages, 0..2);
    assert_eq!(
        folded.segment.args,
        [
            SegmentArg::Input,
            SegmentArg::InputSampler,
            SegmentArg::Uv,
            SegmentArg::Params
        ]
    );

    let amount = 0.2;
    let step = [0.1, 0.0];
    let source = |uv: [f32; 2]| [uv[0], uv[1], 0.5, 0.75];
    let brightened = move |uv: [f32; 2]| {
        let [r, g, b, a] = source(uv);
        [
            a.mul_add(amount, r),
            a.mul_add(amount, g),
            a.mul_add(amount, b),
            a,
        ]
    };
    let values = HashMap::from([
        ((0, "amount"), Value::Float(amount)),
        ((1, "step"), Value::vec(&step)),
    ]);

    for uv in [[0.5, 0.5], [0.2, 0.7]] {
        let mut folded_eval = Eval::new(composition.module());
        let texture = folded_eval.texture(Texture::continuous(16, 16, source));
        let from_folded = folded_eval.call(
            folded_eval.function(&folded.segment.function),
            vec![
                texture,
                Value::Sampler(SamplerFilter::Point),
                Value::vec(&uv),
                block(&folded.segment, &values),
            ],
        );

        let mut plain_eval = Eval::new(composition.module());
        let texture = plain_eval.texture(Texture::continuous(16, 16, brightened));
        let from_plain = plain_eval.call(
            plain_eval.function(&plain.function),
            vec![
                texture,
                Value::Sampler(SamplerFilter::Point),
                Value::vec(&uv),
                block(plain, &values),
            ],
        );

        assert_close(&from_folded, &from_plain, 1e-6);
    }
}

#[test]
fn a_filtering_sampler_is_not_foldable() {
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let blur = parse("blur", SnippetKind::Spatial, BLUR);
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&blur)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");

    let [
        Piece::Color(_),
        Piece::Spatial {
            folded,
            sampler,
            not_foldable,
            ..
        },
    ] = composition.pieces()
    else {
        panic!("expected a colour piece then a spatial piece");
    };
    assert_eq!(*sampler, SamplerFilter::Filtered);
    assert!(
        folded.is_none(),
        "a filtered sample computes prefix(lerp(..))"
    );
    assert_eq!(*not_foldable, Some(FoldBlocker::FilteringSampler));
}

#[test]
fn gathers_are_not_foldable() {
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let gather = parse("gather", SnippetKind::Spatial, GATHER);
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&gather)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let [
        Piece::Color(_),
        Piece::Spatial {
            folded,
            not_foldable,
            ..
        },
    ] = composition.pieces()
    else {
        panic!("expected a colour piece then a spatial piece");
    };
    assert!(folded.is_none(), "a gather must not fold");
    assert_eq!(*not_foldable, Some(FoldBlocker::Gather));
}

#[test]
fn a_colour_prefix_folds_into_a_size_dependent_stage() {
    // `measure` needs the input's extent — supplied by the `size` argument
    // rather than queried from the texture — and stays foldable.
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let measure = parse("measure", SnippetKind::Spatial, MEASURE);
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&measure)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let [
        Piece::Color(_),
        Piece::Spatial {
            plain,
            folded,
            not_foldable,
            ..
        },
    ] = composition.pieces()
    else {
        panic!("expected a colour piece then a spatial piece");
    };
    assert_eq!(*not_foldable, None);
    let folded = folded.as_ref().expect("the stage is foldable");

    let amount = 0.2;
    let scale = 0.5;
    let source = |uv: [f32; 2]| [uv[0], uv[1], 0.5, 0.75];
    let brightened = move |uv: [f32; 2]| {
        let [r, g, b, a] = source(uv);
        [
            a.mul_add(amount, r),
            a.mul_add(amount, g),
            a.mul_add(amount, b),
            a,
        ]
    };
    let values = HashMap::from([
        ((0, "amount"), Value::Float(amount)),
        ((1, "amount"), Value::Float(scale)),
    ]);

    for uv in [[0.5, 0.5], [0.2, 0.7]] {
        let mut folded_eval = Eval::new(composition.module());
        let texture = folded_eval.texture(Texture::continuous(16, 16, source));
        let from_folded = folded_eval.call(
            folded_eval.function(&folded.segment.function),
            vec![
                texture,
                Value::Sampler(SamplerFilter::Point),
                Value::vec(&uv),
                block(&folded.segment, &values),
            ],
        );

        let mut plain_eval = Eval::new(composition.module());
        let texture = plain_eval.texture(Texture::continuous(16, 16, brightened));
        let from_plain = plain_eval.call(
            plain_eval.function(&plain.function),
            vec![
                texture,
                Value::Sampler(SamplerFilter::Point),
                Value::vec(&uv),
                block(plain, &values),
            ],
        );

        assert_close(&from_folded, &from_plain, 1e-6);
    }
}

#[test]
fn a_query_of_the_stage_input_is_rejected() {
    // The extent arrives through `size`; `textureDimensions` on `input`
    // — directly or through a helper — is a parse error naming it.
    let direct = "fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
        let dims = textureDimensions(input);
        return textureSampleLevel(input, input_point_sampler, uv, 0.0);
    }";
    let error = Snippet::parse(&SnippetSource::new("direct", SnippetKind::Spatial, direct))
        .expect_err("querying the input's extent is rejected");
    let SnippetError::Abi { reason, .. } = error else {
        panic!("expected an ABI error, got {error:?}");
    };
    assert!(reason.contains("size"), "{reason}");

    let helper = "fn dims_of(image: texture_2d<f32>) -> vec2<f32> {
        return vec2<f32>(textureDimensions(image));
    }
    fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
        let dims = dims_of(input);
        return textureSampleLevel(input, input_point_sampler, uv, 0.0);
    }";
    let error = Snippet::parse(&SnippetSource::new("helper", SnippetKind::Spatial, helper))
        .expect_err("a transitive query of the input's extent is rejected");
    assert!(matches!(error, SnippetError::Abi { .. }), "{error:?}");

    // The same query on an auxiliary image is still fine.
    let aux = "fn dims_of(image: texture_2d<f32>) -> vec2<f32> {
        return vec2<f32>(textureDimensions(image));
    }
    fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, aux0: texture_2d<f32>) -> vec4<f32> {
        let dims = dims_of(aux0);
        return textureSampleLevel(input, input_point_sampler, uv, 0.0);
    }";
    Snippet::parse(&SnippetSource::new("aux", SnippetKind::Spatial, aux))
        .expect("querying an aux image's extent is allowed");
}

#[test]
fn a_library_helper_is_imported_once() {
    const LIB: &str = "fn boost(x: vec4<f32>) -> vec4<f32> {
        return x + vec4<f32>(0.25, 0.25, 0.25, 0.0);
    }";
    const USES_LIB: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
        return boost(color);
    }";
    let library = || LibrarySource::new("boosters", LIB);
    let source = || SnippetSource::new("user", SnippetKind::Color, USES_LIB).library(library());
    let first = Snippet::parse(&source()).expect("the snippet parses");
    let second = Snippet::parse(&source()).expect("the snippet parses");
    let composition = compose(
        &[Stage::new(&first), Stage::new(&second)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");

    let copies = composition
        .module()
        .functions
        .iter()
        .filter(|(_, function)| function.name.as_deref() == Some("boost"))
        .count();
    assert_eq!(copies, 1, "the helper is imported exactly once");

    let segment = only_colour(&composition);
    let composed = run(
        &composition,
        segment,
        vec![
            Value::vec(&[0.1, 0.1, 0.1, 1.0]),
            block(segment, &HashMap::new()),
        ],
    );
    let expected = Value::vec(&[0.6, 0.6, 0.6, 1.0]);
    assert_close(&composed, &expected, 1e-6);
}

#[test]
fn library_name_collisions_are_rejected() {
    const BOOST_A: &str = "fn boost(x: vec4<f32>) -> vec4<f32> {
        return x * 2.0;
    }";
    const BOOST_B: &str = "fn boost(x: vec4<f32>) -> vec4<f32> {
        return x * 4.0;
    }";
    const USES_BOOST: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
        return boost(color);
    }";
    const OWNS_BOOST: &str = "fn boost(x: vec4<f32>) -> vec4<f32> {
        return x * 8.0;
    }
    fn apply(color: vec4<f32>) -> vec4<f32> {
        return boost(color);
    }";

    // Two libraries with different names providing the same function name.
    let first = Snippet::parse(
        &SnippetSource::new("first", SnippetKind::Color, USES_BOOST)
            .library(LibrarySource::new("fast", BOOST_A)),
    )
    .expect("the snippet parses");
    let second = Snippet::parse(
        &SnippetSource::new("second", SnippetKind::Color, USES_BOOST)
            .library(LibrarySource::new("faster", BOOST_B)),
    )
    .expect("the snippet parses");
    let collision = compose(
        &[Stage::new(&first), Stage::new(&second)],
        ComposeOptions::default(),
    );
    assert!(
        matches!(collision, Err(ComposeError::LibraryConflict { ref name, .. }) if name == "boost"),
        "{collision:?}"
    );

    // Two libraries under one name that are not the same library.
    let other = Snippet::parse(
        &SnippetSource::new("other", SnippetKind::Color, USES_BOOST)
            .library(LibrarySource::new("fast", BOOST_B)),
    )
    .expect("the snippet parses");
    let collision = compose(
        &[Stage::new(&first), Stage::new(&other)],
        ComposeOptions::default(),
    );
    assert!(
        matches!(collision, Err(ComposeError::DuplicateLibrary { ref name }) if name == "fast"),
        "{collision:?}"
    );

    // A snippet's own function colliding with a library's, in another stage.
    let own = parse("own", SnippetKind::Color, OWNS_BOOST);
    let collision = compose(
        &[Stage::new(&own), Stage::new(&first)],
        ComposeOptions::default(),
    );
    assert!(
        matches!(collision, Err(ComposeError::LibraryConflict { ref name, .. }) if name == "boost"),
        "{collision:?}"
    );
}

#[test]
fn a_library_needs_the_snippets_variants() {
    const LIB: &str = "fn boost(x: vec4<f32>) -> vec4<f32> {
        return x * 2.0;
    }";
    const USES_LIB: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
        return boost(color);
    }";
    const USES_LIB_F16: &str = "fn apply(color: vec4<f16>) -> vec4<f16> {
        return boost(color);
    }";
    let error = Snippet::parse(
        &SnippetSource::new("user", SnippetKind::Color, USES_LIB)
            .variant(
                Variant {
                    precision: Precision::F16,
                    subgroups: false,
                },
                USES_LIB_F16,
            )
            .library(LibrarySource::new("boosters", LIB)),
    )
    .expect_err("the library has no f16 source");
    assert!(matches!(error, SnippetError::Abi { .. }), "{error:?}");
}

#[test]
fn the_fold_is_not_equivalent_under_a_filtering_sampler() {
    // `square` does not commute with interpolation, so folding it into a
    // filtered sample changes the result — the reason filtered stages report
    // `FoldBlocker::FilteringSampler`.
    let square = parse("square", SnippetKind::Color, SQUARE);
    let kernel = parse("kernel", SnippetKind::Spatial, KERNEL);
    let composition = compose(
        &[Stage::new(&square), Stage::new(&kernel)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let [
        Piece::Color(_),
        Piece::Spatial {
            plain,
            folded: Some(folded),
            ..
        },
    ] = composition.pieces()
    else {
        panic!("expected a colour piece then a foldable spatial piece");
    };

    let uv = [0.51, 0.5];
    let texel = |x: u32, y: u32| {
        #[allow(clippy::cast_precision_loss, reason = "test texels are few")]
        [x as f32 / 16.0, y as f32 / 16.0, 0.5, 0.75]
    };
    let squared = |x: u32, y: u32| {
        let [r, g, b, a] = texel(x, y);
        [r * r, g * g, b * b, a]
    };
    let values = HashMap::from([((1, "step"), Value::vec(&[0.02, 0.0]))]);

    let mut folded_eval = Eval::new(composition.module());
    let texture = folded_eval.texture(Texture::texels(16, 16, texel));
    let from_folded = folded_eval.call(
        folded_eval.function(&folded.segment.function),
        vec![
            texture,
            Value::Sampler(SamplerFilter::Filtered),
            Value::vec(&uv),
            block(&folded.segment, &values),
        ],
    );
    let mut plain_eval = Eval::new(composition.module());
    let texture = plain_eval.texture(Texture::texels(16, 16, squared));
    let from_plain = plain_eval.call(
        plain_eval.function(&plain.function),
        vec![
            texture,
            Value::Sampler(SamplerFilter::Filtered),
            Value::vec(&uv),
            block(plain, &values),
        ],
    );

    let difference: f32 = from_folded
        .components()
        .iter()
        .zip(from_plain.components())
        .map(|(folded, plain)| (folded - plain).abs())
        .sum();
    assert!(difference > 1e-3, "filtered fold must differ: {difference}");
}

#[test]
fn a_renamed_but_equivalent_working_space_composes() {
    let renamed = SATURATION.replace("WorkingSpace", "Space");
    let saturation = parse("saturation", SnippetKind::Color, &renamed);
    assert!(saturation.needs_working_space());
    let composition =
        compose(&[Stage::new(&saturation)], ComposeOptions::default()).expect("the chain composes");
    let segment = only_colour(&composition);

    let values = HashMap::from([((0, "amount"), Value::Float(0.8))]);
    let colour = [0.2, 0.4, 0.6, 1.0];
    let composed = run(
        &composition,
        segment,
        vec![Value::vec(&colour), block(segment, &values), space()],
    );
    let expected = apply(
        SATURATION,
        vec![
            Value::vec(&colour),
            Value::Struct(vec![Value::Float(0.8)]),
            space(),
        ],
    );
    assert_close(&composed, &expected, 1e-6);
}

#[test]
fn a_non_equivalent_working_space_is_rejected_at_parse() {
    let wider = "struct WorkingSpace { luma: vec3<f32>, pad: f32 }
fn apply(color: vec4<f32>, space: WorkingSpace) -> vec4<f32> {
    return color;
}";
    let error = Snippet::parse(&SnippetSource::new("wider", SnippetKind::Color, wider))
        .expect_err("a wider working-space block is not equivalent");
    assert!(
        matches!(error, SnippetError::Abi { ref reason, .. } if reason.contains("space")),
        "{error:?}"
    );

    let dimmer = "struct WorkingSpace { luma: vec2<f32> }
fn apply(color: vec4<f32>, space: WorkingSpace) -> vec4<f32> {
    return color;
}";
    let error = Snippet::parse(&SnippetSource::new("dimmer", SnippetKind::Color, dimmer))
        .expect_err("a narrower member is not equivalent");
    assert!(matches!(error, SnippetError::Abi { .. }), "{error:?}");
}

#[test]
fn fold_cost_counts_helper_expressions() {
    let contrast = parse("contrast", SnippetKind::Color, CONTRAST);
    let kernel = parse("kernel", SnippetKind::Spatial, KERNEL);
    let composition = compose(
        &[Stage::new(&contrast), Stage::new(&kernel)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let [
        Piece::Color(_),
        Piece::Spatial {
            folded: Some(folded),
            ..
        },
    ] = composition.pieces()
    else {
        panic!("expected a colour piece then a foldable spatial piece");
    };

    let module = module(CONTRAST);
    let ops = |name: &str| {
        module
            .functions
            .iter()
            .find(|(_, function)| function.name.as_deref() == Some(name))
            .map(|(_, function)| {
                u32::try_from(
                    function
                        .expressions
                        .iter()
                        .filter(|(_, expression)| !expression.needs_pre_emit())
                        .count(),
                )
                .expect("the count fits in u32")
            })
            .expect("the function exists")
    };
    assert_eq!(folded.cost.prefix_ops, ops("apply") + ops("scale"));
}

#[test]
fn every_composed_module_emits_for_every_back_end() {
    let brightness = Snippet::parse(
        &SnippetSource::new("brightness", SnippetKind::Color, BRIGHTNESS).variant(
            Variant {
                precision: Precision::F16,
                subgroups: false,
            },
            BRIGHTNESS_F16,
        ),
    )
    .expect("both variants are valid");
    let saturation = parse("saturation", SnippetKind::Color, SATURATION);
    let kernel = parse("kernel", SnippetKind::Spatial, KERNEL);
    let invert = parse("invert", SnippetKind::Color, INVERT);
    let composition = compose(
        &[
            Stage::new(&brightness),
            Stage::new(&saturation),
            Stage::new(&kernel),
            Stage::new(&invert),
        ],
        ComposeOptions {
            precision: Precision::F16,
            subgroups: false,
        },
    )
    .expect("the chain composes");

    let [
        Piece::Color(first),
        Piece::Spatial {
            folded: Some(folded),
            ..
        },
        Piece::Color(_),
    ] = composition.pieces()
    else {
        panic!("unexpected pieces {:?}", composition.pieces());
    };
    assert_eq!(first.variants[0].precision, Precision::F16);
    assert_eq!(first.variants[1].precision, Precision::F32);
    assert_eq!(folded.segment.stages, 0..3);
    assert!(
        composition
            .capabilities()
            .contains(Capabilities::SHADER_FLOAT16)
    );

    let info = validate(composition.module(), composition.capabilities()).expect("valid");
    let source = wgsl(composition.module(), &info).expect("WGSL emits");
    let reparsed = naga::front::wgsl::parse_str(&source).expect("emitted WGSL parses");
    validate(&reparsed, composition.capabilities()).expect("emitted WGSL validates");

    let metal = msl(composition.module(), &info, (2, 4)).expect("MSL emits");
    assert!(metal.contains(&folded.segment.function));

    let words = spirv(composition.module(), &info, (1, 3)).expect("SPIR-V emits");
    assert_eq!(words.first(), Some(&0x0723_0203), "SPIR-V magic number");
}

#[test]
fn subgroup_variants_are_used_where_declared() {
    let brightness = Snippet::parse(
        &SnippetSource::new("brightness", SnippetKind::Color, BRIGHTNESS).variant(
            Variant {
                precision: Precision::F32,
                subgroups: true,
            },
            BRIGHTNESS_SUBGROUPS,
        ),
    )
    .expect("both variants are valid");
    let invert = parse("invert", SnippetKind::Color, INVERT);
    let stages = [Stage::new(&brightness), Stage::new(&invert)];

    let with = compose(
        &stages,
        ComposeOptions {
            precision: Precision::F32,
            subgroups: true,
        },
    )
    .expect("the chain composes");
    assert_eq!(
        only_colour(&with).variants,
        [
            Variant {
                precision: Precision::F32,
                subgroups: true
            },
            Variant::BASE
        ]
    );
    assert!(with.capabilities().contains(Capabilities::SUBGROUP));

    let without = compose(&stages, ComposeOptions::default()).expect("the chain composes");
    assert_eq!(
        only_colour(&without).variants,
        [Variant::BASE, Variant::BASE]
    );
    assert!(!without.capabilities().contains(Capabilities::SUBGROUP));
}

#[test]
fn contract_violations_are_reported() {
    let missing = Snippet::parse(&SnippetSource::new(
        "missing",
        SnippetKind::Color,
        MISSING_COLOR,
    ));
    assert!(
        matches!(missing, Err(SnippetError::Abi { .. })),
        "{missing:?}"
    );

    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let unknown = compose(
        &[Stage::new(&brightness).constant("radius", 1.0)],
        ComposeOptions::default(),
    );
    assert!(
        matches!(unknown, Err(ComposeError::UnknownParam { .. })),
        "{unknown:?}"
    );

    let mistyped = compose(
        &[Stage::new(&brightness).constant("amount", [1.0, 2.0])],
        ComposeOptions::default(),
    );
    assert!(
        matches!(mistyped, Err(ComposeError::ParamType { .. })),
        "{mistyped:?}"
    );

    assert!(matches!(
        compose(&[], ComposeOptions::default()),
        Err(ComposeError::EmptyChain)
    ));
}

#[test]
fn a_colour_prefix_folds_through_a_library_sampler() {
    // The `load` helper pattern every built-in spatial stage uses: samples
    // of `input` inside a library callee still fold.
    const LOAD: &str = "fn load(input: texture_2d<f32>, input_point_sampler: sampler, size: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
        return textureSampleLevel(input, input_point_sampler, (floor(pixel) + 0.5) / size, 0.0);
    }";
    const USES_LOAD: &str = "fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
        return load(input, input_point_sampler, size, uv * size);
    }";
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let spatial = Snippet::parse(
        &SnippetSource::new("loads", SnippetKind::Spatial, USES_LOAD)
            .library(LibrarySource::new("sampling", LOAD)),
    )
    .expect("the snippet parses");
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&spatial)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");

    let [
        Piece::Color(_),
        Piece::Spatial {
            plain,
            folded,
            not_foldable,
            ..
        },
    ] = composition.pieces()
    else {
        panic!("expected a colour piece then a spatial piece");
    };
    assert_eq!(*not_foldable, None, "a sample through `load` folds");
    let folded = folded.as_ref().expect("a sample through `load` folds");
    assert_eq!(folded.cost.samples, SampleCount::Static(1));

    let amount = 0.2;
    let source = |uv: [f32; 2]| [uv[0], uv[1], 0.5, 0.75];
    let brightened = move |uv: [f32; 2]| {
        let [r, g, b, a] = source(uv);
        [
            a.mul_add(amount, r),
            a.mul_add(amount, g),
            a.mul_add(amount, b),
            a,
        ]
    };
    let values = HashMap::from([((0, "amount"), Value::Float(amount))]);

    for uv in [[0.5, 0.5], [0.2, 0.7]] {
        let mut folded_eval = Eval::new(composition.module());
        let texture = folded_eval.texture(Texture::continuous(16, 16, source));
        let from_folded = folded_eval.call(
            folded_eval.function(&folded.segment.function),
            vec![
                texture,
                Value::Sampler(SamplerFilter::Point),
                Value::vec(&uv),
                block(&folded.segment, &values),
            ],
        );

        let mut plain_eval = Eval::new(composition.module());
        let texture = plain_eval.texture(Texture::continuous(16, 16, brightened));
        let from_plain = plain_eval.call(
            plain_eval.function(&plain.function),
            vec![
                texture,
                Value::Sampler(SamplerFilter::Point),
                Value::vec(&uv),
                block(plain, &values),
            ],
        );

        assert_close(&from_folded, &from_plain, 1e-6);
    }
}

#[test]
fn a_library_callee_is_imported_once() {
    // `hsl_to_rgb` calls `hue_to_rgb`; a stage calling both must still see
    // exactly one `hue_to_rgb` in the composed module.
    const HSL: &str = "fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
        if t < 0.0 { return p; }
        return q;
    }
    fn hsl_to_rgb(hsl: vec3<f32>) -> vec3<f32> {
        return vec3<f32>(
            hue_to_rgb(hsl.x, hsl.y, hsl.z + 1.0 / 3.0),
            hue_to_rgb(hsl.x, hsl.y, hsl.z),
            hue_to_rgb(hsl.x, hsl.y, hsl.z - 1.0 / 3.0),
        );
    }";
    const USES_HSL: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
        return vec4<f32>(hsl_to_rgb(color.rgb) + vec3<f32>(hue_to_rgb(0.0, 0.0, color.a)), color.a);
    }";
    let snippet = || {
        Snippet::parse(
            &SnippetSource::new("user", SnippetKind::Color, USES_HSL)
                .library(LibrarySource::new("hsl", HSL)),
        )
        .expect("the snippet parses")
    };
    let first = snippet();
    let second = snippet();
    let composition = compose(
        &[Stage::new(&first), Stage::new(&second)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");

    let copies = |name: &str| {
        composition
            .module()
            .functions
            .iter()
            .filter(|(_, function)| function.name.as_deref() == Some(name))
            .count()
    };
    assert_eq!(copies("hue_to_rgb"), 1, "the callee is imported once");
    assert_eq!(copies("hsl_to_rgb"), 1, "the caller is imported once");
}

#[test]
fn an_f16_library_parses_and_composes() {
    // The library's `enable f16;` belongs to its own parse; the combined
    // snippet source relies on the snippet's own directive.
    const LIB_F32: &str = "fn boost(x: vec4<f32>) -> vec4<f32> {
        return x * 2.0;
    }";
    const LIB_F16: &str = "enable f16;
    fn boost(x: vec4<f16>) -> vec4<f16> {
        return x * 2.0;
    }";
    const USES_LIB: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
        return boost(color);
    }";
    const USES_LIB_F16: &str = "enable f16;
    fn apply(color: vec4<f16>) -> vec4<f16> {
        return boost(color);
    }";
    let snippet = Snippet::parse(
        &SnippetSource::new("user", SnippetKind::Color, USES_LIB)
            .variant(
                Variant {
                    precision: Precision::F16,
                    subgroups: false,
                },
                USES_LIB_F16,
            )
            .library(LibrarySource::new("boosters", LIB_F32).f16(LIB_F16)),
    )
    .expect("the f16 library parses");
    let composition = compose(
        &[Stage::new(&snippet)],
        ComposeOptions {
            precision: Precision::F16,
            subgroups: false,
        },
    )
    .expect("the chain composes");
    let copies = composition
        .module()
        .functions
        .iter()
        .filter(|(_, function)| function.name.as_deref() == Some("boost"))
        .count();
    assert_eq!(copies, 1, "the f16 helper is imported once");
}

#[test]
fn the_same_library_may_register_twice() {
    const LIB: &str = "fn boost(x: vec4<f32>) -> vec4<f32> {
        return x * 2.0;
    }";
    const USES_LIB: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
        return boost(color);
    }";
    let library = || LibrarySource::new("boosters", LIB);
    Snippet::parse(
        &SnippetSource::new("user", SnippetKind::Color, USES_LIB)
            .library(library())
            .library(library()),
    )
    .expect("registering the same library twice is no conflict");
}

#[test]
fn a_library_defines_no_main() {
    const MAIN: &str = "fn main(x: vec4<f32>) -> vec4<f32> {
        return x;
    }";
    const USES: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
        return color;
    }";
    let error = Snippet::parse(
        &SnippetSource::new("user", SnippetKind::Color, USES)
            .library(LibrarySource::new("mains", MAIN)),
    )
    .expect_err("`main` collides with the executor's entry point");
    assert!(matches!(error, SnippetError::Library { .. }), "{error:?}");
}

/// Calls a spatial segment's function at `uv`: `input` is the stage's
/// texture, `aux` every aux image, `values` the uniform block's members.
fn eval_spatial(
    eval: &Eval<'_>,
    segment: &Segment,
    uv: [f32; 2],
    input: &Value,
    aux: &Value,
    values: &HashMap<(usize, &str), Value>,
) -> Value {
    let args = segment
        .args
        .iter()
        .map(|arg| match *arg {
            SegmentArg::Input => input.clone(),
            SegmentArg::InputSampler => Value::Sampler(SamplerFilter::Point),
            SegmentArg::Uv => Value::vec(&uv),
            SegmentArg::Params => block(segment, values),
            SegmentArg::Aux(_) => aux.clone(),
            ref other => panic!("unexpected segment arg {other:?}"),
        })
        .collect();
    eval.call(eval.function(&segment.function), args)
}

/// The spatial piece of a two-stage colour-then-spatial composition.
fn spatial_piece(composition: &Composition) -> (&Segment, &Option<Folded>, &Option<FoldBlocker>) {
    match composition.pieces() {
        [
            Piece::Color(_),
            Piece::Spatial {
                plain,
                folded,
                not_foldable,
                ..
            },
        ] => (plain, folded, not_foldable),
        other => panic!("expected a colour piece then a spatial piece, got {other:?}"),
    }
}

/// Asserts the folded segment's result at each `uv` equals the plain
/// segment's result on the brightened texture — folding applies the prefix
/// per sample; unfolding applies it to the texture.
fn folded_matches_plain(
    composition: &Composition,
    plain: &Segment,
    folded: &Segment,
    aux: impl Fn([f32; 2]) -> [f32; 4],
) {
    let amount = 0.2;
    let source = |uv: [f32; 2]| [uv[0], uv[1], 0.5, 0.75];
    let brightened = move |uv: [f32; 2]| {
        let [r, g, b, a] = source(uv);
        [
            a.mul_add(amount, r),
            a.mul_add(amount, g),
            a.mul_add(amount, b),
            a,
        ]
    };
    let values = HashMap::from([((0, "amount"), Value::Float(amount))]);
    for uv in [[0.5, 0.5], [0.2, 0.7]] {
        let mut folded_eval = Eval::new(composition.module());
        let input = folded_eval.texture(Texture::continuous(16, 16, source));
        let aux_value = folded_eval.texture(Texture::continuous(16, 16, &aux));
        let from_folded = eval_spatial(&folded_eval, folded, uv, &input, &aux_value, &values);

        let mut plain_eval = Eval::new(composition.module());
        let input = plain_eval.texture(Texture::continuous(16, 16, brightened));
        let aux_value = plain_eval.texture(Texture::continuous(16, 16, &aux));
        let from_plain = eval_spatial(&plain_eval, plain, uv, &input, &aux_value, &values);

        assert_close(&from_folded, &from_plain, 1e-6);
    }
}

const TAP: &str =
    "fn tap(tex: texture_2d<f32>, s: sampler, size: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(tex, s, (floor(pixel) + 0.5) / size, 0.0);
}";

#[test]
fn a_helper_folds_only_at_the_sites_bound_to_input() {
    // `tap` is called with `input` and with `aux0` in its texture slot: the
    // folded copy folds the `input` site's sample; the `aux0` site keeps
    // pointing at the unchanged helper.
    const SRC: &str = "fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, aux0: texture_2d<f32>) -> vec4<f32> {
        return tap(input, input_point_sampler, size, uv * size)
             + tap(aux0, input_point_sampler, size, uv * size);
    }";
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let spatial = parse("taps", SnippetKind::Spatial, &(TAP.to_owned() + SRC));
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&spatial)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let (plain, folded, not_foldable) = spatial_piece(&composition);
    assert_eq!(*not_foldable, None);
    let folded = folded.as_ref().expect("the input-bound site folds");
    // Only the `input` site's sample counts.
    assert_eq!(folded.cost.samples, SampleCount::Static(1));
    folded_matches_plain(&composition, plain, &folded.segment, |_| [0.25; 4]);
}

#[test]
fn a_helper_folds_every_sample_it_takes() {
    // Two samples of `input` in the helper both fold.
    const SRC: &str = "fn tap2(tex: texture_2d<f32>, s: sampler, size: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
        return tap(tex, s, size, pixel) + tap(tex, s, size, pixel + vec2<f32>(1.0, 0.0));
    }
    fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
        return tap2(input, input_point_sampler, size, uv * size);
    }";
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let spatial = parse("taps", SnippetKind::Spatial, &(TAP.to_owned() + SRC));
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&spatial)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let (plain, folded, not_foldable) = spatial_piece(&composition);
    assert_eq!(*not_foldable, None);
    let folded = folded.as_ref().expect("the samples fold");
    assert_eq!(folded.cost.samples, SampleCount::Static(2));
    folded_matches_plain(&composition, plain, &folded.segment, |_| [0.25; 4]);
}

#[test]
fn a_fold_reaches_two_calls_deep() {
    // `middle` samples nothing itself; the sample lives in `deepest`.
    const SRC: &str = "fn middle(tex: texture_2d<f32>, s: sampler, size: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
        return tap(tex, s, size, pixel) * 2.0;
    }
    fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
        return middle(input, input_point_sampler, size, uv * size);
    }";
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let spatial = parse("deep", SnippetKind::Spatial, &(TAP.to_owned() + SRC));
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&spatial)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let (plain, folded, not_foldable) = spatial_piece(&composition);
    assert_eq!(*not_foldable, None);
    let folded = folded.as_ref().expect("the transitive sample folds");
    assert_eq!(folded.cost.samples, SampleCount::Static(1));
    folded_matches_plain(&composition, plain, &folded.segment, |_| [0.25; 4]);
}

#[test]
fn a_helper_folds_input_leaving_aux_alone() {
    // One helper samples both textures; only the `input` sample folds.
    const SRC: &str = "fn both(a: texture_2d<f32>, b: texture_2d<f32>, s: sampler, size: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
        return tap(a, s, size, pixel) - tap(b, s, size, pixel);
    }
    fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>, aux0: texture_2d<f32>) -> vec4<f32> {
        return both(input, aux0, input_point_sampler, size, uv * size);
    }";
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let spatial = parse("both", SnippetKind::Spatial, &(TAP.to_owned() + SRC));
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&spatial)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let (plain, folded, not_foldable) = spatial_piece(&composition);
    assert_eq!(*not_foldable, None);
    let folded = folded.as_ref().expect("the input sample folds");
    assert_eq!(folded.cost.samples, SampleCount::Static(1));
    folded_matches_plain(&composition, plain, &folded.segment, |_| [0.25; 4]);
}

#[test]
fn a_helper_gather_blocks_the_fold() {
    const SRC: &str = "fn gathered(tex: texture_2d<f32>, s: sampler, uv: vec2<f32>) -> vec4<f32> {
        return textureGather(0, tex, s, uv);
    }
    fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
        return gathered(input, input_point_sampler, uv);
    }";
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let spatial = parse("gatherer", SnippetKind::Spatial, SRC);
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&spatial)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let (_, folded, not_foldable) = spatial_piece(&composition);
    assert_eq!(*not_foldable, Some(FoldBlocker::Gather));
    assert!(folded.is_none());
}

#[test]
fn a_helper_sampling_in_a_loop_reports_a_dynamic_count() {
    const SRC: &str = "fn spread(tex: texture_2d<f32>, s: sampler, size: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
        var acc = vec4<f32>(0.0);
        for (var i = 0; i < 3; i++) {
            acc += tap(tex, s, size, pixel + vec2<f32>(f32(i), 0.0));
        }
        return acc;
    }
    fn apply(input: texture_2d<f32>, input_point_sampler: sampler, uv: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
        return spread(input, input_point_sampler, size, uv * size);
    }";
    let brightness = parse("brightness", SnippetKind::Color, BRIGHTNESS);
    let spatial = parse("spread", SnippetKind::Spatial, &(TAP.to_owned() + SRC));
    let composition = compose(
        &[Stage::new(&brightness), Stage::new(&spatial)],
        ComposeOptions::default(),
    )
    .expect("the chain composes");
    let (_, folded, not_foldable) = spatial_piece(&composition);
    assert_eq!(*not_foldable, None);
    // The evaluator cannot run loops, so the count is the check here.
    let folded = folded.as_ref().expect("loop samples still fold");
    assert_eq!(folded.cost.samples, SampleCount::Dynamic);
}

#[test]
fn library_conflicts_are_checked_at_the_composed_precision() {
    // `collide` exists only in `a`'s f16 source and `b`'s f32 source: at f32
    // the two never meet in one module.
    const A_F32: &str = "fn only_a(x: vec4<f32>) -> vec4<f32> {
        return x;
    }";
    const A_F16: &str = "enable f16;
    fn collide(x: vec4<f16>) -> vec4<f16> {
        return x;
    }
    fn only_a(x: vec4<f16>) -> vec4<f16> {
        return collide(x);
    }";
    const B_F32: &str = "fn collide(x: vec4<f32>) -> vec4<f32> {
        return x * 2.0;
    }
    fn only_b(x: vec4<f32>) -> vec4<f32> {
        return collide(x);
    }";
    const USES_A: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
        return only_a(color);
    }";
    const USES_B: &str = "fn apply(color: vec4<f32>) -> vec4<f32> {
        return only_b(color);
    }";
    let a = Snippet::parse(
        &SnippetSource::new("a", SnippetKind::Color, USES_A)
            .variant(
                Variant {
                    precision: Precision::F16,
                    subgroups: false,
                },
                "enable f16;\nfn apply(color: vec4<f16>) -> vec4<f16> { return only_a(color); }",
            )
            .library(LibrarySource::new("a", A_F32).f16(A_F16)),
    )
    .expect("the snippet parses");
    let b = Snippet::parse(
        &SnippetSource::new("b", SnippetKind::Color, USES_B)
            .library(LibrarySource::new("b", B_F32)),
    )
    .expect("the snippet parses");

    compose(&[Stage::new(&a), Stage::new(&b)], ComposeOptions::default()).expect(
        "f32 composition sees no conflict between `a`'s f16 `collide` and `b`'s f32 `collide`",
    );

    // At f16, `a` selects its f16 variant — whose `collide` now shares the
    // module with `b`'s f32 `collide` (`b` has no f16, so it falls back).
    // The two names really do collide there.
    let f16 = compose(
        &[Stage::new(&a), Stage::new(&b)],
        ComposeOptions {
            precision: Precision::F16,
            subgroups: false,
        },
    );
    assert!(
        matches!(f16, Err(ComposeError::LibraryConflict { .. })),
        "{f16:?}"
    );
}
