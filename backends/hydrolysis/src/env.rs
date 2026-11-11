//! Operator settings read from environment variables.
//!
//! A set but invalid value is a contract violation by whoever configured the
//! process: it is reported and the setting keeps its default.

use std::env::VarError;

use waterui_core::contract_violation;

/// The boolean in environment variable `name`, or `default` when it is not
/// set or holds no boolean.
pub fn parse_bool_env(component: &str, name: &str, default: bool) -> bool {
    bool_from(component, name, std::env::var(name), default)
}

fn bool_from(component: &str, name: &str, var: Result<String, VarError>, default: bool) -> bool {
    match var {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => {
                contract_violation!(
                    "{component}: invalid {name} value `{raw}`; expected one of 1/0, true/false, yes/no, on/off; the setting keeps its default"
                );
                default
            }
        },
        Err(VarError::NotPresent) => default,
        Err(error) => {
            contract_violation!(
                "{component}: invalid {name} environment value: {error}; the setting keeps its default"
            );
            default
        }
    }
}

/// The positive integer in environment variable `name`, or `default` when it
/// is not set or holds no positive integer.
pub fn parse_positive_u64_env(component: &str, name: &str, default: u64) -> u64 {
    parse_optional_positive_u64_env(component, name).unwrap_or(default)
}

/// A positive integer from environment variable `name`, or `None` when it is
/// not set or holds no positive integer.
pub fn parse_optional_positive_u64_env(component: &str, name: &str) -> Option<u64> {
    positive_u64_from(component, name, std::env::var(name))
}

fn positive_u64_from(component: &str, name: &str, var: Result<String, VarError>) -> Option<u64> {
    match var {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => {
                contract_violation!(
                    "{component}: {name} must be > 0, got `{raw}`; the setting is ignored"
                );
                None
            }
            Ok(parsed) => Some(parsed),
            Err(error) => {
                contract_violation!(
                    "{component}: invalid {name} `{raw}`: {error}; the setting is ignored"
                );
                None
            }
        },
        Err(VarError::NotPresent) => None,
        Err(error) => {
            contract_violation!(
                "{component}: invalid {name} environment value: {error}; the setting is ignored"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bool_from, positive_u64_from};
    use std::env::VarError;

    #[test]
    fn valid_values_parse() {
        assert!(bool_from("t", "N", Ok(" Yes ".into()), false));
        assert!(!bool_from("t", "N", Ok("off".into()), true));
        assert!(bool_from("t", "N", Err(VarError::NotPresent), true));
        assert_eq!(positive_u64_from("t", "N", Ok(" 40 ".into())), Some(40));
        assert_eq!(positive_u64_from("t", "N", Err(VarError::NotPresent)), None);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "expected one of 1/0"))]
    fn an_invalid_boolean_keeps_the_default() {
        assert!(bool_from("t", "N", Ok("maybe".into()), true));
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "must be > 0"))]
    fn a_zero_integer_is_ignored() {
        assert_eq!(positive_u64_from("t", "N", Ok("0".into())), None);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "invalid N `x`"))]
    fn a_non_numeric_integer_is_ignored() {
        assert_eq!(positive_u64_from("t", "N", Ok("x".into())), None);
    }
}
