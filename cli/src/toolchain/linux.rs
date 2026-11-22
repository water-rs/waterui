//! Linux system package toolchain checks.

use std::collections::BTreeSet;

use crate::{
    toolchain::{Host, Installation, Toolchain, ToolchainError, UnfixableToolchain},
    utils::CommandError,
};
use futures_util::StreamExt as _;

/// Linux system dependencies required by `waterui` desktop/media builds.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinuxSystemToolchain;

/// Installation plan for missing Linux system packages.
#[derive(Debug, Clone)]
pub struct LinuxSystemPackagesInstallation {
    manager: LinuxPackageManager,
    missing_packages: Vec<String>,
}

impl LinuxSystemPackagesInstallation {
    const fn new(manager: LinuxPackageManager, missing_packages: Vec<String>) -> Self {
        Self {
            manager,
            missing_packages,
        }
    }

    /// Returns the detected package manager name.
    #[must_use]
    pub const fn package_manager_name(&self) -> &'static str {
        self.manager.name()
    }

    /// Returns the missing packages for this installation plan.
    #[must_use]
    pub fn missing_packages(&self) -> &[String] {
        &self.missing_packages
    }

    /// Returns a command hint for manual installation.
    #[must_use]
    pub fn install_command_hint(&self) -> String {
        self.manager.install_hint(&self.missing_packages)
    }

    /// Build an installation plan for explicit package names using the detected manager.
    ///
    /// This reuses the existing Linux package-manager framework and is useful for
    /// toolchains that discover missing capabilities via probes (for example,
    /// pkg-config modules).
    ///
    /// # Errors
    /// Returns an error when no supported package manager is available.
    pub async fn from_packages(
        host: &Host,
        packages: Vec<String>,
    ) -> Result<Self, UnfixableToolchain> {
        let Some(manager) = LinuxPackageManager::detect(host).await else {
            return Err(UnfixableToolchain::new(
                "Unable to detect Linux package manager",
                unsupported_manager_hint(),
            ));
        };
        if packages.is_empty() {
            return Err(UnfixableToolchain::new(
                "No packages were provided for automatic installation",
                "Re-run `water doctor` and inspect diagnostics.",
            ));
        }
        Ok(Self::new(manager, packages))
    }
}

/// Errors that can occur during Linux package installation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FailToInstallLinuxSystemPackages {
    /// Non-Linux platforms are not supported by this installer.
    #[error("Automatic Linux package installation is only supported on Linux hosts.")]
    UnsupportedPlatform,
    /// No supported package manager was detected.
    #[error("No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk).")]
    UnsupportedPackageManager,
    /// A package-manager command failed.
    #[error("Failed to install Linux system packages: {0}")]
    CommandFailed(#[from] CommandError),
}

/// Errors from Linux package-manager operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LinuxPackageManagerError {
    /// No supported package manager was detected.
    #[error("No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk).")]
    UnsupportedPackageManager,
    /// A package-manager command failed.
    #[error(transparent)]
    Command(#[from] CommandError),
}

/// A dotted-numeric version string could not be parsed.
#[derive(Debug, thiserror::Error)]
#[error("`{version}` is not a dotted-numeric version: {source}")]
struct DottedVersionError {
    version: String,
    #[source]
    source: std::num::ParseIntError,
}

/// Probing a versioned native library with `pkg-config` failed.
#[derive(Debug, thiserror::Error)]
enum NativeProbeError {
    /// The `pkg-config` invocation failed.
    #[error(transparent)]
    Command(#[from] CommandError),
    /// `pkg-config --modversion` succeeded but printed nothing.
    #[error("`pkg-config --modversion {module}` printed nothing")]
    EmptyModVersion {
        /// The probed pkg-config module.
        module: &'static str,
    },
    /// The reported version is not dotted-numeric.
    #[error(transparent)]
    Version(#[from] DottedVersionError),
}

impl Installation for LinuxSystemPackagesInstallation {
    type Error = FailToInstallLinuxSystemPackages;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if !cfg!(target_os = "linux") {
            return Err(FailToInstallLinuxSystemPackages::UnsupportedPlatform);
        }

        let Some(manager) = LinuxPackageManager::detect(host).await else {
            return Err(FailToInstallLinuxSystemPackages::UnsupportedPackageManager);
        };

        install_missing_packages(host, manager, &self.missing_packages).await?;
        Ok(())
    }
}

impl Toolchain for LinuxSystemToolchain {
    type Installation = LinuxSystemPackagesInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if !cfg!(target_os = "linux") {
            return Ok(());
        }

        let Some(manager) = LinuxPackageManager::detect(host).await else {
            return Err(ToolchainError::unfixable(
                "Unable to detect Linux package manager",
                unsupported_manager_hint(),
            ));
        };

        // Which optional native bindings the project's resolved dependency
        // graph selects. Their development packages are required only when
        // the graph pulls the binding crate in — a plain project does not
        // select `cros-libva`, so it must not be told to install `libva`.
        let resolved_dependencies = resolved_dependency_names(host).await?;
        let mut required_packages = manager.required_packages().to_vec();
        for library in VERSIONED_NATIVE_LIBRARIES
            .iter()
            .filter(|library| resolved_dependencies.all.contains(library.required_by))
        {
            if let Some(package) = manager.package_for_native_library(library.module)
                && !required_packages.contains(&package)
            {
                required_packages.push(package);
            }
        }
        // A selected WPE runtime builds its bridge from source with CMake,
        // which every supported distribution packages under that name.
        let wpe_selected = resolved_dependencies.normal.contains("waterui-browser-wpe");
        if wpe_selected && !required_packages.contains(&"cmake") {
            required_packages.push("cmake");
        }
        // Each package's installed check is independent — probe them
        // concurrently with a stable bound.
        let probe_results = probe_installed_packages(host, manager, &required_packages).await;
        let mut missing_packages = Vec::new();
        for (&package, installed) in required_packages.iter().zip(probe_results) {
            let installed = installed.map_err(|error| {
                ToolchainError::unfixable(
                    format!("Failed checking Linux package `{package}`: {error}"),
                    manager.install_hint(&required_packages_to_owned(&required_packages)),
                )
            })?;
            if !installed {
                missing_packages.push(package.to_string());
            }
        }

        if !missing_packages.is_empty() {
            return Err(ToolchainError::fixable(
                LinuxSystemPackagesInstallation::new(manager, missing_packages),
            ));
        }

        check_native_libraries(host, manager, &resolved_dependencies.all, wpe_selected).await
    }
}

/// The project's resolved dependency names, split by edge kind: `all` is
/// every node cargo resolved, `normal` is only what normal dependency edges
/// reach from the root — dev and build dependencies excluded.
struct ResolvedLinuxDependencies {
    all: BTreeSet<String>,
    normal: BTreeSet<String>,
}

