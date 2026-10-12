#[cfg(test)]
mod tests {
    #[test]
    fn embedded_identity_matches_expected() {
        let expected = std::env::var("EXPECTED_PROTOCOL_COMMIT").expect("expected identity");
        assert_eq!(env!("WATERUI_INSPECTOR_PROTOCOL_COMMIT"), expected);
    }
}
