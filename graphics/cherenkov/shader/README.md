# cherenkov-shader

The shader composer shared by [Cherenkov](https://github.com/water-rs/cherenkov)
and [`filtrate`](https://crates.io/crates/filtrate).

Every shader fragment is a naga function. A snippet is authored as WGSL, parsed
by naga's WGSL front end and checked against a declared ABI: colour snippets map
a colour to a colour, and spatial snippets sample their input around a
coordinate, given the input's size in pixels. The composer builds one naga
module from a chain of snippets by working on the IR:

- it imports each snippet's function together with its types, constants and
  helpers, and shares WGSL library helpers so each is imported once;
- it folds a colour prefix into the samples of a spatial stage, following
  helper calls, where the stage's sampling allows it;
- it specializes constant parameters, compacts the module and validates it.

No shader text is built by concatenation, and the composed module goes to wgpu
without conversion.
