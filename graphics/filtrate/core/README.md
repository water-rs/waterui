# filtrate-core

The stable abstraction layer behind [`filtrate`](https://crates.io/crates/filtrate):
the pure-data `Filter` trait with its `ColorFilter` and `SpatialFilter` kinds,
the stage declarations a shader composer turns into programs, `Chain`, and the
parameter and animation primitives.

It describes filters without knowing how they are executed — no GPU, no
`wgpu`, no shader compiler, no dependencies at all — so a filter definition
can be shared by a renderer, a CPU backend, a test, or a tool that only
inspects it.

Most users want `filtrate` instead, which pairs these definitions with the
built-in filters and a reference executor.
