//! The physical-iOS-device destination.
//!
//! Signing goes through the CLI's provisioning integration but never
//! provisions: the bundle identifier and development team come from the
//! caller's environment (`WATERUI_IOS_DEVICE_BUNDLE_ID` and
//! `WATERUI_IOS_DEVICE_TEAM_ID`), `CFBundleIdentifier` is rewritten in the
//! staged bundle, and only an already-installed profile is used — no
//! `-allowProvisioningUpdates`, no registration of anything. A missing or
//! expired profile is an explicit error.
//!
//! Install is overwrite-only (the runner never uninstalls). It is skipped
//! only when the host-side stamp — a hash of the unsigned bundle inputs and
//! the installed app's `devicectl`-reported URL — still matches; a foreign
//! overwrite moves the bundle to a new URL and reinstalls. The status file
//! is pulled back with `devicectl device copy from`.

use std::io::Write;

use eyre::{Context as _, bail, eyre};
use sha2::{Digest as _, Sha256};
use smol::fs;

use crate::apple::physical::ApplePhysicalDevice;
use crate::apple::provisioning::{self, SelectError, SigningRequest};
use crate::device::{Artifact, RunOptions};
use crate::platform::TargetPlatform;
use crate::toolchain::Host;

use super::binary::HarnessManifest;
use super::lock::DestinationLock;
use super::status::{StatusFile, TestStatus};
use super::{
    HarnessBundle, Invocation, MachOPlatform, STATUS_PATH_VARIABLE, TestBinary, TestDestination,
    forward_run,
};

/// The caller-supplied device signing identity — never hardcoded.
#[derive(Debug)]
pub struct DeviceIdentity {
    /// The bundle identifier every device launch is signed under.
    pub bundle_id: String,
    /// The development team identifier the profile is selected under.
    pub team_id: String,
}

/// The physical-device runner bound to one paired iPhone or iPad.
#[derive(Debug)]
pub struct IosDevice {
    device: ApplePhysicalDevice,
    identity: DeviceIdentity,
}

impl IosDevice {
    /// Resolves `selector` — a `CoreDevice` identifier, a UDID, or a name —
    /// against `devicectl`'s paired devices. No match, or more than one, is
    /// an error listing the paired devices. The device must then pass
    /// [`ApplePhysicalDevice::usability`]: developer mode, boot state,
    /// tunnel.
    ///
    /// # Errors
    /// Returns an error when `devicectl` fails, the selector matches
    /// nothing or several devices, or the device is unusable.
    pub async fn resolve(
        host: &Host,
        selector: &str,
        identity: DeviceIdentity,
    ) -> eyre::Result<Self> {
        let devices = ApplePhysicalDevice::scan(host).await?;
        let mut matches: Vec<ApplePhysicalDevice> = devices
            .iter()
            .filter(|device| {
                device.identifier() == selector
                    || device.udid() == selector
                    || device.name() == selector
            })
            .cloned()
            .collect();
        let list = || {
            devices.iter().fold(String::new(), |mut out, device| {
                use std::fmt::Write as _;
                write!(
                    out,
                    "\n  {} (CoreDevice {}, udid {})",
                    device.name(),
                    device.identifier(),
                    device.udid()
                )
                .expect("writing to a String cannot fail");
                out
            })
        };
        let device = match matches.len() {
            0 => bail!(
                "WATERUI_IOS_DEVICE names {selector:?} but no paired iOS device matches:{candidates}",
                candidates = list()
            ),
            1 => matches.swap_remove(0),
            _ => bail!(
                "WATERUI_IOS_DEVICE names {selector:?}, which matches more than one paired device; \
                 use the CoreDevice identifier:{candidates}",
                candidates = list()
            ),
        };
        device
            .usability()
            .map_err(|reason| eyre!("{}", reason.remedy(&device)))?;
        Ok(Self { device, identity })
    }
}

impl TestDestination for IosDevice {
    fn platform(&self) -> MachOPlatform {
        MachOPlatform::IOS
    }