/// Probes every package's installed state concurrently, results in input
/// order. A named function, not an inline closure: the closure's inferred
/// higher-ranked signature does not survive the `Send` check of
/// `Toolchain::check`'s future.
async fn probe_installed_packages(
    host: &Host,
    manager: LinuxPackageManager,
    packages: &[&'static str],
) -> Vec<Result<bool, CommandError>> {
    let mut probes = Vec::with_capacity(packages.len());
    for &package in packages {
        probes.push(manager.check_installed(host, package));
    }
    futures_util::stream::iter(probes)
        .buffered(8)
        .collect::<Vec<_>>()
        .await
}

/// The package names in the dependency graph the checked project resolves
/// for this Linux host: what `cargo metadata --filter-platform <host>`
/// reports in its resolve, run through the host's `cargo`.
///
/// The project is the `Water.toml` at the host's working directory, the same
/// test the doctor's project context applies. A host outside a project has
/// no graph and resolves to an empty set, which selects no optional binding.
/// A project whose graph cargo cannot resolve is an error, never an empty
/// set: an empty graph would silently drop the native libraries it needs.
async fn resolved_dependency_names(
    host: &Host,
) -> Result<ResolvedLinuxDependencies, ToolchainError<LinuxSystemPackagesInstallation>> {
    if !smol::fs::metadata(host.cwd().join("Water.toml"))
        .await
        .is_ok_and(|metadata| metadata.is_file())
    {
        return Ok(ResolvedLinuxDependencies {
            all: BTreeSet::new(),
            normal: BTreeSet::new(),
        });
    }
    let mut command = cargo_metadata::MetadataCommand::new();
    command
        .current_dir(host.cwd())
        .manifest_path(host.cwd().join("Cargo.toml"))
        .other_options(vec![
            String::from("--filter-platform"),
            target_lexicon::Triple::host().to_string(),
        ]);
    let metadata = host.cargo_metadata(&command).await.map_err(|error| {
        ToolchainError::unfixable(
            format!("Failed resolving the project's dependency graph: {error}"),
            "Fix the project's manifest so `cargo metadata` succeeds, then re-run `water doctor`.",
        )
    })?;
    // Only `--no-deps` omits the resolve, and this command never passes it.
    let resolve = metadata
        .resolve
        .as_ref()
        .expect("cargo metadata without --no-deps reports a resolve");
    let resolved: BTreeSet<_> = resolve.nodes.iter().map(|node| &node.id).collect();
    let all: BTreeSet<String> = metadata
        .packages
        .iter()
        .filter(|package| resolved.contains(&package.id))
        .map(|package| package.name.to_string())
        .collect();
    let root = resolve.root.as_ref().ok_or_else(|| {
        ToolchainError::unfixable(
            "cargo metadata reported no root package for the project's resolve graph",
            "Fix the project's manifest so `cargo metadata` reports a root, then re-run `water doctor`.",
        )
    })?;
    let normal = crate::project::normal_dependency_packages(&metadata, &root.to_string())
        .map_err(|error| {
            ToolchainError::unfixable(
                format!("Failed walking the project's normal dependencies: {error}"),
                "Fix the project's manifest so the graph resolves, then re-run `water doctor`.",
            )
        })?
        .into_iter()
        .map(|package| package.name.to_string())
        .collect();
    Ok(ResolvedLinuxDependencies { all, normal })
}

/// The C toolchain every native Linux build invokes.
///
/// rustc shells out to `cc` to drive the linker, so a host carrying
/// development headers without a compiler driver — Ubuntu's package set
/// historically installed `libclang-dev` yet no `cc`, `gcc`, `clang`, or
/// `ld` — dies at first link with `error: linker \'cc\' not found`. Both
/// probes must pass; the repair installs the distribution's compiler
/// bundle, which always provides `cc` and `ld` on the supported managers.
#[derive(Debug, Clone, Copy, Default)]
pub struct CToolchain;

impl Toolchain for CToolchain {
    type Installation = LinuxSystemPackagesInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if !cfg!(target_os = "linux") {
            return Ok(());
        }

        if host.which("cc").await.is_ok() && host.which("ld").await.is_ok() {
            return Ok(());
        }

        let Some(manager) = LinuxPackageManager::detect(host).await else {
            return Err(ToolchainError::unfixable(
                "a C compiler (`cc`) and linker (`ld`) are missing and no supported package manager was found",
                "Install a C toolchain with your distribution's package manager (`build-essential` on Debian/Ubuntu, `gcc` and `binutils` elsewhere) and ensure `cc` and `ld` are on PATH.",
            ));
        };

        Err(ToolchainError::fixable(
            LinuxSystemPackagesInstallation::new(
                manager,
                manager
                    .c_toolchain_packages()
                    .iter()
                    .copied()
                    .map(str::to_owned)
                    .collect(),
            ),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxPackageManager {
    Apt,
    Dnf,
    Pacman,
    Zypper,
    Apk,
}

impl LinuxPackageManager {
    async fn detect(host: &Host) -> Option<Self> {
        if host.which("apt-get").await.is_ok() {
            Some(Self::Apt)
        } else if host.which("dnf").await.is_ok() {
            Some(Self::Dnf)
        } else if host.which("pacman").await.is_ok() {
            Some(Self::Pacman)
        } else if host.which("zypper").await.is_ok() {
            Some(Self::Zypper)
        } else if host.which("apk").await.is_ok() {
            Some(Self::Apk)
        } else {
            None
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Apt => "apt-get",
            Self::Dnf => "dnf",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Apk => "apk",
        }
    }

    /// Packages providing the C compiler driver (`cc`) and linker (`ld`)
    /// a native build invokes — probed separately by [`CToolchain`].
    const fn c_toolchain_packages(self) -> &'static [&'static str] {
        match self {
            Self::Apt => &["build-essential"],
            Self::Dnf | Self::Zypper => &["gcc", "binutils", "glibc-devel"],
            Self::Pacman => &["gcc", "binutils"],
            Self::Apk => &["build-base"],
        }
    }

    const fn required_packages(self) -> &'static [&'static str] {
        match self {
            Self::Apt => &[
                "build-essential",
                "pkg-config",
                "libgtk-4-dev",
                "libpango1.0-dev",
                "libwayland-dev",
                "wayland-protocols",
                "libasound2-dev",
                "libgbm-dev",
                "libxcb1-dev",
                "libclang-dev",
                "libfontconfig-dev",
                "ca-certificates",
            ],
            Self::Dnf => &[
                "gcc",
                "binutils",
                "glibc-devel",
                "pkgconf-pkg-config",
                "gtk4-devel",
                "pango-devel",
                "wayland-devel",
                "wayland-protocols-devel",
                "alsa-lib-devel",
                "mesa-libgbm-devel",
                "libxcb-devel",
                "clang-devel",
                "fontconfig-devel",
                "ca-certificates",
            ],
            Self::Pacman => &[
                "gcc",
                "binutils",
                "pkgconf",
                "gtk4",
                "pango",
                "wayland",
                "wayland-protocols",
                "alsa-lib",
                "mesa",
                "libxcb",
                "clang",
                "fontconfig",
                "ca-certificates",
            ],
            Self::Zypper => &[
                "gcc",
                "binutils",
                "glibc-devel",
                // Tumbleweed carries both `pkg-config` and
                // `pkgconf-pkg-config`; the latter is the current pkgconf build
                // that owns /usr/bin/pkg-config, and `rpm -q` matches a package
                // name rather than a provided capability.
                "pkgconf-pkg-config",
                "gtk4-devel",
                "pango-devel",
                "wayland-devel",
                "wayland-protocols-devel",
                "alsa-devel",
                // Tumbleweed renamed Mesa's split packages: gbm dropped the
                // `Mesa-` prefix while EGL kept it (`Mesa-libEGL-devel`).
                "libgbm-devel",
                "libxcb-devel",
                "clang-devel",
                "fontconfig-devel",
                "ca-certificates",
            ],
            Self::Apk => &[
                "build-base",
                "pkgconf",
                "gtk4.0-dev",
                "pango-dev",
                "wayland-dev",
                "wayland-protocols",
                "alsa-lib-dev",
                "mesa-dev",
                "libxcb-dev",
                "clang-dev",
                "fontconfig-dev",
                "ca-certificates",
            ],
        }
    }

    fn install_hint(self, packages: &[String]) -> String {
        let package_list = packages.join(" ");
        match self {
            Self::Apt => format!("sudo apt-get install -y {package_list}"),
            Self::Dnf => format!("sudo dnf install -y {package_list}"),
            Self::Pacman => format!("sudo pacman -S --noconfirm --needed {package_list}"),
            Self::Zypper => {
                format!(
                    "sudo zypper --non-interactive install --auto-agree-with-licenses {package_list}"
                )
            }
            Self::Apk => format!("sudo apk add {package_list}"),
        }
    }

    async fn check_installed(self, host: &Host, package: &str) -> Result<bool, CommandError> {
        let output = match self {
            Self::Apt => host.output("dpkg-query", ["-W", package]).await?,
            Self::Dnf | Self::Zypper => host.output("rpm", ["-q", package]).await?,
            Self::Pacman => host.output("pacman", ["-Q", package]).await?,
            Self::Apk => host.output("apk", ["info", "-e", package]).await?,
        };
        Ok(output.status.success())
    }

    fn package_for_gtk_pkg_config_probe(self, probe: &str) -> Option<&'static str> {
        if probe.starts_with("gtk4") {
            return Some(match self {
                Self::Apt => "libgtk-4-dev",
                Self::Dnf | Self::Zypper => "gtk4-devel",
                Self::Pacman => "gtk4",
                Self::Apk => "gtk4.0-dev",
            });
        }
        if probe.starts_with("pango") {
            return Some(match self {
                Self::Apt => "libpango1.0-dev",
                Self::Dnf | Self::Zypper => "pango-devel",
                Self::Pacman => "pango",
                Self::Apk => "pango-dev",
            });
        }
        None
    }

    /// Package that provides the development files for a probed pkg-config
    /// module.
    const fn package_for_native_library(self, module: &str) -> Option<&'static str> {
        match module.as_bytes() {
            b"libva" => Some(match self {
                Self::Apt | Self::Apk => "libva-dev",
                Self::Dnf | Self::Zypper => "libva-devel",
                Self::Pacman => "libva",
            }),
            b"libpipewire-0.3" => Some(match self {
                Self::Apt => "libpipewire-0.3-dev",
                Self::Dnf | Self::Zypper => "pipewire-devel",
                Self::Pacman => "libpipewire",
                Self::Apk => "pipewire-dev",
            }),
            b"egl" => Some(match self {
                Self::Apt => "libegl-dev",
                Self::Dnf => "mesa-libEGL-devel",
                Self::Zypper => "Mesa-libEGL-devel",
                Self::Pacman => "mesa",
                Self::Apk => "mesa-dev",
            }),
            b"glib-2.0" => Some(match self {
                Self::Apt => "libglib2.0-dev",
                Self::Dnf | Self::Zypper => "glib2-devel",
                Self::Pacman => "glib2",
                Self::Apk => "glib-dev",
            }),
            b"libsoup-3.0" => Some(match self {
                Self::Apt => "libsoup-3.0-dev",
                Self::Dnf => "libsoup3-devel",
                Self::Zypper => "libsoup-devel",
                Self::Pacman => "libsoup3",
                Self::Apk => "libsoup3-dev",
            }),
            b"json-glib-1.0" => Some(match self {
                Self::Apt => "libjson-glib-dev",
                Self::Dnf | Self::Zypper => "json-glib-devel",
                Self::Pacman => "json-glib",
                Self::Apk => "json-glib-dev",
            }),
            b"libdrm" => Some(match self {
                Self::Apt | Self::Apk => "libdrm-dev",
                Self::Dnf | Self::Zypper => "libdrm-devel",
                Self::Pacman => "libdrm",
            }),
            _ => None,
        }
    }
}

