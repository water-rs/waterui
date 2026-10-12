//! Inspector app launcher and lifecycle utilities.

mod launcher;

pub use launcher::{
    InspectorLaunchOptions, InspectorPlatform, InspectorSession, launch_inspector_session,
};

/// An explicit session credential from `WATERUI_INSPECTOR_TOKEN`, used
/// verbatim; whitespace and newlines are the application's choice to refuse.
///
/// # Errors
/// Fails when the variable is set but empty or not valid UTF-8; the value
/// itself never appears in the message.
pub fn explicit_token(host: &crate::toolchain::Host) -> eyre::Result<Option<String>> {
    let Some(raw) = host.env("WATERUI_INSPECTOR_TOKEN") else {
        return Ok(None);
    };
    let token = raw
        .to_str()
        .ok_or_else(|| eyre::eyre!("WATERUI_INSPECTOR_TOKEN must be valid UTF-8"))?;
    if token.is_empty() {
        eyre::bail!("WATERUI_INSPECTOR_TOKEN must not be empty");
    }
    Ok(Some(token.to_string()))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt as _;

    use crate::toolchain::Host;

    use super::explicit_token;

    #[test]
    fn no_declared_token_means_none() {
        let host = Host::new(
            Vec::<std::path::PathBuf>::new(),
            Vec::<(String, String)>::new(),
        );
        assert_eq!(explicit_token(&host).expect("helper"), None);
    }

    #[test]
    fn a_declared_token_is_used_verbatim() {
        let declared = "  cli-test-credential\n";
        let host = Host::new(
            Vec::<std::path::PathBuf>::new(),
            [(
                String::from("WATERUI_INSPECTOR_TOKEN"),
                String::from(declared),
            )],
        );
        assert_eq!(
            explicit_token(&host).expect("helper").as_deref(),
            Some(declared)
        );
    }

    #[test]
    fn an_empty_token_is_refused() {
        let host = Host::new(
            Vec::<std::path::PathBuf>::new(),
            [(String::from("WATERUI_INSPECTOR_TOKEN"), String::new())],
        );
        let error = explicit_token(&host).expect_err("empty must not pass");
        assert_eq!(
            error.to_string(),
            "WATERUI_INSPECTOR_TOKEN must not be empty"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_token_is_refused() {
        let host = Host::new(
            Vec::<std::path::PathBuf>::new(),
            [(
                std::ffi::OsString::from("WATERUI_INSPECTOR_TOKEN"),
                std::ffi::OsString::from_vec(vec![b'x', 0xff]),
            )],
        );
        let error = explicit_token(&host).expect_err("invalid UTF-8 must not pass");
        assert_eq!(
            error.to_string(),
            "WATERUI_INSPECTOR_TOKEN must be valid UTF-8"
        );
    }
}
