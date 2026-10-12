//! The `adb forward` that puts an Android application's inspector endpoint
//! within reach of the inspector running here.
//!
//! An Android application is inspected from the developer's machine, not
//! from the phone: Android's cached-app freezer suspends whichever of the
//! two is in the background, so an inspector in front of the inspected
//! application stops it from answering (water-rs/waterui#2230). The adb
//! server bridges the gap — a local TCP port it binds carries every byte to
//! the port the application listens on over on the device, and the
//! application never leaves the foreground.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use eyre::{Context as _, Result};
use futures_util::future::{Either, select};
use tracing::debug;

use crate::android::adb::Adb;
use crate::toolchain::Host;

/// How long an `adb forward` verb may take. These talk to the adb server
/// about its own table, so they answer at once or the transport is wedged.
const FORWARD_DEADLINE: Duration = Duration::from_secs(20);

/// A local TCP port the adb server carries to a device port.
#[derive(Debug)]
pub struct AndroidForward {
    host: Host,
    adb: Adb,
    serial: String,
    local_port: u16,
    device_port: u16,
}

impl AndroidForward {
    /// Make `device_port` on `serial` reachable on this machine's loopback.
    ///
    /// Every session registers its own forward — `tcp:0` asks the adb
    /// server for a local port that is free, so no two sessions can
    /// collide on one and no session ever borrows a registration another
    /// process owns. What a session registers it also removes: the table
    /// is never listed, because a line there belongs to whoever wrote it
    /// — possibly a detached inspector still dialling through it.
    ///
    /// # Errors
    /// Returns an error when `adb` cannot be located, its server cannot be
    /// started, or the forward cannot be created.
    pub async fn open(host: &Host, serial: &str, device_port: u16) -> Result<Self> {
        // `locate` is bounded like the forward verbs themselves: nothing is
        // acquired yet, so a wedged server lookup fails rather than holding
        // the caller forever.
        let locate = std::pin::pin!(Adb::locate(host));
        let deadline = std::pin::pin!(smol::Timer::after(FORWARD_DEADLINE));
        let adb = match select(locate, deadline).await {
            Either::Left((result, _)) => result
                .wrap_err("Failed to locate adb for the inspector's connection to the device")?,
            Either::Right(_) => {
                eyre::bail!("timed out locating adb for the inspector's connection to the device");
            }
        };
        let remote = format!("tcp:{device_port}");

        let local_port = adb
            .forward(host, serial, &remote, FORWARD_DEADLINE)
            .await
            .wrap_err_with(|| {
                format!("Failed to forward a local port to {remote} on the device {serial}")
            })?;
        debug!(
            port = local_port,
            device_port, serial, "created the inspector's adb forward"
        );

        Ok(Self {
            host: host.clone(),
            adb,
            serial: serial.to_string(),
            local_port,
            device_port,
        })
    }

    /// The address on this machine that reaches the device's endpoint.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), self.local_port)
    }

    /// The port the inspected application listens on, on the device.
    #[must_use]
    pub const fn device_port(&self) -> u16 {
        self.device_port
    }

    /// Remove this session's registration from the adb server. Only the
    /// port this handle bound is removed — other sessions' registrations,
    /// including a detached inspector's, are left exactly as they were.
    ///
    /// # Errors
    /// Returns an error when the removal fails.
    pub async fn close(self) -> Result<()> {
        self.adb
            .remove_forward(&self.host, &self.serial, self.local_port, FORWARD_DEADLINE)
            .await
            .wrap_err_with(|| {
                format!(
                    "Failed to remove the inspector's adb forward tcp:{} on {}",
                    self.local_port, self.serial
                )
            })?;
        debug!(
            port = self.local_port,
            serial = self.serial,
            "removed the inspector's adb forward"
        );
        Ok(())
    }
}

// The fake `adb` is a shell dispatcher, so these run where one can be
// spawned.
#[cfg(all(test, unix))]
mod tests {
    use std::path::{Path, PathBuf};

