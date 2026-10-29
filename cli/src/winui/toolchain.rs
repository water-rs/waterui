//! `WinUI` toolchain checking.

use crate::toolchain::{Host, Toolchain, ToolchainError, rust::RustToolchain};

/// `WinUI` toolchain checker.
///
/// The `WinUI` backend builds a plain MSVC-target Rust binary and stages the
/// Windows App Runtime self-contained, so its only host requirements are a
/// working Rust toolchain and Windows itself. The host check is a compile-time
/// gate in the callers; this checker verifies the Rust side.
#[derive(Debug, Clone, Copy, Default)]
pub struct WinUiToolchain;

impl Toolchain for WinUiToolchain {
    type Installation = <RustToolchain as Toolchain>::Installation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        RustToolchain::default().check(host).await
    }
}
