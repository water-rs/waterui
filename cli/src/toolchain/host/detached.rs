//! Running a program without handing it any handle of this process.
//!
//! The standard library always lets a child inherit every inheritable handle
//! of its parent on Windows (`CreateProcess` with `bInheritHandles` set),
//! which is right for a child that streams to our terminal and wrong for a
//! launcher whose daemon must not keep our caller's pipe open. This module
//! makes the one call the standard library cannot: `CreateProcessW` with
//! inheritance off. On the other platforms every descriptor the standard
//! library opens is close-on-exec and daemons detach through `fork`, so a
//! plain spawn with null stdio is that same guarantee.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io,
    path::Path,
    process::ExitStatus,
};

#[cfg(not(windows))]
pub fn run(
    program: &OsStr,
    args: &[OsString],
    env: &BTreeMap<OsString, OsString>,
    cwd: &Path,
) -> io::Result<ExitStatus> {
    use std::process::{Command, Stdio};

    Command::new(program)
        .args(args)
        .env_clear()
        .envs(env)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

#[cfg(windows)]
pub fn run(
    program: &OsStr,
    args: &[OsString],
    env: &BTreeMap<OsString, OsString>,
    cwd: &Path,
) -> io::Result<ExitStatus> {
    use std::os::windows::process::ExitStatusExt as _;

    use windows_sys::Win32::{
        Foundation::{CloseHandle, FALSE, WAIT_OBJECT_0},
        System::Threading::{
            CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetExitCodeProcess,
            INFINITE, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
        },
    };

    let mut command_line = command_line(program, args);
    let environment = environment_block(env);
    let directory = wide(cwd.as_os_str());
    // SAFETY: an all-zero STARTUPINFOW is the documented default; `cb` is set
    // below as the API requires.
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = u32::try_from(std::mem::size_of::<STARTUPINFOW>())
        .expect("STARTUPINFOW is far smaller than u32::MAX bytes");
    // SAFETY: an all-zero PROCESS_INFORMATION is the documented out-parameter
    // state before CreateProcessW fills it.
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer names a live, NUL-terminated buffer owned by this
    // frame for the duration of the call; the command line is mutable as the
    // API requires, and `FALSE` for inheritance is the point of this module.
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            FALSE,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            directory.as_ptr(),
            &raw const startup,
            &raw mut process,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both handles were just returned by a successful CreateProcessW
    // and are closed exactly once below.
    unsafe {
        CloseHandle(process.hThread);
        let waited = WaitForSingleObject(process.hProcess, INFINITE);
        let mut code = 0u32;
        let read = if waited == WAIT_OBJECT_0 {
            GetExitCodeProcess(process.hProcess, &raw mut code)
        } else {
            0
        };
        let error = io::Error::last_os_error();
        CloseHandle(process.hProcess);
        if waited != WAIT_OBJECT_0 || read == 0 {
            return Err(error);
        }
        Ok(ExitStatus::from_raw(code))
    }
}

/// A NUL-terminated UTF-16 copy of `value`.
#[cfg(windows)]
fn wide(value: &OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;

    value.encode_wide().chain(std::iter::once(0)).collect()
}

/// The command line `CommandLineToArgvW` in the child parses back into
/// exactly `program` followed by `args`, per the C runtime's rules: an
/// argument is quoted when it is empty or holds a space, tab or quote;
/// inside quotes a quote is escaped with a backslash and backslashes are
/// doubled only when they precede a quote (or the closing quote).
#[cfg(windows)]
fn command_line(program: &OsStr, args: &[OsString]) -> Vec<u16> {
    use std::{iter, os::windows::ffi::OsStrExt as _};

    let mut line = Vec::new();
    for (index, argument) in iter::once(program)
        .chain(args.iter().map(OsString::as_os_str))
        .enumerate()
    {
        if index > 0 {
            line.push(u16::from(b' '));
        }
        let units = argument.encode_wide().collect::<Vec<_>>();
        let needs_quotes = units.is_empty()
            || units
                .iter()
                .any(|unit| b" \t\"".map(u16::from).contains(unit));
        if !needs_quotes {
            line.extend(units);
            continue;
        }
        line.push(u16::from(b'"'));
        let mut backslashes = 0usize;
        for unit in units {
            if unit == u16::from(b'\\') {
                backslashes += 1;
                continue;
            }
            if unit == u16::from(b'"') {
                line.extend(iter::repeat_n(u16::from(b'\\'), backslashes * 2 + 1));
            } else {
                line.extend(iter::repeat_n(u16::from(b'\\'), backslashes));
            }
            backslashes = 0;
            line.push(unit);
        }
        line.extend(iter::repeat_n(u16::from(b'\\'), backslashes * 2));
        line.push(u16::from(b'"'));
    }
    line.push(0);
    line
}

/// The `KEY=VALUE\0…\0` block, keys in the map's order (already sorted).
#[cfg(windows)]
fn environment_block(env: &BTreeMap<OsString, OsString>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;

    let mut block = Vec::new();
    for (key, value) in env {
        block.extend(key.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(all(test, windows))]
mod tests {
    use std::ffi::{OsStr, OsString};

    use super::command_line;

    fn line(program: &str, args: &[&str]) -> String {
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        let units = command_line(OsStr::new(program), &args);
        String::from_utf16(&units[..units.len() - 1]).expect("ASCII input")
    }

    #[test]
    fn quotes_only_what_the_c_runtime_would_split() {
        assert_eq!(line("adb", &["start-server"]), "adb start-server");
        assert_eq!(
            line(r"C:\Program Files\adb.exe", &["", "a b"]),
            r#""C:\Program Files\adb.exe" "" "a b""#
        );
    }

    #[test]
    fn escapes_quotes_and_the_backslashes_before_them() {
        assert_eq!(line("x", &[r#"say "hi""#]), r#"x "say \"hi\"""#);
        assert_eq!(line("x", &[r"dir\"]), r"x dir\");
        assert_eq!(line("x", &[r"a dir\"]), r#"x "a dir\\""#);
        assert_eq!(line("x", &[r#"\\"q"#]), r#"x "\\\\\"q""#);
    }
}
