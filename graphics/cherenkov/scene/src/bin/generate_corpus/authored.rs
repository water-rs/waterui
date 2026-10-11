//! Corpus scenes that were committed directly rather than generated.
//!
//! These scenes were written by tooling that serialised scene JSON with
//! Python's `json.dumps`: integral values are integer literals, indentation
//! is two spaces except for the `blend-space-*` scenes which use one, and
//! object member order follows the authoring order rather than the Rust
//! field order. `Scene::save` cannot reproduce those bytes, so each scene
//! is built here as an ordered JSON tree ([`J`]) rendered by [`py_dumps`].
//!
//! The tree is still validated as a scene: before writing, it is parsed
//! through `serde_json` and the `features` array is recomputed with
//! [`Scene::compute_features`], exactly as `Scene::load` does.

use std::fmt::Write as _;

use cherenkov_scene::corpus;
use cherenkov_scene::kurbo::Affine;
use cherenkov_scene::{Color, GlyphRun, NormalizedCoord, Paint, Scene, SceneError};
use fontique::FontWeight;
use serde_json::Value;

use crate::{Corpus, TextContext, checker_png, font_blob, font_blobs};

/// An ordered JSON value matching the byte layout of Python's
/// `json.dumps`: `I` keeps integral literals integer-typed, `F` is an
/// f64 literal and `F32` an f32 computed value (glyph positions and
/// sizes are f32 in the scene model and round-trip at f32 precision).
#[derive(Clone)]
enum J {
    I(i64),
    F(f64),
    F32(f32),
    S(String),
    A(Vec<Self>),
    O(Vec<(String, Self)>),
}

const fn i(v: i64) -> J {
    J::I(v)
}

const fn f(v: f64) -> J {
    J::F(v)
}

const fn f32v(v: f32) -> J {
    J::F32(v)
}

fn s(v: &str) -> J {
    J::S(v.to_string())
}

const fn a(items: Vec<J>) -> J {
    J::A(items)
}

