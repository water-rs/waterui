//! How a running application announces that it can be inspected.
//!
//! A debug build listens on an ephemeral port with a per-process token, so
//! neither is known ahead of time. Each such process writes one file naming its
//! endpoint; a tool that wants to attach reads the directory instead of being
//! told a port. This is the same idea as a browser's active-port file, widened
//! to more than one application at a time.
//!
//! The file is for a developer's own machine. It is written with the process id
//! in its name so two applications never collide, and removed when the endpoint
//! shuts down — a file whose process is gone is stale and
//! [`Advertisement::is_live`] reports it as such.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What a running application publishes about its inspector endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Advertisement {
    /// Process id of the application.
    pub pid: u32,
    /// Name the application reports for itself.
    pub app_name: String,
    /// Rendering backend the application is running on.
    pub backend: String,
    /// Where the endpoint is listening.
    ///
    /// An address of `0.0.0.0` means the endpoint accepts connections from
    /// another machine — a phone or a simulator inspected from a computer. The
    /// port is what matters to a reader; the host it should dial is the device.
    pub addr: SocketAddr,
    /// Token the endpoint requires.
    pub token: String,
}

impl Advertisement {
    /// Whether the advertising process is still running.
    ///
    /// A crashed application leaves its file behind; reading one is not
    /// evidence that anything is listening.
    #[must_use]
    pub fn is_live(&self) -> bool {
        process_exists(self.pid)
    }

    /// Writes this advertisement and returns where it was written.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the directory cannot be created or the file
    /// cannot be written.
    pub fn publish(&self) -> io::Result<PathBuf> {
        let directory = directory();
        fs::create_dir_all(&directory)?;
        let path = directory.join(file_name(self.pid));
        let body = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        fs::write(&path, body)?;
        Ok(path)
    }

    /// Removes this application's advertisement.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the file exists but cannot be removed. A file
    /// that is already gone is not an error.
    pub fn withdraw(&self) -> io::Result<()> {
        match fs::remove_file(directory().join(file_name(self.pid))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// Every advertisement currently on this machine, newest process first.
///
/// Files left by processes that have exited are skipped, and removed so the
/// directory does not grow without bound.
///
/// # Errors
///
/// Returns an I/O error when the directory exists but cannot be read. A missing
/// directory simply means nothing is advertising.
pub fn list() -> io::Result<Vec<Advertisement>> {
    let directory = directory();
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut found = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Some(advertisement) = read(&path) else {
            let _ = fs::remove_file(&path);
            continue;
        };
        if advertisement.is_live() {
            found.push(advertisement);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    found.sort_by_key(|advertisement| std::cmp::Reverse(advertisement.pid));
    Ok(found)
}

/// Reads one advertisement, or `None` when the file is unreadable or malformed.
fn read(path: &Path) -> Option<Advertisement> {
    let body = fs::read(path).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Where advertisements are written.
///
/// `WATERUI_INSPECTOR_DISCOVERY_DIR` overrides the location, which a test needs
/// so that it does not read or write a developer's real directory.
#[must_use]
pub fn directory() -> PathBuf {
    std::env::var_os("WATERUI_INSPECTOR_DISCOVERY_DIR").map_or_else(
        || std::env::temp_dir().join("waterui-inspector"),
        PathBuf::from,
    )
}

fn file_name(pid: u32) -> String {
    format!("{pid}.json")
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    // `kill` reads 0 as "every process in my group" and would answer yes, so a
    // file claiming pid 0 would look alive forever. No application has it.
    if pid == 0 {
        return false;
    }
    // Signal 0 performs the permission and existence checks without delivering
    // anything, which is the standard way to ask whether a pid is alive.
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: `kill` with signal 0 only inspects process state; it dereferences
    // no pointer and delivers no signal.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(not(unix))]
fn process_exists(_pid: u32) -> bool {
    // Without a cheap portable check, assume the process is alive and let the
    // connection attempt be the thing that fails.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advertisement(pid: u32) -> Advertisement {
        Advertisement {
            pid,
            app_name: String::from("Gallery"),
            backend: String::from("hydrolysis"),
            addr: "127.0.0.1:23106".parse().expect("a valid address"),
            token: String::from("token"),
        }
    }

    #[test]
    fn a_published_advertisement_is_listed_and_withdrawn() {
        let directory = std::env::temp_dir().join(format!(
            "waterui-inspector-test-{}-{}",
            std::process::id(),
            line!()
        ));
        // SAFETY: the test owns this variable for the duration of the process it
        // runs in; nextest gives each test its own process.
        unsafe { std::env::set_var("WATERUI_INSPECTOR_DISCOVERY_DIR", &directory) };

        let mine = advertisement(std::process::id());
        mine.publish().expect("publishing writes the file");

        let listed = list().expect("the directory is readable");
        assert_eq!(listed.len(), 1, "got {listed:?}");
        assert_eq!(listed[0].pid, std::process::id());

        mine.withdraw().expect("withdrawing removes the file");
        assert!(list().expect("still readable").is_empty());

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_advertisement_from_a_dead_process_is_not_listed() {
        let directory = std::env::temp_dir().join(format!(
            "waterui-inspector-test-{}-{}",
            std::process::id(),
            line!()
        ));
        // SAFETY: as above.
        unsafe { std::env::set_var("WATERUI_INSPECTOR_DISCOVERY_DIR", &directory) };

        // Two values that are never a live application: a pid too large to be
        // one, and 0, which `kill` would otherwise read as "my own group".
        advertisement(u32::MAX)
            .publish()
            .expect("publishing writes the file");
        advertisement(0).publish().expect("publishing writes the file");

        assert!(
            list().expect("the directory is readable").is_empty(),
            "a stale advertisement was reported as live"
        );

        let _ = fs::remove_dir_all(&directory);
    }
}
