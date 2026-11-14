//! The handle on a child's whole process tree, whichever the platform offers:
//! the process group the child was spawned to lead on Unix, a job object the
//! child is assigned to on Windows.
//!
//! A lone `kill_on_drop` on the direct child orphans whatever it re-execs
//! (`bun run dev` → `node vite`, `winget` → its installer), so a caller that
//! must be able to stop the whole tree spawns the child through
//! [`ProcessTree::spawn`], the only way to obtain a tree: on Unix it makes the
//! child lead its group before the spawn, so the group is the child's pid by
//! construction and nothing queries it afterwards.

use std::io;

/// A spawned child whose operating-system process identifier is known,
/// whichever process API spawned it.
pub trait Spawned {
    /// The child's process identifier.
    fn id(&self) -> u32;
}

impl Spawned for std::process::Child {
    fn id(&self) -> u32 {
        Self::id(self)
    }
}

impl Spawned for smol::process::Child {
    fn id(&self) -> u32 {
        Self::id(self)
    }
}

/// A child and every process it re-execs, addressable as one.
#[cfg(unix)]
#[derive(Debug)]
pub struct ProcessTree {
    group: nix::unistd::Pid,
}

#[cfg(unix)]
impl ProcessTree {
    /// Make `command`'s child lead a process group of its own, run `spawn` on
    /// it, and return the child with the tree that group forms. `spawn` may
    /// wrap the command in another process API but must spawn exactly this
    /// command. The group is the child's pid because the child was spawned as
    /// its leader, so signalling it never reaches a group `water` shares.
    pub fn spawn<C: Spawned>(
        mut command: std::process::Command,
        spawn: impl FnOnce(std::process::Command) -> io::Result<C>,
    ) -> io::Result<(C, Self)> {
        use std::os::unix::process::CommandExt as _;

        command.process_group(0);
        let child = spawn(command)?;
        let group = nix::unistd::Pid::from_raw(
            i32::try_from(child.id()).expect("process identifiers fit in i32"),
        );
        Ok((child, Self { group }))
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
    /// Run `spawn` on `command` and return the child with a new job that has
    /// taken it. Nothing needs arranging before the spawn: the processes the
    /// child spawns from then on join the job automatically.
    pub fn spawn<C: Spawned>(
        command: std::process::Command,
        spawn: impl FnOnce(std::process::Command) -> io::Result<C>,
    ) -> io::Result<(C, Self)> {
        let child = spawn(command)?;
        let tree = Self::adopt(child.id())?;
        Ok((child, tree))
    }

    /// A new job that kills its members on close, with the process `pid`
    /// assigned to it.
    fn adopt(pid: u32) -> io::Result<Self> {
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
        let job = unsafe { OwnedHandle::from_raw_handle(job) };
        let tree = Self { job };
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
