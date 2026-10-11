//! Validating and emitting modules for each back end.

use naga::{
    Module,
    valid::{Capabilities, ModuleInfo, ValidationFlags, Validator},
};

use crate::errors::EmitError;

/// Validates a module, for example after an executor added entry points to a
/// composed module.
///
/// # Errors
///
/// Returns [`EmitError::Invalid`] when the module does not validate with
/// `capabilities`.
pub fn validate(module: &Module, capabilities: Capabilities) -> Result<ModuleInfo, EmitError> {
    Validator::new(ValidationFlags::all(), capabilities)
        .validate(module)
        .map_err(|error| EmitError::Invalid(format!("{error}: {:?}", error.as_inner())))
}

/// Emits WGSL.
///
/// # Errors
///
/// Returns the back end's error.
pub fn wgsl(module: &Module, info: &ModuleInfo) -> Result<String, EmitError> {
    Ok(naga::back::wgsl::write_string(
        module,
        info,
        naga::back::wgsl::WriterFlags::empty(),
    )?)
}

/// Emits Metal Shading Language for `lang_version` (major, minor).
///
/// # Errors
///
/// Returns the back end's error.
pub fn msl(
    module: &Module,
    info: &ModuleInfo,
    lang_version: (u8, u8),
) -> Result<String, EmitError> {
    let options = naga::back::msl::Options {
        lang_version,
        ..naga::back::msl::Options::default()
    };
    let (source, _) = naga::back::msl::write_string(
        module,
        info,
        &options,
        &naga::back::msl::PipelineOptions::default(),
    )?;
    Ok(source)
}

/// Emits SPIR-V words for `lang_version` (major, minor).
///
/// # Errors
///
/// Returns the back end's error.
pub fn spirv(
    module: &Module,
    info: &ModuleInfo,
    lang_version: (u8, u8),
) -> Result<Vec<u32>, EmitError> {
    let options = naga::back::spv::Options {
        lang_version,
        ..naga::back::spv::Options::default()
    };
    Ok(naga::back::spv::write_vec(module, info, &options, None)?)
}
