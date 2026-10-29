//! Classification of a test binary from its Mach-O.
//!
//! Every target runner invocation starts here: the binary's
//! `LC_BUILD_VERSION` names the platform it was built for and its minimum
//! OS, and a `__TEXT,__info_plist` section marks a harness that must run as
//! an application (`cocoa_ui::native_test_info_plist!`).

use std::io::Cursor;
use std::path::{Path, PathBuf};

use eyre::{Context as _, bail, eyre};
use object::read::macho::{LoadCommandVariant, MachOFile64};
use object::{Endianness, Object as _, ObjectSection as _};
use semver::Version;

/// The `LC_BUILD_VERSION` platform of a thin Mach-O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachOPlatform(pub(crate) u32);

impl MachOPlatform {
    /// `PLATFORM_IOS` — a physical-device build.
    pub const IOS: Self = Self(object::macho::PLATFORM_IOS.0);
    /// `PLATFORM_IOSSIMULATOR`.
    pub const IOSSIMULATOR: Self = Self(object::macho::PLATFORM_IOSSIMULATOR.0);
    /// `PLATFORM_MACCATALYST`.
    pub const MACCATALYST: Self = Self(object::macho::PLATFORM_MACCATALYST.0);

    const fn name(self) -> &'static str {
        match self {
            Self::IOS => "ios",
            Self::IOSSIMULATOR => "ios-simulator",
            Self::MACCATALYST => "mac-catalyst",
            _ => "unknown",
        }
    }
}

impl std::fmt::Display for MachOPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(known) = [Self::IOS, Self::IOSSIMULATOR, Self::MACCATALYST]
            .into_iter()
            .find(|known| *known == *self)
        {
            return f.write_str(known.name());
        }
        write!(f, "platform {}", self.0)
    }
}

/// The Info.plist a harness binary embeds: its exact bytes plus the two
/// keys the runner reads.
#[derive(Debug)]
pub struct HarnessManifest {
    bytes: Vec<u8>,
    bundle_id: String,
    executable: String,
}

impl HarnessManifest {
    /// The embedded bytes, verbatim — the staged `Info.plist` is written
    /// from these, never a hand-written replacement.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// `CFBundleIdentifier`.
    #[must_use]
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }
    /// `CFBundleExecutable`.
    #[must_use]
    pub fn executable(&self) -> &str {
        &self.executable
    }

    /// Parses the section's bytes; `Ok(None)` when the dictionary is not
    /// an application manifest — it declares no `CFBundleExecutable`, the
    /// key that makes a bundle launchable. Such a plist marks a spawned
    /// binary instead (the `native` harness's `LSUIElement` manifest on
    /// Catalyst), which every destination execs directly.
    fn parse(path: &Path, bytes: &[u8]) -> eyre::Result<Option<Self>> {
        let value = plist::Value::from_reader(Cursor::new(bytes)).wrap_err_with(|| {
            format!(
                "the __info_plist section of {} is not a plist",
                path.display()
            )
        })?;
        let plist::Value::Dictionary(dictionary) = value else {
            bail!(
                "the __info_plist section of {} is not a dictionary",
                path.display()
            );
        };
        let string = |key: &str| dictionary.get(key).and_then(plist::Value::as_string);
        let Some(executable) = string("CFBundleExecutable") else {
            return Ok(None);
        };
        Ok(Some(Self {
            bytes: bytes.to_vec(),
            bundle_id: string("CFBundleIdentifier")
                .map(str::to_string)
                .ok_or_else(|| {
                    eyre!(
                        "the __info_plist section of {} has no `CFBundleIdentifier` string",
                        path.display()
                    )
                })?,
            executable: executable.to_string(),
        }))
    }
}

/// One test binary cargo handed the runner, classified from its Mach-O.
#[derive(Debug)]
pub struct TestBinary {
    path: PathBuf,
    platform: MachOPlatform,
    min_os: Version,
    manifest: Option<HarnessManifest>,
}