fn o(pairs: Vec<(&'static str, J)>) -> J {
    J::O(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Serialise `v` the way Python's `json.dumps(v, indent=k)` does.
fn py_dumps(v: &J, indent: usize) -> String {
    let mut out = String::new();
    write_j(v, 0, indent, &mut out);
    out
}

fn write_j(v: &J, depth: usize, indent: usize, out: &mut String) {
    let pad = |d: usize, out: &mut String| {
        for _ in 0..d * indent {
            out.push(' ');
        }
    };
    match v {
        J::I(v) => {
            write!(out, "{v}").expect("writing to a String never fails");
        }
        J::F(v) => out.push_str(&py_float(*v)),
        J::F32(v) => out.push_str(&serde_json::to_string(v).expect("f32 serialises")),
        J::S(v) => write_json_str(v, out),
        J::A(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push('\n');
                pad(depth + 1, out);
                write_j(item, depth + 1, indent, out);
            }
            out.push('\n');
            pad(depth, out);
            out.push(']');
        }
        J::O(pairs) => {
            if pairs.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (idx, (k, v)) in pairs.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push('\n');
                pad(depth + 1, out);
                write_json_str(k, out);
                out.push_str(": ");
                write_j(v, depth + 1, indent, out);
            }
            out.push('\n');
            pad(depth, out);
            out.push('}');
        }
    }
}

fn write_json_str(v: &str, out: &mut String) {
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                write!(out, "\\u{:04x}", u32::from(c)).expect("writing to a String never fails");
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `repr(float)` the way `CPython` prints it: the shortest round-trip
/// decimal in fixed notation for `1e-4 <= |v| < 1e16`, scientific with a
/// signed two-digit exponent otherwise.
fn py_float(v: f64) -> String {
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    // serde_json's float rendering produces the same shortest round-trip
    // digits; take them apart and lay them out the way `repr` does.
    let raw = serde_json::to_string(&v).expect("a finite f64 serialises");
    let (neg, raw) = raw
        .strip_prefix('-')
        .map_or((false, raw.as_str()), |rest| (true, rest));
    let (mantissa, exp) = match raw.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i64>().expect("ryu exponent is an integer")),
        None => (raw, 0),
    };
    let point = i64::try_from(mantissa.find('.').unwrap_or(mantissa.len()))
        .expect("a mantissa length fits i64");
    let digits_all: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits_all.trim_start_matches('0');
    let lead_zeros = i64::try_from(digits_all.len() - digits.len()).expect("fits i64");
    let digits = digits.trim_end_matches('0');
    // Value = D.DDD * 10^e with one significant digit before the point.
    let e = exp + point - lead_zeros - 1;
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if (-4..=15).contains(&e) {
        let split = e + 1;
        if split <= 0 {
            out.push_str("0.");
            for _ in 0..-split {
                out.push('0');
            }
            out.push_str(digits);
        } else if usize::try_from(split).unwrap_or(0) >= digits.len() {
            out.push_str(digits);
            let missing = split - i64::try_from(digits.len()).unwrap_or(i64::MAX);
            for _ in 0..missing {
                out.push('0');
            }
            out.push_str(".0");
        } else {
            let split = usize::try_from(split).expect("split is positive");
            out.push_str(&digits[..split]);
            out.push('.');
            out.push_str(&digits[split..]);
        }
    } else {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        if e < 0 {
            write!(out, "-{:02}", -e).expect("writing to a String never fails");
        } else {
            write!(out, "+{e:02}").expect("writing to a String never fails");
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Scene-shape helpers
// ---------------------------------------------------------------------------

fn pt(x: J, y: J) -> J {
    o(vec![("x", x), ("y", y)])
}

fn colour(space: &str, components: Vec<J>) -> J {
    o(vec![("space", s(space)), ("components", a(components))])
}

fn solid(space: &str, components: Vec<J>) -> J {
    o(vec![("solid", colour(space, components))])
}

fn affine(v: [f64; 6]) -> J {
    a(v.into_iter().map(f).collect())
}

fn rect(x0: J, y0: J, x1: J, y1: J) -> J {
    o(vec![(
        "rect",
        o(vec![("x0", x0), ("y0", y0), ("x1", x1), ("y1", y1)]),
    )])
}

fn circle(cx: J, cy: J, r: J) -> J {
    o(vec![(
        "circle",
        o(vec![("center", pt(cx, cy)), ("radius", r)]),
    )])
}

fn fill(shape: J, paint: J) -> J {
    o(vec![(
        "draw",
        o(vec![(
            "fill",
            o(vec![
                ("shape", shape),
                ("rule", s("non-zero")),
                ("paint", paint),
            ]),
        )]),
    )])
}

fn stroke_draw(shape: J, paint: J, style: J) -> J {
    o(vec![(
        "draw",
        o(vec![(
            "stroke",
            o(vec![("shape", shape), ("paint", paint), ("stroke", style)]),
        )]),
    )])
}

fn glyphs_draw(run: J) -> J {
    o(vec![("draw", o(vec![("glyphs", run)]))])
}

fn layer(transform: J, blend: &str, motion: Option<J>, items: Vec<J>) -> J {
    let mut pairs = vec![
        ("transform", transform),
        ("opacity", f(1.0)),
        ("blend", s(blend)),
    ];
    if let Some(motion) = motion {
        pairs.push(("motion", motion));
    }
    pairs.push(("items", a(items)));
    o(vec![("layer", o(pairs))])
}

fn group(items: Vec<J>, blend_space: &str) -> J {
    o(vec![(
        "group",
        o(vec![
            ("items", a(items)),
            ("opacity", f(1.0)),
            ("blend", s("normal")),
            ("blend_space", s(blend_space)),
        ]),
    )])
}

fn stroke_style(width: f64, join: &str, caps: &str, dash: Vec<f64>, dash_offset: f64) -> J {
    o(vec![
        ("width", f(width)),
        ("join", s(join)),
        ("miter_limit", f(4.0)),
        ("start_cap", s(caps)),
        ("end_cap", s(caps)),
        ("dash_pattern", a(dash.into_iter().map(f).collect())),
        ("dash_offset", f(dash_offset)),
    ])
}

fn stop(offset: f64, space: &str, components: Vec<J>) -> J {
    o(vec![
        ("offset", f(offset)),
        ("color", colour(space, components)),
    ])
}

/// `{kind: {<geometry>, "stops": ..., "extend": ..., "interpolation": ...}}`.
fn gradient(
    kind: &str,
    pairs: Vec<(&'static str, J)>,
    stops: Vec<J>,
    extend: &str,
    interpolation: &str,
) -> J {
    let mut pairs = pairs;
    pairs.push(("stops", a(stops)));
    pairs.push(("extend", s(extend)));
    pairs.push(("interpolation", s(interpolation)));
    J::O(vec![(kind.to_string(), o(pairs))])
}

fn transformed(paint: J, transform: J) -> J {
    o(vec![(
        "transformed",
        o(vec![("paint", paint), ("transform", transform)]),
    )])
}

fn mesh_paint(columns: i64, rows: i64, points: Vec<(i64, i64)>, colors: Vec<J>, smooth: bool) -> J {
    let mut pairs = vec![
        ("columns", i(columns)),
        ("rows", i(rows)),
        (
            "points",
            a(points.into_iter().map(|(x, y)| pt(i(x), i(y))).collect()),
        ),
        ("colors", a(colors)),
    ];
    if smooth {
        pairs.push(("interpolation", s("smoothstep")));
    }
    o(vec![("mesh", o(pairs))])
}

/// A glyph run in the committed layout: run members in the authored order
/// (`stroke` after `paint`), glyph positions as f32 literals.
fn run_j(run: &GlyphRun, paint: J, stroke: Option<J>) -> J {
    let glyphs = run
        .glyphs
        .iter()
        .map(|g| {
            let mut members = vec![
                ("id", i(i64::from(g.id))),
                ("x", f32v(g.x)),
                ("y", f32v(g.y)),
            ];
            if let Some(transform) = g.transform {
                members.push(("transform", affine(transform.as_coeffs())));
            }
            o(members)
        })
        .collect();
    let mut members = vec![
        ("font", s(run.font.to_string().as_str())),
        ("font_index", i(i64::from(run.font_index))),
        ("size", f32v(run.size)),
        (
            "normalized_coords",
            a(run
                .normalized_coords
                .iter()
                .map(|c: &NormalizedCoord| {
                    o(vec![("tag", s(c.tag.as_str())), ("value", f32v(c.value))])
                })
                .collect()),
        ),
        ("glyphs", a(glyphs)),
        ("paint", paint),
    ];
    if let Some(stroke) = stroke {
        members.push(("stroke", stroke));
    }
    o(members)
}

/// A scene object with the committed member order; the `features` list is
/// left empty for [`emit`] to fill from `Scene::compute_features`.
fn scene_j(w: i64, h: i64, clear: Vec<J>, root_transform: J, items: Vec<J>) -> J {
    o(vec![
        ("width", i(w)),
        ("height", i(h)),
        ("working_space", s("linear-display-p3")),
        (
            "clear",
            o(vec![("space", s("srgb")), ("components", a(clear))]),
        ),
        ("features", a(Vec::new())),
        (
            "root",
            o(vec![
                ("transform", root_transform),
                ("opacity", f(1.0)),
                ("blend", s("normal")),
                ("items", a(items)),
            ]),
        ),
    ])
}

fn light_clear() -> Vec<J> {
    vec![f(0.95), f(0.95), f(0.95), f(1.0)]
}

fn dark_clear() -> Vec<J> {
    vec![f(0.2), f(0.2), f(0.2), f(1.0)]
}

const IDENTITY_F: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// The identity root transform as integer literals — the committed
/// files pin `[1, 0, 0, 1, 0, 0]`, not the float spelling.
fn identity_ints() -> J {
    a([1, 0, 0, 1, 0, 0].into_iter().map(i).collect())
}

/// One `features` entry: `{"feature": name}` plus `value` for the
/// payload-carrying variants (`blend`, `blend-space`,
/// `interpolation-space`).
fn feat(name: &str, value: Option<&str>) -> J {
    let mut members = vec![("feature", s(name))];
    if let Some(value) = value {
        members.push(("value", s(value)));
    }
    o(members)
}

/// Render `scene` through `py_dumps` and queue the text plus `blobs`.
///
/// `features` is the committed ordering of the `features` array: the
/// ordering the original authoring tools emitted. The tree is parsed and
/// [`Scene::compute_features`] recomputed; the emitted list is written
/// verbatim only after it matches the recomputed set, so a scene that
/// drifts (a changed paint, a new feature) fails here instead of
/// silently disagreeing with `Scene::load`.
fn emit(
    corpus: &mut Corpus,
    name: &str,
    indent: usize,
    scene: &mut J,
    blobs: Vec<Vec<u8>>,
    features: &[(&str, Option<&str>)],
) -> Result<(), SceneError> {
    let draft = py_dumps(scene, indent);
    let mut parsed: Scene = serde_json::from_str(&draft)?;
    parsed.compute_features();
    let key = |f: &Value| {
        (
            f["feature"].as_str().unwrap_or_default().to_string(),
            f.get("value").and_then(Value::as_str).map(str::to_string),
        )
    };
    let mut computed: Vec<(String, Option<String>)> = parsed
        .features
        .iter()
        .map(|f| key(&serde_json::to_value(f).expect("features serialise")))
        .collect();
    computed.sort();
    let mut given: Vec<(String, Option<String>)> = features
        .iter()
        .map(|(n, v)| (n.to_string(), v.map(str::to_string)))
        .collect();
    given.sort();
    assert_eq!(
        computed, given,
        "authored scene {name} features drifted from compute_features"
    );
    if let J::O(members) = scene
        && let Some((_, v)) = members.iter_mut().find(|(k, _)| k == "features")
    {
        *v = a(features.iter().map(|(n, v)| feat(n, *v)).collect());
    }
    let mut text = py_dumps(scene, indent);
    text.push('\n');
    corpus.push_json(name, text, blobs);
    Ok(())
}

// ---------------------------------------------------------------------------
// Scene families
// ---------------------------------------------------------------------------

/// `blend-plus-lighter*`: a backdrop rect over the left two thirds, then a
/// `plus-lighter` layer drawing a second rect over the right two thirds.
fn blend_plus_lighter(
    corpus: &mut Corpus,
    name: &str,
    back: J,
    front: J,
    extra: &[(&str, Option<&str>)],
) -> Result<(), SceneError> {
    let items = vec![
        fill(rect(f(0.0), f(0.0), f(64.0), f(96.0)), back),
        layer(
            affine(IDENTITY_F),
            "plus-lighter",
            None,
            vec![fill(rect(f(32.0), f(0.0), f(96.0), f(96.0)), front)],
        ),
    ];
    let mut features = vec![("fill", None), ("blend", Some("plus-lighter"))];
    features.extend_from_slice(extra);
    emit(
        corpus,
        name,
        2,
        &mut scene_j(96, 96, dark_clear(), affine(IDENTITY_F), items),
        Vec::new(),
        &features,
    )
}

/// `blend-space-linear` / `blend-space-srgb`: a backdrop rect, then a
/// rect + circle group composited in `space`.
fn blend_space(corpus: &mut Corpus, name: &str, space: &str) -> Result<(), SceneError> {
    let members = vec![
        fill(
            rect(f(20.0), f(20.0), f(88.0), f(88.0)),
            solid("srgb", vec![f(0.85), f(0.15), f(0.2), f(1.0)]),
        ),
        fill(
            circle(f(84.0), f(84.0), f(36.0)),
            solid("srgb", vec![f(0.15), f(0.3), f(0.85), f(0.5)]),
        ),
    ];
    let items = vec![
        fill(
            rect(f(0.0), f(0.0), f(128.0), f(128.0)),
            solid("srgb", vec![f(0.9), f(0.9), f(0.92), f(1.0)]),
        ),
        group(members, space),
    ];
    // Only a non-linear blend space registers a feature.
    let features: &[(&str, Option<&str>)] = if space == "linear" {
        &[("fill", None)]
    } else {
        &[("fill", None), ("blend-space", Some(space))]
    };
    emit(
        corpus,
        name,
        1,
        &mut scene_j(128, 128, light_clear(), affine(IDENTITY_F), items),
        Vec::new(),
        features,
    )
}

/// The swatch palette shared by the `blend-space-srgb-document` grid and
/// its `blend-space-srgb-per-element` counterpart: a 48-step hue ramp
/// sampled at three decimals.
const DOC_COLOURS: [[f64; 3]; 48] = [
    [0.5, 0.891, 0.11],
    [0.559, 0.858, 0.084],
    [0.616, 0.82, 0.065],
    [0.672, 0.776, 0.054],
    [0.725, 0.727, 0.05],
    [0.774, 0.674, 0.054],
    [0.818, 0.618, 0.065],
    [0.857, 0.561, 0.084],
    [0.89, 0.502, 0.111],
    [0.916, 0.443, 0.143],
    [0.935, 0.385, 0.182],
    [0.946, 0.33, 0.226],
    [0.95, 0.277, 0.275],
    [0.946, 0.228, 0.328],
    [0.935, 0.183, 0.384],
    [0.916, 0.144, 0.442],
    [0.89, 0.111, 0.501],
    [0.857, 0.085, 0.559],
    [0.818, 0.066, 0.617],
    [0.774, 0.054, 0.673],
    [0.725, 0.05, 0.725],
    [0.672, 0.054, 0.774],
    [0.617, 0.065, 0.819],
    [0.559, 0.083, 0.857],
    [0.5, 0.109, 0.89],
    [0.441, 0.142, 0.916],
    [0.384, 0.18, 0.935],
    [0.328, 0.224, 0.946],
    [0.275, 0.273, 0.95],
    [0.226, 0.326, 0.946],
    [0.182, 0.382, 0.935],
    [0.143, 0.439, 0.916],
    [0.11, 0.498, 0.889],
    [0.084, 0.557, 0.857],
    [0.065, 0.614, 0.818],
    [0.054, 0.67, 0.774],
    [0.05, 0.723, 0.725],
    [0.054, 0.772, 0.672],
    [0.065, 0.817, 0.616],
    [0.084, 0.856, 0.558],
    [0.11, 0.889, 0.5],
    [0.143, 0.915, 0.441],
    [0.182, 0.934, 0.383],
    [0.226, 0.946, 0.327],
    [0.275, 0.95, 0.275],
    [0.328, 0.946, 0.226],
    [0.383, 0.935, 0.181],
    [0.441, 0.917, 0.143],
];

/// The `idx`-th document swatch: alternating rects and circles over a
/// 4-column grid (56 px tiles at 64 px pitch, 40 px row pitch), alpha
/// cycling 0.45 / 0.575 / 0.7.
fn doc_swatch(idx: i64) -> J {
    let idx32 = i32::try_from(idx).unwrap_or(0);
    let row = idx32 / 8;
    let col = (idx32 % 8) / 2;
    let [red, grn, blu] = DOC_COLOURS[usize::try_from(idx).unwrap_or(0)];
    let alpha = [0.45, 0.575, 0.7][usize::try_from(idx % 3).unwrap_or(0)];
    let paint = solid("srgb", vec![f(red), f(grn), f(blu), f(alpha)]);
    let left = f64::from(6 + 64 * col);
    let top = f64::from(8 + 40 * row);
    if idx % 2 == 0 {
        fill(rect(f(left), f(top), f(left + 56.0), f(top + 52.0)), paint)
    } else {
        fill(circle(f(left + 60.0), f(top + 26.0), f(30.0)), paint)
    }
}

/// `component-rotation-*`: a rotated layer with a `rotation` motion around
/// the scene centre over a pill + dot. The layer transform literals are
/// the committed values (the rotation applied about the pivot) so no libm
/// variance can drift them.
fn component_rotation(
    corpus: &mut Corpus,
    name: &str,
    to: f64,
    transform: [f64; 6],
) -> Result<(), SceneError> {
    let motion = o(vec![
        ("motion", s("rotation")),
        (
            "value",
            o(vec![
                ("from", f(0.0)),
                ("to", f(to)),
                ("pivot", pt(f(32.0), f(32.0))),
                (
                    "animation",
                    o(vec![
                        ("animation", s("curve")),
                        (
                            "value",
                            o(vec![
                                ("duration_ms", i(500)),
                                ("x1", f(0.0)),
                                ("y1", f(0.0)),
                                ("x2", f(1.0)),
                                ("y2", f(1.0)),
                            ]),
                        ),
                    ]),
                ),
            ]),
        ),
    ]);
    let items = vec![layer(
        affine(transform),
        "normal",
        Some(motion),
        vec![
            fill(
                rect(f(30.0), f(12.0), f(38.0), f(34.0)),
                solid("srgb", vec![f(0.1), f(0.5), f(0.9), f(1.0)]),
            ),
            fill(
                circle(f(38.0), f(15.0), f(5.0)),
                solid("srgb", vec![f(0.9), f(0.3), f(0.1), f(1.0)]),
            ),
        ],
    )];
    emit(
        corpus,
        name,
        2,
        &mut scene_j(64, 64, light_clear(), identity_ints(), items),
        Vec::new(),
        &[("fill", None), ("animation", None)],
    )
}

/// `gamut-p3-*` static scenes.
fn gamut(corpus: &mut Corpus) -> Result<(), SceneError> {
    gamut_primaries(corpus)?;
    gamut_saturated(corpus)?;
    gamut_crossing(corpus)
}

/// `gamut-p3-primaries`: primary pairs, linear-P3 against its srgb
/// namesake.
fn gamut_primaries(corpus: &mut Corpus) -> Result<(), SceneError> {
    let primaries: [[i64; 3]; 6] = [
        [1, 0, 0],
        [0, 1, 0],
        [0, 0, 1],
        [0, 1, 1],
        [1, 0, 1],
        [1, 1, 0],
    ];
    let mut items = Vec::new();
    for (row, rgb) in primaries.iter().enumerate() {
        let row = i64::try_from(row).unwrap_or(0);
        let (y0, y1) = (row * 16, row * 16 + 16);
        for (space, x0, x1) in [("linear-p3", 0i64, 48i64), ("srgb", 48, 96)] {
            items.push(fill(
                rect(i(x0), i(y0), i(x1), i(y1)),
                solid(space, vec![i(rgb[0]), i(rgb[1]), i(rgb[2]), f(1.0)]),
            ));
        }
    }
    emit(
        corpus,
        "gamut-p3-primaries",
        2,
        &mut scene_j(96, 96, light_clear(), affine(IDENTITY_F), items),
        Vec::new(),
        &[("fill", None), ("wide-gamut", None)],
    )
}

/// `gamut-p3-saturated`: saturated primaries on a dark field plus two
/// radial P3 gradients. The authored object lists the gradient stops
/// before the centers.
fn gamut_saturated(corpus: &mut Corpus) -> Result<(), SceneError> {
    let mut items = vec![fill(
        rect(i(0), i(0), i(96), i(96)),
        solid("linear-p3", vec![f(0.06), f(0.05), f(0.09), f(1.0)]),
    )];
    for (cx, cy, comps) in [
        (24, 24, [f(0.0), f(1.0), f(1.0), f(1.0)]),
        (72, 24, [f(1.0), f(0.0), f(1.0), f(1.0)]),
        (24, 72, [f(1.0), f(1.0), f(0.0), f(1.0)]),
        (72, 72, [f(0.0), f(1.0), f(0.0), f(1.0)]),
    ] {
        items.push(fill(
            circle(i(cx), i(cy), i(20)),
            solid("linear-p3", comps.to_vec()),
        ));
    }
    let saturated_radial = |cx: i64, cy: i64, stops: Vec<J>| {
        o(vec![(
            "radial",
            o(vec![
                ("stops", a(stops)),
                ("extend", s("pad")),
                ("interpolation", s("linear-p3")),
                ("center0", pt(i(cx), i(cy))),
                ("r0", f(0.0)),
                ("center1", pt(i(cx), i(cy))),
                ("r1", i(20)),
            ]),
        )])
    };
    items.push(fill(
        circle(i(24), i(24), i(20)),
        saturated_radial(
            24,
            24,
            vec![
                stop(0.0, "linear-p3", vec![f(1.0), f(1.0), f(1.0), f(1.0)]),
                stop(0.55, "linear-p3", vec![f(0.0), f(0.9), f(0.9), f(0.9)]),
                stop(1.0, "linear-p3", vec![f(0.0), f(1.0), f(1.0), f(0.0)]),
            ],
        ),
    ));
    items.push(fill(
        circle(i(72), i(72), i(20)),
        saturated_radial(
            72,
            72,
            vec![
                stop(0.6, "linear-p3", vec![i(0), i(0), i(0), i(0)]),
                stop(0.8, "linear-p3", vec![f(1.35), f(0.0), f(0.0), f(1.0)]),
                stop(1.0, "linear-p3", vec![f(1.0), f(0.0), f(0.0), f(0.0)]),
            ],
        ),
    ));
    emit(
        corpus,
        "gamut-p3-saturated",
        2,
        &mut scene_j(96, 96, light_clear(), affine(IDENTITY_F), items),
        Vec::new(),
        &[
            ("fill", None),
            ("radial-gradient", None),
            ("wide-gamut", None),
            ("hdr-color", None),
            ("interpolation-space", Some("linear-p3")),
        ],
    )
}

/// A `gamut-p3-crossing` band: y-range, gradient start/end, padding,
/// grey level and the P3 primary it fades into.
type GamutBand = (i64, i64, i64, i64, i64, i64, i64, f64, [f64; 3]);

/// `gamut-p3-crossing`: five linear bands, each fading a grey into a P3
/// primary (the last band's gradient runs down a diagonal).
fn gamut_crossing(corpus: &mut Corpus) -> Result<(), SceneError> {
    let bands: [GamutBand; 5] = [
        (0, 20, 0, 10, 96, 10, 0, 0.6, [1.0, 0.0, 0.0]),
        (20, 40, 0, 30, 96, 30, 0, 0.6, [0.0, 1.0, 0.0]),
        (40, 60, 0, 50, 96, 50, 0, 0.6, [0.0, 0.0, 1.0]),
        (60, 80, 0, 70, 96, 70, 0, 0.35, [1.0, 0.0, 1.0]),
        (80, 96, 0, 80, 96, 96, 0, 0.55, [0.0, 1.0, 1.0]),
    ];
    let mut items = Vec::new();
    for (y0, y1, sx, sy, ex, ey, _pad, grey, rgb) in bands {
        items.push(fill(
            rect(i(0), i(y0), i(96), i(y1)),
            gradient(
                "linear",
                vec![("start", pt(i(sx), i(sy))), ("end", pt(i(ex), i(ey)))],
                vec![
                    stop(0.0, "linear-p3", vec![f(grey), f(grey), f(grey), f(1.0)]),
                    stop(
                        1.0,
                        "linear-p3",
                        vec![f(rgb[0]), f(rgb[1]), f(rgb[2]), f(1.0)],
                    ),
                ],
                "pad",
                "linear-p3",
            ),
        ));
    }
    emit(
        corpus,
        "gamut-p3-crossing",
        2,
        &mut scene_j(96, 96, light_clear(), affine(IDENTITY_F), items),
        Vec::new(),
        &[
            ("fill", None),
            ("linear-gradient", None),
            ("wide-gamut", None),
            ("interpolation-space", Some("linear-p3")),
        ],
    )
}

/// One mesh scene's varying knobs.
struct MeshSpec {
    name: &'static str,
    columns: i64,
    rows: i64,
    points: Vec<(i64, i64)>,
    colours: Vec<J>,
    smooth: bool,
    root: J,
    transform: Option<[f64; 6]>,
    features: &'static [(&'static str, Option<&'static str>)],
}

/// A mesh spec with the common integer-identity root transform.
fn mesh_spec(
    name: &'static str,
    cells: (i64, i64),
    points: Vec<(i64, i64)>,
    colours: Vec<J>,
    smooth: bool,
    transform: Option<[f64; 6]>,
    features: &'static [(&'static str, Option<&'static str>)],
) -> MeshSpec {
    MeshSpec {
        name,
        columns: cells.0,
        rows: cells.1,
        points,
        colours,
        smooth,
        root: identity_ints(),
        transform,
        features,
    }
}

const MESH: &[(&str, Option<&str>)] = &[
    ("fill", None),
    ("mesh-gradient", None),
    ("wide-gamut", None),
];
const MESH_HDR: &[(&str, Option<&str>)] = &[
    ("fill", None),
    ("mesh-gradient", None),
    ("hdr-color", None),
    ("wide-gamut", None),
];
const MESH_PT: &[(&str, Option<&str>)] = &[
    ("fill", None),
    ("mesh-gradient", None),
    ("wide-gamut", None),
    ("paint-transform", None),
];

/// The four P3 corner colours the base mesh scenes share; the HDR
/// variant pushes three of them past SDR white.
fn mesh_c4() -> Vec<J> {
    vec![
        colour("linear-p3", vec![i(1), i(0), i(0), i(1)]),
        colour("linear-p3", vec![i(0), i(1), i(0), f(0.25)]),
        colour("linear-p3", vec![i(0), i(0), i(1), i(1)]),
        colour("linear-p3", vec![i(1), i(1), i(0), f(0.75)]),
    ]
}

fn mesh_c4_hdr() -> Vec<J> {
    vec![
        colour("linear-p3", vec![i(3), i(0), i(0), i(1)]),
        colour("linear-p3", vec![i(0), i(2), i(0), f(0.25)]),
        colour("linear-p3", vec![i(0), i(0), i(4), i(1)]),
        colour("linear-p3", vec![i(1), i(1), i(0), f(0.75)]),
    ]
}

fn mesh_seam_colours() -> Vec<J> {
    let mut c = mesh_c4();
    c.push(colour("linear-p3", vec![f(0.5), i(0), i(1), i(1)]));
    c.push(colour("linear-p3", vec![i(0), i(1), i(1), f(0.5)]));
    c
}

fn mesh_overlap_colours() -> Vec<J> {
    let mut c = mesh_c4();
    c.push(colour("linear-p3", vec![i(1), i(0), i(1), f(0.5)]));
    c.push(colour("linear-p3", vec![i(0), i(1), i(1), i(1)]));
    c
}

fn mesh_base_points() -> Vec<(i64, i64)> {
    vec![(12, 12), (116, 20), (20, 116), (106, 96)]
}

fn mesh_seam_points() -> Vec<(i64, i64)> {
    vec![(8, 8), (64, 22), (120, 8), (8, 120), (60, 100), (120, 120)]
}

fn mesh_overlap_points() -> Vec<(i64, i64)> {
    vec![
        (12, 12),
        (112, 12),
        (24, 24),
        (12, 112),
        (112, 112),
        (24, 100),
    ]
}

/// The base mesh specs, interpolation defaulting to bilinear.
/// `mesh-reflected` is the only scene whose root is not the identity:
/// it carries the mirror there instead of on the paint.
fn mesh_specs() -> Vec<MeshSpec> {
    let reflected = MeshSpec {
        root: a(vec![i(-1), i(0), i(0), i(1), i(128), i(0)]),
        ..mesh_spec(
            "mesh-reflected",
            (1, 1),
            mesh_base_points(),
            mesh_c4(),
            false,
            None,
            MESH,
        )
    };
    vec![
        mesh_spec(
            "mesh-bilinear",
            (1, 1),
            mesh_base_points(),
            mesh_c4(),
            false,
            None,
            MESH,
        ),
        reflected,
        mesh_spec(
            "mesh-fold",
            (1, 1),
            vec![(8, 8), (120, 8), (100, 120), (28, 90)],
            mesh_c4(),
            false,
            None,
            MESH,
        ),
        mesh_spec(
            "mesh-seam",
            (2, 1),
            mesh_seam_points(),
            mesh_seam_colours(),
            false,
            None,
            MESH,
        ),
        mesh_spec(
            "mesh-overlap",
            (2, 1),
            mesh_overlap_points(),
            mesh_overlap_colours(),
            false,
            None,
            MESH,
        ),
        mesh_spec(
            "mesh-hdr",
            (1, 1),
            mesh_base_points(),
            mesh_c4_hdr(),
            false,
            None,
            MESH_HDR,
        ),
        mesh_spec(
            "mesh-paint-transform",
            (1, 1),
            mesh_base_points(),
            mesh_c4(),
            false,
            Some([-0.8, 0.2, 0.15, 0.7, 110.0, 6.0]),
            MESH_PT,
        ),
    ]
}

/// The `smoothstep` twins of the base specs.
fn mesh_specs_smooth() -> Vec<MeshSpec> {
    vec![
        mesh_spec(
            "mesh-bilinear-smoothstep",
            (1, 1),
            mesh_base_points(),
            mesh_c4(),
            true,
            None,
            MESH,
        ),
        mesh_spec(
            "mesh-seam-smoothstep",
            (2, 1),
            mesh_seam_points(),
            mesh_seam_colours(),
            true,
            None,
            MESH,
        ),
        mesh_spec(
            "mesh-hdr-smoothstep",
            (1, 1),
            mesh_base_points(),
            mesh_c4_hdr(),
            true,
            None,
            MESH_HDR,
        ),
        mesh_spec(
            "mesh-paint-transform-smoothstep",
            (1, 1),
            mesh_base_points(),
            mesh_c4(),
            true,
            Some([-0.8, 0.2, 0.15, 0.7, 110.0, 6.0]),
            MESH_PT,
        ),
    ]
}

/// Mesh gradient scenes; `smooth` adds `"interpolation": "smoothstep"`.
fn mesh_scenes(corpus: &mut Corpus) -> Result<(), SceneError> {
    for spec in mesh_specs().into_iter().chain(mesh_specs_smooth()) {
        let paint = mesh_paint(
            spec.columns,
            spec.rows,
            spec.points,
            spec.colours,
            spec.smooth,
        );
        let paint = match spec.transform {
            Some(t) => transformed(paint, affine(t)),
            None => paint,
        };
        let items = vec![fill(rect(f(8.0), f(8.0), f(120.0), f(120.0)), paint)];
        emit(
            corpus,
            spec.name,
            2,
            &mut scene_j(128, 128, light_clear(), spec.root, items),
            Vec::new(),
            spec.features,
        )?;
    }
    Ok(())
}

/// `paint-transform-*`: a single fill or stroke whose paint carries a
/// `transformed` wrapper.
fn paint_transform(corpus: &mut Corpus) -> Result<(), SceneError> {
    pt_image(corpus)?;
    pt_linear_shear(corpus)?;
    pt_radial(corpus)?;
    pt_sweep(corpus)
}

/// The two-stop red-to-blue srgb gradient the `paint-transform` scenes
/// share.
fn srgb_stops() -> Vec<J> {
    vec![
        stop(0.0, "srgb", vec![f(1.0), f(0.0), f(0.0), f(1.0)]),
        stop(1.0, "srgb", vec![f(0.0), f(0.0), f(1.0), f(1.0)]),
    ]
}

fn pt_frame() -> J {
    rect(f(8.0), f(8.0), f(120.0), f(120.0))
}

/// `paint-transform-image`: the shared 8x8 checker as an image paint
/// over a rounded frame; the outer transform mixes float and integer
/// literals in the committed file.
fn pt_image(corpus: &mut Corpus) -> Result<(), SceneError> {
    let image_paint = o(vec![(
        "image",
        o(vec![
            (
                "image",
                s("779cbc6ed901a1227a775289e056aec7f1ba39f5162b0108328dafe2cc4a2dc8"),
            ),
            ("transform", affine([4.0, 0.0, 0.0, 4.0, 40.0, 40.0])),
            ("extend_x", s("repeat")),
            ("extend_y", s("repeat")),
            ("sampling", s("bilinear")),
        ]),
    )]);
    let frame_rrect = o(vec![(
        "rounded-rect",
        o(vec![
            (
                "rect",
                o(vec![
                    ("x0", f(8.0)),
                    ("y0", f(8.0)),
                    ("x1", f(120.0)),
                    ("y1", f(120.0)),
                ]),
            ),
            (
                "radii",
                o(vec![
                    ("top_left", f(16.0)),
                    ("top_right", f(16.0)),
                    ("bottom_right", f(16.0)),
                    ("bottom_left", f(16.0)),
                ]),
            ),
        ]),
    )]);
    let items = vec![fill(
        frame_rrect,
        transformed(
            image_paint,
            a(vec![f(1.1), f(0.2), f(0.35), f(0.8), i(-8), i(4)]),
        ),
    )];
    emit(
        corpus,
        "paint-transform-image",
        2,
        &mut scene_j(128, 128, light_clear(), affine(IDENTITY_F), items),
        vec![checker_png()],
        &[
            ("fill", None),
            ("image-paint", None),
            ("paint-transform", None),
        ],
    )
}

/// `paint-transform-linear-shear`: a reflected linear gradient under a
/// shearing paint transform.
fn pt_linear_shear(corpus: &mut Corpus) -> Result<(), SceneError> {
    emit(
        corpus,
        "paint-transform-linear-shear",
        2,
        &mut scene_j(
            128,
            128,
            light_clear(),
            affine(IDENTITY_F),
            vec![fill(
                pt_frame(),
                transformed(
                    gradient(
                        "linear",
                        vec![
                            ("start", pt(f(32.0), f(48.0))),
                            ("end", pt(f(96.0), f(80.0))),
                        ],
                        srgb_stops(),
                        "reflect",
                        "srgb",
                    ),
                    affine([1.0, 0.35, 0.45, 1.0, -25.0, -20.0]),
                ),
            )],
        ),
        Vec::new(),
        &[
            ("fill", None),
            ("paint-transform", None),
            ("linear-gradient", None),
            ("interpolation-space", Some("srgb")),
        ],
    )
}

/// `paint-transform-radial-{reflect,stroke}`: the same radial gradient
/// under two paint transforms, once filled and once stroked.
fn pt_radial(corpus: &mut Corpus) -> Result<(), SceneError> {
    let radial = || {
        gradient(
            "radial",
            vec![
                ("center0", pt(f(64.0), f(64.0))),
                ("r0", f(8.0)),
                ("center1", pt(f(80.0), f(72.0))),
                ("r1", f(40.0)),
            ],
            srgb_stops(),
            "reflect",
            "srgb",
        )
    };
    emit(
        corpus,
        "paint-transform-radial-reflect",
        2,
        &mut scene_j(
            128,
            128,
            light_clear(),
            affine(IDENTITY_F),
            vec![fill(
                pt_frame(),
                transformed(radial(), affine([-1.0, 0.15, 0.0, 1.0, 128.0, -4.0])),
            )],
        ),
        Vec::new(),
        &[
            ("fill", None),
            ("paint-transform", None),
            ("radial-gradient", None),
            ("interpolation-space", Some("srgb")),
        ],
    )?;
    emit(
        corpus,
        "paint-transform-radial-stroke",
        2,
        &mut scene_j(
            128,
            128,
            light_clear(),
            affine(IDENTITY_F),
            vec![stroke_draw(
                pt_frame(),
                transformed(radial(), affine([1.5, 0.0, 0.3, 0.6, -25.0, 22.0])),
                stroke_style(9.0, "Miter", "Butt", Vec::new(), 2.0),
            )],
        ),
        Vec::new(),
        &[
            ("stroke", None),
            ("paint-transform", None),
            ("radial-gradient", None),
            ("interpolation-space", Some("srgb")),
        ],
    )
}

/// `paint-transform-sweep`: a padded sweep gradient under the same
/// transform the radial stroke uses.
fn pt_sweep(corpus: &mut Corpus) -> Result<(), SceneError> {
    emit(
        corpus,
        "paint-transform-sweep",
        2,
        &mut scene_j(
            128,
            128,
            light_clear(),
            affine(IDENTITY_F),
            vec![fill(
                pt_frame(),
                transformed(
                    gradient(
                        "sweep",
                        vec![
                            ("center", pt(f(64.0), f(64.0))),
                            ("start_angle", f(0.0)),
                            ("end_angle", f(5.026_548_245_743_669)),
                        ],
                        srgb_stops(),
                        "pad",
                        "srgb",
                    ),
                    affine([1.5, 0.0, 0.3, 0.6, -25.0, 22.0]),
                ),
            )],
        ),
        Vec::new(),
        &[
            ("fill", None),
            ("paint-transform", None),
            ("sweep-gradient", None),
            ("interpolation-space", Some("srgb")),
        ],
    )
}

/// Per-glyph rotation transforms about the glyph origin, the angle
/// sweeping `0.5 .. -0.5` rad in sixteenths. The coefficients are the
/// committed values pinned verbatim so no libm variance can drift them.
const GLYPH_ROTATIONS: [[f64; 6]; 16] = [
    [
        0.877_582_561_890_372_8,
        -0.479_425_538_604_203,
        0.479_425_538_604_203,
        0.877_582_561_890_372_8,
        0.0,
        0.0,
    ],
    [
        0.907_571_133_101_684_9,
        -0.419_898_366_703_805_8,
        0.419_898_366_703_805_8,
        0.907_571_133_101_684_9,
        0.0,
        0.0,
    ],
    [
        0.933_527_548_555_489_6,
        -0.358_505_670_928_617_24,
        0.358_505_670_928_617_24,
        0.933_527_548_555_489_6,
        0.0,
        0.0,
    ],
    [
        0.955_336_489_125_606,
        -0.295_520_206_661_339_55,
        0.295_520_206_661_339_55,
        0.955_336_489_125_606,
        0.0,
        0.0,
    ],
    [
        0.972_901_062_081_450_7,
        -0.231_221_805_634_298_56,
        0.231_221_805_634_298_56,
        0.972_901_062_081_450_7,
        0.0,
        0.0,
    ],
    [
        0.986_143_231_562_925,
        -0.165_896_132_693_415_05,
        0.165_896_132_693_415_05,
        0.986_143_231_562_925,
        0.0,
        0.0,
    ],
    [
        0.995_004_165_278_025_8,
        -0.099_833_416_646_828_13,
        0.099_833_416_646_828_13,
        0.995_004_165_278_025_8,
        0.0,
        0.0,
    ],
    [
        0.999_444_495_882_868_5,
        -0.033_327_160_836_753_61,
        0.033_327_160_836_753_61,
        0.999_444_495_882_868_5,
        0.0,
        0.0,
    ],
    [
        0.999_444_495_882_868_5,
        0.033_327_160_836_753_61,
        -0.033_327_160_836_753_61,
        0.999_444_495_882_868_5,
        0.0,
        0.0,
    ],
    [
        0.995_004_165_278_025_8,
        0.099_833_416_646_828_13,
        -0.099_833_416_646_828_13,
        0.995_004_165_278_025_8,
        0.0,
        0.0,
    ],
    [
        0.986_143_231_562_925,
        0.165_896_132_693_415,
        -0.165_896_132_693_415,
        0.986_143_231_562_925,
        0.0,
        0.0,
    ],
    [
        0.972_901_062_081_450_7,
        0.231_221_805_634_298_5,
        -0.231_221_805_634_298_5,
        0.972_901_062_081_450_7,
        0.0,
        0.0,
    ],
    [
        0.955_336_489_125_606,
        0.295_520_206_661_339_6,
        -0.295_520_206_661_339_6,
        0.955_336_489_125_606,
        0.0,
        0.0,
    ],
    [
        0.933_527_548_555_489_6,
        0.358_505_670_928_617_24,
        -0.358_505_670_928_617_24,
        0.933_527_548_555_489_6,
        0.0,
        0.0,
    ],
    [
        0.907_571_133_101_684_9,
        0.419_898_366_703_805_8,
        -0.419_898_366_703_805_8,
        0.907_571_133_101_684_9,
        0.0,
        0.0,
    ],
    [
        0.877_582_561_890_372_8,
        0.479_425_538_604_203,
        -0.479_425_538_604_203,
        0.877_582_561_890_372_8,
        0.0,
        0.0,
    ],
];

fn ink() -> J {
    solid("srgb", vec![f(0.1), f(0.1), f(0.12), f(1.0)])
}

/// The Latin sheet runs (`The quick brown `, `fox jumps over the `,
/// `lazy dog `, `0123456789!?`) the glyph scenes share.
fn latin_runs(ctx: &mut TextContext) -> Vec<GlyphRun> {
    ctx.shape(
        "NotoSans.ttf",
        corpus::LATIN,
        30.0,
        FontWeight::NORMAL,
        &Paint::Solid(Color::srgb(0.1, 0.1, 0.12)),
    )
}

/// `glyph-stroke-{thin,dashed,transformed}`: the four-line Latin sheet
/// stroked instead of filled. `transformed` also transforms the root.
fn glyph_stroke(
    corpus: &mut Corpus,
    ctx: &mut TextContext,
    name: &str,
    style: &J,
    root_transform: J,
    features: &[(&str, Option<&str>)],
) -> Result<(), SceneError> {
    let runs = latin_runs(ctx);
    let blobs = font_blobs(ctx, &[&runs]);
    let items = runs
        .iter()
        .map(|run| glyphs_draw(run_j(run, ink(), Some(style.clone()))))
        .collect();
    emit(
        corpus,
        name,
        2,
        &mut scene_j(320, 160, light_clear(), root_transform, items),
        blobs,
        features,
    )
}

/// `gamut-text-p3`: the Latin sheet in linear-P3 primaries.
fn gamut_text(corpus: &mut Corpus, ctx: &mut TextContext) -> Result<(), SceneError> {
    let runs = latin_runs(ctx);
    let blobs = font_blobs(ctx, &[&runs]);
    let paints = [
        solid("linear-p3", vec![f(1.0), f(0.0), f(0.0), f(1.0)]),
        solid("linear-p3", vec![f(0.0), f(1.0), f(0.0), f(1.0)]),
        solid("linear-p3", vec![f(0.0), f(0.0), f(1.0), f(1.0)]),
        solid("linear-p3", vec![f(0.2), f(0.2), f(0.2), f(1.0)]),
    ];
    let items = runs
        .iter()
        .zip(paints)
        .map(|(run, paint)| glyphs_draw(run_j(run, paint, None)))
        .collect();
    emit(
        corpus,
        "gamut-text-p3",
        2,
        &mut scene_j(320, 160, light_clear(), affine(IDENTITY_F), items),
        blobs,
        &[("glyphs", None), ("wide-gamut", None)],
    )
}

/// Clone `run`, shift every glyph by `(dx, dy)` and give it
/// `transforms[seq[idx % seq.len()]]`.
fn transformed_run(
    run: &GlyphRun,
    transforms: &[[f64; 6]],
    seq: &[usize],
    dx: f32,
    dy: f32,
) -> GlyphRun {
    let mut out = run.clone();
    for (idx, g) in out.glyphs.iter_mut().enumerate() {
        g.x += dx;
        g.y += dy;
        g.transform = Some(Affine::new(transforms[seq[idx % seq.len()]]));
    }
    out
}

/// `glyph-transform-*`: the Latin sheet with per-glyph transforms.
fn glyph_transform(corpus: &mut Corpus, ctx: &mut TextContext) -> Result<(), SceneError> {
    let runs = latin_runs(ctx);
    let blobs = font_blobs(ctx, &[&runs]);
    let row = &runs[0];
    gt_rotate(corpus, row, &blobs)?;
    gt_skew_scale(corpus, row, &blobs)?;
    gt_stroke(corpus, row, &blobs)?;
    gt_colr(corpus, ctx)
}

/// `glyph-transform-rotate`: rotation sweep on row one; alternating
/// drops and quarter-turns on row two.
fn gt_rotate(corpus: &mut Corpus, row: &GlyphRun, blobs: &[Vec<u8>]) -> Result<(), SceneError> {
    let up = [1.0, 0.0, 0.0, 1.0, 0.0, -3.0];
    let down = [1.0, 0.0, 0.0, 1.0, 0.0, 3.0];
    let quarter = [0.0, 1.0, -1.0, 0.0, 0.0, 0.0];
    let items = vec![
        glyphs_draw(run_j(
            &transformed_run(
                row,
                &GLYPH_ROTATIONS,
                &(0..16).collect::<Vec<_>>(),
                0.0,
                0.0,
            ),
            ink(),
            None,
        )),
        glyphs_draw(run_j(
            &transformed_run(row, &[up, down, quarter], &[0, 1, 2, 1, 0, 2], 0.0, 44.0),
            ink(),
            None,
        )),
    ];
    emit(
        corpus,
        "glyph-transform-rotate",
        2,
        &mut scene_j(320, 160, light_clear(), affine(IDENTITY_F), items),
        blobs.to_vec(),
        &[("glyphs", None), ("glyph-transform", None)],
    )
}

/// `glyph-transform-skew-scale`: skew on row one; alternating
/// non-uniform scales under a linear gradient on row two. The root
/// transform is a committed mix of non-identity coefficients.
fn gt_skew_scale(corpus: &mut Corpus, row: &GlyphRun, blobs: &[Vec<u8>]) -> Result<(), SceneError> {
    let scale_paint = gradient(
        "linear",
        vec![
            ("start", pt(f(12.0), f(40.0))),
            ("end", pt(f(300.0), f(90.0))),
        ],
        vec![
            stop(0.0, "srgb", vec![f(0.9), f(0.15), f(0.2), f(1.0)]),
            stop(1.0, "srgb", vec![f(0.1), f(0.25), f(0.9), f(1.0)]),
        ],
        "pad",
        "srgb",
    );
    let items = vec![
        glyphs_draw(run_j(
            &transformed_run(row, &[[1.0, 0.0, 0.35, 1.0, 0.0, 0.0]], &[0], 0.0, 0.0),
            ink(),
            None,
        )),
        glyphs_draw(run_j(
            &transformed_run(
                row,
                &[
                    [0.7, 0.0, 0.0, 1.4, 0.0, 0.0],
                    [1.4, 0.0, 0.0, 0.7, 0.0, 0.0],
                ],
                &[0, 1],
                0.0,
                44.0,
            ),
            scale_paint,
            None,
        )),
    ];
    emit(
        corpus,
        "glyph-transform-skew-scale",
        2,
        &mut scene_j(
            320,
            160,
            light_clear(),
            affine([0.9, 0.1, 0.0, 1.0, 4.0, 0.0]),
            items,
        ),
        blobs.to_vec(),
        &[
            ("glyphs", None),
            ("glyph-transform", None),
            ("linear-gradient", None),
            ("interpolation-space", Some("srgb")),
        ],
    )
}

/// `glyph-transform-stroke`: the rotation sweep stroked; row two
/// alternates reflection and a non-uniform scale, still stroked.
fn gt_stroke(corpus: &mut Corpus, row: &GlyphRun, blobs: &[Vec<u8>]) -> Result<(), SceneError> {
    let items = vec![
        glyphs_draw(run_j(
            &transformed_run(
                row,
                &GLYPH_ROTATIONS,
                &(0..16).collect::<Vec<_>>(),
                0.0,
                0.0,
            ),
            ink(),
            Some(stroke_style(2.0, "Round", "Round", Vec::new(), 0.75)),
        )),
        glyphs_draw(run_j(
            &transformed_run(
                row,
                &[
                    [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    [1.3, 0.0, 0.0, 0.8, 0.0, 0.0],
                ],
                &[0, 1],
                0.0,
                44.0,
            ),
            ink(),
            Some(stroke_style(2.0, "Round", "Round", Vec::new(), 0.75)),
        )),
    ];
    emit(
        corpus,
        "glyph-transform-stroke",
        2,
        &mut scene_j(320, 160, light_clear(), affine(IDENTITY_F), items),
        blobs.to_vec(),
        &[
            ("glyphs", None),
            ("glyph-stroke", None),
            ("glyph-transform", None),
        ],
    )
}

/// `glyph-transform-colr`: rotate, scale, counter-rotate the three
/// COLR glyphs; the x positions are spread by hand, not by the shaped
/// advance.
fn gt_colr(corpus: &mut Corpus, ctx: &mut TextContext) -> Result<(), SceneError> {
    let colr_runs = ctx.shape(
        "Nabla.ttf",
        corpus::COLR,
        64.0,
        FontWeight::NORMAL,
        &Paint::Solid(Color::srgb(0.1, 0.1, 0.12)),
    );
    let mut run = transformed_run(
        &colr_runs[0],
        &[
            [
                0.921_060_994_002_885_1,
                0.389_418_342_308_650_5,
                -0.389_418_342_308_650_5,
                0.921_060_994_002_885_1,
                0.0,
                0.0,
            ],
            [1.5, 0.0, 0.0, 1.5, 0.0, 0.0],
            [
                0.921_060_994_002_885_1,
                -0.389_418_342_308_650_5,
                0.389_418_342_308_650_5,
                0.921_060_994_002_885_1,
                0.0,
                0.0,
            ],
        ],
        &[0, 1, 2],
        0.0,
        0.0,
    );
    for (g, x) in run.glyphs.iter_mut().zip([30.0, 128.0, 250.0]) {
        g.x = x;
    }
    let items = vec![glyphs_draw(run_j(&run, ink(), None))];
    emit(
        corpus,
        "glyph-transform-colr",
        2,
        &mut scene_j(320, 160, light_clear(), affine(IDENTITY_F), items),
        vec![font_blob(ctx, &run).clone()],
        &[("glyphs", None), ("glyph-transform", None)],
    )
}

/// Register every authored scene.
pub fn add(corpus: &mut Corpus, ctx: &mut TextContext) -> Result<(), SceneError> {
    blend_plus_lighter(
        corpus,
        "blend-plus-lighter",
        solid("srgb", vec![f(0.4), f(0.1), f(0.05), f(1.0)]),
        solid("srgb", vec![f(0.05), f(0.2), f(0.6), f(1.0)]),
        &[],
    )?;
    blend_plus_lighter(
        corpus,
        "blend-plus-lighter-solid-p3",
        solid("linear-p3", vec![f(1.0), f(0.0), f(0.0), f(1.0)]),
        solid("linear-p3", vec![f(0.0), f(0.4), f(0.9), f(1.0)]),
        &[("wide-gamut", None)],
    )?;
    blend_plus_lighter(
        corpus,
        "blend-plus-lighter-solid-hdr",
        solid("linear-p3", vec![f(1.5), f(0.0), f(0.0), f(1.0)]),
        solid("linear-p3", vec![f(0.0), f(0.5), f(1.25), f(1.0)]),
        &[("hdr-color", None), ("wide-gamut", None)],
    )?;

    blend_space(corpus, "blend-space-linear", "linear")?;
    blend_space(corpus, "blend-space-srgb", "srgb-encoded")?;
    blend_space_documents(corpus)?;

    component_rotation(
        corpus,
        "component-rotation-full-turn",
        std::f64::consts::TAU,
        [
            1.0,
            -2.449_293_598_294_706_4e-16,
            2.449_293_598_294_706_4e-16,
            1.0,
            -7.837_739_514_543_06e-15,
            7.105_427_357_601_002e-15,
        ],
    )?;
    component_rotation(
        corpus,
        "component-rotation-half-turn",
        std::f64::consts::PI,
        [
            -1.0,
            1.224_646_799_147_353_2e-16,
            -1.224_646_799_147_353_2e-16,
            -1.0,
            64.0,
            64.0,
        ],
    )?;
    component_rotation(
        corpus,
        "component-rotation-negative-turn",
        -7.853_981_633_974_483,
        [
            3.061_616_997_868_383e-16,
            -1.0,
            1.0,
            3.061_616_997_868_383e-16,
            -1.065_814_103_640_150_3e-14,
            63.999_999_999_999_99,
        ],
    )?;

    gamut(corpus)?;
    mesh_scenes(corpus)?;
    paint_transform(corpus)?;
    glyph_stroke_scenes(corpus, ctx)?;
    gamut_text(corpus, ctx)?;
    glyph_transform(corpus, ctx)?;
    Ok(())
}

/// `blend-space-srgb-{document,per-element}`: the 48-swatch document
/// grid in one srgb-encoded group, then the same grid with every swatch
/// in its own group.
fn blend_space_documents(corpus: &mut Corpus) -> Result<(), SceneError> {
    for (name, wrap_each) in [
        ("blend-space-srgb-document", false),
        ("blend-space-srgb-per-element", true),
    ] {
        let swatches: Vec<J> = (0..48).map(doc_swatch).collect();
        let mut items = vec![fill(
            rect(f(0.0), f(0.0), f(256.0), f(256.0)),
            solid("srgb", vec![f(0.9), f(0.9), f(0.92), f(1.0)]),
        )];
        if wrap_each {
            items.extend(
                swatches
                    .into_iter()
                    .map(|sw| group(vec![sw], "srgb-encoded")),
            );
        } else {
            items.push(group(swatches, "srgb-encoded"));
        }
        emit(
            corpus,
            name,
            1,
            &mut scene_j(256, 256, light_clear(), affine(IDENTITY_F), items),
            Vec::new(),
            &[("fill", None), ("blend-space", Some("srgb-encoded"))],
        )?;
    }
    Ok(())
}

/// `glyph-stroke-{thin,dashed,transformed}`: the Latin sheet stroked at
/// three widths/dash styles.
fn glyph_stroke_scenes(corpus: &mut Corpus, ctx: &mut TextContext) -> Result<(), SceneError> {
    glyph_stroke(
        corpus,
        ctx,
        "glyph-stroke-thin",
        &stroke_style(0.8, "Round", "Round", Vec::new(), 0.75),
        affine(IDENTITY_F),
        &[("glyphs", None), ("glyph-stroke", None)],
    )?;
    glyph_stroke(
        corpus,
        ctx,
        "glyph-stroke-dashed",
        &stroke_style(2.0, "Round", "Round", vec![2.5, 1.5], 0.75),
        affine(IDENTITY_F),
        &[
            ("stroke-dash", None),
            ("glyphs", None),
            ("glyph-stroke", None),
        ],
    )?;
    glyph_stroke(
        corpus,
        ctx,
        "glyph-stroke-transformed",
        &stroke_style(2.5, "Round", "Round", Vec::new(), 0.75),
        affine([0.8, 0.15, 0.2, 1.1, 2.0, 0.0]),
        &[("glyphs", None), ("glyph-stroke", None)],
    )
}
