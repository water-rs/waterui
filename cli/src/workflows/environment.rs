//! Declared environment for launched applications.

use crate::toolchain::Host;

/// Resolve `--env NAME` declarations into a launch's environment map.
///
/// Each name is a key the caller declares; its value comes only from the
/// supplied host's immutable environment — no ambient state is read and no
/// assignment syntax is parsed at the command line, so a command-line token
/// can never inject a value into the launch. Whitespace, newlines and `=` in
/// a value are preserved exactly, as is an empty value.
///
/// # Errors
/// Fails for a malformed declaration or a name the host does not define.
/// Errors name a validated key, never a raw declaration or value.
pub fn declared_environment(host: &Host, names: &[String]) -> eyre::Result<Vec<(String, String)>> {
    names
        .iter()
        .map(|name| {
            if name.is_empty() || name.contains('=') || name.contains('\0') {
                eyre::bail!(
                    "an environment name must be non-empty and must not contain '=' or NUL"
                );
            }
            let raw = host
                .env(name)
                .ok_or_else(|| eyre::eyre!("environment variable {name} is not set"))?;
            let value = raw
                .to_str()
                .ok_or_else(|| eyre::eyre!("environment variable {name} is not valid UTF-8"))?;
            if value.contains('\0') {
                eyre::bail!("environment variable {name} must not contain a NUL byte");
            }
            Ok((name.clone(), value.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::toolchain::Host;

    use super::declared_environment;

    fn host(declared: impl IntoIterator<Item = (String, String)>) -> Host {
        Host::new(Vec::<std::path::PathBuf>::new(), declared)
    }

    #[test]
    fn a_declared_name_resolves_to_the_hosts_value() {
        let host = host([(
            String::from("WATERUI_TEST_VALUE"),
            String::from("real-value"),
        )]);
        assert_eq!(
            declared_environment(&host, &[String::from("WATERUI_TEST_VALUE")]).expect("declared"),
            [(
                String::from("WATERUI_TEST_VALUE"),
                String::from("real-value")
            )]
        );
    }

    #[test]
    fn opaque_values_are_used_verbatim() {
        for value in ["  spaced \n", "with=equals", ""] {
            let host = host([(String::from("WATERUI_TEST_VALUE"), String::from(value))]);
            assert_eq!(
                declared_environment(&host, &[String::from("WATERUI_TEST_VALUE")])
                    .expect("declared")
                    .first()
                    .map(|(_, value)| value.as_str()),
                Some(value)
            );
        }
    }

    #[test]
    fn a_missing_name_fails_without_inventing_a_value() {
        let host = host([]);
        let error = declared_environment(&host, &[String::from("WATERUI_TEST_ABSENT")])
            .expect_err("absent must not resolve");
        assert_eq!(
            error.to_string(),
            "environment variable WATERUI_TEST_ABSENT is not set"
        );
    }

    #[test]
    fn an_assignment_syntax_declaration_is_rejected_without_echo() {
        let host = host([(
            String::from("WATERUI_TEST_VALUE"),
            String::from("s3cret-placeholder"),
        )]);
        let error = declared_environment(
            &host,
            &[String::from("WATERUI_TEST_VALUE=s3cret-placeholder")],
        )
        .expect_err("assignment syntax is not a name");
        assert!(!error.to_string().contains("s3cret-placeholder"), "{error}");
    }

    #[test]
    fn an_empty_name_is_rejected() {
        let host = host([]);
        let _ = declared_environment(&host, &[String::new()]).expect_err("empty must not resolve");
    }

    #[test]
    fn a_nul_name_is_rejected() {
        let host = host([]);
        let _ = declared_environment(&host, &[String::from("KEY\0TAIL")])
            .expect_err("NUL must not resolve");
    }

    #[cfg(unix)]
    #[test]
    fn a_nul_value_is_rejected() {
        use std::os::unix::ffi::OsStringExt as _;

        let host = Host::new(
            Vec::<std::path::PathBuf>::new(),
            [(
                std::ffi::OsString::from("WATERUI_TEST_VALUE"),
                std::ffi::OsString::from_vec(vec![b'x', 0x00, b'y']),
            )],
        );
        let error = declared_environment(&host, &[String::from("WATERUI_TEST_VALUE")])
            .expect_err("NUL must not resolve");
        assert_eq!(
            error.to_string(),
            "environment variable WATERUI_TEST_VALUE must not contain a NUL byte"
        );
    }

    #[test]
    fn an_empty_declaration_list_resolves_to_nothing() {
        let host = host([]);
        assert_eq!(declared_environment(&host, &[]).expect("empty"), vec![]);
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_value_fails_naming_only_the_key() {
        use std::os::unix::ffi::OsStringExt as _;

        let host = Host::new(
            Vec::<std::path::PathBuf>::new(),
            [(
                std::ffi::OsString::from("WATERUI_TEST_VALUE"),
                std::ffi::OsString::from_vec(vec![b'x', 0xff]),
            )],
        );
        let error = declared_environment(&host, &[String::from("WATERUI_TEST_VALUE")])
            .expect_err("invalid UTF-8 must not resolve");
        assert_eq!(
            error.to_string(),
            "environment variable WATERUI_TEST_VALUE is not valid UTF-8"
        );
    }
}
