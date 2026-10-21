//! The handle on a child's whole process tree, whichever the platform offers:
//! the process group the child was spawned to lead on Unix, a job object the
//! child is assigned to on Windows.
//!
//! A lone `kill_on_drop` on the direct child orphans whatever it re-execs
//! (`bun run dev` → `node vite`, `winget` → its installer), so a caller that
//! must be able to stop the whole tree spawns the child with
//! [`ProcessTree::lead`] and adopts it with [`ProcessTree::adopt`].

use std::io;

/// A child and every process it re-execs, addressable as one.
#[cfg(unix)]
#[derive(Debug)]
pub struct ProcessTree {
    group: nix::unistd::Pid,
}

#[cfg(unix)]
impl ProcessTree {
    /// Make `command`'s child lead a process group of its own, which
    /// [`ProcessTree::adopt`] then addresses.
    pub fn lead(command: &mut std::process::Command) {
        use std::os::unix::process::CommandExt as _;

        command.process_group(0);
    }

    /// The process group the process `pid` leads. It was spawned with
    /// [`ProcessTree::lead`]; a process that did not end up leading its own
    /// group is refused rather than signalled, because a group signal would
    /// then reach whatever group it shares — `water` included.
    pub fn adopt(pid: u32) -> io::Result<Self> {
        let pid =
            nix::unistd::Pid::from_raw(i32::try_from(pid).expect("process identifiers fit in i32"));
        let group = nix::unistd::getpgid(Some(pid))?;
        if group != pid {
            return Err(io::Error::other(format!(
                "process {pid} belongs to group {group} instead of leading its own"
            )));
        }
        Ok(Self { group })
    }

    /// Signal every process in the group: SIGTERM when `graceful`, else
    /// SIGKILL.
    pub fn signal(&self, graceful: bool) {
        let signal = if graceful {
            nix::sys::signal::Signal::SIGTERM
        } else {
            nix::sys::signal::Signal::SIGKILL
        };
        let _ = nix::sys::signal::killpg(self.group, signal);
    }
}

/// A child and every process it re-execs, addressable as one: a job object
/// that kills its members when the last handle closes, so the tree cannot
/// outlive `water` even when it exits without dropping the guard.
#[cfg(windows)]
#[derive(Debug)]
pub struct ProcessTree {
    job: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl ProcessTree {
    /// Nothing to arrange before spawn: the job object takes the child in
    /// [`ProcessTree::adopt`], and the processes it spawns from then on join
    /// automatically.
    pub const fn lead(_command: &mut std::process::Command) {}

    /// A new job that kills its members on close, with the process `pid`
    /// assigned to it.
    pub fn adopt(pid: u32) -> io::Result<Self> {
        use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
        use windows_sys::Win32::System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
        };

        // SAFETY: an unnamed job with default security; the null pointers
        // are the documented arguments for that.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `job` is a fresh handle nothing else owns; the tree closes
        // it exactly once, and closing the last handle kills the members.
        let tree = Self {
            job: unsafe { OwnedHandle::from_raw_handle(job) },
        };
        // SAFETY: an all-zero limit block is the documented starting
        // point; only the kill-on-close flag is set below.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .expect("the limit block is far smaller than u32::MAX bytes");
        // SAFETY: the job handle is live and `limits` is a fully
        // initialised block of the size passed.
        let configured = unsafe {
            SetInformationJobObject(
                tree.job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size,
            )
        };
        if configured == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: opening a process by identifier with the two rights
        // `AssignProcessToJobObject` requires.
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `process` is a fresh handle nothing else owns.
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        // SAFETY: both handles are live.
        let assigned =
            unsafe { AssignProcessToJobObject(tree.job.as_raw_handle(), process.as_raw_handle()) };
        if assigned == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(tree)
    }

    /// Terminate every process in the job. Windows has no graceful signal a
    /// console process observes, so `graceful` selects nothing here; a
    /// caller that wants a grace period escalates to this call after it.
    pub fn signal(&self, graceful: bool) {
        use std::os::windows::io::AsRawHandle as _;
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;

        if graceful {
            return;
        }
        // SAFETY: the job handle is live for as long as `self`.
        let _ = unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) };
    }
}
