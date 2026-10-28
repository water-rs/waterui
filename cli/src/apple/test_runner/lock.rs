//! The per-destination lock a second `cargo test` or nextest run waits on.
//!
//! The bound is 600 s, as in the simulator script it replaces. When the
//! bound passes the runner fails with the lock path — it is never a retry
//! loop. Inside one run, nextest serializes the launches through the
//! `apple-app-launch` test group (`.config/nextest.toml`).

use std::path::Path;
use std::time::Duration;

use eyre::{Context as _, bail, eyre};
use smol::Timer;

/// The longest an invocation waits for another run to release the
/// destination.
pub const LOCK_BOUND: Duration = Duration::from_secs(600);

/// An exclusive file lock, released on drop.
#[derive(Debug)]
pub struct DestinationLock {
    _flock: nix::fcntl::Flock<std::fs::File>,
}

impl DestinationLock {
    /// Acquires the exclusive lock at `path`, waiting at most
    /// [`LOCK_BOUND`].
    ///
    /// # Errors
    /// Returns an error naming `path` when the bound passes, or when the
    /// lock file cannot be opened or locked.
    pub async fn acquire(path: &Path) -> eyre::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).wrap_err_with(|| {
                format!("failed to create the lock directory {}", parent.display())
            })?;
        }
        let lock_path = path.to_path_buf();
        let acquire = smol::unblock(move || -> eyre::Result<nix::fcntl::Flock<std::fs::File>> {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&lock_path)
                .wrap_err_with(|| format!("failed to open the lock {}", lock_path.display()))?;
            let flock = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusive)
                .map_err(|(_, error)| eyre!("failed to lock {}: {error}", lock_path.display()))?;
            Ok(flock)
        });
        let acquire = std::pin::pin!(acquire);
        let bound = std::pin::pin!(Timer::after(LOCK_BOUND));
        match futures_util::future::select(acquire, bound).await {
            futures_util::future::Either::Left((result, _)) => Ok(Self { _flock: result? }),
            futures_util::future::Either::Right((_instant, _)) => bail!(
                "the destination lock {} is still held after {LOCK_BOUND:?}: another run is using \
                 this destination",
                path.display()
            ),
        }
    }
}
