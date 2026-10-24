//! Shared naga module metadata for the DXIL build and the writer tests.

/// Whether `module` contains f16 scalars, vectors or matrices: those need
/// shader model 6.2 and `-enable-16bit-types`, so the emission for an
/// f16-free module stays at 6.0.
pub fn uses_f16(module: &naga::Module) -> bool {
    let f16 = |scalar: naga::Scalar| scalar.kind == naga::ScalarKind::Float && scalar.width == 2;
    module.types.iter().any(|(_, ty)| match ty.inner {
        naga::TypeInner::Scalar(scalar) => f16(scalar),
        naga::TypeInner::Vector { scalar, .. } | naga::TypeInner::Matrix { scalar, .. } => {
            f16(scalar)
        }
        _ => false,
    })
}