/// A native library whose Rust binding only compiles against headers newer than
/// the ones some distributions ship.
///
/// Presence is not enough for these: `dpkg-query -W libva-dev` succeeds on
/// Ubuntu 22.04 while `cros-libva` still fails to compile, so the doctor asks
/// `pkg-config` for the version, which is the same mechanism the bindings' own
/// build scripts use to find the headers.
#[derive(Debug, Clone, Copy)]
struct VersionedNativeLibrary {
    /// pkg-config module whose `Version:` field carries the axis compared below.
    module: &'static str,
    /// Human name of that axis, used in diagnostics.
    version_axis: &'static str,
    /// Lowest version of `version_axis` the Rust binding compiles against.
    minimum_version: &'static str,
    /// Crate that imposes the floor. The requirement applies only when this
    /// crate is in the checked project's resolved dependency graph — an
    /// optional binding a project never selects cannot fail that project's
    /// build, so the doctor must not demand the native library for it
    /// (a plain project does not select `cros-libva` and must not be told
    /// to install `libva`).
    required_by: &'static str,
    /// Version of that crate the floor was read from; `crate_versions_match_lockfile`
    /// keeps it honest against the workspace lockfile.
    required_by_version: &'static str,
    /// Where the floor is written down inside that crate, quoted in diagnostics.
    requirement_source: &'static str,
    /// pkg-config variable carrying the upstream release number, for modules
    /// whose `Version:` field is some other number.
    release_version_variable: Option<&'static str>,
    /// What a user whose distribution ships an older build can actually do.
    distribution_hint: &'static str,
}

/// Native libraries checked by version rather than by presence.
///
/// Every floor here is read out of the crate that imposes it, never guessed:
///
/// * **`libva` / VA-API 1.19** — `cros-libva`'s `build.rs` parses
///   `VA_MAJOR_VERSION` / `VA_MINOR_VERSION` out of `va/va_version.h` and emits
///   `cargo::rustc-cfg=libva_1_19_or_higher` when the VA-API version is at least
///   1.19 (`build.rs:96-110`). `src/buffer/av1.rs` gates individual struct
///   fields on that cfg (`av1.rs:626`, `av1.rs:1085`) but not the surrounding
///   initialisers, so below VA-API 1.19 the crate does not compile at all — the
///   `E0061` / `E0560` errors in <https://github.com/water-rs/waterui/issues/376>.
///   `libva.pc` sets `Version:` to `va_api_version`, not to the libva release
///   number (`pkgconfig/meson.build`, `pkg.generate(libva, …, version:
///   va_api_version)`), so `pkg-config --modversion libva` reports exactly the
///   number the cfg gate tests. The release number is exported next to it as
///   the `libva_version` pkg-config variable and is only used for diagnostics.
///
/// * **`libpipewire-0.3` / `PipeWire` 0.3.65** — `libspa-sys` declares only
///   `version = "0.3"` for `libpipewire-0.3` in `[package.metadata.system-deps]`,
///   which does not describe the headers it needs, so the floor comes from the
///   APIs `libspa` uses with no feature gate: `spa_meta_first` and
///   `spa_meta_region_is_valid` (`libspa/src/buffer/meta.rs:107,157`) only reach
///   Rust through `libspa-sys`'s `wrap_static_fns` bindgen pass once upstream
///   turned them from macros into `static inline` functions, in `PipeWire` 0.3.59;
///   `spa_video_info_raw::flags` and its `uint64_t modifier`
///   (`libspa/src/param/video/raw.rs:258,266`) were added to
///   `spa/include/spa/param/video/raw.h` in `PipeWire` 0.3.65 — 0.3.64 still has
///   `int64_t modifier` and no `flags`. The later of the two wins.
///   `libspa-0.2.pc` cannot carry this: `PipeWire` builds it with
///   `version : spaversion` where `spaversion = '0.2'` is a constant
///   (`meson.build:23`, `spa/meson.build:26`), so it reports `0.2` on every
///   release. `libpipewire-0.3.pc` is built with `version : pipewire_version`
///   (`src/pipewire/meson.build:127`) and is the module that carries the
///   release number.
const VERSIONED_NATIVE_LIBRARIES: &[VersionedNativeLibrary] = &[
    VersionedNativeLibrary {
        module: "libva",
        version_axis: "VA-API",
        minimum_version: "1.19",
        required_by: "cros-libva",
        required_by_version: "0.0.12",
        requirement_source: "its build.rs only emits `libva_1_19_or_higher`, which src/buffer/av1.rs requires, from VA-API 1.19 up",
        release_version_variable: Some("libva_version"),
        distribution_hint: "VA-API 1.19 first ships in libva 2.19. Ubuntu 24.04 (libva 2.20) and Debian 13 trixie (libva 2.22) are new enough; Ubuntu 22.04 (libva 2.14) and Debian 12 bookworm (libva 2.17) are not, and neither has a backport in its updates or backports pocket. Upgrade the distribution, or build libva 2.19 or newer from https://github.com/intel/libva and put its prefix on PKG_CONFIG_PATH.",
    },
    VersionedNativeLibrary {
        module: "libpipewire-0.3",
        version_axis: "PipeWire",
        minimum_version: "0.3.65",
        required_by: "libspa",
        required_by_version: "0.10.1",
        requirement_source: "it uses spa_video_info_raw::flags, added in PipeWire 0.3.65, and spa_meta_first, a static inline function only since 0.3.59, without a feature gate",
        release_version_variable: None,
        distribution_hint: "Ubuntu 24.04 (PipeWire 1.0.5) and Debian 12 bookworm (PipeWire 0.3.65) are new enough; Ubuntu 22.04 (PipeWire 0.3.48) is not, and has no backport in its updates or backports pocket. Upgrade the distribution, or build PipeWire 0.3.65 or newer from https://gitlab.freedesktop.org/pipewire/pipewire and put its prefix on PKG_CONFIG_PATH.",
    },
];