    async fn run(
        &self,
        host: &Host,
        invocation: &Invocation,
        interrupts: smol::channel::Receiver<()>,
        stdout: &mut (impl Write + Send + ?Sized),
        stderr: &mut (impl Write + Send + ?Sized),
    ) -> eyre::Result<TestStatus> {
        let Invocation {
            binary,
            arguments,
            forwarded_env,
        } = invocation;
        binary.require_platform(self.platform())?;
        // iOS offers no bare-process spawn on a device; an unmarked binary
        // is a hard error, never skipped silently.
        let manifest = binary.manifest().ok_or_else(|| {
            eyre!(
                "{} carries no application manifest (__TEXT,__info_plist); the physical-device \
                 runner runs only application harnesses — select them with --test native_app",
                binary.path().display()
            )
        })?;
        if !self.device.supports_deployment_target(binary.min_os()) {
            bail!(
                "{} requires iOS {} but {} reports no OS new enough",
                binary.path().display(),
                binary.min_os(),
                self.device.name()
            );
        }

        // The lock wraps staging too: `HarnessBundle::stage` deletes and
        // recreates `<binary>.app`, which a concurrent invocation of the
        // same binary would clobber mid-sign or mid-install.
        let home = host
            .home_dir()
            .ok_or_else(|| eyre!("cannot locate the user's home directory for the device lock"))?;
        let _lock = DestinationLock::acquire(
            &home
                .join("Library/Caches/dev.waterui.test-runner")
                .join(format!("device-{}.lock", self.device.udid())),
        )
        .await?;

        let device_dir = binary
            .state_dir()?
            .join(format!("device-{}", self.device.udid()));
        std::fs::create_dir_all(&device_dir)?;
        let (bundle, selection) = self
            .stage_and_select_profile(host, binary, manifest)
            .await?;
        self.install_unless_stamped(host, binary, &bundle, &selection, &device_dir)
            .await?;

        let status = StatusFile::device_relative();
        let mut options = RunOptions::new();
        options.insert_env_var(STATUS_PATH_VARIABLE.to_string(), status.env_value());
        for (key, value) in forwarded_env.pairs() {
            options.insert_env_var(key.clone(), value.clone());
        }
        for argument in arguments.as_slice() {
            options.push_argument(argument.to_string_lossy().into_owned());
        }
        let artifact = Artifact::new(
            self.identity.bundle_id.clone(),
            bundle.layout().app_path().to_path_buf(),
        );
        let running =
            self.device
                .launch_monitored(host, &artifact, &options, manifest.executable())?;
        forward_run(running, interrupts, stdout, stderr).await?;

        // Pull the container's `tmp/` directory — it always exists on a
        // real device, so the copy itself only fails on a real transport
        // problem (a locked device or a dropped tunnel). A successful pull
        // that lacks this launch's uniquely-named file means the harness
        // died before writing its status — the distinction a copy of the
        // file alone cannot make.
        let pull_dir = device_dir.join("status-pull");
        let _ = std::fs::remove_dir_all(&pull_dir);
        self.device
            .copy_from_data_container(host, &self.identity.bundle_id, "tmp/", &pull_dir)
            .await
            .wrap_err("pulling the harness status directory from the device's data container")?;
        // devicectl may place the directory's contents at the destination
        // root or under `tmp/`; the launch's name is unique either way.
        let name = status.local_name();
        let mut content = fs::read(pull_dir.join(&name)).await.ok();
        if content.is_none() {
            content = fs::read(pull_dir.join("tmp").join(&name)).await.ok();
        }
        let _ = std::fs::remove_dir_all(&pull_dir);
        StatusFile::conclude(content.as_deref(), &self.identity.bundle_id)
    }
}

impl IosDevice {
    /// Stage the harness bundle, rewrite its identifier and select an
    /// already-installed development profile — never provisioning, so a
    /// `NoMatch` selection is an explicit error with renewal guidance.
    /// Signing itself happens only when the install is not skipped.
    async fn stage_and_select_profile(
        &self,
        host: &Host,
        binary: &TestBinary,
        manifest: &HarnessManifest,
    ) -> eyre::Result<(HarnessBundle, provisioning::ProfileSelection)> {
        let mut bundle = HarnessBundle::stage(binary, manifest, "iphoneos").await?;
        bundle.rewrite_identifier(&self.identity.bundle_id)?;

        let entitlements = plist::Dictionary::new();
        let request = SigningRequest::new(
            &self.identity.team_id,
            &self.identity.bundle_id,
            Some(self.device.udid()),
            &entitlements,
            TargetPlatform::IOS,
        );
        match provisioning::select_development_profile(host, &request).await {
            Ok(selection) => Ok((bundle, selection)),
            Err(SelectError::NoMatch(report)) => bail!(
                "no installed development provisioning profile covers {} for team {} on {} \
                 ({report}) — the test runner never provisions: renew the profile in Xcode \
                 (Settings → Accounts → Manage Certificates / Devices) and retry. Free Personal \
                 Team profiles expire after 7 days.",
                self.identity.bundle_id,
                self.identity.team_id,
                self.device.name()
            ),
            Err(error) => Err(error.into()),
        }
    }

