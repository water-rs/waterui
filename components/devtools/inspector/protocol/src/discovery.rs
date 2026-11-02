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
use std::io::{self, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What a running application publishes about its inspector endpoint.
#[derive(Clone, Serialize, Deserialize)]
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

// The token is a credential: the file carries it to whoever is allowed to
// read the file; diagnostics are not.
impl std::fmt::Debug for Advertisement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Advertisement")
            .field("pid", &self.pid)
            .field("app_name", &self.app_name)
            .field("backend", &self.backend)
            .field("addr", &self.addr)
            .field("token", &"[redacted]")
            .finish()
    }
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
    /// Files left behind by processes that are gone are swept first. An
    /// application that is killed never runs its shutdown, so without this the
    /// directory grows for as long as nothing happens to read it.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the directory cannot be created or the file
    /// cannot be written.
    pub fn publish(&self) -> io::Result<PathBuf> {
        self.publish_in(&directory()?)
    }

    /// The filesystem half of [`Advertisement::publish`], at an explicit
    /// directory so a private location never needs the process env.
    fn publish_in(&self, directory: &Path) -> io::Result<PathBuf> {
        fs::create_dir_all(directory)?;
        let _ = list_in(directory);
        let path = directory.join(file_name(self.pid));
        let body = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        // The token lives in the file: it is owner-only from creation,
        // and the rename replaces a stale destination, never follows it.
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(&body)?;
        file.persist(&path).map_err(|persist| persist.error)?;
        Ok(path)
    }

    /// Removes this application's advertisement.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the file exists but cannot be removed. A file
    /// that is already gone is not an error.
    pub fn withdraw(&self) -> io::Result<()> {
        self.withdraw_in(&directory()?)
    }

    /// The filesystem half of [`Advertisement::withdraw`], at an explicit
    /// directory.
    fn withdraw_in(&self, directory: &Path) -> io::Result<()> {
        match fs::remove_file(directory.join(file_name(self.pid))) {
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
    list_in(&directory()?)
}

/// The filesystem half of [`list`], at an explicit directory.
fn list_in(directory: &Path) -> io::Result<Vec<Advertisement>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        // Managed names only — the canonical `file_name` a pid maps to;
        // foreign files and an in-flight publish's temp name are left alone.
        let managed_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|stem| stem.parse::<u32>().ok())
            .is_some_and(|pid| path.file_name() == Some(file_name(pid).as_ref()));
        if !managed_name {
            continue;
        }
        if !entry.file_type()?.is_file() {
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
/// `WATERUI_INSPECTOR_DISCOVERY_DIR` overrides the location, which an
/// application sandbox that cannot share the machine cache passes as its
/// own application-private path — and which a test uses so it does not
/// read or write a developer's real directory. Without the override the
/// directory is the platform's per-user cache: the token the file
/// carries is only as private as the account that owns it. There is no
/// fallback into the shared temp directory, where another user could
/// read the credential.
///
/// # Errors
///
/// Returns an I/O error when no override is set and the platform reports
/// no per-user cache directory.
pub fn directory() -> io::Result<PathBuf> {
    directory_from(
        std::env::var_os("WATERUI_INSPECTOR_DISCOVERY_DIR"),
        dirs::cache_dir(),
    )
}

/// The boundary handling behind [`directory`] — split out so it can be
/// exercised without touching the process environment.
fn directory_from(
    override_dir: Option<std::ffi::OsString>,
    cache_dir: Option<PathBuf>,
) -> io::Result<PathBuf> {
    if let Some(override_dir) = override_dir {
        return Ok(PathBuf::from(override_dir));
    }
    cache_dir
        .map(|cache_dir| cache_dir.join("waterui-inspector"))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no per-user cache directory; set WATERUI_INSPECTOR_DISCOVERY_DIR",
            )
        })
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

