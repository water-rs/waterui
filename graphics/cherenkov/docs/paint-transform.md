# Independent paint coordinates (#68)

Decision for the public API log (#2): `Paint::Transformed(TransformedPaint)`
adds an independent coordinate system to an existing paint. `TransformedPaint`
contains `Arc<Paint>` and a paint-to-shape `Affine`. `Paint::transformed(self,
transform)` is the convenience builder; `TransformedPaint::new` creates a
shared source suitable for signal updates. Existing paint variants, recording
signatures and untransformed serialized values keep their meaning.

For geometry transform G, paint transform P, and device sample d, sample the
underlying paint at P^-1 G^-1 d. Shape coverage, clipping, stroke width and
glyph outlines depend only on G. An outer A around an inner B composes A*B;
successive `.transformed(B).transformed(A)` calls have the same meaning. Image
patterns compose P before their existing texel-to-paint transform. Shader
paints map their sampling coordinates relative to their existing untransformed
shape-bounds domain; shader evaluation and its retained texture stay on GPU.

Finite, invertible reflections are supported. Non-finite or non-invertible
paint transforms fail the render with RenderError::Render, even for a solid.
No implicit identity substitution or transparent rendering occurs. Identity
leaves the ordinary paint path unchanged. Invalid serialized transforms cannot
bypass render-time validation. Backends still require the underlying paint's
capability: a transformed shader does not make shaders available on Raster.

Live operands use the existing Recorder/Live machinery. A signal may map to
`TransformedPaint { paint: Arc::clone(&source), transform }`; the source's stops
or mesh vertices remain shared. Only users of that operand become dirty.
Picture recording accepts the same paint value without subscriptions. No new
recording scope, command variant, second display list or host repaint is needed.

Images compose the new mapping with their pattern affine in f64 before GPU
upload, preserving nearest-texel boundary decisions. Other GPU paints store
the inverse in two entries of the existing variable-size
paint resource buffer only for a transformed paint. The ordinary instance
layout stays unchanged. CPU preparation retains the paint-to-shape mapping
separately from sampled layer placement, so property updates cannot erase it.

Verification: shared GPU/CPU tests check live dirty-command counts, retained
versus full output, invariant stroke coverage, nested transform order, analytic
radial samples, reflections and invalid transforms. New corpus scenes cover a
radial stroke, reflected radial paint, sheared linear paint and a transformed
sweep and a noncommuting image-pattern transform; every backend claiming the underlying paint is checked against the
oracle. Original corpus files are untouched. Gate/corpus/Ir results are recorded
in the delivery report after execution; this decision is not a performance claim.
