# GPU stroked glyph runs (#67)

GlyphStyle::Stroke expands unhinted font outlines with the complete kurbo
stroke style in run units, before the enclosing drawing transform. Width,
joins, caps, miter limit, dash pattern and dash offset have the same semantics
as a path stroke. Paint remains in run coordinates. COLR fonts use their base
outline when stroked; palette paint graphs remain a filled-glyph operation.
A missing outline is an explicit font error. Empty outlines draw nothing.
Per-glyph transforms compose into the stroke placement about the glyph
origin, between the font scale and the glyph position (#69); a non-finite
or non-invertible transform is a render error, and a missing outline an
explicit font error.

The GPU backend resolves these semantic glyphs into its existing cached stroke
coverage operations. It does not change normal filled-glyph atlas keys, frame
instances, frontend commands or retained dirty ranges. Cache identity includes
the realized outline coordinates and every stroke parameter through the same
stroke hashing as ordinary paths. Style, variation, size and geometry changes
therefore cannot reuse stale stroke coverage. The oracle independently obtains
the font outline and emits a semantic scene stroke before normal oracle coverage.

New corpus scenes and cache-update tests cover outlined text, dashed and joined
strokes, fractional placement, enclosing nonuniform transforms and
retained updates. Gate/corpus/Ir measurements accompany the delivery report.