/// pkg-config modules a desktop build needs present, with no version floor.
///
/// `khronos-egl`'s build script locates EGL through `pkg-config --exists egl`
/// and fails with `Package 'egl' not found` when no `.pc` file is installed —
/// the failure in <https://github.com/water-rs/cli/issues/155> on Fedora,
/// where nothing else pulls `mesa-libEGL-devel` in. Presence is the whole
/// check: any provider of `egl.pc` satisfies it, whether it came from the
/// distribution's Mesa package or a source build on `PKG_CONFIG_PATH`.
const REQUIRED_NATIVE_MODULES: &[&str] = &["egl"];

/// The WPE bridge's `pkg-config` build prerequisites — the modules
/// `browser-wpe/native/CMakeLists.txt` asks for. They are probed only when
/// the project's normal dependency graph selects `waterui-browser-wpe`.
const WPE_NATIVE_MODULES: &[&str] = &["glib-2.0", "libsoup-3.0", "json-glib-1.0", "libdrm"];

/// Outcome of probing one [`VersionedNativeLibrary`] with `pkg-config`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeLibraryStatus {
    /// The module is present and new enough.
    Satisfied,
    /// `pkg-config` does not know the module at all.
    ModuleMissing,
    /// The module is present but older than the Rust binding accepts.
    TooOld {
        /// `pkg-config --modversion` output.
        installed: String,
        /// Upstream release number, when the module exports one separately.
        release: Option<String>,
    },
}

impl VersionedNativeLibrary {
    /// Message shown when the library is installed but too old.
    fn outdated_message(self, installed: &str, release: Option<&str>) -> String {
        let Self {
            module,
            version_axis,
            minimum_version,
            required_by,
            required_by_version,
            ..
        } = self;
        let release = release.map_or_else(String::new, |release| format!(" (release {release})"));
        format!(
            "`{module}` is too old: pkg-config reports {version_axis} {installed}{release}, but `{required_by} {required_by_version}` needs {version_axis} {minimum_version} or newer"
        )
    }

    /// Suggestion shown when the library is installed but too old.
    ///
    /// `package` is the distribution package that already provides the module,
    /// so the text can say plainly that installing it again changes nothing.
    fn outdated_suggestion(self, package: Option<&str>) -> String {
        let Self {
            module,
            minimum_version,
            required_by,
            requirement_source,
            distribution_hint,
            ..
        } = self;
        let already_installed = package.map_or_else(
            || format!("The package providing `{module}` is already installed"),
            |package| format!("`{package}` is already installed"),
        );
        format!(
            "{already_installed}, so installing it again will not help. `{required_by}` needs the newer headers because {requirement_source}. {distribution_hint} `pkg-config --modversion {module}` has to report {minimum_version} or newer."
        )
    }
}

/// Check the probed native libraries: the version-floored ones and the
/// presence-only ones.
///
/// Every missing module collects into one fixable installation, and a missing
/// module outranks an outdated library: an unfixable `TooOld` used to return
/// immediately, which hid every fixable gap behind it — Debian 12's libva is
/// below the `cros-libva` floor, so its missing `libpipewire-0.3-dev` never
/// surfaced (<https://github.com/water-rs/cli/issues/155>). The outdated
/// diagnostic is still reported once nothing installable remains.
async fn check_native_libraries(
    host: &Host,
    manager: LinuxPackageManager,
    resolved_dependencies: &BTreeSet<String>,
    wpe_selected: bool,
) -> Result<(), ToolchainError<LinuxSystemPackagesInstallation>> {
    if !pkg_config_available(host).await {
        return Err(ToolchainError::unfixable(
            "pkg-config not found",
            "Install pkg-config and ensure it is in PATH, then re-run `water doctor`.",
        ));
    }

    let mut missing_packages = Vec::new();
    let mut outdated = None;
    for &library in VERSIONED_NATIVE_LIBRARIES
        .iter()
        .filter(|library| resolved_dependencies.contains(library.required_by))
    {
        let package = manager.package_for_native_library(library.module);
        let status = probe_native_library(host, library).await.map_err(|error| {
            ToolchainError::unfixable(
                format!(
                    "Failed checking `{}` with pkg-config: {error}",
                    library.module
                ),
                format!(
                    "Ensure `pkg-config --modversion {}` works, then re-run `water doctor`.",
                    library.module
                ),
            )
        })?;

        match status {
            NativeLibraryStatus::Satisfied => {}
            NativeLibraryStatus::ModuleMissing => {
                let package = package.ok_or_else(|| {
                    ToolchainError::unfixable(
                        format!("`{}` is not known to pkg-config", library.module),
                        format!(
                            "Install the development package providing `{}` for this distribution, then re-run `water doctor`.",
                            library.module
                        ),
                    )
                })?;
                if !missing_packages.iter().any(|existing| existing == package) {
                    missing_packages.push(package.to_owned());
                }
            }
            NativeLibraryStatus::TooOld { installed, release } => {
                if outdated.is_none() {
                    outdated = Some((library, installed, release));
                }
            }
        }
    }

    // WPE's bridge compiles against these modules only when the project's
    // normal dependency graph selects `waterui-browser-wpe` — dev and build
    // dependencies do not make a runtime a build prerequisite.
    let mut presence_modules: Vec<&str> = REQUIRED_NATIVE_MODULES.to_vec();
    if wpe_selected {
        presence_modules.extend(WPE_NATIVE_MODULES);
    }
    // Each module's presence probe is independent — collect them
    // concurrently with a stable bound before deduplicating the packages.
    // Plain pushes rather than a `map` closure: the closure's inferred
    // higher-ranked signature does not survive `Toolchain::check`'s `Send`
    // check.
    let mut probes = Vec::with_capacity(presence_modules.len());
    for &module in &presence_modules {
        probes.push(pkg_config_module_present(host, module));
    }
    let presence_results = futures_util::stream::iter(probes)
        .buffered(8)
        .collect::<Vec<_>>()
        .await;
    for (&module, present) in presence_modules.iter().zip(presence_results) {
        let present = present.map_err(|error| {
            ToolchainError::unfixable(
                format!("Failed checking `{module}` with pkg-config: {error}"),
                format!("Ensure `pkg-config --exists {module}` works, then re-run `water doctor`."),
            )
        })?;
        if present {
            continue;
        }
        let package = manager.package_for_native_library(module).ok_or_else(|| {
            ToolchainError::unfixable(
                format!("`{module}` is not known to pkg-config"),
                format!("Install the development package providing `{module}` for this distribution, then re-run `water doctor`."),
            )
        })?;
        if !missing_packages.iter().any(|existing| existing == package) {
            missing_packages.push(package.to_owned());
        }
    }

    if !missing_packages.is_empty() {
        return Err(ToolchainError::fixable(
            LinuxSystemPackagesInstallation::new(manager, missing_packages),
        ));
    }

    if let Some((library, installed, release)) = outdated {
        let package = manager.package_for_native_library(library.module);
        return Err(ToolchainError::unfixable(
            library.outdated_message(&installed, release.as_deref()),
            library.outdated_suggestion(package),
        ));
    }

    Ok(())
}

/// Returns `true` when `pkg-config` can be executed.
async fn pkg_config_available(host: &Host) -> bool {
    host.output("pkg-config", ["--version"])
        .await
        .is_ok_and(|output| output.status.success())
}

/// Returns `true` when `pkg-config` knows the module at all.
async fn pkg_config_module_present(host: &Host, module: &str) -> Result<bool, CommandError> {
    Ok(host
        .output("pkg-config", ["--exists", module])
        .await?
        .status
        .success())
}

