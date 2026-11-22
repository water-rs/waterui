//! The commit the build script embedded.

pub const COMMIT: &str = env!("GIT_HEAD_PROBE_COMMIT");