    /// Sign and overwrite-install unless the host-side stamp says the
    /// device already runs this exact bundle. The stamp pairs the
    /// *pre-signing* input hash — development codesign stamps a signing
    /// time into every signature, so nothing post-signing is stable — with
    /// the installed app's URL, which a foreign overwrite install changes
    /// (the bundle lands under a new UUID path). Codesign is skipped too,
    /// since its output is identical by definition when the inputs match.
    async fn install_unless_stamped(
        &self,
        host: &Host,
        binary: &TestBinary,
        bundle: &HarnessBundle,
        selection: &provisioning::ProfileSelection,
        device_dir: &std::path::Path,
    ) -> eyre::Result<()> {
        let input_hash = Self::input_hash(binary, bundle, &self.identity, selection)?;
        let stamp_path = device_dir.join("stamp");
        let stamp = Self::read_stamp(&stamp_path);
        // The URL probe only runs when a stamp exists to check against;
        // without one the decision is already "install".
        let installed_url = if stamp.is_some() {
            self.device
                .installed_app_url(
                    host,
                    &self.identity.bundle_id,
                    &device_dir.join("apps.json"),
                )
                .await?
        } else {
            None
        };
        if install_decision(
            stamp
                .as_ref()
                .map(|(hash, url)| (hash.as_str(), url.as_str())),
            &input_hash,
            installed_url.as_deref(),
        ) == InstallDecision::Skip
        {
            return Ok(());
        }

        let entitlements = plist::Dictionary::new();
        let request = SigningRequest::new(
            &self.identity.team_id,
            &self.identity.bundle_id,
            Some(self.device.udid()),
            &entitlements,
            TargetPlatform::IOS,
        );
        crate::apple::app_bundle::apply_development_signature(
            host,
            bundle.layout(),
            &request,
            selection,
        )
        .await?;
        crate::apple::physical::install_device_app(
            host,
            self.device.selector(),
            bundle.layout().app_path(),
        )
        .await?;
        let url = self
            .device
            .installed_app_url(
                host,
                &self.identity.bundle_id,
                &device_dir.join("apps.json"),
            )
            .await?
            .ok_or_else(|| {
                eyre!(
                    "devicectl reports {} absent right after installing it on {}",
                    self.identity.bundle_id,
                    self.device.name()
                )
            })?;
        Self::write_stamp(&stamp_path, &input_hash, &url)
    }

    /// The install-skip key: the test binary's bytes, the rewritten
    /// `Info.plist`, the bundle identifier and team, and the selected
    /// profile's UUID — everything the signed bundle's bytes depend on, so
    /// a renewed profile or an identity change reinstalls even when the
    /// binary is unchanged.
    fn input_hash(
        binary: &TestBinary,
        bundle: &HarnessBundle,
        identity: &DeviceIdentity,
        selection: &provisioning::ProfileSelection,
    ) -> eyre::Result<String> {
        let profile_uuid = selection
            .data()
            .get("UUID")
            .and_then(plist::Value::as_string)
            .ok_or_else(|| {
                eyre!(
                    "the selected provisioning profile {} carries no UUID",
                    selection.path().display()
                )
            })?;
        let mut hasher = Sha256::new();
        let binary_bytes = std::fs::read(binary.path())
            .wrap_err_with(|| format!("reading {}", binary.path().display()))?;
        let plist_bytes = std::fs::read(bundle.layout().info_plist_path())
            .wrap_err("reading the staged Info.plist")?;
        for input in [
            binary_bytes.as_slice(),
            plist_bytes.as_slice(),
            identity.bundle_id.as_bytes(),
            identity.team_id.as_bytes(),
            profile_uuid.as_bytes(),
        ] {
            hasher.update(input.len().to_le_bytes());
            hasher.update(input);
        }
        Ok(hex::encode(hasher.finalize()))
    }

    /// The recorded `(input hash, installed URL)` stamp — `None` when no
    /// stamp exists or it does not parse; a corrupt stamp simply means
    /// "install".
    fn read_stamp(stamp_path: &std::path::Path) -> Option<(String, String)> {
        let content = std::fs::read_to_string(stamp_path).ok()?;
        let mut lines = content.lines();
        match (lines.next(), lines.next()) {
            (Some(hash), Some(url)) => Some((hash.to_string(), url.to_string())),
            _ => None,
        }
    }

