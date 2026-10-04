# Review of origin/feat/cpu-features for #66

Source tip: 78e946368f7443939d84619429d00f9e2e92b5f8. No source commit is
cherry-picked wholesale. The table records all 37 commits absent dev. Reuse
means selected native backend code was reviewed and adapted, not that every
claim in the old branch is endorsed. The requested new CPU capabilities are
images, sweep, mesh and stroked glyph runs, plus #68 paint transforms. The old
branch's coverage, oracle, shadow, COLR and SIMD redesigns are explicitly not
part of this capability port because they do not preserve dev's contracts.

| Source commit | Verdict | Reason |
|---|---|---|
| `47485fb` | Rejected as a whole; redesigned | Dev already supplies Raster and the shared Engine/Surface/thread/retained compiler. Importing this standalone frontend would undo #43/#64. Native paint support is added to the current compiler instead. |
| `71217fc` | Rewritten | Sweep preparation uses bounded remainder arithmetic and rejects nonfinite angles; the old repeated-add loop can fail to terminate. Extend::None already exists in dev and is enabled in the CPU bench capability list. |
| `133e63c` | Partly reused, adapted | Retained the scalar blend equations after review. Group/layer integration uses current Operation scopes; preserve dev normal/linear arithmetic and clip the complete composite operation, including Clear and DestIn. |
| `98ee05d` | Rewritten | Use shared Uploads<Rgba8>/ImageUpload and retained Arc pixels. Honor color-space/premultiplied metadata, validate dimensions, avoid fourfold capacity over-allocation, account residency and invalidate removed references. |
| `dde1ed2` | Rejected | Old COLR painter substitutes black for non-solid gradient foregrounds, approximates radial transforms with sqrt(abs(det)), leaves sweep angles untransformed, ignores image foreground opacity and caches by formatted Debug hashes without equality. None is acceptable as a native capability port. |
| `f175d13` | Rejected for this port | Replaces signed-area coverage and quantized filled-glyph sampling. These change existing pixels; the four requested capabilities do not require changing dev coverage or filled-glyph semantics. |
| `530fae9` | Rejected | Convex raw-deposit specialization is coupled to the replacement coverage pipeline. Keep current dev raster behavior and measure its cost explicitly. |
| `78d8995` | Rejected as a replacement | Sparse coverage compiler and exact clip intersection replace current coverage/storage, not the shared retained compiler. Import would change existing scene pixels; no duplicate frontend or alternate raster path is added. |
| `1918e78` | Not applicable | Backtick edits apply only to the rejected legacy coverage design document. |
| `5f90f6d` | Not imported | Formatting of legacy coverage/raster code. Reused blend equations are formatted by the current workspace formatter. |
| `633d3e2` | Adapted | Reject SVG-in-OpenType explicitly in current Renderer font validation. Do not import the obsolete CPU Font wrapper. |
| `e109f0e` | Superseded | Error propagation for the old CPU-owned thread/channel APIs is replaced by the current generic frontend contract. Changing Surface::update signatures would conflict with dev. |
| `e2d9efa` | Rewritten selectively | Old tests assert Mesh/GlyphStroke/BlendSpace are unsupported; those assertions are now wrong. New capability tests execute those paths; remaining unsupported features retain explicit errors. |
| `8e1159d` | Rejected LRU design; corrected accounting | The legacy batch sum counts repeated keys and can reject a render because caching cannot fit it. Current frame masks remain usable independently of optional retention; deduplicate keys, bound cached bytes and reserve residency for images. |
| `861bd59` | Partly reused, mostly rewritten | Retain reviewed bilinear mesh inversion/color rules, with shared placements and independently derived oracle. Stroke glyphs use native retained path strokes. Encoded blends adapt to shared groups. Do not import its standalone layer blend-space API, generalized shadow pipeline or changed filled-glyph sampler. |
| `cae8a55` | Rejected for this port | Changes shadow sigma/shape spread and oracle output, unrelated to the four requested missing CPU capabilities. Existing shadow/oracle semantics remain dev's. |
| `2a4c026` | Not imported | Formatting for the rejected effects/shadow pipeline; current new files pass the workspace formatter. |
| `40d3d90` | Test lesson retained; rewritten | Tests must change actual stroke fields, since kurbo defaults are already round. New live stroke tests change width, dash offset, cap, join and miter, compare cold pixels and verify dirty counts. |
| `5aefdb1` | Rejected for this port | Swaps oracle polygon intersection for i_overlay and changes winding resolution. Original oracle/83-scene metrics are an invariant; independent mesh/stroke additions do not require replacing clipping. |
| `a5d23dc` | Not applicable | No i_overlay dependency is introduced because the oracle intersection replacement is rejected. |
| `774ac04` | Not applicable | Half-tolerance shadow compiler adjustment depends on the rejected generalized shadow implementation. |
| `9ad389a` | Adapted | Opt-in lower/glyph/shade tracing is retained around current shared-lowering/native-raster phases. There is no separate compiled-clip phase in dev; comparisons disclose that difference. |
| `81ba2b8` | Reused dependency intent | Declare tracing and update the existing locked CPU dependency list; no new tracing version is selected. |
| `a6e23de` | Rejected for this port | Renderer-owned SIMD dispatch is sensible, but its band storage and coverage contracts belong to the rejected replacement rasterizer. No claim that current scalar raster matches its time is made. |
| `e49b3ca` | Not applicable | Portable SIMD dependencies are not needed by this port. |
| `4efdaa1` | Rejected for this port | Coverage-span SIMD assumes the sparse Coverage type absent from dev; importing it alone cannot preserve the current raster contract. |
| `33bc1f8` | Not applicable | Owned test pool is sound but exercises the unported SIMD rasterizer. |
| `8d147bd` | Not applicable | Corrects SIMD transparency comparison in the unported kernel. |
| `f7e2c6b` | Not applicable; correctness lesson recorded | Coverage must follow shuffled channel-vector pixel order. Current scalar raster has no lane permutation; do not import the original incorrect or repaired kernel in isolation. |
| `67bf6ff` | Superseded | Current benchmark adapter already populates Submission phases; no obsolete adapter is copied. |
| `1002ea9` | Not applicable | Packed constant-span kernel relies on legacy sparse coverage/band storage. |
| `22076a7` | Not applicable | Packed varying-coverage implementation is part of the unported SIMD pipeline. |
| `4304b9d` | Not applicable | Positive-coverage specialization requires the legacy Coverage span guarantee, which current accumulators do not expose. |
| `23e9a80` | Not applicable | Documents the unported SIMD layouts. |
| `1d33650` | Not applicable | Mask removal relies on positive-only geometry spans; no such assumption is added to current raster code. |
| `8756562` | Not applicable | SIMD band clearing is not separable from the unported dispatch/band implementation without a new measured optimization. |
| `78e9463` | Test lesson retained | Negative-zero destination makes zero-coverage preservation observable. It validates the historical SIMD binary used for diagnostics, not the current scalar port. |

No fallback backend or component rasterization is introduced. COLR, arbitrary
per-glyph transforms and generalized shadow semantics from the rejected branch
remain explicit capability gaps, not silently approximated output. They should
not be advertised as delivered by #66. GPU implementations of #54/#67 live on
their own branches; shared scene/oracle vocabulary is repeated where needed so
each bundle builds independently of those branches.

Historical CPU raster diagnostics use an immutable build of the exact source
tip, one Rayon thread, 5 warmup + 60 measured frames. No prior saved CPU raster
numbers were found. VM wall time is not acceptance evidence; the report labels
this reconstructed reference and compares native shade phases with their
architectural differences disclosed.

## STATUS QUESTION

- The legacy CPU branch's COLR colour fonts, arbitrary per-glyph transforms,
  generalized shadow pipeline plus its replacement coverage rasterizer and SIMD
  kernels are **not imported**; the per-commit reasons are in the verdict table
  above.
- The four requested CPU capabilities (image, sweep, mesh, stroke) **are**
  implemented, but #66 must not be described as whole-legacy-branch feature
  parity.
- Historical CPU prior saved numbers are **unavailable** — none exist in the
  searched paths; diagnostics use the reconstructed exact-tip (`78e9463`)
  baseline instead.