    use super::AndroidForward;
    use crate::toolchain::Host;
    use crate::toolchain::testing::TestMachine;

    /// A scratch machine whose `adb` logs every invocation's argv and
    /// answers `forward tcp:0` / `forward --remove` from canned responses.
    fn adb_machine() -> (TestMachine, Host, PathBuf) {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);
        (machine, host, log)
    }

    /// The logged adb argv — one invocation per line.
    fn adb_argv(log: &Path) -> String {
        match std::fs::read_to_string(log) {
            Ok(argv) => argv,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => panic!("read the fake adb argv log: {error}"),
        }
    }

    /// Stage what port `adb forward tcp:0 …` reports binding — and,
    /// where a test needs the reuse path's bait, what `forward --list`
    /// would answer if anything still called it.
    fn stage(machine: &TestMachine, bound_port: &str) {
        machine.respond("ADB_FORWARD_PORT", bound_port);
    }

    /// With nothing forwarded, a session asks the adb server for a port of
    /// its own (`tcp:0`) and dials the one it answers.
    #[test]
    fn opening_a_forward_binds_a_port_the_adb_server_chose() {
        let (machine, host, log) = adb_machine();
        stage(&machine, "49321\n");

        let forward = smol::block_on(AndroidForward::open(&host, "pixel-9-pro:5555", 23106))
            .expect("the forward opens");

        assert_eq!(forward.local_addr().to_string(), "127.0.0.1:49321");
        assert_eq!(forward.device_port(), 23106);
        let argv = adb_argv(&log);
        assert!(
            argv.contains("-s pixel-9-pro:5555 forward tcp:0 tcp:23106"),
            "{argv}"
        );
    }

    /// Ending the session takes the registration off the adb server, so a
    /// local port is not held for a device endpoint nothing is reading.
    #[test]
    fn closing_a_forward_removes_it_from_the_adb_server() {
        let (machine, host, log) = adb_machine();
        stage(&machine, "49322\n");

        let forward = smol::block_on(AndroidForward::open(&host, "pixel-9-pro:5555", 23106))
            .expect("the forward opens");
        smol::block_on(forward.close()).expect("the forward is removed");

        let argv = adb_argv(&log);
        assert!(
            argv.contains("-s pixel-9-pro:5555 forward --remove tcp:49322"),
            "{argv}"
        );
    }

    /// Two sessions for the same device endpoint each bind their own local
    /// port — even with the first one's registration sitting in the
    /// server's table, which the reuse path would have taken — and
    /// closing one removes only its own registration, never the other's.
    #[test]
    fn each_session_removes_only_its_own_registration() {
        let (machine, host, log) = adb_machine();

        stage(&machine, "49321\n");
        let first = smol::block_on(AndroidForward::open(&host, "pixel-9-pro:5555", 23106))
            .expect("the first forward opens");

        // The first registration is on the server — the old code listed
        // and reused it; this session must bind its own anyway.
        machine.respond("ADB_FORWARD_LIST", "pixel-9-pro:5555 tcp:49321 tcp:23106\n");
        stage(&machine, "49322\n");
        let second = smol::block_on(AndroidForward::open(&host, "pixel-9-pro:5555", 23106))
            .expect("the second forward opens");
        assert_eq!(second.local_addr().to_string(), "127.0.0.1:49322");

        smol::block_on(first.close()).expect("the first forward is removed");
        // The first close removes exactly its own port — the second
        // session's registration is not touched.
        let argv = adb_argv(&log);
        assert!(
            argv.contains("-s pixel-9-pro:5555 forward --remove tcp:49321"),
            "{argv}"
        );
        assert!(
            !argv.contains("forward --remove tcp:49322"),
            "the sibling's registration was removed: {argv}"
        );
        smol::block_on(second.close()).expect("the second forward is removed");

        let argv = adb_argv(&log);
        assert!(
            argv.contains("-s pixel-9-pro:5555 forward --remove tcp:49322"),
            "{argv}"
        );
        assert!(
            !argv.contains("forward --list"),
            "the table is never listed: {argv}"
        );
    }
}
