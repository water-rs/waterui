//! Build script for waterkit-location.

fn main() {
    if std::env::var("CARGO_FEATURE_APPLE").is_ok() {
        swift_bridge_build::parse_bridges(["src/apple.rs"]);
    }
}