impl TestBinary {
    /// Reads `LC_BUILD_VERSION` (platform, `minos`) and the optional
    /// `__TEXT,__info_plist` section with `object`. A fat or non-Mach-O
    /// file, or one without `LC_BUILD_VERSION`, is an error naming the
    /// path. The path is made absolute first: every location derived from
    /// it — the staged bundle, the status file — reaches processes that do
    /// not share the runner's working directory (a `LaunchServices` launch
    /// starts at `/`).
    ///
    /// # Errors
    /// Returns an error when `path` cannot be read or is not a thin Mach-O
    /// carrying `LC_BUILD_VERSION`, or when an embedded manifest is not a
    /// plist dictionary with `CFBundleIdentifier` and `CFBundleExecutable`.
    pub fn read(path: &Path) -> eyre::Result<Self> {
        let path = &std::fs::canonicalize(path)
            .wrap_err_with(|| format!("failed to resolve the test binary {}", path.display()))?;
        let data = std::fs::read(path)
            .wrap_err_with(|| format!("failed to read the test binary {}", path.display()))?;
        let macho = MachOFile64::<Endianness>::parse(&*data).map_err(|error| {
            eyre!(
                "{} is not a thin Mach-O a test runner can classify: {error}",
                path.display()
            )
        })?;
        let endian = macho.endian();
        let mut build_version = None;
        let mut commands = macho
            .macho_load_commands()
            .wrap_err_with(|| format!("failed to read load commands of {}", path.display()))?;
        while let Some(load_command) = commands
            .next()
            .wrap_err_with(|| format!("failed to read load commands of {}", path.display()))?
        {
            if let Ok(LoadCommandVariant::BuildVersion(version, _)) = load_command.variant() {
                let minos = version.minos.get(endian);
                build_version = Some((
                    MachOPlatform(version.platform.get(endian).0),
                    Version::new(
                        u64::from(minos.major()),
                        u64::from(minos.minor()),
                        u64::from(minos.update()),
                    ),
                ));
            }
        }
        let (platform, min_os) = build_version
            .ok_or_else(|| eyre!("{} has no LC_BUILD_VERSION load command", path.display()))?;

        let manifest = macho
            .sections()
            .find(|section| {
                section.name().is_ok_and(|name| name == "__info_plist")
                    && section
                        .segment_name()
                        .is_ok_and(|segment| segment == Some("__TEXT"))
            })
            .map(|section| section.data())
            .transpose()
            .wrap_err_with(|| {
                format!(
                    "failed to read the __info_plist section of {}",
                    path.display()
                )
            })?
            .map(|bytes| HarnessManifest::parse(path, bytes))
            .transpose()?
            .flatten();

        Ok(Self {
            path: path.clone(),
            platform,
            min_os,
            manifest,
        })
    }

    /// The binary's path as cargo handed it over.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The `LC_BUILD_VERSION` platform.
    #[must_use]
    pub const fn platform(&self) -> MachOPlatform {
        self.platform
    }

    /// The `LC_BUILD_VERSION` minimum OS — the deployment target the
    /// binary was built against.
    #[must_use]
    pub const fn min_os(&self) -> &Version {
        &self.min_os
    }

    /// The embedded harness manifest, when the binary carries the
    /// `__info_plist` section.
    #[must_use]
    pub const fn manifest(&self) -> Option<&HarnessManifest> {
        self.manifest.as_ref()
    }