/// Ask `pkg-config` for a module's version and compare it against the floor.
async fn probe_native_library(
    host: &Host,
    library: VersionedNativeLibrary,
) -> Result<NativeLibraryStatus, NativeProbeError> {
    let output = host
        .output("pkg-config", ["--modversion", library.module])
        .await?;
    if !output.status.success() {
        return Ok(NativeLibraryStatus::ModuleMissing);
    }
    let installed = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if installed.is_empty() {
        return Err(NativeProbeError::EmptyModVersion {
            module: library.module,
        });
    }

    if version_at_least(&installed, library.minimum_version)? {
        return Ok(NativeLibraryStatus::Satisfied);
    }

    let release = match library.release_version_variable {
        Some(variable) => {
            let output = host
                .output(
                    "pkg-config",
                    [format!("--variable={variable}").as_str(), library.module],
                )
                .await?;
            let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            (output.status.success() && !value.is_empty()).then_some(value)
        }
        None => None,
    };

    Ok(NativeLibraryStatus::TooOld { installed, release })
}

/// Compare two dotted-numeric pkg-config versions.
///
/// Every module checked here publishes a plain dotted-numeric `Version:` field
/// (`1.19.0`, `0.3.65`, `1.4.7`), and a missing trailing component reads as
/// zero, so `1.19` and `1.19.0` compare equal. A component that is not a number
/// is reported instead of being silently accepted: passing an unreadable
/// version would turn this check back into the presence check it replaces.
///
/// # Errors
/// Returns an error when either version has a non-numeric component.
fn version_at_least(installed: &str, minimum: &str) -> Result<bool, DottedVersionError> {
    let installed = parse_version(installed)?;
    let minimum = parse_version(minimum)?;
    let len = installed.len().max(minimum.len());
    for index in 0..len {
        let left = installed.get(index).copied().unwrap_or(0);
        let right = minimum.get(index).copied().unwrap_or(0);
        if left != right {
            return Ok(left > right);
        }
    }
    Ok(true)
}

/// Split a dotted-numeric version into its components.
fn parse_version(version: &str) -> Result<Vec<u64>, DottedVersionError> {
    version
        .split('.')
        .map(|component| {
            component
                .parse::<u64>()
                .map_err(|source| DottedVersionError {
                    version: version.to_owned(),
                    source,
                })
        })
        .collect()
}

async fn run_with_optional_sudo(
    host: &Host,
    command: &str,
    args: &[String],
) -> Result<(), CommandError> {
    if host.which("sudo").await.is_ok() {
        let mut sudo_args = Vec::with_capacity(args.len() + 1);
        sudo_args.push(command.to_string());
        sudo_args.extend(args.iter().cloned());
        host.run("sudo", sudo_args.iter().map(String::as_str))
            .await?;
    } else {
        host.run(command, args.iter().map(String::as_str)).await?;
    }
    Ok(())
}

async fn install_missing_packages(
    host: &Host,
    manager: LinuxPackageManager,
    packages: &[String],
) -> Result<(), CommandError> {
    if packages.is_empty() {
        return Ok(());
    }

    match manager {
        LinuxPackageManager::Apt => {
            if packages.iter().any(|package| package.ends_with(":amd64")) {
                ensure_apt_foreign_architecture(host, "amd64").await?;
            }
            run_with_optional_sudo(host, "apt-get", &[String::from("update")]).await?;
            let mut args = vec![String::from("install"), String::from("-y")];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo(host, "apt-get", &args).await?;
        }
        LinuxPackageManager::Dnf => {
            let mut args = vec![String::from("install"), String::from("-y")];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo(host, "dnf", &args).await?;
        }
        LinuxPackageManager::Pacman => {
            let mut args = vec![
                String::from("-S"),
                String::from("--noconfirm"),
                String::from("--needed"),
            ];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo(host, "pacman", &args).await?;
        }
        LinuxPackageManager::Zypper => {
            let mut args = vec![
                String::from("--non-interactive"),
                String::from("install"),
                String::from("--auto-agree-with-licenses"),
            ];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo(host, "zypper", &args).await?;
        }
        LinuxPackageManager::Apk => {
            let mut args = vec![String::from("add")];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo(host, "apk", &args).await?;
        }
    }

    Ok(())
}

async fn ensure_apt_foreign_architecture(
    host: &Host,
    architecture: &str,
) -> Result<(), CommandError> {
    let output = host.run("dpkg", ["--print-foreign-architectures"]).await?;
    if output.lines().any(|line| line.trim() == architecture) {
        return Ok(());
    }
    run_with_optional_sudo(
        host,
        "dpkg",
        &[String::from("--add-architecture"), architecture.to_string()],
    )
    .await
}

fn required_packages_to_owned(packages: &[&str]) -> Vec<String> {
    packages
        .iter()
        .map(|package| (*package).to_string())
        .collect()
}

fn unsupported_manager_hint() -> String {
    let apt_hint = LinuxPackageManager::Apt.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Apt.required_packages(),
    ));
    let dnf_hint = LinuxPackageManager::Dnf.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Dnf.required_packages(),
    ));
    let pacman_hint = LinuxPackageManager::Pacman.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Pacman.required_packages(),
    ));
    let zypper_hint = LinuxPackageManager::Zypper.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Zypper.required_packages(),
    ));
    let alpine_hint = LinuxPackageManager::Apk.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Apk.required_packages(),
    ));
    format!(
        "Install required packages manually. Debian/Ubuntu: `{apt_hint}`; Fedora/RHEL: `{dnf_hint}`; Arch: `{pacman_hint}`; openSUSE: `{zypper_hint}`; Alpine: `{alpine_hint}`."
    )
}

/// Returns `true` when a supported Linux package manager is available.
pub async fn has_supported_package_manager(host: &Host) -> bool {
    LinuxPackageManager::detect(host).await.is_some()
}

/// Build an installation plan that repairs missing GTK pkg-config probes.
///
/// Supported probe names include `gtk4` and `pango>=1.50`.
///
/// # Errors
/// Returns an error if no package manager is available or if a probe cannot be
/// mapped to an installable system package.
pub async fn gtk4_pkg_config_repair_installation(
    host: &Host,
    missing_modules: &[String],
) -> Result<LinuxSystemPackagesInstallation, UnfixableToolchain> {
    let Some(manager) = LinuxPackageManager::detect(host).await else {
        return Err(UnfixableToolchain::new(
            "Unable to detect Linux package manager",
            unsupported_manager_hint(),
        ));
    };

    let mut packages = Vec::new();
    for module in missing_modules {
        let package = manager
            .package_for_gtk_pkg_config_probe(module)
            .ok_or_else(|| {
                UnfixableToolchain::new(
                    format!("No package mapping is defined for GTK probe `{module}`"),
                    "Install a package that provides the missing module via pkg-config, then re-run `water doctor`.",
                )
            })?;
        if !packages.iter().any(|existing| existing == package) {
            packages.push(package.to_owned());
        }
    }

    LinuxSystemPackagesInstallation::from_packages(host, packages).await
}

