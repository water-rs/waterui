# waterui-math

GPU math formula rendering for WaterUI with Vello.

## Input formats

- LaTeX math subset
- MathML Core subset
- Typst math subset

## Rendering model

- Glyphs are shaped via Parley with a math-first font stack.
- Radical signs are built from OpenType `MATH` table variants/assemblies.
- Radical glyph pieces are rendered from font outlines (not handwritten geometry).
- Fraction bars and overbars use vector rectangles.

## Default math font stack

`STIX Two Math, Noto Sans Math, Latin Modern Math, Cambria Math, serif`

`Math::font_family(...)` behavior:

- Empty input: falls back to default stack.
- Name without comma: prepends that family to the default stack.
- Comma-separated input: treated as a full explicit stack.

`Math::font_stack(...)` behavior:

- Accepts a full CSS-like font stack string.
- Panics on empty stack (fast-fail).

## Offscreen visual verification

- Single render export:
  - `WATERUI_MATH_OFFSCREEN_OUT=/tmp/math.png cargo test -p waterui-math renders_math_scene_offscreen_via_gpu_surface`
- Gallery export:
  - `WATERUI_MATH_GALLERY_DIR=/tmp/waterui_preview_tests_math/gallery cargo test -p waterui-math renders_common_llm_formulas_without_clipping`
