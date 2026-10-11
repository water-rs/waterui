# GPU mesh paint (#54)

A mesh consists of row-major bilinear quadrilateral patches. Geometry is
bilinear in the two unit-square coordinates. Vertex colours are converted to
premultiplied linear Display P3 before bilinear interpolation. There is no
implicit padding beyond the patches: samples outside every patch are transparent.
When patches overlap, the last row-major patch owns the sample. A folded patch
can have two inverse coordinates; choose the larger vertical coordinate, then
the larger horizontal coordinate. A zero-Jacobian sample is transparent.
These rules apply equally to reflected and nonuniformly transformed patches.

The GPU solves the inverse analytically per covered sample. Six existing paint
buffer entries hold each patch's four corners and premultiplied colours; ordinary
instance layout and all existing paint encodings remain unchanged. There is no
pre-rasterized texture, alternate renderer or component-side approximation.
Nonfinite or unrepresentable GPU coordinates produce an explicit render error.

The independent f64 oracle solves the horizontal-coordinate quadratic; the GPU
solves the vertical-coordinate quadratic. Analytic fixtures, retained updates
and corpus scenes cover skewed patches, alpha, HDR, shared edges, reflection,
overlap and collapsed geometry. Captured scene mesh grids validate their vertex
counts while deserializing. Live mesh operands use the existing retained command
subscriptions and lower only the changed command.

This branch stacks on #68 (f6fed82, itself based on dev 4e57ef3) so mesh
sampling is tested with independent paint coordinates. The shader evaluates
mesh inversion at the mapped paint point, preserving shape/stroke coverage.
The mesh-paint-transform corpus fixture uses a reflected, sheared mapping.
Land #68 first; this branch's bundle includes its prerequisite commits.