    /// `<deps>/../waterui-test-runner`, created on first use.
    ///
    /// # Errors
    /// Returns an error when the binary has no parent directory or the
    /// state directory cannot be created.
    pub fn state_dir(&self) -> eyre::Result<PathBuf> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| eyre!("{} has no parent directory", self.path.display()))?
            .join("..")
            .join("waterui-test-runner");
        std::fs::create_dir_all(&dir).wrap_err_with(|| {
            format!(
                "failed to create the runner state directory {}",
                dir.display()
            )
        })?;
        Ok(dir)
    }

    /// The binary's platform must match `destination`.
    ///
    /// # Errors
    /// Returns an error naming both platforms when they differ.
    pub fn require_platform(&self, destination: MachOPlatform) -> eyre::Result<()> {
        if self.platform != destination {
            bail!(
                "{} was built for {} but this runner's destination is {destination}",
                self.path.display(),
                self.platform
            );
        }
        Ok(())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A thin arm64 Mach-O carrying `LC_BUILD_VERSION` (platform 17.2.3)
    /// and an optional `__TEXT,__info_plist` section — emitted at test
    /// time, never committed. (`object::write` is not an enabled feature;
    /// the file is small enough to lay out directly.)
    pub fn macho_fixture(platform: u32, plist: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        let push32 = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        let push64 = |out: &mut Vec<u8>, v: u64| out.extend_from_slice(&v.to_le_bytes());
        let name16 = |n: &[u8]| {
            let mut name = [0u8; 16];
            name[..n.len()].copy_from_slice(n);
            name
        };
        let with_plist = plist.is_some();
        let sizeofcmds = 24u32 + if with_plist { 152 } else { 0 };
        // mach_header_64
        push32(&mut out, 0xFEED_FACF); // MH_MAGIC_64
        push32(&mut out, 0x0100_000C); // CPU_TYPE_ARM64
        push32(&mut out, 0); // cpusubtype
        push32(&mut out, 2); // MH_EXECUTE
        push32(&mut out, if with_plist { 2 } else { 1 }); // ncmds
        push32(&mut out, sizeofcmds);
        push32(&mut out, 0); // flags
        push32(&mut out, 0); // reserved
        // LC_BUILD_VERSION: platform, minos = 17.2.3, sdk = 17.2.3, ntools = 0
        push32(&mut out, 0x32); // LC_BUILD_VERSION
        push32(&mut out, 24);
        push32(&mut out, platform);
        push32(&mut out, 17 << 16 | 2 << 8 | 3);
        push32(&mut out, 17 << 16 | 2 << 8 | 3);
        push32(&mut out, 0);
        if let Some(plist) = plist {
            let data_off = u64::from(32 + sizeofcmds);
            let data_size = u64::try_from(plist.len()).expect("size fits");
            // LC_SEGMENT_64 __TEXT with a single __info_plist section
            push32(&mut out, 0x19); // LC_SEGMENT_64
            push32(&mut out, 152);
            out.extend_from_slice(&name16(b"__TEXT"));
            push64(&mut out, 0); // vmaddr
            push64(&mut out, data_size); // vmsize
            push64(&mut out, data_off); // fileoff
            push64(&mut out, data_size); // filesize
            push32(&mut out, 7); // maxprot
            push32(&mut out, 5); // initprot
            push32(&mut out, 1); // nsects
            push32(&mut out, 0); // flags
            out.extend_from_slice(&name16(b"__info_plist"));
            out.extend_from_slice(&name16(b"__TEXT"));
            push64(&mut out, 0); // addr
            push64(&mut out, data_size); // size
            push32(&mut out, u32::try_from(data_off).expect("offset fits")); // offset
            push32(&mut out, 0); // align
            push32(&mut out, 0); // reloff
            push32(&mut out, 0); // nreloc
            push32(&mut out, 0); // flags
            push32(&mut out, 0); // reserved1
            push32(&mut out, 0); // reserved2
            push32(&mut out, 0); // reserved3
            out.extend_from_slice(plist);
        }
        out
    }

    /// A manifest the fixture embeds.
    pub fn manifest_plist() -> Vec<u8> {
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert(
            "CFBundleIdentifier".to_string(),
            plist::Value::String("dev.waterui.tests".to_string()),
        );
        dictionary.insert(
            "CFBundleExecutable".to_string(),
            plist::Value::String("harness".to_string()),
        );
        let mut bytes = Vec::new();
        plist::Value::Dictionary(dictionary)
            .to_writer_xml(&mut bytes)
            .expect("emit the fixture manifest");
        bytes
    }

    /// Writes `macho_fixture` into the machine's scratch root.
    pub fn stage_binary(dir: &Path, platform: u32, plist: Option<&[u8]>) -> PathBuf {
        let path = dir.join("deps").join("harness-abc123");
        std::fs::create_dir_all(path.parent().expect("deps parent")).expect("mkdir deps");
        std::fs::write(&path, macho_fixture(platform, plist)).expect("write fixture");
        path
    }

    #[test]
    fn an_unmarked_binary_has_no_manifest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let binary = TestBinary::read(&stage_binary(
            dir.path(),
            MachOPlatform::IOSSIMULATOR.0,
            None,
        ))
        .expect("read the fixture");
        assert_eq!(binary.platform(), MachOPlatform::IOSSIMULATOR);
        assert_eq!(binary.min_os(), &Version::new(17, 2, 3));
        assert!(binary.manifest().is_none());
    }

    #[test]
    fn a_marked_binary_carries_its_manifest_verbatim() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plist = manifest_plist();
        let binary = TestBinary::read(&stage_binary(
            dir.path(),
            MachOPlatform::IOSSIMULATOR.0,
            Some(&plist),
        ))
        .expect("read the fixture");
        let manifest = binary.manifest().expect("the fixture is marked");
        assert_eq!(manifest.bytes(), plist.as_slice());
        assert_eq!(manifest.bundle_id(), "dev.waterui.tests");
        assert_eq!(manifest.executable(), "harness");
    }

    #[test]
    fn platform_mismatch_names_both() {
        let dir = tempfile::tempdir().expect("tempdir");
        let binary = TestBinary::read(&stage_binary(dir.path(), MachOPlatform::IOS.0, None))
            .expect("read the fixture");
        let error = binary
            .require_platform(MachOPlatform::IOSSIMULATOR)
            .unwrap_err();
        assert!(error.to_string().contains("ios"));
        assert!(error.to_string().contains("simulator"));
    }

    #[test]
    fn non_macho_is_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("not-a-binary");
        std::fs::write(&path, b"definitely not mach-o").expect("write");
        assert!(TestBinary::read(&path).is_err());
    }
}
