//! Resource ownership at the native application and embedding boundaries.

use std::{
    ffi::{CStr, c_char},
    path::PathBuf,
};
use waterui_core::{Environment, ResourceContext};

/// Copies the host's asset and font directories into this instance.
///
/// # Panics
/// Panics if either path is not UTF-8.
///
/// # Safety
/// Both paths must be valid NUL-terminated UTF-8 strings for this call.
#[must_use]
pub unsafe fn from_host(assets: *const c_char, fonts: *const c_char) -> ResourceContext {
    // SAFETY: both pointers follow the string-lifetime contract above.
    let assets = unsafe { CStr::from_ptr(assets) }
        .to_str()
        .expect("UTF-8 asset path");
    // SAFETY: the font path follows the same contract.
    let fonts = unsafe { CStr::from_ptr(fonts) }
        .to_str()
        .expect("UTF-8 font path");
    ResourceContext::new(PathBuf::from(assets), PathBuf::from(fonts))
}

pub(crate) fn install_application(
    env: &mut Environment,
    fonts: &mut crate::fonts::FontRegistrations,
) {
    let resources = env.get::<ResourceContext>().cloned().unwrap_or_else(|| {
        ResourceContext::application().expect("application resource directories")
    });
    fonts.register_bundle_fonts(&resources);
    env.insert(resources);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_paths_are_owned_and_instances_are_isolated() {
        let resources = {
            let assets = std::ffi::CString::new("/first/assets").unwrap();
            let fonts = std::ffi::CString::new("/first/fonts").unwrap();
            // SAFETY: the strings are live and NUL terminated for the call.
            unsafe { from_host(assets.as_ptr(), fonts.as_ptr()) }
        };
        let mut first = Environment::new();
        first.insert(resources);
        let mut second = first.clone();
        second.insert(ResourceContext::new("/second/assets", "/second/fonts"));
        assert_eq!(
            ResourceContext::from_environment(&first).assets(),
            std::path::Path::new("/first/assets")
        );
        assert_eq!(
            ResourceContext::from_environment(&second).fonts(),
            std::path::Path::new("/second/fonts")
        );
    }
}
