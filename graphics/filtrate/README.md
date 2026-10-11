# filtrate

A filter library whose filters are shader functions. Every built-in filter
(blurs, colour adjustments, convolutions, distortions, blends with an
auxiliary image, …) is a sequence of stages, and every stage is a WGSL
function for the shared naga-IR composer, `cherenkov-shader`. The same
definitions run in the Cherenkov engine, which fuses them into its own
passes, and in this crate's reference executor, which runs a chain on any
`wgpu` texture: images, decoded video frames, or render targets.

## Crates

| Crate | Role |
| --- | --- |
| `filtrate` | The built-in filters, their WGSL stages, SIMD CPU kernels, and the reference wgpu executor (`Executor`). |
| `filtrate-core` | `no_std`, dependency-free definitions: the `Filter` trait with its `ColorFilter` and `SpatialFilter` kinds, stage declarations, `Chain`, and the parameter and animation primitives. |
| `filtrate-derive` | `#[derive(Filter)]` for single-stage filters. |

## Quick start

```rust
use filtrate::filters::{Blur, Brightness, Grayscale};
use filtrate::{Executor, FilterExt, SpatialFilter};

let chain = Grayscale(1.0_f32).then(Blur(5.0_f32)).then(Brightness(0.2_f32));
// The blur makes the chain spatial: it reads five pixels each way.
assert_eq!(chain.footprint(), 5.0);
let executor = Executor::new(chain);
// Hand `executor` to your render loop: `Effect::setup`, then
// `Effect::render` once per frame.
```

Reactive frontends implement `FilterParam` for their signal types so filter
parameters animate without rebuilding anything; plain `f32` works for static
values.

## The contract

- **Stages are functions.** A colour stage is
  `fn apply(color, params…) -> color`; a spatial stage samples `input`
  through the composer's ABI — `input_point_sampler` when it only fetches
  exact texels, `input_sampler` when it filters — at a normalized `uv`.
  Colours are premultiplied, in the stage's operating space.
- **Kinds are types.** A `ColorFilter` maps each pixel's colour to a colour
  and says whether it is `LINEAR`: a linear map on premultiplied RGBA with
  an identity alpha row and no offset, the property that lets an engine push
  it down into each primitive's shading. A `SpatialFilter` reports its
  `footprint`, the farthest texel it reads in pixels. `Chain<A, B>` is a
  colour filter exactly when both halves are.
- **Working space.** Colours are linear Display P3. Luma coefficients come
  from the working-space constants every stage can take, and each stage
  declares its operating space; an executor converts around stages that
  operate in sRGB.
- **Shape and auxiliary inputs.** A spatial stage can read the clip shape's
  signed distance field or mask, and auxiliary images: the filter's own (a
  blend's second image, a LUT) or the input of its previous stage (bloom's
  composite reads what its extraction pass started from).
- **CPU kernels.** A colour filter may carry a SIMD CPU kernel (`CpuKernel`,
  or `cpu = path` in the derive). `Brightness`, `Saturation`, `Grayscale`
  and `ColorMatrix` do, and each is cross-checked against its shader.

## The reference executor

`Executor` composes a chain and runs the reference alternative of the
composition: one full-screen fragment pass per piece, every spatial stage
materialized, no colour prefix folded into a spatial stage's samples.
Intermediates are `Rgba16Float`, so every materialization point rounds to
f16 and extended values survive. Output matches input by default.

An effect declares its output dimensions through
`Effect::output_size(input_width, input_height)`. The host queries that method
before allocating the output texture, including after reactive size changes.
`Executor` can declare fixed, scaled or state-dependent dimensions:

```rust
use filtrate::{Effect, Executor, filters::Brightness};

let effect = Executor::new(Brightness(0.1_f32))
    .with_output_size(|width, height| (width * 2, height * 2));
assert_eq!(effect.output_size(320, 180), (640, 360));
```

The final pass maps output pixel centres into input coordinates; colour
passes use nearest texels and spatial passes retain their declared sampling.
Intermediates keep the input resolution. The executor rejects a supplied
output whose dimensions differ from its declaration. A sizing callback may
read reactive state; the host owns the subscription that schedules a frame
when that state changes. Texture sizing does not change view layout.

## WebGL (wasm)

The optional `webgl` feature turns on wgpu's WebGL2 backend. It only exists
for `wasm32-unknown-unknown`; enabling it on a native target is a
`compile_error!`. Every executor pass is a fragment pass, so WebGL2 runs the
same program as every other backend, provided it can render to
`Rgba16Float`.

## License

MIT OR Apache-2.0, at your option.
