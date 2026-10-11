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
    /// A registration for the same device port left by an earlier session
    /// is reused rather than doubled: the adb server keeps its table across
    /// clients, and a detached inspector is still dialling through it. A
    /// new registration asks for `tcp:0`, so the adb server picks a local
    /// port that is free and no two sessions can collide on one.
    ///
    /// # Errors
    /// Returns an error when `adb` cannot be located, its server cannot be
    /// started, or the forward cannot be listed or created.
    pub async fn open(host: &Host, serial: &str, device_port: u16) -> Result<Self> {
        let adb = Adb::locate(host)
            .await
            .wrap_err("Failed to locate adb for the inspector's connection to the device")?;
        let remote = format!("tcp:{device_port}");

        let existing = adb
            .forwards(host, serial, FORWARD_DEADLINE)
            .await
            .wrap_err("Failed to list the device's adb forwards")?
            .into_iter()
            .find(|forward| forward.remote == remote);

        let local_port = if let Some(forward) = existing {
            debug!(
                port = forward.local_port,
                device_port, serial, "reusing the inspector's adb forward"
            );
            forward.local_port
        } else {
            let port = adb
                .forward(host, serial, &remote, FORWARD_DEADLINE)
                .await
                .wrap_err_with(|| {
                    format!("Failed to forward a local port to {remote} on the device {serial}")
                })?;
            debug!(
                port,
                device_port, serial, "created the inspector's adb forward"
            );
            port
        };

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

    /// Remove the registration from the adb server.
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
    /// answers `forward --list` / `forward tcp:0` from canned responses.
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

    /// Stage what `adb forward --list` answers and what port
    /// `adb forward tcp:0 …` reports binding.
    fn stage(machine: &TestMachine, list: &str, bound_port: &str) {
        machine.respond("ADB_FORWARD_LIST", list);
        machine.respond("ADB_FORWARD_PORT", bound_port);
    }

    /// With nothing forwarded, a session asks the adb server for a port of
    /// its own (`tcp:0`) and dials the one it answers.
    #[test]
    fn opening_a_forward_binds_a_port_the_adb_server_chose() {
        let (machine, host, log) = adb_machine();
        stage(&machine, "", "49321\n");

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
        stage(&machine, "", "49322\n");

        let forward = smol::block_on(AndroidForward::open(&host, "pixel-9-pro:5555", 23106))
            .expect("the forward opens");
        smol::block_on(forward.close()).expect("the forward is removed");

        let argv = adb_argv(&log);
        assert!(
            argv.contains("-s pixel-9-pro:5555 forward --remove tcp:49322"),
            "{argv}"
        );
    }

    /// A detached inspector leaves its forward behind; the next session for
    /// the same device endpoint reuses it instead of binding a second local
    /// port onto the same place.
    #[test]
    fn an_existing_forward_for_the_same_endpoint_is_reused() {
        let (machine, host, log) = adb_machine();
        stage(
            &machine,
            "pixel-9-pro:5555 tcp:49400 tcp:23106\n",
            "49999\n",
        );

        let forward = smol::block_on(AndroidForward::open(&host, "pixel-9-pro:5555", 23106))
            .expect("the forward opens");

        assert_eq!(forward.local_addr().to_string(), "127.0.0.1:49400");
        let argv = adb_argv(&log);
        assert!(!argv.contains("forward tcp:0"), "{argv}");
    }

    /// Registrations for other endpoints — another device's, and another
    /// port on this one — are not mistaken for this session's, and the
    /// local ports they hold are not reused.
    #[test]
    fn forwards_onto_other_endpoints_are_not_reused() {
        let (machine, host, log) = adb_machine();
        stage(
            &machine,
            "emulator-5554 tcp:49400 tcp:23106\npixel-9-pro:5555 tcp:49401 tcp:23107\n",
            "49500\n",
        );

        let forward = smol::block_on(AndroidForward::open(&host, "pixel-9-pro:5555", 23106))
            .expect("the forward opens");

        assert_eq!(forward.local_addr().to_string(), "127.0.0.1:49500");
        let argv = adb_argv(&log);
        assert!(
            argv.contains("-s pixel-9-pro:5555 forward tcp:0 tcp:23106"),
            "{argv}"
        );
    }
}
