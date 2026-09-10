# waterui-math

Mathematical formula rendering for WaterUI.

```rust
use waterui_math::view::Math;

Math::new(r"\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}").display().font_size(28.0)
```

## How it works

```
LaTeX ──(pulldown-latex)──► MathItem tree ──► layout ──► Scene2D commands
                                  │                          │
                                  └──► MathML ──(MathCAT)──►   └──► any backend
                                          speech ──► a11y
```

A formula is parsed into a semantic tree, laid out against the chosen face's
OpenType `MATH` table, and drawn through the engine-independent `Scene2D`
contract — so it renders on the classic compute pipeline, on the CPU/GPU split
engine that adapters without compute shaders fall to, and on dew's CPU scene,
without knowing which it is talking to.

## What the font supplies

Every measurement comes from the `MATH` table: the axis the fraction bar centres
on, the numerator and denominator shifts and their minimum gaps, the radical's
rule thickness, vertical gap and extra ascender, the script shifts and the
minimum gap between a superscript and a subscript, and italic correction so a
script clears a slanted base's overhang. Constants that come in a display and a
non-display flavour are resolved by style when the constants are read, so layout
cannot reach for the wrong one.

Glyphs that grow — radicals, parentheses, braces, brackets — go through one
mechanism: `MathVariants`, taking a designed variant when one is large enough
and assembling a multi-part glyph when none is. Nothing measures a glyph outline
to discover where its parts are.

## Spacing

The gap between two adjacent atoms is a function of the class on each side, so
`a+b` and `a=b` are spaced differently and `f(x)` closes up. The table lives in
`src/spacing.toml` and is TeX's, which MathML Core reproduces.

## Accessibility

The semantic tree is kept after layout, not consumed by it. `mathml::to_mathml`
publishes it as MathML — a formula drawn as anonymous filled paths has no
content at all to a screen reader.

MathML is the payload a platform's math accessibility API takes, but it is
markup, not a sentence: read out, `<mfrac><mi>a</mi><mi>b</mi></mfrac>` is
noise. `speech::speak` turns it into the sentence, through
[MathCAT](https://nsoiffer.github.io/MathCAT/) — the ClearSpeak / MathSpeak
engine assistive-technology vendors ship — and that sentence is what reaches
the accessibility tree:

```rust
let formula = latex::parse(r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}")?;
let spoken = speech::speak(&mathml::to_mathml(&formula, MathStyle::Display))?;
```

```text
x is equal to; the fraction with numerator; negative b plus or minus;
the square root of b squared minus 4 eigh c; and denominator 2 eigh
```

`eigh` is how the engine spells the letter `a` so a text-to-speech voice does
not read it as the article.

The drawing answers `SceneContent::accessibility_label` with that sentence, and
the backend names the formula's node with it. It follows the source signal, so a
formula bound to state stays current. `.a11y_label("Quadratic formula")` still
wins wherever the application names the formula itself — the speech is what the
node says when nobody named it. A formula that cannot be spoken leaves the node
unnamed and logs the reason; there is deliberately no generic "math formula" to
announce instead, because it tells a listener nothing and would hide the
failure.

MathCAT reads its speech rules from a `Rules` directory. This crate takes it
with the `include-zip` feature, which compiles the whole rule set into the
binary as one bzip2 archive served from an in-memory filesystem — nothing to
install beside the application, and nothing for the asset pipeline to carry.
That costs about 2.7 MiB of release artifact (0.93 MiB of rules, the rest code
and tables), paid only by a build that links this crate.

## Fonts

Only a face carrying an OpenType `MATH` table can set mathematics. The default
is **STIX Two Math**, declared in this crate's manifest so the WaterUI CLI
bundles it. A face without the table is refused rather than substituted: it
supplies no layout constants, so drawing with it would not be the same formula
set differently, it would be a formula with no geometry.

## Not yet supported

Matrices and arrays, `cases`, multi-line environments, negation, and font/style
changes (`\mathbf`, `\mathbb`, …) are reported as errors naming the construct
rather than silently dropped.

## Reference

Layout follows [MathML Core Ch. 3](https://www.w3.org/TR/mathml-core/#layout-algorithms)
and the [OpenType MATH specification](https://learn.microsoft.com/en-us/typography/opentype/spec/math),
rather than TeXbook Appendix G, which is bound to 1982 TFM font metrics. For the
mapping between the two, see Ulrik Vieth, ["OpenType Math Illuminated", TUGboat
30:1 (2009)](https://www.tug.org/TUGboat/tb30-1/tb30-1-vieth.pdf).
