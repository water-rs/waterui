# #66 validation and delivery

Base: dev 4e57ef3805f1b87745c5a834126a3cab8279e619, through prerequisite
#68 at f6fed8291bb90d8814441a813f7fa134cab66b63. Land #68 first.

`bash ~/iteration66-5.sh` passed its complete chain:
`merge-gate.sh gaps-66-i5` (Rust 1.98.1 fmt, workspace/bench/each-feature
Clippy -D warnings, 363/363 nextest, doctests, release build, corpus),
plus `cargo +1.95.0 check --locked --workspace --all-targets`.
Original 83 GPU scenes: per-scene metrics and PNG bytes identical to dev,
mean FLIP 0.004523845590774329. No native GPU wall-clock timing was used.

Inclusive Callgrind Ir at the second instrumented steady frame, #43 methodology,
immutable `bench-gaps-66-i5`:

| Scene | Dev lower | Branch lower | Delta | Dev encode | Branch encode | Delta |
|---|---:|---:|---:|---:|---:|---:|
| map | 4,516,991 | 4,516,995 | +0.000% | 1,038,914 | 996,396 | -4.093% |
| chart | 272,921 | 272,927 | +0.002% | 137,084 | 136,233 | -0.621% |
| text-page | 643,556 | 643,561 | +0.001% | 101,903 | 101,904 | +0.001% |
| ui-list | 1,112,898 | 1,112,911 | +0.001% | 172,413 | 169,749 | -1.545% |
| effects | 229,475 | 229,727 | +0.110% | 21,746 | 21,224 | -2.400% |

All ten deltas meet the +1% ceiling.

Design: [cpu-capabilities.md](cpu-capabilities.md); all 37 source-commit verdicts: [cpu-port-review.md](cpu-port-review.md). Tests cover upload alpha metadata, direct-image live operands, sweep domains/errors, bilinear color and dirty updates, glyph style/cache equality, encoded blends, clipped Clear, cache accounting and paint coordinates. The source branch's COLR, per-glyph transforms, general shadows and replacement coverage/SIMD pipeline are not claimed as ported.

New capability scenes, measured against the oracle:

| Scene | Backend | FLIP mean | FLIP max |
|---|---|---:|---:|
| glyph-stroke-dashed | cherenkov-cpu | 0.0153972088 | 0.3504391121 |
| glyph-stroke-thin | cherenkov-cpu | 0.0041397815 | 0.1821257107 |
| glyph-stroke-transformed | cherenkov-cpu | 0.0049321299 | 0.2586246407 |
| mesh-bilinear | cherenkov-cpu | 0.0008415879 | 0.0040174677 |
| mesh-fold | cherenkov-cpu | 0.0008617953 | 0.0043589525 |
| mesh-hdr | cherenkov-cpu | 0.0009488369 | 0.0069291046 |
| mesh-overlap | cherenkov-cpu | 0.0009187066 | 0.0073660512 |
| mesh-paint-transform | cherenkov-cpu | 0.0008564839 | 0.0035275191 |
| mesh-reflected | cherenkov-cpu | 0.0008415879 | 0.0040174677 |
| mesh-seam | cherenkov-cpu | 0.0008437237 | 0.0027064028 |
| paint-transform-image | cherenkov-cpu | 0.0010811445 | 0.0250099539 |
| paint-transform-linear-shear | cherenkov-cpu | 0.0009095400 | 0.0028096122 |
| paint-transform-radial-reflect | cherenkov-cpu | 0.0008923897 | 0.0028084856 |
| paint-transform-radial-stroke | cherenkov-cpu | 0.0019149636 | 0.1812492065 |
| paint-transform-sweep | cherenkov-cpu | 0.0006425436 | 0.0017415139 |

# CPU corpus comparison — corpus-cpu-dev (baseline) vs corpus-cpu-66-i5

baseline binary bench-gaps-67-i5-backends sha256=93493fe66289a95be6fe984b6b6713501bc25d0477a2a4c46b94a27bb3d24d3a
  provenance: built from 67-i5 worktree; cpu/ sources unchanged from dev
candidate binary bench-gaps-66-i5-backends sha256=99da6563cc4a2ec6dfc5bf00c09ac68940d8dbcd3422f47ae506c52c6bc45cd1
baseline reports: 88 scenes (corpus-cpu-dev); candidate reports: 103 scenes (corpus-cpu-66-i5)