/// Install named packages with the detected Linux package manager.
///
/// # Errors
/// Returns an error when no supported package manager is available, or when
/// installation fails.
pub async fn install_named_packages(
    host: &Host,
    packages: &[&'static str],
) -> Result<(), LinuxPackageManagerError> {
    let Some(manager) = LinuxPackageManager::detect(host).await else {
        return Err(LinuxPackageManagerError::UnsupportedPackageManager);
    };

    let packages: Vec<String> = packages
        .iter()
        .map(|package| (*package).to_string())
        .collect();
    install_missing_packages(host, manager, &packages).await?;
    Ok(())
}

/// Install a JDK package using the detected Linux package manager.
///
/// # Errors
/// Returns an error when no supported package manager is available, or when
/// installation fails.
pub async fn install_java_jdk(host: &Host) -> Result<(), LinuxPackageManagerError> {
    let Some(manager) = LinuxPackageManager::detect(host).await else {
        return Err(LinuxPackageManagerError::UnsupportedPackageManager);
    };

    let packages: Vec<String> = match manager {
        LinuxPackageManager::Apt => vec![String::from("openjdk-21-jdk")],
        LinuxPackageManager::Dnf | LinuxPackageManager::Zypper => {
            vec![String::from("java-21-openjdk-devel")]
        }
        LinuxPackageManager::Pacman => vec![String::from("jdk-openjdk")],
        LinuxPackageManager::Apk => vec![String::from("openjdk21-jdk")],
    };

    install_missing_packages(host, manager, &packages).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        LinuxPackageManager, NativeLibraryStatus, REQUIRED_NATIVE_MODULES,
        VERSIONED_NATIVE_LIBRARIES, VersionedNativeLibrary, WPE_NATIVE_MODULES, version_at_least,
    };

    /// Stand-in for `pkg-config --modversion` / `--variable=…` on a machine that
    /// has no pkg-config, so the probe logic is testable without a subprocess.
    fn interpret_pkg_config(
        library: VersionedNativeLibrary,
        modversion: Option<&str>,
        release: Option<&str>,
    ) -> NativeLibraryStatus {
        let Some(modversion) = modversion else {
            return NativeLibraryStatus::ModuleMissing;
        };
        if version_at_least(modversion.trim(), library.minimum_version).unwrap() {
            NativeLibraryStatus::Satisfied
        } else {
            NativeLibraryStatus::TooOld {
                installed: modversion.trim().to_owned(),
                release: release.map(str::to_owned),
            }
        }
    }

    fn library(module: &str) -> VersionedNativeLibrary {
        *VERSIONED_NATIVE_LIBRARIES
            .iter()
            .find(|library| library.module == module)
            .expect("library is checked by the doctor")
    }

    #[test]
    fn version_comparison_pads_missing_components() {
        assert!(version_at_least("1.19.0", "1.19").unwrap());
        assert!(version_at_least("1.19", "1.19.0").unwrap());
        assert!(version_at_least("1.20.0", "1.19").unwrap());
        assert!(!version_at_least("1.14.0", "1.19").unwrap());
        assert!(!version_at_least("1.2.0", "1.19").unwrap());
        assert!(version_at_least("1.4.7", "0.3.65").unwrap());
        assert!(!version_at_least("0.3.48", "0.3.65").unwrap());
        assert!(version_at_least("0.3.65", "0.3.65").unwrap());
    }

    #[test]
    fn version_comparison_rejects_unreadable_versions() {
        let error = version_at_least("1.19.0-rc1", "1.19").unwrap_err();
        assert!(error.to_string().contains("dotted-numeric"));
    }

    #[test]
    fn ubuntu_2204_libva_is_reported_as_too_old() {
        let libva = library("libva");
        // Ubuntu 22.04 ships libva 2.14.0, which is VA-API 1.14.0.
        let status = interpret_pkg_config(libva, Some("1.14.0\n"), Some("2.14.0"));
        let NativeLibraryStatus::TooOld { installed, release } = status else {
            panic!("VA-API 1.14 must not satisfy the cros-libva floor");
        };
        let message = libva.outdated_message(&installed, release.as_deref());
        assert_eq!(
            message,
            "`libva` is too old: pkg-config reports VA-API 1.14.0 (release 2.14.0), but `cros-libva 0.0.12` needs VA-API 1.19 or newer"
        );

        let suggestion = libva.outdated_suggestion(Some("libva-dev"));
        assert!(suggestion.starts_with(
            "`libva-dev` is already installed, so installing it again will not help."
        ));
        assert!(suggestion.contains("Ubuntu 22.04 (libva 2.14)"));
        assert!(suggestion.contains("build libva 2.19 or newer"));
        assert!(suggestion.contains("pkg-config --modversion libva"));
    }

    #[test]
    fn ubuntu_2404_libva_satisfies_the_floor() {
        // Ubuntu 24.04 ships libva 2.20.0, which is VA-API 1.20.0.
        let status = interpret_pkg_config(library("libva"), Some("1.20.0\n"), Some("2.20.0"));
        assert_eq!(status, NativeLibraryStatus::Satisfied);
    }

    #[test]
    fn ubuntu_2204_pipewire_is_reported_as_too_old() {
        let pipewire = library("libpipewire-0.3");
        let status = interpret_pkg_config(pipewire, Some("0.3.48\n"), None);
        let NativeLibraryStatus::TooOld { installed, release } = status else {
            panic!("PipeWire 0.3.48 must not satisfy the libspa floor");
        };
        assert_eq!(release, None);
        let message = pipewire.outdated_message(&installed, release.as_deref());
        assert_eq!(
            message,
            "`libpipewire-0.3` is too old: pkg-config reports PipeWire 0.3.48, but `libspa 0.10.1` needs PipeWire 0.3.65 or newer"
        );

        let suggestion = pipewire.outdated_suggestion(Some("libpipewire-0.3-dev"));
        assert!(suggestion.contains("spa_video_info_raw::flags"));
        assert!(suggestion.contains("Ubuntu 22.04 (PipeWire 0.3.48)"));
    }

    #[test]
    fn missing_module_is_reported_as_missing_not_outdated() {
        assert_eq!(
            interpret_pkg_config(library("libva"), None, None),
            NativeLibraryStatus::ModuleMissing
        );
    }

    #[test]
    fn outdated_suggestion_without_a_package_mapping_still_reads() {
        let suggestion = library("libva").outdated_suggestion(None);
        assert!(suggestion.starts_with("The package providing `libva` is already installed,"));
    }

    #[test]
    fn every_manager_maps_the_probed_modules() {
        for manager in [
            LinuxPackageManager::Apt,
            LinuxPackageManager::Dnf,
            LinuxPackageManager::Pacman,
            LinuxPackageManager::Zypper,
            LinuxPackageManager::Apk,
        ] {
            for library in VERSIONED_NATIVE_LIBRARIES {
                assert!(
                    manager.package_for_native_library(library.module).is_some(),
                    "{} has no package mapping for {}",
                    manager.name(),
                    library.module
                );
            }
            for &module in REQUIRED_NATIVE_MODULES.iter().chain(WPE_NATIVE_MODULES) {
                assert!(
                    manager.package_for_native_library(module).is_some(),
                    "{} has no package mapping for {module}",
                    manager.name(),
                );
            }
        }
    }

    #[test]
    fn egl_maps_to_the_distro_specific_mesa_package() {
        // The module is the same everywhere; the package that owns `egl.pc`
        // is not — Tumbleweed kept the `Mesa-` prefix Fedora dropped for gbm.
        assert_eq!(
            LinuxPackageManager::Apt.package_for_native_library("egl"),
            Some("libegl-dev")
        );
        assert_eq!(
            LinuxPackageManager::Dnf.package_for_native_library("egl"),
            Some("mesa-libEGL-devel")
        );
        assert_eq!(
            LinuxPackageManager::Zypper.package_for_native_library("egl"),
            Some("Mesa-libEGL-devel")
        );
    }

    /// The floors are read out of specific crate versions, so the versions the
    /// diagnostics name have to be the ones the framework actually resolves.
    /// The lockfile that pins them is the enclosing workspace's own — read
    /// out of the checkout, not fetched: this crate lives in the framework's
    /// repository, so its lock is the file at the workspace root.
    #[test]
    #[ignore = "reads the enclosing workspace's lockfile"]
    fn crate_versions_match_lockfile() {
        let lockfile = String::from_utf8(
            std::fs::read(crate::pinned_framework::checkout().join("Cargo.lock"))
                .expect("the workspace lockfile is readable"),
        )
        .expect("the framework lockfile is UTF-8");
        for library in VERSIONED_NATIVE_LIBRARIES {
            // The floor's crate is conditional: a binding the workspace's
            // default resolve does not select is absent from the lockfile,
            // and no entry can pin it. The assertion applies whenever the
            // resolve does select it.
            if !lockfile.contains(&format!("name = \"{}\"", library.required_by)) {
                continue;
            }
            let entry = format!(
                "name = \"{}\"\nversion = \"{}\"\n",
                library.required_by, library.required_by_version
            );
            assert!(
                lockfile.contains(&entry),
                "{} {} is no longer the resolved version; re-read its version floor before changing this constant",
                library.required_by,
                library.required_by_version
            );
        }
    }

    #[test]
    fn dnf_packages_include_validated_core_deps() {
        let required = LinuxPackageManager::Dnf.required_packages();
        assert!(required.contains(&"gtk4-devel"));
        assert!(required.contains(&"pango-devel"));
        assert!(required.contains(&"wayland-devel"));
        assert!(required.contains(&"mesa-libgbm-devel"));
        assert!(required.contains(&"libxcb-devel"));
        assert!(required.contains(&"alsa-lib-devel"));
        assert!(required.contains(&"clang-devel"));
        assert!(required.contains(&"fontconfig-devel"));
    }

    #[test]
    fn zypper_packages_use_tumbleweed_names() {
        // Verified against `zypper` on openSUSE Tumbleweed: the distribution
        // has no `pkg-config` or `Mesa-libgbm-devel` package — a name that
        // only resolves as a capability still fails `rpm -q` forever.
        let required = LinuxPackageManager::Zypper.required_packages();
        assert!(required.contains(&"pkgconf-pkg-config"));
        assert!(required.contains(&"libgbm-devel"));
        assert!(!required.contains(&"pkg-config"));
        assert!(!required.contains(&"Mesa-libgbm-devel"));
    }

    #[test]
    fn apt_hint_uses_apt_get_install() {
        let hint = LinuxPackageManager::Apt
            .install_hint(&[String::from("libwayland-dev"), String::from("libva-dev")]);
        assert_eq!(hint, "sudo apt-get install -y libwayland-dev libva-dev");
    }

    #[test]
    fn apt_required_packages_include_gtk4_dev() {
        let required = LinuxPackageManager::Apt.required_packages();
        assert!(required.contains(&"libgtk-4-dev"));
        assert!(required.contains(&"libpango1.0-dev"));
    }

    #[test]
    fn pacman_hint_uses_needed_flag() {
        let hint = LinuxPackageManager::Pacman
            .install_hint(&[String::from("wayland"), String::from("libva")]);
        assert_eq!(hint, "sudo pacman -S --noconfirm --needed wayland libva");
    }

    #[test]
    fn dnf_probe_mapping_covers_pango_and_gtk4() {
        assert_eq!(
            LinuxPackageManager::Dnf.package_for_gtk_pkg_config_probe("gtk4"),
            Some("gtk4-devel")
        );
        assert_eq!(
            LinuxPackageManager::Dnf.package_for_gtk_pkg_config_probe("pango>=1.50"),
            Some("pango-devel")
        );
    }
}

