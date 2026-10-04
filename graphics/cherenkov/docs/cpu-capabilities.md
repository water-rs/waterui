# CPU capabilities on the shared front end (#66)

The Raster backend consumes the shared retained Content/Operation compiler.
There is no second Engine, Surface, thread protocol or reactive tree. Live
operands invalidate their command slot; Pictures share static commands.
Prepared image pixels and mesh patches are immutable Arc allocations. Layer
placement transforms them without resolving or copying their source data.

New native CPU paints are RGBA8 image patterns (nearest/bilinear sampling and
all Extend modes), sweep gradients and premultiplied linear-P3 bilinear meshes.
Direct images use the same paint sampler. Uploads honor sRGB, Display-P3 and
linear-sRGB metadata and encoded premultiplied alpha. Decoding happens once at
registration. Image handles invalidate prepared references when removed.
Registered images, cached glyph masks and decoded bitmap glyphs share
Budget::cpu; active frame data and framebuffers remain separate, non-evictable
allocations. MemoryUsage keeps dev's accounting model: framebuffers,
registered pixels and cached masks/bitmaps, not complete process/retained-
operation heap accounting. Duplicate masks do not inflate byte accounting.
Oversized masks render in their current frame without being retained in the
cache.

Sweep angles are finite radians. Nonpositive spans wrap into (0, 2π]; positive
spans retain their extent, matching the existing sweep domain. Nonfinite or
unrepresentable domains fail explicitly. A mesh uses bilinear patch geometry
and color; outside all patches is transparent, last row-major patch owns an
overlap, and greatest-v then greatest-u owns a folded inverse. Collapsed
zero-Jacobian samples are transparent. The oracle independently solves the
inverse using the other coordinate polynomial.

GlyphStyle::Stroke obtains unhinted font outlines and records native CPU path
stroke operations with full style identity. Width and dashes are in run units;
paint coordinates remain run coordinates. Existing filled-glyph behavior stays
unchanged. #68 paint-to-shape transforms apply to all these paints. Image
pattern mappings compose in f64 before sampling, preventing nearest-neighbor
boundary changes from separately rounded transforms.

Bitmap-only sbix and CBDT/CBLC fonts are decoded as image glyphs on the render
thread. Per-glyph transforms compose as `translate(x,y) * t * scale(size)` on
the em-space image rectangle, and strike selection uses run size multiplied by
the largest singular value of `CTM * translate(x,y) * t`. Missing glyphs in
the selected strike draw nothing. Bitmap-font strokes, COLR and SVG colour
fonts remain unsupported.

Group and layer blends use the existing public BlendMode vocabulary. Groups
also support BlendSpace::SrgbEncoded. Opacity applies before conversion and
compositing. A clip bounds the composite operation, including Clear/DestIn;
it is not just an extra source-alpha multiplier. The existing normal/linear
source-over arithmetic is preserved exactly.

The source branch also attempted a different coverage compiler, SIMD band
storage, exact general shadows and COLR support. Their review decisions are in
[cpu-port-review.md](cpu-port-review.md). This port does not silently accept
unsupported shader/filter paints. COLRv0/v1 colour
glyphs are native: each glyph's paint graph is cached as a
foreground-independent node tree under a structural (glyph, coords) key,
then expanded into ordinary fills, clips and groups at the glyph's
placement. Brush transforms are exact — gradient geometry keeps its own
paint transform instead of a sqrt(det) approximation — and a foreground
brush whose paint carries no alpha channel (image, mesh, shader) keeps its
COLR alpha as group opacity. For fonts with both COLR and bitmap strikes,
COLR glyphs take precedence and other glyphs use the selected bitmap strike.
SVG colour fonts and non-PNG/BGRA bitmap formats remain unsupported.

Opt-in diagnostics use RUST_LOG=cherenkov_cpu::profile=debug. lower_ns covers
retained lowering, glyph_ns mask resolution, and shade_ns framebuffer clearing
plus native band rasterization (including clip work). Unlike the historical
compiler there is no separate clip phase. VM time comparisons are diagnostic;
Callgrind Ir is the CPU-side no-regression acceptance evidence.

The delivery commit is based on #68's final accepted commit (f6fed82), whose
source/test tree was integrated before this branch's full gate. #68's validation
document is retained as provenance. All #66 performance deltas still use the
requested dev 4e57ef3 baseline, not the prerequisite branch.
