//! Cookie helpers shared by the web view backends.

use cookie::time::{OffsetDateTime, SignedDuration};

/// A cookie expiry given as seconds since 1970 as the date the cookie
/// carries, keeping whole seconds truncated toward zero.
///
/// # Errors
///
/// A non-finite value, or one outside `OffsetDateTime`'s range, names no date
/// a cookie can carry; it fails naming the cookie and the raw value.
pub fn cookie_expiry(name: &str, seconds: f64) -> Result<OffsetDateTime, waterui_core::Error> {
    seconds
        .is_finite()
        .then(|| SignedDuration::checked_seconds_f64(seconds.trunc()))
        .flatten()
        .and_then(|offset| OffsetDateTime::UNIX_EPOCH.checked_add(offset))
        .ok_or_else(|| {
            waterui_core::Error::msg(format!(
                "cookie {name:?} has an expiry of {seconds} s since 1970, which is not a representable date"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::cookie_expiry;

    #[test]
    fn cookie_expiry_converts_a_normal_date() {
        let expiry =
            cookie_expiry("session", 1_700_000_000.75).expect("a finite in-range expiry converts");
        assert_eq!(expiry.unix_timestamp(), 1_700_000_000);
        let expiry =
            cookie_expiry("session", -86_400.5).expect("a finite pre-1970 expiry converts");
        assert_eq!(expiry.unix_timestamp(), -86_400);
    }

    #[test]
    fn cookie_expiry_rejects_non_finite_values() {
        for (seconds, text) in [
            (f64::NAN, "NaN"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
        ] {
            let error = cookie_expiry("session", seconds).expect_err("a non-finite expiry fails");
            let message = error.to_string();
            assert!(message.contains("session"), "{message}");
            assert!(message.contains(text), "{message}");
        }
    }

    #[test]
    fn cookie_expiry_rejects_out_of_range_values() {
        for seconds in [1e300, -1e300, f64::MAX] {
            let error =
                cookie_expiry("session", seconds).expect_err("an out-of-range expiry fails");
            let message = error.to_string();
            assert!(message.contains("session"), "{message}");
            assert!(message.contains(&seconds.to_string()), "{message}");
        }
    }
}