/// Host-driven checks exercise the real `check` code path through fake
/// package-manager binaries. They only make sense where `check` probes:
/// on non-Linux hosts it returns `Ok(())` unconditionally.
#[cfg(all(test, target_os = "linux"))]
mod host_tests {
    use super::LinuxSystemToolchain;
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    const APT_PACKAGES: &str = "build-essential pkg-config libgtk-4-dev libpango1.0-dev libwayland-dev \
         wayland-protocols libasound2-dev libgbm-dev libxcb1-dev \
         libclang-dev libfontconfig-dev ca-certificates";

    /// A machine whose apt package set is complete and whose pkg-config
    /// reports in-range versions for the version-checked native libraries and
    /// knows every presence-only module.
    fn complete_apt_machine() -> TestMachine {
        let machine = TestMachine::new();
        for tool in ["apt-get", "dpkg-query", "pkg-config"] {
            machine.install(tool);
        }
        machine.respond_pkg_config_module("libva", "1.20.0");
        machine.respond_pkg_config_var("libva_version", "2.20.0");
        machine.respond_pkg_config_module("libpipewire-0.3", "0.3.65");
        machine.respond_pkg_config_module("egl", "1.5.0");
        machine
    }

    /// Stage the project the machine's working directory holds: a
    /// `Water.toml`, and a `cargo` whose `metadata` resolves a root `app`
    /// package with a normal dependency edge to each of `crates`.
    fn stage_project(machine: &TestMachine, crates: &[&str]) {
        const ROOT: &str = "app";
        machine.install("cargo");
        machine.file("Water.toml", "");
        let id = |name: &str| {
            format!("registry+https://github.com/rust-lang/crates.io-index#{name}@0.0.0")
        };
        let packages: Vec<_> = std::iter::once(&ROOT)
            .chain(crates)
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "version": "0.0.0",
                    "id": id(name),
                    "license": null,
                    "license_file": null,
                    "description": null,
                    "source": null,
                    "dependencies": [],
                    "manifest_path": format!("/{name}/Cargo.toml"),
                    "categories": [],
                    "keywords": [],
                    "readme": null,
                    "repository": null,
                    "homepage": null,
                    "documentation": null,
                    "edition": "2021",
                    "links": null,
                    "default_run": null,
                    "rust_version": null,
                    "metadata": null,
                    "features": {},
                    "targets": [],
                    "publish": null,
                    "authors": []
                })
            })
            .collect();
        let root_deps: Vec<_> = crates
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "pkg": id(name),
                    "dep_kinds": [{ "kind": null, "target": null }]
                })
            })
            .collect();
        let crate_ids: Vec<_> = crates.iter().map(|name| id(name)).collect();
        let nodes: Vec<_> = std::iter::once(serde_json::json!({
            "id": id(ROOT),
            "dependencies": crate_ids,
            "deps": root_deps,
            "features": []
        }))
        .chain(crates.iter().map(|name| {
            serde_json::json!({
                "id": id(name),
                "dependencies": [],
                "deps": [],
                "features": []
            })
        }))
        .collect();
        let metadata = serde_json::json!({
            "packages": packages,
            "workspace_members": [id(ROOT)],
            "workspace_default_members": [id(ROOT)],
            "resolve": { "nodes": nodes, "root": id(ROOT) },
            "target_directory": "/target",
            "version": 1,
            "workspace_root": "/",
            "metadata": null
        });
        machine.respond("CARGO_METADATA", &metadata.to_string());
    }

    #[test]
    fn unfixable_without_package_manager() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(LinuxSystemToolchain.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "no package manager must be unfixable: {result:?}"
        );
    }

    #[test]
    fn ok_when_apt_packages_and_library_versions_satisfy_floors() {
        let machine = complete_apt_machine();
        stage_project(&machine, &["waterui-video", "cros-libva", "libspa"]);
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            format!("{APT_PACKAGES} libva-dev libpipewire-0.3-dev"),
        )]);
        smol::block_on(LinuxSystemToolchain.check(&host))
            .expect("complete apt package set must satisfy the check");
    }

    #[test]
    fn unselected_bindings_require_no_native_library() {
        // A plain project's resolved graph selects neither `cros-libva` nor
        // `libspa`: the host reports no libva or libpipewire module and the
        // check must still pass instead of demanding an unused VA-API stack.
        let machine = TestMachine::new();
        for tool in ["apt-get", "dpkg-query", "pkg-config"] {
            machine.install(tool);
        }
        stage_project(&machine, &["waterui"]);
        machine.respond_pkg_config_module("egl", "1.5.0");
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            APT_PACKAGES.to_string(),
        )]);
        smol::block_on(LinuxSystemToolchain.check(&host))
            .expect("a graph without the optional bindings must not require libva or pipewire");
    }

    #[test]
    fn outside_a_project_no_binding_is_required() {
        // Without a `Water.toml` at the working directory there is no project
        // graph, even when a `cargo` that would resolve the bindings exists.
        let machine = TestMachine::new();
        for tool in ["apt-get", "dpkg-query", "pkg-config"] {
            machine.install(tool);
        }
        stage_project(&machine, &["waterui-video", "cros-libva", "libspa"]);
        std::fs::remove_file(machine.root().join("Water.toml")).expect("remove Water.toml");
        machine.respond_pkg_config_module("egl", "1.5.0");
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            APT_PACKAGES.to_string(),
        )]);
        smol::block_on(LinuxSystemToolchain.check(&host))
            .expect("a host outside a project must not require optional bindings");
    }

    #[test]
    fn unresolvable_project_graph_is_unfixable() {
        // A project whose `cargo metadata` fails (no response staged) is an
        // error, not an empty graph.
        let machine = TestMachine::new();
        for tool in ["apt-get", "dpkg-query", "pkg-config", "cargo"] {
            machine.install(tool);
        }
        machine.file("Water.toml", "");
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            APT_PACKAGES.to_string(),
        )]);
        let result = smol::block_on(LinuxSystemToolchain.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "a failing cargo metadata must be unfixable: {result:?}"
        );
    }

    #[test]
    fn selected_binding_with_missing_library_is_fixable() {
        let machine = TestMachine::new();
        for tool in ["apt-get", "dpkg-query", "pkg-config"] {
            machine.install(tool);
        }
        stage_project(&machine, &["waterui-video", "cros-libva"]);
        machine.respond_pkg_config_module("egl", "1.5.0");
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            APT_PACKAGES.to_string(),
        )]);
        let Err(ToolchainError::Fixable(installation)) =
            smol::block_on(LinuxSystemToolchain.check(&host))
        else {
            panic!(
                "a missing libva module for a selected cros-libva must map to an installable package"
            );
        };
        assert_eq!(
            installation.missing_packages(),
            &[String::from("libva-dev")]
        );
    }

    #[test]
    fn fixable_installation_lists_only_missing_packages() {
        let machine = complete_apt_machine();
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            String::from("pkg-config libgtk-4-dev"),
        )]);
        let Err(ToolchainError::Fixable(installation)) =
            smol::block_on(LinuxSystemToolchain.check(&host))
        else {
            panic!("missing apt packages must produce a fixable installation");
        };
        assert_eq!(installation.package_manager_name(), "apt-get");
        let missing = installation.missing_packages();
        assert!(missing.iter().any(|package| package == "libgbm-dev"));
        assert!(!missing.iter().any(|package| package == "libgtk-4-dev"));
        assert!(!missing.iter().any(|package| package == "libva-dev"));
    }

    #[test]
    fn unfixable_when_pkg_config_missing() {
        let machine = TestMachine::new();
        machine.install("apt-get");
        machine.install("dpkg-query");
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            APT_PACKAGES.to_string(),
        )]);
        let result = smol::block_on(LinuxSystemToolchain.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "absent pkg-config must be an unfixable diagnostic: {result:?}"
        );
    }

    #[test]
    fn unfixable_when_libva_below_floor() {
        let machine = complete_apt_machine();
        stage_project(&machine, &["waterui-video", "cros-libva"]);
        // Re-stage libva at the Ubuntu 22.04 version: VA-API 1.14 is below
        // the cros-libva floor of 1.19.
        machine.respond_pkg_config_module("libva", "1.14.0");
        machine.respond_pkg_config_var("libva_version", "2.14.0");
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            format!("{APT_PACKAGES} libva-dev"),
        )]);
        let result = smol::block_on(LinuxSystemToolchain.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "an outdated libva must be unfixable (reinstalling changes nothing): {result:?}"
        );
    }

    #[test]
    fn fixable_when_versioned_module_absent_from_pkg_config() {
        let machine = TestMachine::new();
        for tool in ["apt-get", "dpkg-query", "pkg-config"] {
            machine.install(tool);
        }
        stage_project(&machine, &["waterui-video", "cros-libva", "libspa"]);
        // libva and egl are staged; libpipewire-0.3 is unknown to pkg-config.
        machine.respond_pkg_config_module("libva", "1.20.0");
        machine.respond_pkg_config_var("libva_version", "2.20.0");
        machine.respond_pkg_config_module("egl", "1.5.0");
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            format!("{APT_PACKAGES} libva-dev"),
        )]);
        let Err(ToolchainError::Fixable(installation)) =
            smol::block_on(LinuxSystemToolchain.check(&host))
        else {
            panic!("an absent libpipewire module must map to an installable package");
        };
        assert_eq!(
            installation.missing_packages(),
            &[String::from("libpipewire-0.3-dev")]
        );
    }

    #[test]
    fn c_toolchain_ok_when_cc_and_ld_on_path() {
        let machine = TestMachine::new();
        machine.install("cc");
        machine.install("ld");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(super::CToolchain.check(&host))
            .expect("cc and ld on PATH must satisfy the C toolchain check");
    }

    #[test]
    fn c_toolchain_fixable_with_package_manager() {
        let machine = TestMachine::new();
        machine.install("apt-get");
        machine.install("dpkg-query");
        let host = machine.host(Vec::<(String, String)>::new());
        let Err(ToolchainError::Fixable(installation)) =
            smol::block_on(super::CToolchain.check(&host))
        else {
            panic!("a missing C toolchain on a managed host must be fixable");
        };
        assert_eq!(installation.package_manager_name(), "apt-get");
        assert_eq!(
            installation.missing_packages(),
            &[String::from("build-essential")]
        );
    }

    #[test]
    fn fixable_when_presence_module_absent_from_pkg_config() {
        let machine = complete_apt_machine();
        // Drop the staged egl response: `pkg-config --exists egl` fails.
        std::fs::remove_file(machine.responses().join("PKG_CONFIG_egl"))
            .expect("remove the staged egl module");
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            APT_PACKAGES.to_string(),
        )]);
        let Err(ToolchainError::Fixable(installation)) =
            smol::block_on(LinuxSystemToolchain.check(&host))
        else {
            panic!("an absent egl module must map to an installable package");
        };
        assert_eq!(
            installation.missing_packages(),
            &[String::from("libegl-dev")]
        );
    }

    #[test]
    fn c_toolchain_unfixable_without_package_manager() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(super::CToolchain.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "a missing C toolchain without a package manager must be unfixable: {result:?}"
        );
    }

    #[test]
    fn missing_module_stays_fixable_when_another_library_is_too_old() {
        // Debian 12: libva is below the cros-libva floor, and libpipewire-0.3
        // is not installed at all. The outdated library must not mask the
        // installable package behind an unfixable verdict.
        let machine = TestMachine::new();
        for tool in ["apt-get", "dpkg-query", "pkg-config"] {
            machine.install(tool);
        }
        stage_project(&machine, &["waterui-video", "cros-libva", "libspa"]);
        machine.respond_pkg_config_module("libva", "1.14.0");
        machine.respond_pkg_config_var("libva_version", "2.14.0");
        machine.respond_pkg_config_module("egl", "1.5.0");
        let host = machine.host([(
            String::from("WATERUI_FAKE_DPKG_INSTALLED"),
            format!("{APT_PACKAGES} libva-dev"),
        )]);
        let Err(ToolchainError::Fixable(installation)) =
            smol::block_on(LinuxSystemToolchain.check(&host))
        else {
            panic!("a missing module must outrank an unrelated outdated library");
        };
        assert_eq!(
            installation.missing_packages(),
            &[String::from("libpipewire-0.3-dev")]
        );
    }
}