#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, WAIT_TIMEOUT};
    // `SYNCHRONIZE` is a *standard* access right, shared by every kernel object
    // type, so `windows-sys` emits it exactly once — under the first module
    // whose metadata needed it, which happens to be file access rights. It is
    // the same bit a process handle takes.
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
    };

    // Pid 0 is the System Idle Process, which no application is.
    if pid == 0 {
        return false;
    }
    let access = PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE as PROCESS_ACCESS_RIGHTS;
    // SAFETY: `OpenProcess` takes no pointer and returns a handle or null.
    let handle = unsafe { OpenProcess(access, FALSE, pid) };
    if handle.is_null() {
        return false;
    }
    // A process handle is signalled once the process exits, so a zero-timeout
    // wait that times out is the process still running. `GetExitCodeProcess`
    // would be the obvious alternative and is wrong here: it reports
    // `STILL_ACTIVE` (259), which a live process can also legitimately exit
    // with, and the two are then indistinguishable.
    //
    // SAFETY: the handle came from the call above and is closed below.
    let alive = unsafe { WaitForSingleObject(handle, 0) } == WAIT_TIMEOUT;
    // SAFETY: as above; the handle is not used again.
    unsafe { CloseHandle(handle) };
    alive
}

#[cfg(not(any(unix, windows)))]
#[expect(
    clippy::missing_const_for_fn,
    reason = "this arm answers from a constant only because the target has no process table to ask; the `unix` and `windows` arms call into the OS, and `process_exists` has one signature on all three. Marking just this one `const` would propagate to `ProcessHandle::is_live` and make a public method `const` on some targets and not others."
)]
fn process_exists(_pid: u32) -> bool {
    // Without a cheap portable check, assume the process is alive and let the
    // connection attempt be the thing that fails.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;
    #[cfg(unix)]
    use std::process::Command;

    fn advertisement(pid: u32) -> Advertisement {
        Advertisement {
            pid,
            app_name: String::from("Gallery"),
            backend: String::from("hydrolysis"),
            addr: "127.0.0.1:23106".parse().expect("a valid address"),
            token: String::from("token"),
        }
    }

    /// The token is a credential: `Debug` carries the advertised
    /// identity — pid, name, backend, address — never the secret value.
    #[test]
    fn the_debug_form_never_shows_the_token() {
        let mut advertised = advertisement(1234);
        advertised.token = String::from("s3cret-advertisement-token");
        let rendered = format!("{advertised:?}");
        assert!(
            !rendered.contains("s3cret-advertisement-token"),
            "{rendered}"
        );
        assert!(rendered.contains("1234"), "{rendered}");
        assert!(rendered.contains("Gallery"), "{rendered}");
        assert!(rendered.contains("hydrolysis"), "{rendered}");
        assert!(rendered.contains("23106"), "{rendered}");
        // `Debug` redacts the readout only — the wire and on-disk form still
        // carry the real token, which is how the client authenticates.
        let serialized = serde_json::to_value(&advertised).expect("serialization");
        assert_eq!(
            serialized["token"], "s3cret-advertisement-token",
            "{serialized}"
        );
    }

    #[test]
    fn a_published_advertisement_is_listed_and_withdrawn() {
        let directory = tempfile::TempDir::new().expect("a private directory");

        let mine = advertisement(std::process::id());
        mine.publish_in(directory.path())
            .expect("publishing writes the file");

        let listed = list_in(directory.path()).expect("the directory is readable");
        assert_eq!(listed.len(), 1, "got {listed:?}");
        assert_eq!(listed[0].pid, std::process::id());

        mine.withdraw_in(directory.path())
            .expect("withdrawing removes the file");
        let listed = list_in(directory.path()).expect("still readable");
        assert!(listed.is_empty(), "expected empty, got {listed:?}");
    }

    #[test]
    fn an_advertisement_from_a_dead_process_is_not_listed() {
        let directory = tempfile::TempDir::new().expect("a private directory");

        // Two values that are never a live application: a pid too large to be
        // one, and 0, which `kill` would otherwise read as "my own group".
        advertisement(u32::MAX)
            .publish_in(directory.path())
            .expect("publishing writes the file");
        advertisement(0)
            .publish_in(directory.path())
            .expect("publishing writes the file");

        assert!(
            list_in(directory.path())
                .expect("the directory is readable")
                .is_empty(),
            "a stale advertisement was reported as live"
        );
    }

    #[test]
    fn publishing_sweeps_advertisements_left_by_dead_processes() {
        let directory = tempfile::TempDir::new().expect("a private directory");

        let dead = advertisement(u32::MAX);
        dead.publish_in(directory.path())
            .expect("publishing writes the file");

        advertisement(std::process::id())
            .publish_in(directory.path())
            .expect("publishing writes the file");

        assert!(
            !directory.path().join(file_name(u32::MAX)).exists(),
            "publishing left a dead process's file on disk"
        );
    }

    /// The override wins; without it the platform's per-user cache hosts
    /// the directory; no cache and no override is an error, never a
    /// shared-temp fallback.
    #[test]
    fn the_discovery_directory_resolves_override_then_cache() {
        let overridden = directory_from(
            Some(std::ffi::OsString::from("/private/advert")),
            Some(PathBuf::from("/cache")),
        )
        .expect("an override always resolves");
        assert_eq!(overridden, PathBuf::from("/private/advert"));

        let cached = directory_from(None, Some(PathBuf::from("/cache")))
            .expect("a cache resolves the default");
        assert_eq!(cached, PathBuf::from("/cache/waterui-inspector"));

        directory_from(None, None).expect_err("no cache and no override is an error");
    }

    /// A published file is born owner-only — the token it carries never
    /// sits a moment as world-readable, even under a loose `umask`. The
    /// child sets the umask so the parent's stays what it was.
    #[cfg(unix)]
    #[test]
    fn a_published_file_is_owner_only_under_a_loose_umask() {
        if let Some(dir) = std::env::var_os("WATERUI_INSPECTOR_CHILD_DIR") {
            // SAFETY: the child process's own umask only.
            unsafe { libc::umask(0) };
            advertisement(4242)
                .publish_in(Path::new(&dir))
                .expect("publishing writes the file");
            return;
        }

        let directory = tempfile::TempDir::new().expect("a private directory");
        let output = Command::new(std::env::current_exe().expect("the test binary"))
            .arg("discovery::tests::a_published_file_is_owner_only_under_a_loose_umask")
            .arg("--exact")
            .env("WATERUI_INSPECTOR_CHILD_DIR", directory.path())
            .output()
            .expect("the child runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let mode = fs::metadata(directory.path().join("4242.json"))
            .expect("the published file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
    }

    /// A stale advertisement already sitting at the destination is
    /// replaced outright — the new file is owner-only, not the old
    /// entry's mode carried over.
    #[cfg(unix)]
    #[test]
    fn publishing_replaces_a_stale_file_with_a_private_one() {
        let directory = tempfile::TempDir::new().expect("a private directory");
        let stale = directory
            .path()
            .join(format!("{}.json", std::process::id()));
        fs::write(&stale, b"{}").expect("the stale file");
        fs::set_permissions(&stale, fs::Permissions::from_mode(0o644))
            .expect("the stale file's loose mode");

        let path = advertisement(std::process::id())
            .publish_in(directory.path())
            .expect("publishing replaces the stale file");
        assert_eq!(path, stale);

        let mode = fs::metadata(&path).expect("the file").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
        let parsed: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("the new body"))
                .expect("the new body is the advertisement's JSON");
        assert_eq!(parsed["app_name"], "Gallery");
    }

    /// A symlink at the destination is never followed: the file it
    /// points at keeps its own bytes and the advertisement lands as a
    /// regular, owner-only file at the destination path.
    #[cfg(unix)]
    #[test]
    fn publishing_never_writes_through_a_destination_symlink() {
        let directory = tempfile::TempDir::new().expect("a private directory");
        let owned = tempfile::TempDir::new().expect("the target's directory");
        // A valid, live advertisement — the sweep keeps it, so any change
        // to its bytes is a write-through.
        let sentinel = Advertisement {
            pid: std::process::id(),
            app_name: String::from("Sentinel"),
            backend: String::from("hydrolysis"),
            addr: "127.0.0.1:1".parse().expect("a valid address"),
            token: String::from("sentinel-token"),
        };
        let target = owned.path().join("sentinel.json");
        fs::write(&target, serde_json::to_vec_pretty(&sentinel).expect("JSON"))
            .expect("the sentinel");
        let dest = directory
            .path()
            .join(format!("{}.json", std::process::id()));
        std::os::unix::fs::symlink(&target, &dest).expect("the symlink");

        advertisement(std::process::id())
            .publish_in(directory.path())
            .expect("publishing over a symlink destination");

        let after: Advertisement =
            serde_json::from_slice(&fs::read(&target).expect("the sentinel"))
                .expect("the sentinel still parses");
        assert_eq!(after.app_name, "Sentinel");
        assert_eq!(after.token, "sentinel-token");
        let meta = fs::symlink_metadata(&dest).expect("the destination");
        assert!(meta.is_file() && !meta.is_symlink(), "{meta:?}");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        serde_json::from_slice::<serde_json::Value>(&fs::read(&dest).expect("the body"))
            .expect("the destination holds the new advertisement");
    }

    /// `list` manages only its own canonical names: foreign files,
    /// pid-parseable but noncanonical names, a managed-named directory,
    /// and an in-flight publish's temp file are never reported nor swept.
    #[cfg(unix)]
    #[test]
    fn list_never_touches_foreign_or_in_flight_files() {
        let directory = tempfile::TempDir::new().expect("a private directory");

        // A foreign file the override directory happens to hold.
        let foreign = directory.path().join("notes.json");
        fs::write(&foreign, "not an advertisement").expect("a foreign file");
        // A directory wearing a managed name: `file_type` keeps it out.
        fs::create_dir(directory.path().join(file_name(4243))).expect("a dir");
        // Pid-parseable names the canonical `file_name` never produces —
        // valid advertisements under `+` and leading-zero stems stay put.
        let pid = std::process::id();
        for name in [format!("+{pid}.json"), format!("0{pid}.json")] {
            fs::write(
                directory.path().join(name),
                serde_json::to_vec_pretty(&advertisement(pid)).expect("JSON"),
            )
            .expect("a noncanonical name");
        }
        // An in-flight publish's temp file holding a finished body.
        let mut pending = tempfile::NamedTempFile::new_in(directory.path())
            .expect("an in-flight publish's temp file");
        let mut pending_ad = advertisement(pid);
        pending_ad.app_name = String::from("Pending");
        pending
            .write_all(&serde_json::to_vec_pretty(&pending_ad).expect("JSON"))
            .expect("the pending body");

        advertisement(pid)
            .publish_in(directory.path())
            .expect("publishing");

        let listed = list_in(directory.path()).expect("listing");
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].pid, pid);
        assert!(foreign.exists(), "the foreign file was swept");
        assert!(pending.path().exists(), "the temp file was swept");
        assert!(
            directory.path().join(file_name(4243)).exists(),
            "the managed-named directory was swept"
        );
        assert!(directory.path().join(format!("+{pid}.json")).exists());
        assert!(directory.path().join(format!("0{pid}.json")).exists());

        // Persisting over the canonical name replaces the earlier file —
        // the pending body is the advertisement now listed.
        pending
            .persist(directory.path().join(file_name(pid)))
            .expect("persisting onto the canonical name");
        let listed = list_in(directory.path()).expect("listing again");
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].app_name, "Pending");
        let mode = fs::metadata(directory.path().join(file_name(pid)))
            .expect("the published file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
    }
}
