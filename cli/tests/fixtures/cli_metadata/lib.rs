//! Assert the CLI identity emitted by the unmodified production build script.

#[test]
fn embeds_the_expected_cli_identity() {
    assert_eq!(
        env!("WATERUI_CLI_COMMIT"),
        std::env::var("EXPECTED_CLI_COMMIT").expect("expected fixture identity")
    );
}
