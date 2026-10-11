# #67 validation and delivery

Base: origin/dev at 4e57ef3805f1b87745c5a834126a3cab8279e619. Design: [GPU stroked glyph runs](stroked-glyphs.md).

`bash ~/iteration67-5.sh` passed: full `merge-gate.sh gaps-67-i5`
(fmt, workspace/bench/each-feature Clippy on Rust 1.98.1, 342 nextest tests,
doctests, release build, corpus), plus `cargo +1.95.0 check --locked --workspace --all-targets`.
First four attempts caught branch-artifact contamination, an oversized oracle
function and unsuffixed f64 test opacity; no failed result was accepted.

Original 83 scenes: identical per-scene metrics and PNG bytes against dev;
mean FLIP 0.004523845590774329. All 91 current branch corpus reports were produced.
New glyph scenes are measured against the independent oracle. GPU thin, dashed
and transformed PNGs were viewed directly: thin hollow outlines, heavier dashed
outlines and nonuniformly transformed outlined text. No pixel-count heuristic.

Inclusive Callgrind Ir: second instrumented steady frame, the #43 harness,
immutable binary `bench-gaps-67-i5`. No native wall-clock acceptance.

| Scene | Dev lower | Branch lower | Delta | Dev encode | Branch encode | Delta |
|---|---:|---:|---:|---:|---:|---:|
| map | 4,516,991 | 4,516,991 | +0.000% | 1,038,914 | 1,038,914 | +0.000% |
| chart | 272,921 | 272,589 | -0.122% | 137,084 | 137,587 | +0.367% |
| text-page | 643,556 | 639,478 | -0.634% | 101,903 | 102,349 | +0.438% |
| ui-list | 1,112,898 | 1,108,440 | -0.401% | 172,413 | 172,864 | +0.262% |
| effects | 229,475 | 229,513 | +0.017% | 21,746 | 21,443 | -1.393% |

All ten deltas satisfy the +1% ceiling.

| New scene | GPU FLIP mean | Vello backend FLIP mean |
|---|---:|---:|
| glyph-stroke-thin | 0.0055504900 | 0.0741456661 |
| glyph-stroke-dashed | 0.0167476773 | 0.0953789756 |
| glyph-stroke-transformed | 0.0060814353 | 0.0519904773 |

CPU remains explicitly unsupported on this branch; #66 implements its stroke capability. Tests exercise live style updates (width, cap, join, miter, dashes and offset), retained dirty counts and exact cold/warm pixel equality.
