//! Resource ownership at the native application and embedding boundaries.

use std::{
    ffi::{CStr, OsStr, c_char},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};
use waterui_core::{Environment, ResourceContext};

/// Copies the host's asset and font directories into this instance.
///
/// # Safety
/// Both paths must be valid NUL-terminated strings for this call.
#[must_use]
pub unsafe fn from_host(assets: *const c_char, fonts: *const c_char) -> ResourceContext {
    // A path is bytes on Apple platforms, so a non-UTF-8 one is still exact.
    // SAFETY: both pointers follow the string-lifetime contract above.
    let assets = unsafe { CStr::from_ptr(assets) };
    // SAFETY: the font path follows the same contract.
    let fonts = unsafe { CStr::from_ptr(fonts) };
    ResourceContext::new(
        PathBuf::from(OsStr::from_bytes(assets.to_bytes())),
        PathBuf::from(OsStr::from_bytes(fonts.to_bytes())),
    )
}

pub(crate) fn install_application(
    env: &mut Environment,
    fonts: &mut crate::fonts::FontRegistrations,
) {
    let resources = match env.get::<ResourceContext>().cloned() {
        Some(resources) => resources,
        None => match ResourceContext::application() {
            Ok(resources) => resources,
            Err(error) => {
                tracing::error!(
                    "cannot locate the application's resource directories: {error}; \
                     the app runs without bundled assets and fonts"
                );
                return;
            }
        },
    };
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
            ResourceContext::from_environment(&first).map(ResourceContext::assets),
            Some(std::path::Path::new("/first/assets"))
        );
        assert_eq!(
            ResourceContext::from_environment(&second).map(ResourceContext::fonts),
            Some(std::path::Path::new("/second/fonts"))
        );
    }

    /// A path is bytes: a host directory whose name is not UTF-8 reaches the
    /// instance exactly.
    #[test]
    fn a_non_utf8_host_path_is_kept_byte_for_byte() {
        let assets = std::ffi::CString::new(b"/assets/\xFF".to_vec()).unwrap();
        let fonts = std::ffi::CString::new("/fonts").unwrap();
        // SAFETY: the strings are live and NUL terminated for the call.
        let resources = unsafe { from_host(assets.as_ptr(), fonts.as_ptr()) };
        assert_eq!(
            resources.assets(),
            std::path::Path::new(OsStr::from_bytes(b"/assets/\xFF"))
        );
    }
}
