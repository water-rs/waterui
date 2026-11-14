# filtrate-derive

Procedural macros for [`filtrate`](https://crates.io/crates/filtrate):
`#[derive(Filter)]` generates a single-stage filter — the `Filter`
implementation, its `ColorFilter` or `SpatialFilter` kind, and optionally its
`CpuKernel` — from one `#[filter(...)]` attribute.

Not useful on its own: `filtrate` re-exports what you need, so depend on that
rather than on this crate directly.
