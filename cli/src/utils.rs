use std::path::PathBuf;

use color_eyre::eyre;
use smol::{process::Command, unblock};

pub async fn which(name: &'static str) -> Result<PathBuf, which::Error> {
    unblock(move || which::which(name)).await
}

pub async fn run_command(name: &'static str, args: &[&str]) -> eyre::Result<String> {
    let result = Command::new(name)
        .args(args)
        .kill_on_drop(true)
        .output()
        .await?;

    if result.status.success() {
        Ok(String::from_utf8_lossy(&result.stdout).to_string())
    } else {
        Err(eyre::eyre!(
            "Command {} failed with status {}",
            name,
            result.status
        ))
    }
}

macro_rules! impl_display {
    ($ty:ty) => {};
}
