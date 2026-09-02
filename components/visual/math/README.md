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
                                  └──► MathML ──► a11y        └──► any backend
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
publishes it as MathML, which is what assistive technology reads — a formula
drawn as anonymous filled paths has no content at all to a screen reader.

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