    fn write_stamp(stamp_path: &std::path::Path, input_hash: &str, url: &str) -> eyre::Result<()> {
        std::fs::write(stamp_path, format!("{input_hash}\n{url}\n"))
            .wrap_err("writing the install stamp")
    }
}

/// Whether the install can be skipped: only when the stamp's input hash
/// matches the run's inputs *and* `devicectl` still reports the recorded
/// URL — a foreign overwrite install lands under a new UUID and
/// invalidates the stamp. Every other case (no stamp, app absent, either
/// mismatch) installs.
#[derive(Debug, PartialEq, Eq)]
enum InstallDecision {
    /// Codesign and install are both skipped.
    Skip,
    /// The full sign → overwrite-install → restamp path runs.
    Install,
}

fn install_decision(
    stamp: Option<(&str, &str)>,
    input_hash: &str,
    installed_url: Option<&str>,
) -> InstallDecision {
    match (stamp, installed_url) {
        (Some((hash, url)), Some(installed)) if hash == input_hash && url == installed => {
            InstallDecision::Skip
        }
        _ => InstallDecision::Install,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::test_runner::{ForwardedEnv, TestArguments};
    use crate::toolchain::testing::TestMachine;

    /// A device the fake `devicectl` reports.
    fn device_fixture() -> ApplePhysicalDevice {
        ApplePhysicalDevice::fixture()
    }

    #[test]
    fn an_unmarked_binary_is_refused_before_any_tool_runs() {
        let machine = TestMachine::new();
        let binary_path = crate::apple::test_runner::binary::tests::stage_binary(
            machine.root(),
            MachOPlatform::IOS.0,
            None,
        );
        let binary = TestBinary::read(&binary_path).expect("read fixture");
        let host = machine.host(std::iter::empty::<(&str, &str)>());
        let (_tx, interrupts) = smol::channel::unbounded();
        let arguments = TestArguments::new(vec![]);
        let forwarded = ForwardedEnv::new([]);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let device = IosDevice {
            device: device_fixture(),
            identity: DeviceIdentity {
                bundle_id: "dev.test.harness".to_string(),
                team_id: "TESTTEAM42".to_string(),
            },
        };
        let error = smol::block_on(device.run(
            &host,
            &Invocation {
                binary,
                arguments,
                forwarded_env: forwarded,
            },
            interrupts,
            &mut stdout,
            &mut stderr,
        ))
        .expect_err("an unmarked binary cannot run on a device");
        assert!(error.to_string().contains("no application manifest"));
    }

    #[test]
    fn the_install_skip_needs_the_matching_hash_and_url() {
        let stamp = Some(("h1", "file:///apps/AAA/harness.app/"));
        let url = Some("file:///apps/AAA/harness.app/");
        // Stamp matching hash + url: skip.
        assert_eq!(install_decision(stamp, "h1", url), InstallDecision::Skip);
        // No stamp: install.
        assert_eq!(install_decision(None, "h1", url), InstallDecision::Install);
        // Stamp but app absent: install.
        assert_eq!(
            install_decision(stamp, "h1", None),
            InstallDecision::Install
        );
        // Hash mismatch — a renewed profile or a new binary: install.
        assert_eq!(install_decision(stamp, "h2", url), InstallDecision::Install);
        // URL mismatch — a foreign overwrite moved the bundle: install.
        assert_eq!(
            install_decision(stamp, "h1", Some("file:///apps/BBB/harness.app/")),
            InstallDecision::Install
        );
    }

    #[test]
    fn copy_from_pulls_the_status_file_back() {
        let machine = TestMachine::new();
        machine.install("xcrun");
        let container = machine.dir("container");
        std::fs::create_dir_all(container.join("tmp")).expect("remote tmp");
        std::fs::write(container.join("tmp/status"), "0").expect("remote status");
        let host = machine.host([("WATERUI_FAKE_DEVICE_CONTAINER", container.as_os_str())]);
        let local = machine.root().join("pulled");
        smol::block_on(device_fixture().copy_from_data_container(
            &host,
            "dev.test.harness",
            "tmp/status",
            &local,
        ))
        .expect("the pull succeeds");
        assert_eq!(std::fs::read_to_string(&local).expect("local"), "0");

        // A remote path that does not exist is an error.
        let missing = machine.root().join("missing");
        assert!(
            smol::block_on(device_fixture().copy_from_data_container(
                &host,
                "dev.test.harness",
                "tmp/absent",
                &missing,
            ))
            .is_err()
        );
    }
}
