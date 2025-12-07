pub mod build;
pub mod task;

pub async fn run_command(cmd: &mut smol::process::Command) -> color_eyre::eyre::Result<()> {
    let status = cmd.kill_on_drop(true).status().await?;
    if !status.success() {
        color_eyre::eyre::bail!("command {:?} failed with status {}", cmd, status);
    }
    Ok(())
}
