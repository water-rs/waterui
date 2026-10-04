# Bitmap colour fonts

The GPU and CPU backends render embedded PNG and BGRA bitmap glyphs from sbix
and CBDT/CBLC fonts. Both backends render COLR; SVG colour fonts are unsupported.
The public font and glyph-run APIs are unchanged.

## Detection and strikes

Font registration parses the bitmap tables and records every strike. If a font
contains both sbix and CBDT, sbix takes precedence. EBDT/EBLC are monochrome
tables and are ignored. Malformed or empty strike tables are font-resource
errors.

At device realization, the renderer computes device ppem as the run size
multiplied by the largest singular value of the full content-to-device
transform composed with the glyph's position and per-glyph transform:
`size × σ_max(CTM × translate(x,y) × t)`, where `t` is identity when absent.
It chooses the smallest strike whose ppem is greater than or equal to device
ppem; if there is no such strike, it chooses the largest. Equal-size strikes
are ordered by their table index. A non-finite or non-positive device ppem
selects the largest strike. This choice is made for each realization, so
scaling, rotation, skew and per-glyph transforms can select a different strike
without changing the device-independent prepared glyph.

## Placement and compositing

Glyph placement is stored in em-space, using bitmap bearings, inner bearings,
strike ppem and units per em. The em-space image rectangle is placed by
`translate(x,y) × t × scale(size)`, with `t` identity when absent. CBDT's
top-left origin and sbix's bottom-left origin are converted to the renderer's
y-down coordinates. Rotation, skew and non-uniform scale transform the bitmap
quad itself, not only its axis-aligned bounds; pure translations can fold into
the glyph origin.

PNG data is expanded to straight-alpha RGBA8 and decoded as sRGB. BGRA data is
reordered to RGBA8 and treated as premultiplied sRGB. A bitmap glyph is
composited like a linear/bilinearly sampled image with Pad extension in both
axes. It carries its own colour: the glyph run's paint does not tint it.

If the selected strike has no bitmap for a glyph, that glyph draws nothing.
There is no substitution from another strike, an outline, or `.notdef`.

## Unsupported formats and caching

Non-PNG/BGRA bitmap payloads and masks return the `color-font` unsupported
error. Non-finite or non-invertible per-glyph transforms are render errors;
bitmap-font strokes return the `glyph-stroke` unsupported error. sbix `dupe`
records resolve one level to their target glyph in the same strike; absent
targets and chains of duplicates are font errors.

Decoded entries are keyed by exact `FontId`, strike table index and glyph id.
The key intentionally excludes run size and transform; the cached placement
is em-space, so one decoded image can be reused at different sizes and
transforms. Repeated unchanged frames reuse prepared output and do not decode
the glyph again. Removing a font evicts its bitmap entries. Critical trimming
clears the cache. The CPU cache shares the configured CPU byte budget with
glyph masks, clears on an over-budget insertion, and rejects a single bitmap
larger than that budget.