common measured scenes compared: 57 — metrics identical and engine PNG bytes identical

## Newly supported / not-baseline scenes (45)
- blend-color: baseline unsupported; measured now
- blend-color-burn: baseline unsupported; measured now
- blend-color-dodge: baseline unsupported; measured now
- blend-darken: baseline unsupported; measured now
- blend-difference: baseline unsupported; measured now
- blend-exclusion: baseline unsupported; measured now
- blend-hard-light: baseline unsupported; measured now
- blend-hue: baseline unsupported; measured now
- blend-lighten: baseline unsupported; measured now
- blend-luminosity: baseline unsupported; measured now
- blend-multiply: baseline unsupported; measured now
- blend-overlay: baseline unsupported; measured now
- blend-saturation: baseline unsupported; measured now
- blend-screen: baseline unsupported; measured now
- blend-soft-light: baseline unsupported; measured now
- glyph-stroke-dashed: not in baseline corpus render
- glyph-stroke-thin: not in baseline corpus render
- glyph-stroke-transformed: not in baseline corpus render
- grad-linear-2-none: baseline unsupported; measured now
- grad-linear-8-none: baseline unsupported; measured now
- grad-radial-2-none: baseline unsupported; measured now
- grad-radial-8-none: baseline unsupported; measured now
- grad-sweep-2-none: baseline unsupported; measured now
- grad-sweep-2-pad: baseline unsupported; measured now
- grad-sweep-2-reflect: baseline unsupported; measured now
- grad-sweep-2-repeat: baseline unsupported; measured now
- grad-sweep-8-none: baseline unsupported; measured now
- grad-sweep-8-pad: baseline unsupported; measured now
- grad-sweep-8-reflect: baseline unsupported; measured now
- grad-sweep-8-repeat: baseline unsupported; measured now
- img-bilinear: baseline unsupported; measured now
- img-nearest: baseline unsupported; measured now
- imgpattern-repeat: baseline unsupported; measured now
- mesh-bilinear: not in baseline corpus render
- mesh-fold: not in baseline corpus render
- mesh-hdr: not in baseline corpus render
- mesh-overlap: not in baseline corpus render
- mesh-paint-transform: not in baseline corpus render
- mesh-reflected: not in baseline corpus render
- mesh-seam: not in baseline corpus render
- paint-transform-image: not in baseline corpus render
- paint-transform-linear-shear: not in baseline corpus render
- paint-transform-radial-reflect: not in baseline corpus render
- paint-transform-radial-stroke: not in baseline corpus render
- paint-transform-sweep: not in baseline corpus render

## Still unsupported in both (1)
- text-colr: unsupported in both

# CPU raster diagnostic comparison

Historical reference is an immutable build of origin/feat/cpu-features tip
78e946368f7443939d84619429d00f9e2e92b5f8, reconstructed on this VM; no saved
prior CPU shade timings were found. Both runs use one Rayon worker, 5 warmup
and 60 measured frames, tracing target cherenkov_cpu::profile=debug. This is
wall-clock diagnostic data only, never no-regression acceptance on this VM.
Historical shade follows separate coverage/clip compilation and uses SIMD;
current shade includes dev's native band coverage/clip rasterization. These
phase boundaries and raster algorithms differ, so the table is not a speedup
claim. Inclusive lower/encode Callgrind tables supply acceptance evidence.

| Scene | Historical shade µs | Current shade µs | Diagnostic delta | Current lower µs | Current glyph µs |
|---|---:|---:|---:|---:|---:|
| map | 7807.8 | 32203.7 | +312.5% | 4595.1 | 0.1 |
| chart | 1927.3 | 5380.9 | +179.2% | 280.8 | 3.4 |
| text-page | 1952.5 | 4038.6 | +106.8% | 175.4 | 56.4 |
| ui-list | 5009.0 | 81418.3 | +1525.4% | 567.3 | 68.9 |
| effects | 9072.2 | 330681.9 | +3545.0% | 157.7 | 0.1 |

Root directly viewed all 15 new CPU PNGs — seven meshes, three outlined-glyph
scenes, five paint-coordinate scenes — with consistent geometry and color
appearance.
