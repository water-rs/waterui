//! Perceptual comparison of Apple E2E screenshots, using the oracle's FLIP.

use std::{error::Error as StdError, process::ExitCode};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod app;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod decode;
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
mod score;

type Error = Box<dyn StdError + Send + Sync>;

fn main() -> ExitCode {
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_writer(std::io::stderr)
        .finish();
    tracing::subscriber::with_default(subscriber, || match run() {
        Ok(status) => status,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::from(2)
        }
    })
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn run() -> Result<ExitCode, Error> {
    app::run()
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn run() -> Result<ExitCode, Error> {
    Err("Apple screenshot comparison requires ARM64 macOS".into())
}
