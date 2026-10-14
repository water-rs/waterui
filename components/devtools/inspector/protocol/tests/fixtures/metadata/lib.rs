#[cfg(test)]
mod tests {
    #[test]
    fn embedded_identity_matches_expected() {
        // The stamp is compile-time (`build.rs` emits it); the expectation is
        // a runtime oracle — reading both at compile time would let a stale
        // rebuild pass an incremental-identity check that should fail.
        let expected = std::env::var("EXPECTED_PROTOCOL_COMMIT").ok();
        let actual = option_env!("WATERUI_INSPECTOR_PROTOCOL_COMMIT").map(str::to_string);
        assert_eq!(actual, expected);
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.0.0");
    }
}
