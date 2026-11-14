# #54 validation and delivery

Base: dev 4e57ef3805f1b87745c5a834126a3cab8279e619, through prerequisite
#68 at f6fed8291bb90d8814441a813f7fa134cab66b63. Land #68 first.

`bash ~/iteration54-4.sh` passed its complete chain:
`merge-gate.sh gaps-54-i4` (Rust 1.98.1 fmt, workspace/bench/each-feature
Clippy -D warnings, 357/357 nextest, doctests, release build, corpus),
plus `cargo +1.95.0 check --locked --workspace --all-targets`.
Original 83 GPU scenes: per-scene metrics and PNG bytes identical to dev,
mean FLIP 0.004523845590774329. No native GPU wall-clock timing was used.

Inclusive Callgrind Ir at the second instrumented steady frame, #43 methodology,
immutable `bench-gaps-54-i4`:

| Scene | Dev lower | Branch lower | Delta | Dev encode | Branch encode | Delta |
|---|---:|---:|---:|---:|---:|---:|
| map | 4,516,991 | 4,516,995 | +0.000% | 1,038,914 | 996,396 | -4.093% |
| chart | 272,921 | 272,969 | +0.018% | 137,084 | 136,591 | -0.360% |
| text-page | 643,556 | 644,069 | +0.080% | 101,903 | 100,558 | -1.320% |
| ui-list | 1,112,898 | 1,113,438 | +0.049% | 172,413 | 170,044 | -1.374% |
| effects | 229,475 | 229,839 | +0.159% | 21,746 | 21,223 | -2.405% |

All ten deltas meet the +1% ceiling.

Design: [mesh-paint.md](mesh-paint.md). Tests cover analytic premultiplied bilinear color, live dirty counts, empty outside samples, grid overflow and independent inverse recovery. Seven new mesh scenes cover skew, reflection, seams, overlap, folds, HDR and independent paint transforms.

New capability scenes, measured against the oracle:

| Scene | Backend | FLIP mean | FLIP max |
|---|---|---:|---:|
| mesh-bilinear | cherenkov | 0.0026507427 | 0.0069147109 |
| mesh-fold | cherenkov | 0.0022383214 | 0.0069625466 |
| mesh-hdr | cherenkov | 0.0026916358 | 0.0109934087 |
| mesh-overlap | cherenkov | 0.0029285568 | 0.0132505737 |
| mesh-paint-transform | cherenkov | 0.0022643283 | 0.0062375094 |
| mesh-reflected | cherenkov | 0.0026507427 | 0.0069147109 |
| mesh-seam | cherenkov | 0.0029589039 | 0.0055674614 |

Direct visual review: root viewed all seven new GPU mesh engine PNGs
(skew/fold/HDR/overlap/paint-transform/reflection/seam); output matches
intended geometry and agrees visually with CPU images.