/// `check_native_libraries` probes pkg-config through the fake dispatcher, so
/// the WPE module selection is verifiable on any Unix host.
#[cfg(all(test, unix))]
mod wpe_module_tests {
    use std::collections::BTreeSet;

    use super::{LinuxPackageManager, WPE_NATIVE_MODULES, check_native_libraries};
    use crate::toolchain::ToolchainError;
    use crate::toolchain::testing::TestMachine;

    /// A machine whose pkg-config knows `egl` and every WPE module except
    /// `libsoup-3.0`.
    fn machine_without_libsoup() -> TestMachine {
        let machine = TestMachine::new();
        machine.install("pkg-config");
        machine.respond_pkg_config_module("egl", "1.5.0");
        for module in WPE_NATIVE_MODULES {
            if *module != "libsoup-3.0" {
                machine.respond_pkg_config_module(module, "1.0");
            }
        }
        machine
    }

    #[test]
    fn a_selected_wpe_runtime_requires_its_pkg_config_modules() {
        let machine = machine_without_libsoup();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(check_native_libraries(
            &host,
            LinuxPackageManager::Apt,
            &BTreeSet::new(),
            true,
        ));
        let Err(ToolchainError::Fixable(installation)) = result else {
            panic!("a missing libsoup-3.0 must be fixable: {result:?}");
        };
        assert_eq!(
            installation.missing_packages(),
            &[String::from("libsoup-3.0-dev")]
        );
    }

    #[test]
    fn an_unselected_wpe_runtime_probes_no_wpe_module() {
        let machine = machine_without_libsoup();
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(check_native_libraries(
            &host,
            LinuxPackageManager::Apt,
            &BTreeSet::new(),
            false,
        ))
        .expect("the WPE modules are not prerequisites without the runtime");
    }
}
