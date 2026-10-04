# #68 validation and delivery

Base: origin/dev 4e57ef3805f1b87745c5a834126a3cab8279e619. The public API
decision for #2 is [paint-transform.md](paint-transform.md).

`bash ~/iteration68-8.sh`: full merge-gate green on Rust 1.98.1 (fmt,
workspace/bench/each-feature Clippy, 352/352 nextest, doctests, corpus), plus
`cargo +1.95.0 check --locked --workspace --all-targets` passed.
Original 83 GPU scenes retain identical metrics and PNG bytes; mean FLIP
0.004523845590774329. Current corpus produces 93 reports per backend;
unsupported underlying paints are explicitly reported, not rendered by fallback.

The first behavioral failure exposed two f32 affine maps changing nearest-image
texel boundaries. Composing paint and image mappings once in f64 fixed it.
Iteration7 passed correctness but text encode was +1.064% Ir, mostly allocator
work. Iteration8 specializes Paint::clone at recording sites; all five scenes
were remeasured with the code change. Explicit identity wrappers also use the
ordinary paint path, including live initial values.

Inclusive Callgrind Ir, second instrumented steady frame, #43 methodology:

| Scene | Dev lower | Branch lower | Delta | Dev encode | Branch encode | Delta |
|---|---:|---:|---:|---:|---:|---:|
| map | 4,516,991 | 4,516,995 | +0.000% | 1,038,914 | 996,396 | -4.093% |
| chart | 272,921 | 272,923 | +0.001% | 137,084 | 136,234 | -0.620% |
| text-page | 643,556 | 643,557 | +0.000% | 101,903 | 101,649 | -0.249% |
| ui-list | 1,112,898 | 1,112,903 | +0.000% | 172,413 | 169,749 | -1.545% |
| effects | 229,475 | 229,914 | +0.191% | 21,746 | 21,223 | -2.405% |

All ten deltas meet +1%. No native wall-clock acceptance.

| New scene | GPU mean FLIP | CPU mean FLIP | Vello mean FLIP |
|---|---:|---:|---:|
| paint-transform-radial-stroke | 0.0041004262 | 0.0019149636 | 0.0968698357 |
| paint-transform-radial-reflect | 0.0031268271 | 0.0008923897 | 0.0689897284 |
| paint-transform-linear-shear | 0.0032270737 | 0.0009095400 | 0.3170459110 |
| paint-transform-sweep | 0.0022842426 | unsupported | 0.0237568295 |
| paint-transform-image | 0.0023300757 | unsupported | 0.4408517332 |

GPU PNGs were viewed directly: radial stroked border, reflected radial paint,
sheared linear paint, sweep wedge and independently transformed checker pattern.
Vello's elevated oracle color/image error is material and is not claimed fixed:
untransformed image-repeat already has mean FLIP 0.446 versus transformed-image
0.441; radial-reflect 0.078 versus transformed 0.069. A separate Vello test proves
paint P on fixed geometry equals native layer P on inverse-transformed geometry
at interior samples, isolating the new coordinate contract from existing errors.
CPU sweep/image support belongs to #66. Tests cover GPU/CPU live dirty counts,
retained/full identity, nested order, invalid maps, GPU shader UV mapping,
noncommuting image composition, and explicit identity pixel equality.
