pub(crate) fn parse_bool_env(component: &str, name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => panic!(
                "{component}: invalid {name} value `{raw}`; expected one of 1/0, true/false, yes/no, on/off"
            ),
        },
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => panic!("{component}: invalid {name} environment value: {error}"),
    }
}

pub(crate) fn parse_positive_u64_env(component: &str, name: &str, default: u64) -> u64 {
    parse_optional_positive_u64_env(component, name).unwrap_or(default)
}

/// A positive integer from environment variable `name`, or `None` when it is
/// not set. A set but invalid value panics.
pub(crate) fn parse_optional_positive_u64_env(component: &str, name: &str) -> Option<u64> {
    match std::env::var(name) {
        Ok(raw) => {
            let parsed = raw
                .trim()
                .parse::<u64>()
                .unwrap_or_else(|error| panic!("{component}: invalid {name} `{raw}`: {error}"));
            if parsed == 0 {
                panic!("{component}: {name} must be > 0");
            }
            Some(parsed)
        }
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => panic!("{component}: invalid {name} environment value: {error}"),
    }
}
