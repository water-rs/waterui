//! Framework channel resolution and persisted dependency selection.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::project::Project;
use cargo_lock::{Dependency as LockedDependency, Lockfile};
use cargo_toml::{Dependency, DependencyDetail, PatchSet};
use eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use smol::process::Command;
use zenwave::{Client as _, Method, StatusCode};

/// A framework distribution channel, independent of the Rust toolchain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameworkChannel {
    /// The integration branch, resolved to an exact compilation-checked
    /// commit; the Apple backend's `dev` HEAD resolves the same way.
    Dev,
    /// An immutable revision certified by the complete nightly suite,
    /// including the backend pin the suite's certification records.
    Nightly,
    /// Published packages and the compatible native backends bundled with the CLI.
    #[default]
    Stable,
}

impl fmt::Display for FrameworkChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Dev => "dev",
            Self::Nightly => "nightly",
            Self::Stable => "stable",
        })
    }
}

impl FromStr for FrameworkChannel {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "dev" => Ok(Self::Dev),
            "nightly" => Ok(Self::Nightly),
            "stable" => Ok(Self::Stable),
            _ => Err("framework channel must be dev, nightly or stable".into()),
        }
    }
}

/// The GitHub release a certified channel resolved from — the provenance the
/// persisted selection keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FrameworkRelease {
    /// The repository the release lives in.
    repository: String,
    /// The commit the release certifies.
    revision: String,
    /// The release tag the manifest rode in on.
    tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "channel", rename_all = "lowercase")]
enum Source {
    /// A published framework release whose `framework.json` resolved every
    /// scaffold pin.
    Stable {
        /// The release the selection resolved from — absent in manifests
        /// written before the stable channel carried a manifest.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        release: Option<FrameworkRelease>,
    },
    Dev {
        repository: String,
        revision: String,
        lock_sha256: String,
    },
    Nightly {
        repository: String,
        revision: String,
        tag: String,
        lock_sha256: String,
    },
    /// A local checkout reached through `waterui_path`: a filesystem source,
    /// not a channel. Never persisted — `waterui_path` itself is the record.
    #[serde(skip)]
    Local { root: PathBuf },
}

/// The persisted framework and backend selection used without channel re-resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFramework {
    #[serde(flatten)]
    source: Source,
    #[serde(
        default,
        rename = "minimum-cli-version",
        skip_serializing_if = "Option::is_none"
    )]
    minimum_cli_version: Option<cargo_toml::SemVer>,
    /// The framework's own `rust-version` — `[workspace.package].rust-version`
    /// of the manifest at the selected revision — persisted with the
    /// selection so the project's Rust floor is known without a checkout.
    #[serde(
        default,
        rename = "rust-version",
        skip_serializing_if = "Option::is_none"
    )]
    rust_version: Option<cargo_toml::SemVer>,
    /// The framework's `[package.metadata.waterui]` table at the selected
    /// revision, carried verbatim from its manifest.
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    metadata: toml::Table,
    scaffold: BTreeMap<String, String>,
    /// The scaffold packages the selected channel withholds: a git-pinned
    /// requirement is not one `stable` distributes, whatever the registry
    /// holds for that name, so a stable manifest omits its `scaffold`
    /// entries and records the pin under `experimental-packages` instead.
    /// Empty on `dev`/`nightly` and on a local checkout — they distribute
    /// every scaffold package.
    #[serde(
        default,
        rename = "experimental-packages",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    experimental_packages: BTreeMap<String, ExperimentalPackage>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    packages: BTreeMap<String, DependencyDetail>,
    #[serde(default, skip_serializing_if = "PatchSet::is_empty")]
    patches: PatchSet,
}

/// The selection in one line, as `water create` reports it: the channel and
/// what it resolved to — the release tag for stable and nightly, the commit
/// for dev — with the short revision after it.
impl fmt::Display for ResolvedFramework {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let short = |revision: &str| revision.get(..8).unwrap_or(revision).to_owned();
        match &self.source {
            Source::Stable {
                release: Some(release),
            } => write!(
                formatter,
                "stable {} ({})",
                release.tag,
                short(&release.revision)
            ),
            Source::Stable { release: None } => formatter.write_str("stable"),
            Source::Nightly { tag, revision, .. } => {
                write!(formatter, "nightly {tag} ({})", short(revision))
            }
            Source::Dev { revision, .. } => write!(formatter, "dev {}", short(revision)),
            Source::Local { root } => write!(formatter, "local checkout {}", root.display()),
        }
    }
}

/// A scaffold package a channel withholds.
///
/// Its workspace requirement pins a git revision because the package has no
/// registry release, so the manifest records the pin — git URL, commit and
/// declared version — by name instead of emitting `scaffold` entries for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentalPackage {
    /// The repository the framework pins the package to.
    pub git: String,
    /// The pinned commit.
    pub rev: String,
    /// The declared version requirement.
    pub version: String,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
}

/// Whether a resolved package took the place of one the lock pins.
///
/// The managed crate's graph is a superset of the project's: `waterui-ffi`
/// and its feature-gated platform dependencies add packages the project
/// never locked, and among them a second major of a name the project already
/// carries (`annotate-snippets` 0.11 beside the locked 0.12, through
/// bindgen). Cargo keeps both, so that is an addition, not a change. A change
/// is a resolved version that Cargo could only have reached by moving a
/// locked one — a version the locked entry's caret requirement accepts, from
/// the same source.
fn replaces_locked_package(
    allowed: &BTreeSet<LockedPackage>,
    name: &str,
    version: &semver::Version,
    source: Option<&str>,
) -> bool {
    let identity = LockedPackage {
        name: name.to_owned(),
        version: version.to_string(),
        source: source.map(str::to_owned),
    };
    if allowed.contains(&identity) {
        return false;
    }
    allowed
        .iter()
        .filter(|locked| locked.name == identity.name && locked.source == identity.source)
        .any(|locked| {
            semver::VersionReq::parse(&format!("^{}", locked.version))
                .expect("a lockfile version is a valid caret requirement")
                .matches(version)
        })
}

impl From<&cargo_lock::Package> for LockedPackage {
    fn from(package: &cargo_lock::Package) -> Self {
        Self {
            name: package.name.to_string(),
            version: package.version.to_string(),
            source: package.source.as_ref().map(ToString::to_string),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct Certification {
    schema_version: u32,
    channel: FrameworkChannel,
    repository: String,
    revision: String,
    tag: String,
    lockfiles: BTreeMap<String, String>,
    /// Submodule path -> commit the certification recorded for the revision.
    #[serde(default)]
    submodules: BTreeMap<String, String>,
    scaffold: BTreeMap<String, String>,
    /// The scaffold packages the certification withholds from `scaffold` —
    /// `stable` records every git-pinned package here; `nightly` carries
    /// them in `scaffold`, so this is empty. The manifest's record must equal
    /// the set the framework manifest's own dependency shapes derive.
    #[serde(default, rename = "experimental-packages")]
    experimental_packages: BTreeMap<String, ExperimentalPackage>,
    /// The framework's `[package.metadata.waterui]` table, verbatim — the CLI
    /// floor and every future framework-owned fact ride inside it.
    metadata: toml::Table,
}

/// Make `document`'s `[patch]` tables carry `patches` in place of `previous`:
/// the entries of `previous` are removed, those of `patches` written, and
/// sources left empty are dropped, so a manifest moving between a channel, a
/// local checkout and the registry never keeps a stale override.
pub(crate) fn rewrite_patch_tables(
    document: &mut toml_edit::DocumentMut,
    previous: &PatchSet,
    patches: &PatchSet,
) -> Result<()> {
    for (source, dependencies) in previous {
        if let Some(table) = document
            .get_mut("patch")
            .and_then(|patch| patch.get_mut(source))
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            for name in dependencies.keys() {
                table.remove(name);
            }
        }
    }
    let patches = toml_edit::ser::to_document(patches)?;
    for (source, dependencies) in patches.iter() {
        // `[patch]` and `[patch.<source>]` are written as explicit tables:
        // indexing into a missing key would vivify an inline value and
        // hoist `patch = { … }` above `[package]`.
        let patch = document
            .entry("patch")
            .or_insert_with(toml_edit::table)
            .as_table_mut()
            .ok_or_else(|| eyre!("[patch] is not a table"))?;
        patch.set_implicit(true);
        let table = patch
            .entry(source)
            .or_insert_with(toml_edit::table)
            .as_table_like_mut()
            .ok_or_else(|| eyre!("[patch.{source}] is not a table"))?;
        for (name, dependency) in dependencies
            .as_table_like()
            .expect("serialized patch dependencies are tables")
            .iter()
        {
            table.insert(name, dependency.clone());
        }
    }
    if let Some(patch) = document
        .get_mut("patch")
        .and_then(toml_edit::Item::as_table_mut)
    {
        let empty: Vec<String> = patch
            .iter()
            .filter(|(_, sources)| {
                sources
                    .as_table_like()
                    .is_some_and(toml_edit::TableLike::is_empty)
            })
            .map(|(source, _)| source.to_owned())
            .collect();
        for source in empty {
            patch.remove(&source);
        }
        if patch.is_empty() {
            document.remove("patch");
        }
    }
    Ok(())
}

impl ResolvedFramework {
    /// The selected distribution channel — `None` for a local checkout, which
    /// is a filesystem source rather than a channel.
    #[must_use]
    pub const fn channel(&self) -> Option<FrameworkChannel> {
        match self.source {
            Source::Stable { .. } => Some(FrameworkChannel::Stable),
            Source::Dev { .. } => Some(FrameworkChannel::Dev),
            Source::Nightly { .. } => Some(FrameworkChannel::Nightly),
            Source::Local { .. } => None,
        }
    }

    /// The framework a manifest resolves its generated code against: the
    /// channel selection `framework` records, or the checkout `waterui_path`
    /// names. A manifest carrying neither has no framework to resolve — an
    /// explicit channel selection creates the record.
    ///
    /// # Errors
    /// Returns an error when the manifest records no framework source, or the
    /// local checkout's framework facts cannot be read.
    pub(crate) async fn for_manifest(
        manifest: &crate::project::Manifest,
        project_root: &Path,
    ) -> Result<Self> {
        if let Some(framework) = &manifest.framework {
            return framework.clone().validated().wrap_err(
                "the recorded framework selection predates a metadata key this CLI \
                 requires; re-run `water channel` to resolve it again",
            );
        }
        let Some(waterui_path) = &manifest.waterui_path else {
            bail!(
                "the project records no framework selection; run `water channel` \
                 to select one or point `waterui_path` at a checkout"
            );
        };
        Self::for_local_checkout(&project_root.join(waterui_path)).await
    }

    /// The framework facts a local checkout supplies: its own
    /// `[package.metadata.waterui]` table, the scaffold requirements its
    /// `[workspace.dependencies]` declares, and the backend/workspace pins its
    /// gitlinks and lockfile record.
    pub(crate) async fn for_local_checkout(root: &Path) -> Result<Self> {
        let manifest: toml::Value = toml::from_str(
            &smol::fs::read_to_string(root.join("Cargo.toml"))
                .await
                .wrap_err_with(|| {
                    format!(
                        "the WaterUI checkout at {} has no Cargo.toml",
                        root.display()
                    )
                })?,
        )?;
        let metadata = framework_metadata(&manifest)?;
        let minimum_cli_version = minimum_cli_version(&metadata)?;
        let rust_version = manifest_rust_version(&manifest)?;
        if let Some(minimum) = &minimum_cli_version {
            validate_installed_cli(minimum, &checkout_cli_update())?;
        }
        let mut scaffold = framework_scaffold(&manifest)?;
        let lock: Lockfile = smol::fs::read_to_string(root.join("Cargo.lock"))
            .await
            .wrap_err_with(|| {
                format!(
                    "the WaterUI checkout at {} has no Cargo.lock",
                    root.display()
                )
            })?
            .parse()?;
        complete_scaffold(&mut scaffold, &lock)?;
        Self {
            source: Source::Local {
                root: root.to_path_buf(),
            },
            minimum_cli_version,
            rust_version,
            metadata,
            scaffold,
            // A local checkout is a filesystem source, not a channel: it
            // withholds nothing.
            experimental_packages: BTreeMap::new(),
            packages: BTreeMap::new(),
            patches: PatchSet::default(),
        }
        .validated()
    }

    /// Hold the framework to the metadata keys the CLI reads later without a
    /// `Result` in hand — the scaffold's `minSdk` above all. A framework that
    /// reaches a template context has passed here, so a template accessor
    /// failing on it is an internal invariant, not an input error.
    fn validated(mut self) -> Result<Self> {
        self.android_min_api_level()?;
        // The stable split is an invariant of the source, not of the writer:
        // a selection persisted before `experimental-packages` existed keeps
        // the withheld set inside `scaffold`, so re-derive it on load —
        // `dependency` honoring a stale `-git` entry would resurrect a
        // package the channel no longer distributes.
        if matches!(self.source, Source::Stable { .. }) {
            let withheld = split_experimental_packages(&mut self.scaffold);
            self.experimental_packages.extend(withheld);
        }
        Ok(self)
    }

    /// Resolve a certified framework manifest (`framework.json`) from disk —
    /// the `--framework-manifest` source that pins a project to the channel
    /// and revision it declares. The file is verified exactly as a manifest
    /// downloaded from its release is.
    ///
    /// # Errors
    /// Returns an error when the file cannot be read or parsed, fails
    /// verification, or the revision it certifies cannot be fetched.
    pub(crate) async fn resolve_manifest(path: &Path) -> Result<(Self, Option<Vec<u8>>)> {
        let repository = framework_repository();
        let slug = repository_slug(repository)?;
        let certification = load_manifest(path, repository).await?;
        let revision = certification.revision.clone();
        Self::construct(repository, slug, &revision, Some(certification)).await
    }

    pub(crate) fn validate_cli(&self) -> Result<()> {
        if let Some(minimum) = &self.minimum_cli_version {
            let update = match &self.source {
                Source::Stable { .. } => registry_cli_update(minimum),
                Source::Dev { .. } | Source::Nightly { .. } | Source::Local { .. } => {
                    checkout_cli_update()
                }
            };
            validate_installed_cli(minimum, &update)?;
        }
        Ok(())
    }

    pub(crate) fn scaffold_value(&self, key: &str) -> &str {
        self.scaffold
            .get(key)
            .unwrap_or_else(|| panic!("resolved framework carries no `{key}` scaffold metadata"))
    }

    /// Assert the selected channel distributes `name` — a scaffold package a
    /// generated crate links. The stable channel withholds every git-pinned
    /// scaffold package, so scaffolding one must fail before anything is
    /// written, naming the package, the channel and the fix.
    ///
    /// # Errors
    /// Returns an error when the channel records `name` under
    /// `experimental-packages`.
    pub(crate) fn require_distributable(&self, name: &str) -> Result<()> {
        let Some(package) = self.experimental_packages.get(name) else {
            return Ok(());
        };
        let channel = self
            .channel()
            .map_or_else(|| "local".to_owned(), |channel| channel.to_string());
        bail!(
            "`{name}` is an experimental package the {channel} framework channel does not \
             distribute: this framework revision pins it to {} at {}, and the {channel} \
             channel distributes only registry requirements. Scaffold it on \
             `--channel dev` or `--channel nightly`.",
            package.git,
            package.rev,
        );
    }

    /// The Rust floor the selected framework declares — its
    /// `[workspace.package].rust-version` at the resolved revision. A record
    /// written before this key existed carries `None`; the caller falls back
    /// to the CLI's own `rust-version`.
    #[must_use]
    pub const fn rust_version(&self) -> Option<&cargo_toml::SemVer> {
        self.rust_version.as_ref()
    }

    /// The canonical path `member` occupies inside the framework tree —
    /// the `member.subdirectory` workspace member — as the selected
    /// revision's `[package.metadata.waterui]` declares it under
    /// `member.path_key`. `None` names a revision from before the
    /// declaration: it carries no such crate to resolve.
    fn member_path(&self, member: FrameworkMember) -> Option<&str> {
        self.metadata
            .get(member.path_key)
            .and_then(toml::Value::as_str)
    }

    /// The `(repository, revision)` the framework tree itself resolves from
    /// on a channel selection — `dev` and `nightly` the selected revision,
    /// `stable` the certified release's provenance. `None` names a local
    /// checkout: its members resolve by path instead.
    ///
    /// # Errors
    /// Returns an error when a `stable` selection predates release
    /// provenance.
    fn git_coordinates(&self) -> Result<Option<(&str, &str)>> {
        match &self.source {
            Source::Stable { release } => {
                let Some(release) = release else {
                    bail!(
                        "the recorded stable framework selection predates release provenance; \
                         re-run `water channel` to resolve it again"
                    );
                };
                Ok(Some((&release.repository, &release.revision)))
            }
            Source::Dev {
                repository,
                revision,
                ..
            }
            | Source::Nightly {
                repository,
                revision,
                ..
            } => Ok(Some((repository, revision))),
            Source::Local { .. } => Ok(None),
        }
    }

    /// The dependency a generated crate declares for `member` — an in-tree
    /// framework workspace crate resolved through `member.path_key` —
    /// on a channel selection: `dev` and `nightly` name the framework
    /// repository at the selected revision, and `stable` names the
    /// certified release's repository and revision — the crate stays a Git
    /// member on every channel while the framework packages it inherits
    /// resolve from the registry through the source's `[patch]` table. A
    /// local checkout resolves the member by path instead.
    ///
    /// One source serves every consumer: for `waterui-apple` the Rust
    /// dependency, the root `Package.swift` Swift package and the embedded
    /// source, and for `hydrolysis` the generated backend's dependency and
    /// the requirement an independent crate like `hydrolysis-m3` carries.
    ///
    /// # Errors
    /// Returns an error when the selected revision declares no
    /// `member.path_key` — it carries no such member crate — or a `stable`
    /// selection predates release provenance.
    pub(crate) fn member_source(&self, member: FrameworkMember) -> Result<DependencyDetail> {
        let Some((repository, revision)) = self.git_coordinates()? else {
            unreachable!("a local checkout resolves {} by path", member.package)
        };
        self.member_path(member).ok_or_else(|| {
            eyre!(
                "the framework at {repository}@{revision} declares no `{}`; \
                 it carries no `{}` crate",
                member.path_key,
                member.package
            )
        })?;
        Ok(DependencyDetail {
            git: Some(repository.to_owned()),
            rev: Some(revision.to_owned()),
            ..DependencyDetail::default()
        })
    }

    /// The Gradle root inside the Hydrolysis host checkout the generated
    /// project `includeBuild`s — the `hydrolysis-android-host-subdirectory`
    /// scaffold metadata (#1428).
    ///
    /// # Errors
    /// Returns an error naming the missing key when the resolved framework
    /// predates the host coordinates.
    pub(crate) fn hydrolysis_android_host_subdirectory(&self) -> Result<&str> {
        self.scaffold
            .get("hydrolysis-android-host-subdirectory")
            .map(String::as_str)
            .ok_or_else(|| {
                eyre!(
                    "resolved framework carries no `hydrolysis-android-host-subdirectory` \
                     scaffold metadata: it predates the in-tree host (#1428); re-run \
                     `water channel` to resolve it again"
                )
            })
    }

    /// The checkout the Hydrolysis Android host lives inside — the
    /// framework tree itself: a channel framework's repository at the
    /// selected revision (materialized into a managed checkout), or the
    /// `waterui_path` checkout itself.
    ///
    /// # Errors
    /// Returns an error when a `stable` selection predates release
    /// provenance.
    pub(crate) fn hydrolysis_android_host(&self) -> Result<HydrolysisAndroidHost<'_>> {
        if let Some((url, revision)) = self.git_coordinates()? {
            Ok(HydrolysisAndroidHost::Git { url, revision })
        } else {
            let Source::Local { root } = &self.source else {
                unreachable!("`None` coordinates name only a local checkout")
            };
            Ok(HydrolysisAndroidHost::Local { root })
        }
    }

    /// The Android API floor the selected framework's native runtime
    /// supports — the `android-min-api-level` its
    /// `[package.metadata.waterui]` table declares. The backend's Gradle
    /// `minSdk` declares the same floor independently; CI holds the two to
    /// agreement.
    ///
    /// # Errors
    /// Returns an error when the resolved framework's metadata does not
    /// declare a valid `android-min-api-level` integer.
    pub(crate) fn android_min_api_level(&self) -> Result<u32> {
        const KEY: &str = "package.metadata.waterui.android-min-api-level";
        let origin = match &self.source {
            Source::Stable { release } => release.as_ref().map_or_else(
                || "the stable framework manifest".to_owned(),
                |release| format!("the framework manifest certified by {}", release.tag),
            ),
            Source::Dev {
                repository,
                revision,
                ..
            }
            | Source::Nightly {
                repository,
                revision,
                ..
            } => format!("the framework manifest at {repository}@{revision}"),
            Source::Local { root } => format!("{}", root.join("Cargo.toml").display()),
        };
        let value = self
            .metadata
            .get("android-min-api-level")
            .ok_or_else(|| eyre!("{origin} does not declare {KEY}"))?;
        value
            .as_integer()
            .and_then(|level| u32::try_from(level).ok())
            .ok_or_else(|| eyre!("{origin} declares an invalid {KEY}: {value}"))
    }

    pub(crate) fn patches(&self) -> PatchSet {
        self.patches.clone()
    }

    /// Rewrite a project manifest's dependencies and `[patch]` tables for this
    /// framework, clearing `previous_patches` first: the entries the manifest
    /// carried for whatever it was built against before, a channel's or a
    /// local checkout's.
    pub(crate) fn update_manifest(
        &self,
        document: &mut toml_edit::DocumentMut,
        previous_patches: &PatchSet,
    ) -> Result<()> {
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(dependencies) = document
                .get_mut(section)
                .and_then(toml_edit::Item::as_table_like_mut)
            {
                self.update_dependencies(dependencies)?;
            }
        }
        if let Some(targets) = document
            .get_mut("target")
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            for (_, target) in targets.iter_mut() {
                for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(dependencies) = target
                        .get_mut(section)
                        .and_then(toml_edit::Item::as_table_like_mut)
                    {
                        self.update_dependencies(dependencies)?;
                    }
                }
            }
        }
        rewrite_patch_tables(document, previous_patches, &self.patches)
    }

    fn update_dependencies(&self, dependencies: &mut dyn toml_edit::TableLike) -> Result<()> {
        for (name, dependency) in dependencies.iter_mut() {
            let package = dependency
                .get("package")
                .and_then(toml_edit::Item::as_str)
                .unwrap_or(&name)
                .to_owned();
            // The `*-path` members are workspace members, not registry
            // packages: a scaffolded project's dependency on one is pinned
            // by `member_source`, never rewritten to a version requirement.
            if FRAMEWORK_MEMBERS
                .iter()
                .any(|member| member.package == package)
                || !self.scaffold.contains_key(&format!("{package}-version"))
            {
                continue;
            }
            if dependency.is_str() {
                let decor = dependency
                    .as_value()
                    .expect("string dependency")
                    .decor()
                    .clone();
                let mut value = toml_edit::Value::InlineTable(toml_edit::InlineTable::new());
                *value.decor_mut() = decor;
                *dependency = toml_edit::Item::Value(value);
            }
            let table = dependency
                .as_table_like_mut()
                .ok_or_else(|| eyre!("invalid dependency {name}"))?;
            if table.contains_key("workspace") {
                bail!(
                    "{name} inherits its source; select the framework at its Cargo workspace root"
                );
            }
            for key in [
                "version",
                "git",
                "rev",
                "branch",
                "tag",
                "path",
                "registry",
                "registry-index",
            ] {
                table.remove(key);
            }
            let source = toml_edit::ser::to_document(&self.dependency(&package))?;
            for key in ["version", "git", "rev"] {
                if let Some(value) = source.get(key) {
                    table.insert(key, value.clone());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn validate_dependencies(
        &self,
        metadata: &cargo_metadata::Metadata,
        contents: &[u8],
    ) -> Result<()> {
        let (repository, revision) = match &self.source {
            Source::Stable { .. } | Source::Local { .. } => return Ok(()),
            Source::Dev {
                repository,
                revision,
                ..
            }
            | Source::Nightly {
                repository,
                revision,
                ..
            } => (repository, revision),
        };
        let source = format!("git+{repository}?rev={revision}#{revision}");
        let locked = self.cargo_lock(contents)?;
        let allowed = self.allowed_packages(&locked.packages);
        let packages: BTreeMap<_, _> = metadata
            .packages
            .iter()
            .map(|package| (package.id.clone(), package))
            .collect();
        let resolve = metadata
            .resolve
            .as_ref()
            .ok_or_else(|| eyre!("framework verification requires a resolved Cargo graph"))?;
        let nodes: BTreeMap<_, _> = resolve
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        let is_framework_source = |package: &cargo_metadata::Package| {
            package
                .source
                .as_ref()
                .is_some_and(|candidate| candidate.repr == source)
        };
        if !metadata.packages.iter().any(is_framework_source) {
            return Err(self.unresolved_revision_error(metadata, repository, revision));
        }
        let mut pending: Vec<_> = metadata
            .packages
            .iter()
            .filter(|package| {
                is_framework_source(package) || self.packages.contains_key(package.name.as_str())
            })
            .map(|package| package.id.clone())
            .collect();
        let mut visited = BTreeSet::new();
        let mut conflicts = Vec::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id.clone()) {
                continue;
            }
            let package = packages[&id];
            let identity = LockedPackage {
                name: package.name.to_string(),
                version: package.version.to_string(),
                source: package.source.as_ref().map(|source| source.repr.clone()),
            };
            // A conflict is a resolved package that took a locked package's
            // place — the same predicate the generated build applies below —
            // not a package the resolution merely added beside it (#203). An
            // extracted crate carries no `Water.lock` entry the predicate
            // could see, so it stays held to its declared pin directly.
            if package.source.is_some()
                && (replaces_locked_package(
                    &allowed,
                    &package.name,
                    &package.version,
                    package.source.as_ref().map(|source| source.repr.as_str()),
                ) || self.unsanctioned_extracted(&identity))
            {
                conflicts.push(identity);
            }
            pending.extend(nodes[&id].dependencies.iter().cloned());
        }
        // Report the whole divergent set at once: a resolution that moved
        // several pinned packages is one conflict, not a sequence of
        // one-package-at-a-time errors (#177).
        if !conflicts.is_empty() {
            conflicts.sort();
            bail!(
                "framework dependencies conflict with Water.lock: {}; select a compatible channel explicitly",
                conflicts
                    .iter()
                    .map(|package| format!("{} {}", package.name, package.version))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Ok(())
    }

    /// The error a project resolving none of its selected framework source
    /// hits: the resolution carried the framework repository at another
    /// revision, or at no git revision at all. Name the selected revision,
    /// what the resolved lock carries and the `water channel` command that
    /// re-pins the project — the shape the lock-divergence errors above
    /// take.
    fn unresolved_revision_error(
        &self,
        metadata: &cargo_metadata::Metadata,
        repository: &str,
        revision: &str,
    ) -> eyre::Report {
        let revisions: Vec<String> = metadata
            .packages
            .iter()
            .filter_map(|package| package.source.as_ref())
            .filter_map(|source| source.repr.parse::<cargo_lock::SourceId>().ok())
            .filter(|source| {
                source.is_git()
                    && canonical_git_url(source.url().as_str()) == canonical_git_url(repository)
            })
            .filter_map(|source| source.precise().map(str::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let resolved = match revisions.as_slice() {
            [] => "no framework revision".to_owned(),
            [revision] => format!("framework revision {revision}"),
            revisions => format!("framework revisions {}", revisions.join(", ")),
        };
        let channel = self
            .channel()
            .expect("stable and local selections return early");
        eyre!(
            "the project does not resolve its selected framework revision {revision}; \
             the resolved lock carries {resolved} for the framework repository; \
             run `water channel {channel}` to reconcile the project"
        )
    }

    /// The error a `Water.lock` whose contents no longer hash to the
    /// checksum the selection records hits — a hand edit, or a merge that
    /// took `Water.toml` from one side and `Water.lock` from the other.
    /// Name the channel and the selected revision, the recorded and the
    /// found checksums, and the `water channel` command that writes the
    /// pair again — the same shape `unresolved_revision_error` takes.
    fn lock_mismatch_error(&self, revision: &str, expected: &str, found: &str) -> eyre::Report {
        let channel = self
            .channel()
            .expect("stable and local selections return early");
        // `--rev` pins a commit of the dev channel only; the certified
        // channels re-resolve their own exact revision.
        let reconcile = match channel {
            FrameworkChannel::Dev => format!("water channel {channel} --rev {revision}"),
            FrameworkChannel::Nightly | FrameworkChannel::Stable => {
                format!("water channel {channel}")
            }
        };
        eyre!(
            "Water.lock does not match the selected framework revision {revision} \
             on the {channel} channel; Water.toml records the lock checksum \
             {expected} and the file on disk hashes to {found}; run `{reconcile}` \
             to rewrite Water.lock for the selected revision"
        )
    }

    pub(crate) async fn prepare_build(
        &self,
        project: &Project,
        directory: &std::path::Path,
        features: &[String],
    ) -> Result<()> {
        self.validate_cli()?;
        let project_lock: Lockfile = smol::fs::read_to_string(project.lockfile_path().await?)
            .await?
            .parse()?;
        // Cargo resolves a member's lockfile at the workspace root, so the
        // seed has to land there — a `Cargo.lock` written into a member
        // directory (a preview module under `managed_backends/ffi/modules`)
        // is never read (#197).
        let workspace_root = {
            let manifest_dir = directory.to_path_buf();
            smol::unblock(move || {
                cargo_metadata::MetadataCommand::new()
                    .current_dir(manifest_dir)
                    .no_deps()
                    .exec()
            })
            .await?
            .workspace_root
            .into_std_path_buf()
        };
        let lock_path = workspace_root.join("Cargo.lock");
        let previous: Option<Lockfile> = match smol::fs::read_to_string(&lock_path).await {
            Ok(contents) => Some(contents.parse()?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let allow_new = previous.as_ref().is_none_or(|previous| {
            let previous: BTreeSet<_> = previous
                .packages
                .iter()
                .map(LockedDependency::from)
                .collect();
            project_lock
                .packages
                .iter()
                .filter(|package| package.name.as_str() == "waterui")
                .any(|package| !previous.contains(&LockedDependency::from(package)))
        });
        let canonical = if self.channel() == Some(FrameworkChannel::Stable) {
            None
        } else {
            Some(smol::fs::read(project.root().join("Water.lock")).await?)
        };
        let canonical_lock = canonical
            .as_deref()
            .map(|contents| self.cargo_lock(contents))
            .transpose()?;
        // Every identity an input lock records is an acceptable resolution
        // outcome — a second version beside a locked one is an addition, not
        // a change — so `allowed` still sees all three locks.
        let allowed = self.allowed_packages(
            previous
                .iter()
                .flat_map(|lock| lock.packages.iter())
                .chain(canonical_lock.iter().flat_map(|lock| lock.packages.iter()))
                .chain(project_lock.packages.iter()),
        );
        let packages = seed_packages(canonical_lock.as_ref(), &project_lock, previous.as_ref());
        let mut seed = project_lock;
        seed.packages = packages;
        smol::fs::write(&lock_path, seed.to_string()).await?;
        let root = directory.to_path_buf();
        let features = features.to_vec();
        let result = async {
            // A previous lock resolved without the canonical pins can carry a
            // generation the seeded locks contradict — an `accesskit_winit`
            // wanting an `accesskit` newer than the pin the channel certifies
            // (#203). That is the project's state, not something to resolve
            // past: name it and say how the seed is regenerated.
            let metadata = managed_crate_metadata(&root, &features)
                .await
                .map_err(|error| {
                    eyre!(
                        "the managed crate's lock at {} conflicts with the channel's pins and does not resolve ({error}); remove it and `Cargo.lock.seed` and build again to regenerate them from the channel's resolution",
                        lock_path.display()
                    )
                })?;
            validate_resolved_cli(&metadata)?;
            if !allow_new {
                for package in &metadata.packages {
                    if package.source.is_some()
                        && replaces_locked_package(
                            &allowed,
                            &package.name,
                            &package.version,
                            package.source.as_ref().map(|source| source.repr.as_str()),
                        )
                    {
                        bail!("generated build would change locked dependency {}; update the framework channel explicitly", package.name);
                    }
                }
            }
            if let Some(canonical) = canonical {
                self.validate_dependencies(&metadata, &canonical)?;
            }
            Ok(())
        }.await;
        if let Err(error) = &result {
            let restore = if let Some(previous) = previous {
                smol::fs::write(&lock_path, previous.to_string()).await
            } else {
                smol::fs::remove_file(&lock_path).await
            };
            restore.wrap_err_with(|| {
                format!("failed to preserve the previous lock after resolution failed: {error}")
            })?;
        }
        result
    }

    /// The channel's certified lock — `Water.lock` at the pinned revision,
    /// parsed with the framework's own source pin — where the channel carries
    /// one. `stable` resolves the application's own `Cargo.lock`, so it returns
    /// `None`. A local checkout certifies nothing either, but its own
    /// `Cargo.lock` is still the pin the checkout resolved its transitive
    /// dependencies to: without it a generated workspace resolves the
    /// packages only it reaches — the video stack's `hyper-util`,
    /// `rustls-platform-verifier`, `xcb` — at whatever the registry holds
    /// newest, and the lock guard then rejects the drift. Only the sourced
    /// entries apply: a source-less workspace member resolves by path inside
    /// the generated workspace, where a seed entry would name nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when `Water.lock` (or a local checkout's `Cargo.lock`)
    /// cannot be read or no longer matches the pinned revision's checksum.
    pub(crate) async fn canonical_lock(&self, project_root: &Path) -> Result<Option<Lockfile>> {
        if let Source::Local { root } = &self.source {
            let contents = smol::fs::read(root.join("Cargo.lock")).await?;
            let mut lock: Lockfile = std::str::from_utf8(&contents)?.parse()?;
            lock.packages.retain(|package| package.source.is_some());
            return Ok(Some(lock));
        }
        if self.channel() != Some(FrameworkChannel::Dev)
            && self.channel() != Some(FrameworkChannel::Nightly)
        {
            return Ok(None);
        }
        let contents = smol::fs::read(project_root.join("Water.lock")).await?;
        Ok(Some(self.cargo_lock(&contents)?))
    }

    pub(crate) fn cargo_lock(&self, contents: &[u8]) -> Result<Lockfile> {
        let (repository, revision, expected) = match &self.source {
            Source::Stable { .. } => bail!("stable uses the application's Cargo.lock"),
            Source::Local { .. } => {
                bail!("a local framework checkout has no canonical lock")
            }
            Source::Dev {
                repository,
                revision,
                lock_sha256,
            }
            | Source::Nightly {
                repository,
                revision,
                lock_sha256,
                ..
            } => (repository, revision, lock_sha256),
        };
        let found = hex::encode(Sha256::digest(contents));
        if found != *expected {
            return Err(self.lock_mismatch_error(revision, expected, &found));
        }
        let mut lock: Lockfile = std::str::from_utf8(contents)?.parse()?;
        annotate_workspace_lock(&mut lock, repository, revision)?;
        Ok(lock)
    }

    /// The `(repository, revision)` a channel framework resolves its packages
    /// from — `None` on the stable channel, which resolves from the registry,
    /// and on a local checkout, which resolves by path.
    ///
    /// Generated crates use this to point `[patch]` entries the framework's own
    /// table does not carry at the same source the framework resolves to.
    pub(crate) const fn git_source(&self) -> Option<(&str, &str)> {
        match &self.source {
            Source::Stable { .. } | Source::Local { .. } => None,
            Source::Dev {
                repository,
                revision,
                ..
            }
            | Source::Nightly {
                repository,
                revision,
                ..
            } => Some((repository.as_str(), revision.as_str())),
        }
    }

    /// The locked identities a generated project's resolution may produce
    /// for each recorded package: the recorded one, plus — for a crate the
    /// patch tables pin to a repository of its own — the same package at the
    /// pin and at the framework's own source. Cargo vendors a git
    /// dependency's submodules, so a submodule crate's path edges resolve
    /// inside the framework's source while its `[patch]` edge resolves at
    /// the submodule repository — the same commit either way (#807).
    fn allowed_packages<'p>(
        &self,
        packages: impl IntoIterator<Item = &'p cargo_lock::Package>,
    ) -> BTreeSet<LockedPackage> {
        let mut allowed = BTreeSet::new();
        let Some((repository, revision)) = self.git_source() else {
            return packages.into_iter().map(LockedPackage::from).collect();
        };
        let framework_source = format!("git+{repository}?rev={revision}#{revision}");
        // Crate name → the `git+<repo>?rev=<rev>` source its patch pins it to.
        let pinned: BTreeMap<&str, String> = self
            .patches
            .values()
            .flatten()
            .filter_map(|(name, dependency)| {
                let Dependency::Detailed(detail) = dependency else {
                    return None;
                };
                let (git, rev) = detail.git.as_deref().zip(detail.rev.as_deref())?;
                Some((name.as_str(), format!("git+{git}?rev={rev}#{rev}")))
            })
            .collect();
        for package in packages {
            let identity = LockedPackage::from(package);
            if let Some(source) = &identity.source
                && let Some(pin) = pinned.get(identity.name.as_str())
            {
                if source == &framework_source {
                    allowed.insert(LockedPackage {
                        source: Some(pin.clone()),
                        ..identity.clone()
                    });
                } else if source == pin {
                    allowed.insert(LockedPackage {
                        source: Some(framework_source.clone()),
                        ..identity.clone()
                    });
                }
            }
            allowed.insert(identity);
        }
        allowed
    }

    /// Whether `identity` names a scaffold's extracted crate resolving at
    /// anything but its declared source. An extracted crate carries no
    /// `Water.lock` entry, so `replaces_locked_package` cannot see the drift.
    fn unsanctioned_extracted(&self, identity: &LockedPackage) -> bool {
        self.packages.contains_key(identity.name.as_str()) && !self.sanctioned_source(identity)
    }

    /// Whether `identity` resolves a scaffold package at the source its
    /// declared requirement sanctions — the declared `git + rev`, or the
    /// registry at the pinned `=version`. An extracted crate never enters
    /// `Water.lock`; the declared pin is the certification of what it must
    /// resolve to.
    fn sanctioned_source(&self, identity: &LockedPackage) -> bool {
        let Some(detail) = self.packages.get(identity.name.as_str()) else {
            return false;
        };
        let Some(source) = &identity.source else {
            return false;
        };
        if let (Some(git), Some(rev)) = (&detail.git, &detail.rev) {
            let Ok(source) = source.parse::<cargo_lock::SourceId>() else {
                return false;
            };
            let declared = cargo_lock::package::GitReference::Rev(rev.clone());
            return source.is_git()
                && source.git_reference() == Some(&declared)
                && canonical_git_url(source.url().as_str()) == canonical_git_url(git);
        }
        source.as_str() == "registry+https://github.com/rust-lang/crates.io-index"
            && detail.version.as_ref().is_some_and(|requirement| {
                identity
                    .version
                    .parse::<cargo_toml::SemVer>()
                    .is_ok_and(|version| requirement.matches(&version))
            })
    }

    pub(crate) fn dependency(&self, name: &str) -> DependencyDetail {
        match &self.source {
            Source::Stable { .. } => {
                let requirement = self.scaffold_value(&format!("{name}-version"));
                // The registry substitutes for a declared git pin only once
                // the workspace names the crate by version alone.
                let git = self.scaffold.get(&format!("{name}-git"));
                DependencyDetail {
                    version: Some(
                        git.map_or_else(|| format!("={requirement}"), |_| requirement.to_owned())
                            .parse()
                            .expect("resolved package version is valid"),
                    ),
                    git: git.cloned(),
                    rev: git.map(|_| self.scaffold_value(&format!("{name}-rev")).to_owned()),
                    ..Default::default()
                }
            }
            Source::Dev { .. } | Source::Nightly { .. } => self.packages[name].clone(),
            Source::Local { .. } => {
                unreachable!("a local checkout resolves framework crates by path")
            }
        }
    }

    /// Resolve a channel's exact framework selection.
    ///
    /// `dev` resolves the integration branch head once it has passed its
    /// compilation gate; `nightly` and `stable` resolve the newest eligible
    /// GitHub release carrying a `framework.json` — a published `nightly-*`
    /// prerelease, a published `v<semver>` release — and pin what it
    /// certifies. `rev` pins `dev` to an exact commit of the branch's own
    /// history instead of its head; a certified channel is already exact and
    /// rejects the pin.
    ///
    /// # Errors
    /// Returns an error when the channel has no eligible release, the manifest
    /// fails verification, or the certified revision cannot be fetched.
    pub(crate) async fn resolve(
        channel: FrameworkChannel,
        rev: Option<&str>,
    ) -> Result<(Self, Option<Vec<u8>>)> {
        let repository = framework_repository();
        let slug = repository_slug(repository)?;
        match channel {
            FrameworkChannel::Stable | FrameworkChannel::Nightly => {
                if rev.is_some() {
                    bail!(
                        "--rev pins a commit of the dev channel; a {channel} release is already an exact revision"
                    );
                }
                let certification = latest_certification(repository, channel).await?;
                let revision = certification.revision.clone();
                Self::construct(repository, slug, &revision, Some(certification)).await
            }
            FrameworkChannel::Dev => {
                let revision = match rev {
                    Some(rev) => resolve_dev_at(repository, slug, rev).await?,
                    None => resolve_dev(repository, slug).await?,
                };
                Self::construct(repository, slug, &revision, None).await
            }
        }
    }

    /// Build the resolved selection for the framework tree at `revision`, plus
    /// the certification a certified channel carries.
    ///
    /// Every channel shares this path: the fetched root manifest supplies the
    /// scaffold requirements and framework metadata, the fetched lock the
    /// workspace versions, and the submodule pins — the certification's record
    /// for a certified channel, the repository's gitlinks for `dev` — the
    /// backend revisions. The certification is then held to the tree it names:
    /// its scaffold table must agree with the manifest's, its lock hash with
    /// the fetched lock.
    async fn construct(
        repository: &str,
        slug: &str,
        revision: &str,
        certification: Option<Certification>,
    ) -> Result<(Self, Option<Vec<u8>>)> {
        validate_revision(revision)?;
        let base = format!("https://raw.githubusercontent.com/{slug}/{revision}");
        let manifest_bytes = fetch(&format!("{base}/Cargo.toml")).await?;
        let root: toml::Value = toml::from_str(std::str::from_utf8(&manifest_bytes)?)?;
        let metadata = framework_metadata(&root)?;
        let minimum_cli_version = minimum_cli_version(&metadata)?;
        let rust_version = manifest_rust_version(&root)?;
        let mut scaffold = framework_scaffold(&root)?;
        let lock_bytes = fetch(&format!("{base}/Cargo.lock")).await?;
        let lock_sha256 = hex::encode(Sha256::digest(&lock_bytes));
        let lock: Lockfile = std::str::from_utf8(&lock_bytes)?.parse()?;

        let channel = certification
            .as_ref()
            .map_or(FrameworkChannel::Dev, |certification| certification.channel);
        // A scaffold package the framework pins to a git revision is not
        // one the stable channel distributes: its scaffold entries are
        // withheld and the pin recorded under `experimental-packages`, the
        // same split `channel_scaffold` in `framework_manifest.py` makes for
        // the manifest. `dev`/`nightly` distribute it through `scaffold`.
        let experimental_packages = if channel == FrameworkChannel::Stable {
            split_experimental_packages(&mut scaffold)
        } else {
            BTreeMap::new()
        };
        if let Some(minimum) = &minimum_cli_version {
            let update = match channel {
                FrameworkChannel::Stable => registry_cli_update(minimum),
                FrameworkChannel::Dev | FrameworkChannel::Nightly => checkout_cli_update(),
            };
            validate_installed_cli(minimum, &update)?;
        }

        // `.gitmodules` names each submodule path's repository at the
        // revision; the pin's commit half comes from the tree's gitlinks
        // (`dev`) or the certification (a certified channel). `stable`
        // resolves from the registry and carries neither.
        let submodule_repositories = match channel {
            FrameworkChannel::Stable => BTreeMap::new(),
            FrameworkChannel::Dev | FrameworkChannel::Nightly => {
                match fetch_optional(&format!("{base}/.gitmodules")).await? {
                    Some(bytes) => parse_gitmodules(std::str::from_utf8(&bytes)?),
                    // Every submodule was extracted; the revision records none.
                    None => BTreeMap::new(),
                }
            }
        };

        let (mut source, submodules) = if let Some(certification) = &certification {
            (
                certified_source(
                    certification,
                    repository,
                    revision,
                    &metadata,
                    &scaffold,
                    &experimental_packages,
                    &lock_sha256,
                )?,
                certification.submodules.clone(),
            )
        } else {
            (
                Source::Dev {
                    repository: repository.to_owned(),
                    revision: revision.to_owned(),
                    lock_sha256,
                },
                dev_submodules(slug, revision, &submodule_repositories).await?,
            )
        };
        complete_scaffold(&mut scaffold, &lock)?;

        let (packages, patches, lockfile) = match channel {
            // A stable project resolves its graph from the registry; nothing is
            // pinned to the framework repository, so there is no package detail
            // or canonical lock to persist.
            FrameworkChannel::Stable => (
                BTreeMap::new(),
                stable_member_substitutions(&root, &lock, repository, revision, &metadata)?,
                None,
            ),
            FrameworkChannel::Dev | FrameworkChannel::Nightly => {
                let patches: PatchSet = root
                    .get("patch")
                    .cloned()
                    .map(toml::Value::try_into)
                    .transpose()?
                    .unwrap_or_default();
                // A path under a submodule belongs to the submodule's
                // repository at the pinned commit, not the superproject's —
                // whose tree holds a gitlink there, not the crate.
                let pins = submodule_pins(submodule_repositories, &submodules);
                let mut patches = rebase_patches_onto_source(patches, repository, revision, &pins);
                patch_framework_members(&mut patches, &lock, repository, revision);
                let packages = resolve_packages(&scaffold, &lock, repository, revision)?;

                let foreign = foreign_locked_packages(&lock, &packages, repository).await?;
                let lockfile = merge_foreign_lock(&lock, lock_bytes, foreign, &mut source);
                (packages, patches, Some(lockfile))
            }
        };
        Ok((
            Self {
                source,
                minimum_cli_version,
                rust_version,
                metadata,
                scaffold,
                experimental_packages,
                packages,
                patches,
            }
            .validated()?,
            lockfile,
        ))
    }
}

/// Resolve a managed crate's dependency metadata for the feature selection
/// the build was invoked with.
async fn managed_crate_metadata(
    root: &std::path::Path,
    features: &[String],
) -> std::result::Result<cargo_metadata::Metadata, cargo_metadata::Error> {
    let root = root.to_path_buf();
    let features = features.to_vec();
    smol::unblock(move || {
        cargo_metadata::MetadataCommand::new()
            .current_dir(root)
            .features(cargo_metadata::CargoOpt::SomeFeatures(features))
            .exec()
    })
    .await
}

/// The framework lock only covers its own workspace; an extracted backend
/// pins its own repository revision whose graph (`winit`, `accesskit_winit`,
/// `redox_syscall`, …) no framework entry names. Fold each pinned
/// repository's own lock packages into the channel's — after the
/// certification checked the framework's bytes — so `Water.lock` seeds and
/// gates the backend graphs a generated project actually resolves
/// (water-rs/cli#197). The `Source`'s recorded lock hash follows the merged
/// bytes.
fn merge_foreign_lock(
    lock: &Lockfile,
    lock_bytes: Vec<u8>,
    foreign: Vec<cargo_lock::Package>,
    source: &mut Source,
) -> Vec<u8> {
    if foreign.is_empty() {
        return lock_bytes;
    }
    let mut merged = lock.clone();
    merged.packages.extend(foreign);
    merged.packages.sort_by(|left, right| {
        left.name
            .as_str()
            .cmp(right.name.as_str())
            .then(left.version.cmp(&right.version))
    });
    let merged_bytes = merged.to_string().into_bytes();
    if let Source::Dev { lock_sha256, .. } | Source::Nightly { lock_sha256, .. } = source {
        *lock_sha256 = hex::encode(Sha256::digest(&merged_bytes));
    }
    merged_bytes
}

/// The packages the generated crate's `Cargo.lock` seed carries.
///
/// The seed is one resolution, not a union of locks: a name the pinned
/// framework lock records resolves only to the identities the channel
/// certifies, the project lock supplies every name the framework does
/// not know, and the previous generated lock fills what neither names so
/// packages only the managed crate adds stay put. Unioning the three
/// keyed on the locked identity let a canonical name enter twice at
/// divergent resolutions — the `wasm-bindgen`/`js-sys` lockstep split of
/// #177 — and no resolution of the generated crate could then satisfy
/// `Water.lock`: the divergent candidate either moved a shared edge off
/// its pin or contradicted the pair a fresh version requires. Names the
/// canonical lock does not record are exempt — the project may carry any
/// version of a package the framework never names.
pub(crate) fn seed_packages(
    canonical: Option<&Lockfile>,
    project: &Lockfile,
    previous: Option<&Lockfile>,
) -> Vec<cargo_lock::Package> {
    let canonical_names: BTreeSet<&str> = canonical
        .into_iter()
        .flat_map(|lock| lock.packages.iter().map(|package| package.name.as_str()))
        .collect();
    let mut packages: Vec<cargo_lock::Package> = canonical
        .into_iter()
        .flat_map(|lock| lock.packages.iter().cloned())
        .collect();
    let mut rest: BTreeMap<LockedDependency, cargo_lock::Package> = BTreeMap::new();
    for package in project
        .packages
        .iter()
        .chain(previous.into_iter().flat_map(|lock| lock.packages.iter()))
    {
        if !canonical_names.contains(package.name.as_str()) {
            rest.insert(LockedDependency::from(package), package.clone());
        }
    }
    packages.extend(rest.into_values());
    packages
}

/// `dev` has no certification; the repository tree's own gitlinks record which
/// submodule revisions the revision was built against — every submodule
/// `.gitmodules` names.
async fn dev_submodules(
    slug: &str,
    revision: &str,
    submodule_repositories: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let mut submodules = BTreeMap::new();
    // The `.gitmodules` entries (`kit`, `utils/nami`, …) pin no scaffold
    // fact, but a `[patch]` path under one rebases onto the submodule's
    // repository at the gitlink's commit — the same record the
    // certification supplies for `nightly`.
    for path in submodule_repositories.keys() {
        if let Some(commit) = submodule_pin(slug, revision, path).await? {
            submodules.insert(path.clone(), commit);
        }
    }
    Ok(submodules)
}

/// Marry each `.gitmodules` path's repository URL to its recorded commit, the
/// pin a `[patch]` path under it rebases onto.
fn submodule_pins(
    repositories: BTreeMap<String, String>,
    submodules: &BTreeMap<String, String>,
) -> BTreeMap<String, SubmodulePin> {
    repositories
        .into_iter()
        .filter_map(|(path, url)| {
            submodules.get(&path).map(|commit| {
                (
                    path,
                    SubmodulePin {
                        repository: canonical_git_url(&url).to_owned(),
                        commit: commit.clone(),
                    },
                )
            })
        })
        .collect()
}

/// The persisted source a certification proves, checked against the tree it
/// names: the certified scaffold table, metadata, CLI floor and lock hash must
/// all agree with the fetched framework manifest before the release pin is
/// trusted.
fn certified_source(
    certification: &Certification,
    repository: &str,
    revision: &str,
    metadata: &toml::Table,
    scaffold: &BTreeMap<String, String>,
    experimental_packages: &BTreeMap<String, ExperimentalPackage>,
    lock_sha256: &str,
) -> Result<Source> {
    let channel = certification.channel;
    if certification.metadata != *metadata {
        bail!("{channel} framework metadata does not match its certification");
    }
    for (key, value) in scaffold {
        if certification.scaffold.get(key) != Some(value) {
            bail!("{channel} certification scaffold `{key}` does not match the framework manifest");
        }
    }
    // The withheld set must agree too: a stable manifest may neither drop a
    // git-pinned package silently nor leave it in `scaffold` while also
    // recording it as experimental.
    if certification.experimental_packages != *experimental_packages {
        bail!("{channel} certification experimental packages do not match the framework manifest");
    }
    for name in experimental_packages.keys() {
        if certification
            .scaffold
            .contains_key(&format!("{name}-version"))
        {
            bail!("{channel} certification scaffold `{name}-version` names a withheld package");
        }
    }
    let expected = certification
        .lockfiles
        .get("Cargo.lock")
        .ok_or_else(|| eyre!("{channel} certification has no dependency lock"))?;
    if lock_sha256 != *expected {
        bail!("{channel} dependency lock does not match its certification");
    }
    let release = FrameworkRelease {
        repository: repository.to_owned(),
        revision: revision.to_owned(),
        tag: certification.tag.clone(),
    };
    Ok(match certification.channel {
        FrameworkChannel::Stable => Source::Stable {
            release: Some(release),
        },
        FrameworkChannel::Nightly => Source::Nightly {
            repository: repository.to_owned(),
            revision: revision.to_owned(),
            tag: certification.tag.clone(),
            lock_sha256: lock_sha256.to_owned(),
        },
        FrameworkChannel::Dev => unreachable!("verify_certification rejects a dev manifest"),
    })
}

/// The workspace crates a scaffolded project pins; each `{name}-version`
/// scaffold entry comes from the framework's own lockfile at the selected
/// revision.
/// A crate the framework workspace carries in-tree — never a scaffold
/// package with a requirement of its own. The manifest declares it through a
/// `{path_key}` `[package.metadata.waterui]` entry naming its directory, and
/// every channel resolves it as a member of the selected framework source:
/// `dev`/`nightly`/`stable` pin the framework repository at the selected
/// revision, a `waterui_path` checkout supplies it by path at the canonical
/// `subdirectory` slot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameworkMember {
    /// The `[package.metadata.waterui]` key declaring the member's directory
    /// inside the framework tree.
    pub(crate) path_key: &'static str,
    /// The crate the member carries.
    pub(crate) package: &'static str,
    /// The member's canonical directory inside the checkout — the slot a
    /// `waterui_path` checkout supplies.
    pub(crate) subdirectory: &'static str,
}

/// The objc2 Rust backend, `waterui-apple` at `backends/apple`.
pub(crate) const APPLE_BACKEND: FrameworkMember = FrameworkMember {
    path_key: "apple-backend-path",
    package: "waterui-apple",
    subdirectory: "backends/apple",
};

/// The self-drawn Hydrolysis backend, `hydrolysis` at `backends/hydrolysis`
/// (#1635).
pub(crate) const HYDROLYSIS: FrameworkMember = FrameworkMember {
    path_key: "hydrolysis-path",
    package: "hydrolysis",
    subdirectory: "backends/hydrolysis",
};

/// Every crate resolved through a `{name}-path` member declaration.
pub(crate) const FRAMEWORK_MEMBERS: &[FrameworkMember] = &[APPLE_BACKEND, HYDROLYSIS];

const FRAMEWORK_PACKAGES: &[&str] = &[
    "waterui",
    "waterui-core",
    "waterui-testing",
    "waterui-ffi",
    "waterui-locale",
    "waterui-browser-cef",
    "waterui-preview",
    "waterui-preview-protocol",
    "waterui-mcp",
];

/// The repository the CLI's pinned `waterui-*` dependencies resolve from —
/// where certified manifests, releases, and `dev` revisions live. `build.rs`
/// bakes it in from the git source in `Cargo.toml` so the pin is declared
/// exactly once.
pub(crate) fn framework_repository() -> &'static str {
    env!("WATERUI_FRAMEWORK_REPOSITORY").trim_end_matches(".git")
}

/// A repository's `owner/name` slug, from its GitHub URL.
fn repository_slug(repository: &str) -> Result<&str> {
    repository
        .strip_prefix("https://github.com/")
        .ok_or_else(|| eyre!("{repository} must identify its GitHub source"))
}

/// The framework's own metadata table — `[package.metadata.waterui]` of the
/// manifest at the selected revision — carried verbatim into every published
/// `framework.json` and every persisted selection.
fn framework_metadata(manifest: &toml::Value) -> Result<toml::Table> {
    manifest
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("waterui"))
        .map_or_else(
            || Ok(toml::Table::new()),
            |metadata| {
                metadata
                    .clone()
                    .try_into()
                    .wrap_err("invalid package.metadata.waterui metadata")
            },
        )
}

/// The scaffold facts the framework manifest itself declares: each
/// `scaffold-packages` entry's requirement from `[workspace.dependencies]` —
/// `{name}-version`, plus `{name}-git` and `{name}-rev` when the requirement
/// pins a repository — and every backend coordinate — `{name}-backend-url`,
/// plus the `{name}-backend-version` of a backend pinned by release or the
/// `{name}-backend-revision` of one pinned by commit, rather than by
/// gitlink — and every in-tree member declaration — `{name}-path`, the
/// crate directory a `{name}-backend-path` names for the native backends —
/// and the `{name}-host-subdirectory` of the Android host the framework
/// tree itself carries — from `[package.metadata.waterui]`.
///
/// `framework_manifest.py` emits exactly this table into every `framework.json`
/// it publishes; both must produce the same table for the same tree.
fn framework_scaffold(manifest: &toml::Value) -> Result<BTreeMap<String, String>> {
    let metadata = framework_metadata(manifest)?;
    let workspace = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table)
        .ok_or_else(|| eyre!("framework manifest has no [workspace.dependencies]"))?;
    let packages = metadata
        .get("scaffold-packages")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| {
            eyre!("framework manifest has no package.metadata.waterui.scaffold-packages")
        })?;
    let mut scaffold = BTreeMap::new();
    for package in packages {
        let name = package.as_str().ok_or_else(|| {
            eyre!("package.metadata.waterui.scaffold-packages entries must be crate names")
        })?;
        let dependency = workspace.get(name).ok_or_else(|| {
            eyre!("scaffold package {name} has no [workspace.dependencies] requirement")
        })?;
        let requirement = dependency
            .as_str()
            .or_else(|| dependency.get("version").and_then(toml::Value::as_str))
            .ok_or_else(|| eyre!("workspace.dependencies.{name} declares no version"))?;
        scaffold.insert(format!("{name}-version"), requirement.to_owned());
        // A scaffold package pinned from git keeps that source: a bare
        // `{name}-version` cannot express the commit the framework builds
        // against, and the registry may not carry it at all.
        if let Some(git) = dependency.get("git").and_then(toml::Value::as_str) {
            let revision = dependency
                .get("rev")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| {
                    eyre!("workspace.dependencies.{name} must pin an immutable Git revision")
                })?;
            validate_revision(revision)
                .wrap_err_with(|| format!("workspace.dependencies.{name}.rev"))?;
            scaffold.insert(format!("{name}-git"), git.to_owned());
            scaffold.insert(format!("{name}-rev"), revision.to_owned());
        }
    }
    for (key, value) in &metadata {
        if !(key.ends_with("-backend-url")
            || key.ends_with("-backend-version")
            || key.ends_with("-backend-revision")
            || key.ends_with("-path")
            || key.ends_with("-host-subdirectory"))
        {
            continue;
        }
        let value = value
            .as_str()
            .ok_or_else(|| eyre!("package.metadata.waterui.{key} must be a string"))?;
        if key.ends_with("-backend-revision") {
            validate_revision(value).wrap_err_with(|| format!("package.metadata.waterui.{key}"))?;
        }
        if key.ends_with("-host-subdirectory") || key.ends_with("-path") {
            validate_host_subdirectory(value)
                .wrap_err_with(|| format!("package.metadata.waterui.{key}"))?;
        }
        scaffold.insert(key.clone(), value.to_owned());
    }
    Ok(scaffold)
}

/// Move every git-pinned scaffold package out of `scaffold` — the split
/// `channel_scaffold` in `framework_manifest.py` makes for `stable`: a
/// package the framework pins to a git revision is not one the channel
/// distributes, so its `{name}-*` entries leave the scaffold table and the pin
/// is recorded by name instead.
fn split_experimental_packages(
    scaffold: &mut BTreeMap<String, String>,
) -> BTreeMap<String, ExperimentalPackage> {
    let mut experimental = BTreeMap::new();
    // `-git` is the marker: a `{name}-git` scaffold entry is a git pin the
    // stable channel withholds; its `-rev`/`-version` siblings are lifted out in
    // the second pass.
    for (key, value) in std::mem::take(scaffold) {
        if let Some(name) = key.strip_suffix("-git") {
            experimental.insert(
                name.to_owned(),
                ExperimentalPackage {
                    version: String::new(),
                    git: value,
                    rev: String::new(),
                },
            );
        } else {
            scaffold.insert(key, value);
        }
    }
    for (name, package) in &mut experimental {
        package.rev = scaffold
            .remove(&format!("{name}-rev"))
            .expect("a `-git` scaffold entry carries `-rev`");
        package.version = scaffold
            .remove(&format!("{name}-version"))
            .expect("a `-git` scaffold entry carries `-version`");
    }
    experimental
}

/// The Rust floor a root manifest declares — `[workspace.package].rust-version`,
/// or `[package].rust-version` when the manifest is a plain package. Shared by
/// framework resolution (the framework's own manifest at the selected
/// revision) and the doctor (the project's and a local checkout's manifests).
pub(crate) fn manifest_rust_version(manifest: &toml::Value) -> Result<Option<cargo_toml::SemVer>> {
    let declared = manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("rust-version"))
        .or_else(|| {
            manifest
                .get("package")
                .and_then(|package| package.get("rust-version"))
        });
    declared
        .map(|value| {
            let text = value
                .as_str()
                .ok_or_else(|| eyre!("rust-version must be a string"))?;
            crate::utils::parse_semver_version(text).wrap_err("invalid rust-version")
        })
        .transpose()
}

/// The CLI floor a `package.metadata.waterui` metadata table declares —
/// read the same way from a checked-out root manifest and from a
/// certification's `metadata` table.
fn minimum_cli_version(metadata: &toml::Table) -> Result<Option<cargo_toml::SemVer>> {
    metadata
        .get("minimum-cli-version")
        .cloned()
        .map(toml::Value::try_into)
        .transpose()
        .wrap_err("invalid package.metadata.waterui.minimum-cli-version")
}

/// The CLI update hint for a framework that is not a registry release — a
/// local checkout or a git-pinned `dev`/`nightly` source pairs with the
/// development line of this repository.
fn checkout_cli_update() -> String {
    format!(
        "cargo install {} --git {} --locked",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_REPOSITORY")
    )
}

fn registry_cli_update(minimum: &cargo_toml::SemVer) -> String {
    format!(
        "cargo install {} --version '>={minimum}' --locked",
        env!("CARGO_PKG_NAME")
    )
}

fn validate_installed_cli(minimum: &cargo_toml::SemVer, update: &str) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION")
        .parse()
        .expect("CLI package version is valid");
    validate_cli_version(minimum, &current, update)
}

fn validate_cli_version(
    minimum: &cargo_toml::SemVer,
    current: &cargo_toml::SemVer,
    update: &str,
) -> Result<()> {
    if current.cmp_precedence(minimum).is_lt() {
        // The fallback command is what a registry or git source already
        // selected; the detected install source may replace it with `water
        // update` or `brew upgrade water`.
        let update = crate::self_update::cli_update_command(update);
        bail!(
            "This WaterUI framework requires waterui-cli >= {minimum}, but the running CLI is {current}.\nUpdate the CLI: {update}\nThen verify the installed version with `water --version`."
        );
    }
    Ok(())
}

pub(crate) async fn validate_local_cli(root: &Path) -> Result<()> {
    let contents = smol::fs::read_to_string(root.join("Cargo.toml")).await?;
    let manifest = toml::from_str(&contents)?;
    if let Some(minimum) = minimum_cli_version(&framework_metadata(&manifest)?)? {
        validate_installed_cli(&minimum, &checkout_cli_update())?;
    }
    Ok(())
}

pub(crate) fn validate_resolved_cli(metadata: &cargo_metadata::Metadata) -> Result<()> {
    for package in metadata
        .packages
        .iter()
        .filter(|package| package.name.as_str() == "waterui")
    {
        let Some(value) = package
            .metadata
            .get("waterui")
            .and_then(|metadata| metadata.get("minimum-cli-version"))
        else {
            continue;
        };
        let minimum: cargo_toml::SemVer = serde_json::from_value(value.clone())
            .wrap_err("invalid package.metadata.waterui.minimum-cli-version")?;
        let source = package
            .source
            .as_ref()
            .map(|source| source.repr.parse::<cargo_lock::SourceId>())
            .transpose()?;
        let update = match source {
            Some(source) if !source.is_git() => registry_cli_update(&minimum),
            Some(_) | None => checkout_cli_update(),
        };
        validate_installed_cli(&minimum, &update)?;
    }
    Ok(())
}

fn resolve_packages(
    scaffold: &BTreeMap<String, String>,
    lock: &Lockfile,
    repository: &str,
    revision: &str,
) -> Result<BTreeMap<String, DependencyDetail>> {
    let mut packages = BTreeMap::new();
    for (key, version) in scaffold {
        let Some(name) = key.strip_suffix("-version") else {
            continue;
        };
        // A `{name}-backend-version` entry pins a backend repository's release
        // tag, not a crate — there is no package to resolve for it.
        if name.ends_with("-backend") {
            continue;
        }
        // The scaffold value is a requirement, not the resolved version: a
        // declaration that names an older but still-satisfied version
        // (`"0.2.0"` where the lock carries 0.2.1) must still find the lock
        // candidate and inherit its source. String equality would take the
        // lock-absent arm and pin `=<requirement>` — a release the framework
        // never certified. `sanctioned_source` matches the same way.
        let requirement: cargo_toml::VersionReq = version.parse().wrap_err_with(|| {
            eyre!(
                "framework scaffold requirement {name} = {version:?} is not a version requirement"
            )
        })?;
        let candidates: Vec<_> = lock
            .packages
            .iter()
            .filter(|package| {
                package.name.as_str() == name && requirement.matches(&package.version)
            })
            .collect();
        // A scaffold package the workspace pins from git resolves from that
        // pin on every channel: an extracted crate never enters the framework
        // lock, and for one built in-tree the lock only witnesses that the
        // framework resolves the same commit.
        if let Some(git) = scaffold.get(&format!("{name}-git")) {
            let pinned = scaffold.get(&format!("{name}-rev")).ok_or_else(|| {
                eyre!("framework scaffold declares {name}-git without {name}-rev")
            })?;
            match candidates.as_slice() {
                [] => {}
                [package] => assert_declared_git_source(name, package, git, pinned)?,
                _ => bail!("framework lock has multiple sources for {name} {version}"),
            }
            packages.insert(
                name.to_owned(),
                DependencyDetail {
                    version: Some(version.parse()?),
                    git: Some(git.clone()),
                    rev: Some(pinned.clone()),
                    ..Default::default()
                },
            );
            continue;
        }
        let package = match candidates.as_slice() {
            [package] => *package,
            // An extracted crate the framework no longer builds never enters
            // its lock — `waterui-dew` releases from water-rs/dew (#614) — so
            // the scaffold's declared requirement is the resolution, the same
            // `=<version>` the registry-source arm below produces for a crate
            // the framework still carries.
            [] => {
                packages.insert(
                    name.to_owned(),
                    DependencyDetail {
                        version: Some(format!("={version}").parse()?),
                        ..Default::default()
                    },
                );
                continue;
            }
            _ => bail!("framework lock has multiple sources for {name} {version}"),
        };
        let mut dependency = DependencyDetail::default();
        match &package.source {
            None => {
                dependency.git = Some(repository.to_owned());
                dependency.rev = Some(revision.to_owned());
            }
            Some(source) if source.is_default_registry() => {
                dependency.version = Some(format!("={version}").parse()?);
            }
            Some(source) if source.is_git() => {
                let Some(cargo_lock::package::GitReference::Rev(revision)) = source.git_reference()
                else {
                    bail!(
                        "framework package {name} must use an immutable Git revision in the framework manifest"
                    );
                };
                validate_revision(revision)?;
                if source.precise() != Some(revision.as_str()) {
                    bail!("framework package {name} does not resolve its declared revision");
                }
                dependency.git = Some(source.url().to_string());
                dependency.rev = Some(revision.clone());
            }
            Some(source) => bail!("unsupported framework package source for {name}: {source}"),
        }
        packages.insert(name.to_owned(), dependency);
    }
    Ok(packages)
}

/// Assert `package`'s lock source is the git repository a declared
/// `{name}-git`/`{name}-rev` scaffold pair names — the witness that the
/// framework builds the same commit a scaffolded project receives.
fn assert_declared_git_source(
    name: &str,
    package: &cargo_lock::Package,
    git: &str,
    revision: &str,
) -> Result<()> {
    let Some(source) = &package.source else {
        bail!("framework package {name} is a workspace member, not the declared {git}");
    };
    let declared = cargo_lock::package::GitReference::Rev(revision.to_owned());
    if !(source.is_git()
        && source.git_reference() == Some(&declared)
        && source.precise() == Some(revision)
        && canonical_git_url(source.url().as_str()) == canonical_git_url(git))
    {
        bail!("framework package {name} resolves {source}, not the declared {git}@{revision}");
    }
    Ok(())
}

#[derive(Deserialize)]
struct SubmoduleEntry {
    sha: String,
    #[serde(rename = "type")]
    kind: String,
}

/// Fill in what the framework manifest cannot carry itself: every framework
/// package's version from the framework's own lock.
fn complete_scaffold(scaffold: &mut BTreeMap<String, String>, lock: &Lockfile) -> Result<()> {
    for &name in FRAMEWORK_PACKAGES {
        let candidates: Vec<_> = lock
            .packages
            .iter()
            .filter(|package| package.name.as_str() == name)
            .collect();
        let version = match candidates.as_slice() {
            [package] => package.version.to_string(),
            [] => bail!("framework lock has no package named {name}"),
            _ => bail!("framework lock has multiple packages named {name}"),
        };
        scaffold.insert(format!("{name}-version"), version);
    }
    Ok(())
}

/// The commit `path`'s gitlink records at `revision`, or `None` when `path`
/// is not a submodule there — a `.gitmodules` entry can outlive the gitlink
/// it once named, and the patch paths under it then belong in the tree.
async fn submodule_pin(slug: &str, revision: &str, path: &str) -> Result<Option<String>> {
    let Some(bytes) = fetch_optional(&format!(
        "https://api.github.com/repos/{slug}/contents/{path}?ref={revision}"
    ))
    .await?
    else {
        return Ok(None);
    };
    // A present-but-ordinary path lists as a directory array or carries a
    // non-submodule type; neither is a pin.
    let Ok(entry) = serde_json::from_slice::<SubmoduleEntry>(&bytes) else {
        return Ok(None);
    };
    Ok((entry.kind == "submodule").then_some(entry.sha))
}

/// The `path → url` pairs `.gitmodules` records — git-config syntax rather
/// than TOML (values go unquoted), so a line scan keyed on `[submodule]`
/// sections.
fn parse_gitmodules(contents: &str) -> BTreeMap<String, String> {
    let mut submodules = BTreeMap::new();
    let mut submodule = false;
    let mut path = None::<String>;
    let mut url = None::<String>;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') {
            if submodule && let (Some(path), Some(url)) = (path.take(), url.take()) {
                submodules.insert(path, url);
            }
            submodule = line.starts_with("[submodule");
            continue;
        }
        if !submodule {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            match key.trim() {
                "path" => path = Some(value.trim().trim_matches('"').to_owned()),
                "url" => url = Some(value.trim().trim_matches('"').to_owned()),
                _ => {}
            }
        }
    }
    if submodule && let (Some(path), Some(url)) = (path, url) {
        submodules.insert(path, url);
    }
    submodules
}

fn validate_revision(revision: &str) -> Result<()> {
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("framework revision must be a full Git commit hash");
    }
    Ok(())
}

/// Where the Hydrolysis Android host resolves from for a resolved
/// framework: the framework tree itself. The host lives in the framework
/// repository — `hydrolysis-android-host-subdirectory` names the Gradle
/// composite root inside it — so a generated project builds the host from
/// the exact revision it builds the `hydrolysis` crate from (#1428, #1635):
/// there is no second repository to pin.
#[derive(Debug, Clone, Copy)]
pub(crate) enum HydrolysisAndroidHost<'a> {
    /// A channel framework: the framework repository at the selected
    /// revision — `stable` answers the certified release's provenance.
    Git {
        /// Git URL the framework checkout is fetched from.
        url: &'a str,
        /// Full commit hash the checkout pins.
        revision: &'a str,
    },
    /// A `waterui_path` checkout: the local framework tree the host lives
    /// inside — no clone exists for a filesystem source.
    Local {
        /// Root of the framework checkout.
        root: &'a Path,
    },
}

/// A `{name}-host-subdirectory` names the Gradle root inside the host
/// checkout the generated project `includeBuild`s — a plain relative path,
/// never absolute and never escaping the checkout.
fn validate_host_subdirectory(subdirectory: &str) -> Result<()> {
    let path = Path::new(subdirectory);
    if subdirectory.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("host subdirectory must be a relative path inside the checkout");
    }
    Ok(())
}

async fn fetch(url: &str) -> Result<Vec<u8>> {
    fetch_optional(url)
        .await?
        .ok_or_else(|| eyre!("framework resolution returned HTTP 404 from {url}"))
}

/// [`fetch`] that answers `None` when the resource does not exist —
/// `.gitmodules` is absent on a revision whose submodules were all
/// extracted. zenwave surfaces a non-success status as `Err`, so the 404
/// arrives as an [`Error::Http`], never as a response to inspect.
async fn fetch_optional(url: &str) -> Result<Option<Vec<u8>>> {
    let mut client = zenwave::client();
    let mut request = client
        .method(Method::GET, url)?
        .header("User-Agent", env!("CARGO_PKG_NAME"))?;
    if let Some(token) = github_api_token(url) {
        request = request.header("Authorization", &format!("Bearer {token}"))?;
    }
    let response = match request.await {
        Ok(response) => response,
        Err(zenwave::Error::Http { status, .. }) if status == StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    Ok(Some(response.into_body().into_bytes().await?.to_vec()))
}

/// The token a GitHub REST request carries: `WATERUI_GITHUB_TOKEN`, else the
/// `GITHUB_TOKEN` every Actions job holds. Unauthenticated requests share
/// sixty an hour across every machine behind one address — CI runners and
/// cloud VMs first of all — and the same token `water update` sends. Only
/// `api.github.com` sees it; release downloads and raw file reads are not
/// metered and must not receive a credential.
fn github_api_token(url: &str) -> Option<String> {
    if !url.starts_with("https://api.github.com/") {
        return None;
    }
    ["WATERUI_GITHUB_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|token| !token.trim().is_empty())
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    use std::process::Command as StdCommand;

    use super::*;

    /// A stable-channel resolution carrying every scaffold fact the templates
    /// may read — the shape `resolve` produces, built in place because the
    /// real resolution lives on the network. `stable` withholds the git-pinned
    /// scaffold packages under `experimental-packages`, the split a published
    /// `framework.json` makes for the channel.
    pub fn stable_framework() -> ResolvedFramework {
        let revision = |seed: char| seed.to_string().repeat(40);
        let scaffold = FRAMEWORK_PACKAGES
            .iter()
            .map(|name| (format!("{name}-version"), "0.4.1".to_owned()))
            .chain([
                ("hydrolysis-m3-version".to_owned(), "0.2.0".to_owned()),
                ("apple-backend-path".to_owned(), "backends/apple".to_owned()),
                (
                    "hydrolysis-path".to_owned(),
                    "backends/hydrolysis".to_owned(),
                ),
                (
                    "android-backend-url".to_owned(),
                    "https://github.com/water-rs/android-backend.git".to_owned(),
                ),
                ("android-backend-revision".to_owned(), revision('c')),
                (
                    "hydrolysis-android-host-subdirectory".to_owned(),
                    "backends/hydrolysis/android".to_owned(),
                ),
            ])
            .collect();
        ResolvedFramework {
            source: Source::Stable {
                release: Some(FrameworkRelease {
                    repository: framework_repository().to_owned(),
                    revision: revision('a'),
                    tag: "v0.4.1".to_owned(),
                }),
            },
            minimum_cli_version: None,
            rust_version: None,
            metadata: toml::toml! {
                android-min-api-level = 31
                apple-backend-path = "backends/apple"
                hydrolysis-path = "backends/hydrolysis"
            },
            scaffold,
            experimental_packages: experimental_scaffold_packages(),
            packages: BTreeMap::new(),
            patches: PatchSet::default(),
        }
    }

    /// The git-pinned scaffold packages the checkout fixture's
    /// `[workspace.dependencies]` declares — `waterui-dew`, `waterui-gtk` and
    /// `waterui-winui` are git pins, so `stable` withholds them
    /// under `experimental-packages` while `dev`/`nightly` distribute the
    /// pins through `scaffold`.
    fn experimental_scaffold_packages() -> BTreeMap<String, ExperimentalPackage> {
        let experimental = |version: &str, git: &str, seed: char| ExperimentalPackage {
            version: version.to_owned(),
            git: git.to_owned(),
            rev: seed.to_string().repeat(40),
        };
        BTreeMap::from([
            (
                "waterui-dew".to_owned(),
                experimental("0.2.1", "https://github.com/water-rs/dew", 'b'),
            ),
            (
                "waterui-gtk".to_owned(),
                experimental("0.2.0", "https://github.com/water-rs/gtk-backend", 'g'),
            ),
            (
                "waterui-winui".to_owned(),
                experimental("0.1.0", "https://github.com/water-rs/waterui-winui", 'e'),
            ),
        ])
    }

    /// A stable-channel resolution whose `scaffold` comes from running the
    /// real `framework_scaffold` emission over the checkout fixture manifest
    /// — the path the published `framework.json` takes — rather than a
    /// hand-assembled map. Tests that exercise the scaffold contract
    /// end-to-end resolve this, so a dropped or renamed
    /// `[package.metadata.waterui]` key fails them instead of only the
    /// emitter's unit test. `stable` still withholds the git-pinned scaffold
    /// packages under `experimental-packages`; the emitted `-git`/`-rev`
    /// facts move there.
    pub fn stable_checkout_framework() -> ResolvedFramework {
        let revision = |seed: char| seed.to_string().repeat(40);
        let manifest = toml::Value::Table(
            local_checkout_manifest()
                .parse::<toml::Table>()
                .expect("the checkout fixture manifest parses"),
        );
        let mut emitted =
            framework_scaffold(&manifest).expect("the checkout fixture emits its scaffold");
        let mut experimental_packages = BTreeMap::new();
        for name in ["waterui-dew", "waterui-gtk", "waterui-winui"] {
            experimental_packages.insert(
                name.to_owned(),
                ExperimentalPackage {
                    version: emitted
                        .remove(&format!("{name}-version"))
                        .expect("the fixture pins the package version"),
                    git: emitted
                        .remove(&format!("{name}-git"))
                        .expect("the fixture pins the package git source"),
                    rev: emitted
                        .remove(&format!("{name}-rev"))
                        .expect("the fixture pins the package revision"),
                },
            );
        }
        let scaffold = FRAMEWORK_PACKAGES
            .iter()
            .map(|name| (format!("{name}-version"), "0.4.1".to_owned()))
            .chain(emitted)
            .collect();
        ResolvedFramework {
            source: Source::Stable {
                release: Some(FrameworkRelease {
                    repository: framework_repository().to_owned(),
                    revision: revision('a'),
                    tag: "v0.4.1".to_owned(),
                }),
            },
            minimum_cli_version: None,
            rust_version: None,
            metadata: toml::toml! {
                android-min-api-level = 31
                apple-backend-path = "backends/apple"
                hydrolysis-path = "backends/hydrolysis"
            },
            scaffold,
            experimental_packages,
            packages: BTreeMap::new(),
            patches: PatchSet::default(),
        }
    }

    /// A `dev`-channel resolution: the manifest's scaffold facts — including
    /// the git-pinned packages `stable` withholds, which `dev` distributes
    /// through `scaffold`.
    pub fn dev_framework() -> ResolvedFramework {
        let mut framework = stable_framework();
        let revision = 'a'.to_string().repeat(40);
        framework.source = Source::Dev {
            repository: framework_repository().to_owned(),
            revision: revision.clone(),
            lock_sha256: 'f'.to_string().repeat(64),
        };
        for (name, package) in std::mem::take(&mut framework.experimental_packages) {
            framework
                .scaffold
                .insert(format!("{name}-version"), package.version);
            framework
                .scaffold
                .insert(format!("{name}-git"), package.git);
            framework
                .scaffold
                .insert(format!("{name}-rev"), package.rev);
        }
        framework.packages = resolve_packages(
            &framework.scaffold,
            &test_lock(),
            framework_repository(),
            &revision,
        )
        .expect("the fixture lock resolves every scaffold requirement");
        framework
    }

    /// A `nightly`-channel resolution.
    pub fn nightly_framework() -> ResolvedFramework {
        let mut framework = dev_framework();
        framework.source = Source::Nightly {
            repository: framework_repository().to_owned(),
            revision: 'a'.to_string().repeat(40),
            tag: "nightly-2026.09.15".to_owned(),
            lock_sha256: 'f'.to_string().repeat(64),
        };
        framework
    }

    /// A local framework checkout fixture: the repository's own root manifest
    /// and a lock naming the workspace crates, inside a git worktree. Like the
    /// repository today it carries no backend gitlink: both backend pins are
    /// literals in the manifest.
    pub fn write_local_checkout(root: &Path) {
        std::fs::create_dir_all(root).expect("checkout dir");
        std::fs::write(root.join("Cargo.toml"), local_checkout_manifest()).expect("manifest");
        let lock = test_lock();
        std::fs::write(root.join("Cargo.lock"), lock.to_string()).expect("lockfile");
        let git = |args: &[String]| {
            let status = StdCommand::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .expect("git must run");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init".to_owned(), "-q".to_owned()]);
        git(&[
            "add".to_owned(),
            "Cargo.toml".to_owned(),
            "Cargo.lock".to_owned(),
        ]);
        git(&[
            "-c".to_owned(),
            "user.name=waterui-test".to_owned(),
            "-c".to_owned(),
            "user.email=waterui-test@waterui.dev".to_owned(),
            "commit".to_owned(),
            "-qm".to_owned(),
            "init".to_owned(),
        ]);
    }

    /// A checkout whose manifest declares no `apple-backend-path` — a
    /// revision from before the backend's return, carrying no native Apple
    /// crate.
    pub fn write_apple_pathless_checkout(root: &Path) {
        write_local_checkout(root);
        let manifest_path = root.join("Cargo.toml");
        let mut manifest: toml::Value =
            toml::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest["package"]["metadata"]["waterui"]
            .as_table_mut()
            .unwrap()
            .remove("apple-backend-path");
        std::fs::write(manifest_path, toml::to_string(&manifest).unwrap()).unwrap();
    }

    /// The same fixture as it existed while the backends still rode
    /// gitlinks: no `apple-backend-path` and no `android-backend-revision`
    /// in the manifest, the submodule pins recorded in the index.
    pub fn write_pre_decoupling_checkout(root: &Path) {
        write_local_checkout(root);
        write_submodule_pin(root, "backends/apple", 'b');
        write_submodule_pin(root, "backends/android", 'c');
        let manifest = local_checkout_manifest()
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                !(line.starts_with("apple-backend-path")
                    || line.starts_with("android-backend-revision"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !manifest.contains("apple-backend-path")
                && !manifest.contains("android-backend-revision"),
            "the fixture manifest moved; the pre-decoupling rewrite must be revisited"
        );
        std::fs::write(root.join("Cargo.toml"), manifest).expect("manifest");
        let git = |args: &[&str]| {
            let status = StdCommand::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .expect("git must run");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["add", "Cargo.toml"]);
        // Synthetic pins are no real commit: the host's hooks must not run.
        git(&[
            "-c",
            "user.name=waterui-test",
            "-c",
            "user.email=waterui-test@waterui.dev",
            "commit",
            "--no-verify",
            "-qm",
            "pre-decoupling manifest",
        ]);
    }

    /// Record a gitlink pin the way a checked-out submodule records it —
    /// `160000` is the mode `git submodule` writes into the index.
    fn write_submodule_pin(root: &Path, path: &str, seed: char) {
        let status = StdCommand::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},{}", seed.to_string().repeat(40), path),
            ])
            .status()
            .expect("git must run");
        assert!(status.success(), "git update-index failed");
    }

    /// The manifest a local checkout fixture carries: the framework's own
    /// metadata and the workspace requirements `scaffold-packages` names.
    fn local_checkout_manifest() -> &'static str {
        include_str!("../../tests/fixtures/framework_checkout_manifest.toml")
    }

    /// A framework lock naming every workspace crate a scaffolded project pins.
    pub fn test_lock() -> Lockfile {
        Lockfile {
            packages: FRAMEWORK_PACKAGES
                .iter()
                .map(|name| package(name, "0.4.1", None))
                .collect(),
            version: cargo_lock::ResolveVersion::V4,
            root: None,
            metadata: BTreeMap::default(),
            patch: cargo_lock::Patch::default(),
        }
    }

    pub fn package(name: &str, version: &str, source: Option<&str>) -> cargo_lock::Package {
        cargo_lock::Package {
            name: name.parse().unwrap(),
            version: version.parse().unwrap(),
            source: source.map(|source| source.parse().unwrap()),
            checksum: None,
            dependencies: Vec::new(),
            replace: None,
        }
    }

    /// Write a stub crate at `dir` whose manifest declares `features` — cargo
    /// validates every `dep/feature` a manifest forwards, so the stubs cover
    /// the feature sets the generated manifests name. A `[patch.crates-io]`
    /// path entry is the only source that redirects a fixture's registry pins
    /// without a crates.io index, so generated manifests resolve the same
    /// version the channel pins against the stub on disk.
    pub fn write_vendor_stub(dir: &Path, name: &str, features: &[&str]) {
        std::fs::create_dir_all(dir.join("src")).expect("stub crate dir");
        let mut stub_manifest = toml_edit::DocumentMut::new();
        stub_manifest["package"]["name"] = toml_edit::value(name);
        stub_manifest["package"]["version"] = toml_edit::value("0.4.1");
        stub_manifest["package"]["edition"] = toml_edit::value("2021");
        for feature in features {
            stub_manifest["features"][feature] = toml_edit::value(toml_edit::Array::new());
        }
        std::fs::write(dir.join("Cargo.toml"), stub_manifest.to_string()).expect("stub manifest");
        std::fs::write(dir.join("src/lib.rs"), "").expect("stub lib");
    }
}

async fn resolve_dev(repository: &str, slug: &str) -> Result<String> {
    gated_dev_head(repository, slug, "dev.yml", "framework").await
}

/// A `dev` selection pinned to an exact commit: `rev` names a commit of the
/// integration branch's own history, verified against the branch's head
/// through the same GitHub API the tip resolution already uses. Anything
/// else — a fork commit, another branch's tip, a commit `dev` never merged —
/// is not dev history and cannot stand in for the channel.
async fn resolve_dev_at(repository: &str, slug: &str, rev: &str) -> Result<String> {
    validate_rev(rev)?;
    let head = remote_dev_head(repository, "framework").await?;
    let revision = normalize_revision(slug, rev).await?;
    let status = dev_ancestor_status(slug, &revision, &head).await?;
    ensure_dev_ancestor(&status, &revision, &head)?;
    if !gate_passed(slug, "dev.yml", &revision).await? {
        bail!("no successful dev gate run exists for --rev {revision}");
    }
    Ok(revision)
}

/// The spelling `--rev` accepts: a commit hash — hex only, at least the four
/// characters a unique abbreviation needs and at most the full forty.
/// Anything else is a ref, which ancestry cannot pin.
fn validate_rev(rev: &str) -> Result<()> {
    if !(4..=40).contains(&rev.len()) || !rev.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("--rev must be a commit hash of 4-40 hex characters, got `{rev}`");
    }
    Ok(())
}

/// The full commit id `rev` names in `slug` — the object the commits API
/// names back, so an abbreviation `225259c80` persists the same forty
/// characters the tip resolution would.
async fn normalize_revision(slug: &str, rev: &str) -> Result<String> {
    let response = fetch(&format!(
        "https://api.github.com/repos/{slug}/commits/{rev}"
    ))
    .await
    .wrap_err_with(|| format!("--rev {rev} does not name a commit in {slug}"))?;
    let commit: serde_json::Value = serde_json::from_slice(&response)?;
    commit["sha"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| eyre!("--rev {rev} did not resolve to a commit in {slug}"))
}

/// The compare status of `revision...head` — the ancestry fact the compare
/// API reports for the pin against the branch's own head.
async fn dev_ancestor_status(slug: &str, revision: &str, head: &str) -> Result<String> {
    let url = format!("https://api.github.com/repos/{slug}/compare/{revision}...{head}");
    let response = fetch(&url).await?;
    let compare: serde_json::Value = serde_json::from_slice(&response)?;
    compare["status"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| eyre!("malformed compare response from {url}: `status` is missing"))
}

/// The compare statuses a pin is accepted under: `ahead` — `dev` moved past
/// the pin — or `identical` — the pin is the head itself. `behind` and
/// `diverged` name a commit carrying work `dev` never contained.
fn ensure_dev_ancestor(status: &str, revision: &str, head: &str) -> Result<()> {
    if matches!(status, "ahead" | "identical") {
        return Ok(());
    }
    bail!(
        "--rev {revision} is not an ancestor of the framework's dev head {head} \
         (compare status: {status})"
    );
}

/// The `dev` branch head of `repository` — the tip `git ls-remote` reports.
async fn remote_dev_head(repository: &str, what: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["ls-remote", repository, "refs/heads/dev"])
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "could not resolve {what} dev: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let revision = std::str::from_utf8(&output.stdout)?
        .split_whitespace()
        .next()
        .ok_or_else(|| eyre!("{what} repository has no dev branch"))?
        .to_owned();
    validate_revision(&revision)?;
    Ok(revision)
}

/// Whether `revision` carries a successful `gate` run on `dev` — the query
/// behind the channel's promise that a resolved commit passed its
/// compilation gate. `event=push` scopes the run to the branch itself: a
/// pull-request run on the same commit is not the gate.
async fn gate_passed(slug: &str, gate: &str, revision: &str) -> Result<bool> {
    let response = fetch(&format!("https://api.github.com/repos/{slug}/actions/workflows/{gate}/runs?branch=dev&head_sha={revision}&status=success&event=push&per_page=1")).await?;
    let runs: serde_json::Value = serde_json::from_slice(&response)?;
    Ok(gate_run_succeeded(&runs, revision))
}

/// The successful-run decision over a workflow-runs response: some listed
/// run names the revision and concluded `success`.
fn gate_run_succeeded(runs: &serde_json::Value, revision: &str) -> bool {
    runs["workflow_runs"].as_array().is_some_and(|runs| {
        runs.iter()
            .any(|run| run["head_sha"].as_str() == Some(revision) && run["conclusion"] == "success")
    })
}

/// The `dev` HEAD of `repository`, held to the channel's promise that the
/// resolved commit passed `gate` — the workflow file gating `dev` in that
/// repository: `dev.yml` for the framework, `ci.yml` for a backend.
async fn gated_dev_head(repository: &str, slug: &str, gate: &str, what: &str) -> Result<String> {
    let revision = remote_dev_head(repository, what).await?;
    if !gate_passed(slug, gate, &revision).await? {
        bail!("{what} dev revision {revision} has not passed its compilation gate");
    }
    Ok(revision)
}

/// The newest distribution `channel` accepts, with its certification
/// manifest loaded and verified against the release it rode in on.
///
/// Stable is the registry: the highest published `waterui` version names the
/// framework release tag (`v<version>`, the version group release-plz tags),
/// so the version comes from the crates.io sparse index and the manifest from
/// the release's download URL — neither is a GitHub API request, so a
/// resolution costs nothing against the sixty-an-hour unauthenticated limit
/// that paging through every crate release used to exhaust (#110). Nightly
/// prereleases exist only on GitHub and are still listed there.
async fn latest_certification(
    repository: &str,
    channel: FrameworkChannel,
) -> Result<Certification> {
    let slug = repository_slug(repository)?;
    let (tag, manifest_url, release) = match channel {
        FrameworkChannel::Stable => {
            let version = newest_registry_version(&fetch(&sparse_index_url("waterui")).await?)?;
            let tag = format!("v{version}");
            let manifest_url =
                format!("https://github.com/{slug}/releases/download/{tag}/framework.json");
            (tag, manifest_url, None)
        }
        FrameworkChannel::Nightly => {
            let release = newest_nightly_release(slug).await?;
            let asset = certification_asset(&release)?;
            (
                release.tag_name.clone(),
                asset.browser_download_url.clone(),
                Some(release),
            )
        }
        FrameworkChannel::Dev => unreachable!("dev is not a certified channel"),
    };
    let Some(bytes) = fetch_optional(&manifest_url).await? else {
        return Err(match channel {
            FrameworkChannel::Stable => StableReleaseWithoutManifest { tag }.into(),
            FrameworkChannel::Nightly => eyre!("nightly {tag} has no certification manifest"),
            FrameworkChannel::Dev => unreachable!("dev is not a certified channel"),
        });
    };
    let certification = parse_certification(&bytes)?;
    if certification.tag != tag {
        bail!("{channel} certification does not match its release");
    }
    verify_certification(&certification, release.as_ref(), repository)?;
    certifies_channel(&certification, channel)?;
    Ok(certification)
}

/// The crates.io sparse index entry for `name`: one JSON line per published
/// version, served from a CDN with no request metering.
fn sparse_index_url(name: &str) -> String {
    let prefix = match name.len() {
        1 => "1".to_owned(),
        2 => "2".to_owned(),
        3 => format!("3/{}", &name[..1]),
        _ => format!("{}/{}", &name[..2], &name[2..4]),
    };
    format!("https://index.crates.io/{prefix}/{name}")
}

/// One version line of a sparse index entry, reduced to what selection reads.
#[derive(Deserialize)]
struct IndexVersion {
    vers: cargo_toml::SemVer,
    yanked: bool,
}

/// The highest published, unyanked, bare-semver version in a sparse index
/// entry: prereleases are not stable distributions, and a yanked version has
/// no release a user should scaffold against.
fn newest_registry_version(index: &[u8]) -> Result<cargo_toml::SemVer> {
    std::str::from_utf8(index)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<IndexVersion>(line).map_err(eyre::Report::from))
        .filter_map(|entry| match entry {
            Ok(entry) if entry.yanked || !entry.vers.pre.is_empty() => None,
            Ok(entry) => Some(Ok(entry.vers)),
            Err(error) => Some(Err(error)),
        })
        .try_fold(None, |newest: Option<cargo_toml::SemVer>, version| {
            let version = version?;
            Ok::<_, eyre::Report>(Some(match newest {
                Some(newest) if newest.cmp_precedence(&version).is_ge() => newest,
                _ => version,
            }))
        })?
        .ok_or_else(|| {
            eyre!(
                "no stable framework release carries a manifest yet; \
                 select dev or nightly explicitly"
            )
        })
}

/// The newest published `nightly-*` prerelease of the framework repository.
async fn newest_nightly_release(slug: &str) -> Result<Release> {
    let mut releases = Vec::new();
    let mut page = 1;
    loop {
        let bytes = fetch(&format!(
            "https://api.github.com/repos/{slug}/releases?per_page=100&page={page}"
        ))
        .await?;
        let batch: Vec<Release> = serde_json::from_slice(&bytes)?;
        let complete = batch.len() < 100;
        releases.extend(batch.into_iter().filter(is_nightly_release));
        if complete {
            break;
        }
        page += 1;
    }
    releases
        .into_iter()
        .max_by(|left, right| left.published_at.cmp(&right.published_at))
        .ok_or_else(|| eyre!("no certified nightly exists; select dev or stable explicitly"))
}

/// A release selected for `channel` must carry that channel's manifest: the
/// tag alone does not bind the contents to the distribution it names.
fn certifies_channel(certification: &Certification, channel: FrameworkChannel) -> Result<()> {
    if certification.channel != channel {
        bail!(
            "{} certifies the {} channel, not {channel}",
            certification.tag,
            certification.channel
        );
    }
    Ok(())
}

/// The schema a manifest declares, read before the rest of it so an
/// unsupported schema is reported as such rather than as whichever field it
/// happens to lack.
#[derive(Deserialize)]
struct CertificationSchema {
    schema_version: u32,
}

const CERTIFICATION_SCHEMA_VERSION: u32 = 2;

fn parse_certification(bytes: &[u8]) -> Result<Certification> {
    let schema: CertificationSchema = serde_json::from_slice(bytes)?;
    if schema.schema_version != CERTIFICATION_SCHEMA_VERSION {
        bail!(
            "framework manifest schema version {} is not supported; this CLI requires schema version {CERTIFICATION_SCHEMA_VERSION}",
            schema.schema_version
        );
    }
    Ok(serde_json::from_slice(bytes)?)
}

/// A published `nightly-*` prerelease: the only release shape the nightly
/// channel certifies.
fn is_nightly_release(release: &Release) -> bool {
    release.prerelease && !release.draft && release.tag_name.starts_with("nightly-")
}

/// The first stable framework release whose GitHub release publishes a
/// `framework.json`. Manifest publishing began here; every earlier tag
/// resolves nothing on the stable channel.
const FIRST_STABLE_MANIFEST_RELEASE: &str = "0.5.0";

/// The stable channel selected a release that predates manifest publishing:
/// it has no `framework.json` to resolve. The diagnostic names the first
/// manifest-carrying release and the channels that resolve today.
#[derive(Debug, thiserror::Error)]
#[error(
    "stable release {tag} carries no framework.json — it predates manifest publishing. \
     The first stable release carrying a manifest is {FIRST_STABLE_MANIFEST_RELEASE}; \
     until it is published, use `water create <name> --channel dev` or, once a certified \
     nightly exists, `--channel nightly`."
)]
struct StableReleaseWithoutManifest {
    /// The tag of the release the stable channel resolved to.
    tag: String,
}

/// The `framework.json` asset of a nightly release.
fn certification_asset(release: &Release) -> Result<&ReleaseAsset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == "framework.json")
        .ok_or_else(|| eyre!("nightly {} has no certification manifest", release.tag_name))
}

/// Read and verify a `framework.json` from disk: the same schema, channel,
/// repository, revision and CLI checks a downloaded manifest passes, with no
/// release for the tag to be checked against.
async fn load_manifest(path: &Path, repository: &str) -> Result<Certification> {
    let contents = smol::fs::read(path)
        .await
        .wrap_err_with(|| format!("failed to read framework manifest {}", path.display()))?;
    let certification = parse_certification(&contents)
        .wrap_err_with(|| format!("invalid framework manifest {}", path.display()))?;
    verify_certification(&certification, None, repository)?;
    Ok(certification)
}

/// The checks a `framework.json` must pass before it resolves anything —
/// identical whether the manifest was downloaded from `release` or read from
/// disk via `--framework-manifest`, where there is no release to check the
/// tag against.
fn verify_certification(
    certification: &Certification,
    release: Option<&Release>,
    repository: &str,
) -> Result<()> {
    let channel = certification.channel;
    if certification.schema_version != CERTIFICATION_SCHEMA_VERSION {
        bail!(
            "framework manifest schema version {} is not supported; this CLI requires schema version {CERTIFICATION_SCHEMA_VERSION}",
            certification.schema_version
        );
    }
    if channel == FrameworkChannel::Dev {
        bail!("framework manifest channel `dev` is not a certified distribution");
    }
    if certification.repository != repository_slug(repository)? {
        bail!(
            "{channel} manifest names a different repository ({})",
            certification.repository
        );
    }
    if let Some(release) = release
        && certification.tag != release.tag_name
    {
        bail!("{channel} certification does not match its release");
    }
    validate_revision(&certification.revision)?;
    if let Some(minimum) = &minimum_cli_version(&certification.metadata)? {
        let update = match channel {
            FrameworkChannel::Stable => registry_cli_update(minimum),
            FrameworkChannel::Dev | FrameworkChannel::Nightly => checkout_cli_update(),
        };
        validate_installed_cli(minimum, &update)?;
    }
    Ok(())
}

/// A submodule the resolved revision pins: the repository `.gitmodules`
/// names for the path and the commit the revision's gitlink — or a certified
/// channel's certification — records.
struct SubmodulePin {
    /// The submodule's repository, canonicalized like [`framework_repository`].
    repository: String,
    /// The pinned commit.
    commit: String,
}

/// The `[patch]` rows a `stable` project's Git-sourced members need.
///
/// `[patch.crates-io]` carries the framework's own Git pins verbatim — the
/// member's registry requirements (`cherenkov`, `nami`, `waterkit-*`, the
/// `wgpu` fork) resolve nowhere else — plus a member-source entry for every
/// `*-path` member the workspace's own table path-pins, so a crates.io
/// requirement for the member (`hydrolysis-m3`'s `hydrolysis` edge) joins
/// the member the generated crate links. A `path` entry that is not a
/// declared member is left off the table: its crates.io requirements keep
/// resolving the published release.
///
/// `[patch.<framework repository>]` substitutes every `path` member the
/// revision's root table names — plus the `waterui` facade itself — with
/// the exact version the certified lock records, so a member's
/// workspace-internal path dependencies resolve to the published packages
/// rather than a second copy inside the Git source. The `*-path` member
/// crates themselves — `waterui-apple`, `hydrolysis` — are never
/// substituted: they are Git members on every channel.
///
/// A member the table names but the lock does not record has no registry
/// identity to substitute — the revision is not one `stable` can carry the
/// native backend for.
fn stable_member_substitutions(
    root: &toml::Value,
    lock: &Lockfile,
    repository: &str,
    revision: &str,
    metadata: &toml::Table,
) -> Result<PatchSet> {
    let mut patches = PatchSet::default();
    // The `*-path` members this revision declares — the crates a generated
    // crate links from the framework's own Git source. A revision that
    // declares none resolves nothing from the repository, so nothing needs
    // substituting.
    let member_packages: BTreeSet<&str> = FRAMEWORK_MEMBERS
        .iter()
        .filter(|member| metadata.get(member.path_key).is_some())
        .map(|member| member.package)
        .collect();
    if member_packages.is_empty() {
        return Ok(patches);
    }
    let crates_io_source = root
        .get("patch")
        .and_then(|patch| patch.get("crates-io"))
        .and_then(toml::Value::as_table);
    let mut crates_io = BTreeMap::new();
    let mut members: BTreeSet<String> = BTreeSet::new();
    if let Some(table) = crates_io_source {
        for (name, dependency) in table {
            if dependency.get("path").is_some() {
                members.insert(name.clone());
                // The member whose crates.io name the workspace redirects —
                // the independent dependent's edge — resolves to the
                // framework source instead of the registry.
                if member_packages.contains(name.as_str()) {
                    crates_io.insert(
                        name.clone(),
                        Dependency::Detailed(Box::new(DependencyDetail {
                            git: Some(repository.to_owned()),
                            rev: Some(revision.to_owned()),
                            ..DependencyDetail::default()
                        })),
                    );
                }
                continue;
            }
            crates_io.insert(
                name.clone(),
                dependency
                    .clone()
                    .try_into()
                    .wrap_err_with(|| format!("invalid patch.crates-io.{name}"))?,
            );
        }
    }
    if !crates_io.is_empty() {
        patches.insert("crates-io".to_owned(), crates_io);
    }
    // The facade's own `[patch.crates-io]` entry is a member like the rest —
    // name it anyway so a revision that drops its row still substitutes the
    // root package a Git member's `waterui` edge resolves to.
    members.insert("waterui".to_owned());
    for member in &member_packages {
        members.remove(*member);
    }
    let mut dependencies = BTreeMap::new();
    for name in members {
        let candidates: Vec<_> = lock
            .packages
            .iter()
            .filter(|package| package.name.as_str() == name)
            .collect();
        let [package] = candidates.as_slice() else {
            bail!(
                "framework lock records no unique `{name}` package identity; a stable \
                 selection cannot substitute the git-sourced member with its registry version"
            );
        };
        dependencies.insert(
            name,
            Dependency::Detailed(Box::new(DependencyDetail {
                version: Some(
                    format!("={}", package.version)
                        .parse()
                        .expect("a lockfile version is a valid version requirement"),
                ),
                ..DependencyDetail::default()
            })),
        );
    }
    if !dependencies.is_empty() {
        patches.insert(repository.to_owned(), dependencies);
    }
    Ok(patches)
}

/// Rebase a fetched root manifest's `[patch]` tables onto the channel's own
/// sources: a path entry under one of the revision's submodules becomes
/// `git + rev` on the submodule's repository at the recorded commit, and any
/// other path entry becomes `git + rev` on the framework repository at the
/// resolved revision.
///
/// A fetched table keyed on the framework repository itself is dropped in
/// any spelling — Cargo canonicalizes a source's query, fragment, `.git`
/// suffix and trailing slash away, so every one names the patched source
/// itself, and a patch may not point at the source it patches. No
/// repository-source mirror is synthesized for the path entries either:
/// mirroring them at the channel's revision was the same-source patch Cargo
/// rejects (#807), and the extracted crates that once named framework
/// crates by `git` (#758) are consumed from the registry, where
/// `[patch.crates-io]` already applies.
fn rebase_patches_onto_source(
    mut patches: PatchSet,
    repository: &str,
    revision: &str,
    submodules: &BTreeMap<String, SubmodulePin>,
) -> PatchSet {
    patches.retain(|source, _| !same_git_source(source, repository));
    for dependencies in patches.values_mut() {
        for dependency in dependencies.values_mut() {
            let Dependency::Detailed(detail) = dependency else {
                continue;
            };
            let Some(path) = detail.path.take() else {
                continue;
            };
            let path = path.trim_start_matches("./");
            let pin = submodules.iter().find_map(|(root, pin)| {
                (path == root.as_str() || path.starts_with(&format!("{root}/"))).then_some(pin)
            });
            let (git, rev) = pin.map_or((repository, revision), |pin| {
                (pin.repository.as_str(), pin.commit.as_str())
            });
            detail.git = Some(git.to_owned());
            detail.rev = Some(rev.to_owned());
        }
    }
    patches
}

/// Patch the channel's `[patch.crates-io]` table with every framework member
/// crate the workspace's own table does not name.
///
/// The checkout's patch table only lists the members its own crates depend on
/// through `path`; a member resolved through `[workspace.dependencies]` —
/// `waterui-ffi`, `waterui-internal`, `waterui-media`, `waterui-mcp`,
/// `waterui-preview`, `waterui-preview-protocol`, `waterui-testing`, … — can
/// still be requested by bare version from a generated or transitive manifest,
/// and without an entry that request resolves a published release built from a
/// different revision (water-rs/cli#197). The lock's source-less packages are
/// the workspace member set — the same record `resolve_packages` resolves
/// scaffold requirements against — so they are the names the table must pin at
/// the revision the channel selected.
fn patch_framework_members(
    patches: &mut PatchSet,
    lock: &Lockfile,
    repository: &str,
    revision: &str,
) {
    let crates_io = patches.entry("crates-io".to_owned()).or_default();
    for package in &lock.packages {
        let name = package.name.as_str();
        if package.source.is_some()
            || !(name.starts_with("waterui")
                || FRAMEWORK_MEMBERS
                    .iter()
                    .any(|member| member.package == name))
        {
            continue;
        }
        crates_io.entry(name.to_owned()).or_insert_with(|| {
            Dependency::Detailed(Box::new(DependencyDetail {
                git: Some(repository.to_owned()),
                rev: Some(revision.to_owned()),
                ..DependencyDetail::default()
            }))
        });
    }
}

/// Rewrite a workspace lockfile's member identities to the git source the
/// workspace resolves from: every source-less package is a member built from
/// `repository` at `revision`, and a dependency edge naming one points at that
/// same source.
fn annotate_workspace_lock(lock: &mut Lockfile, repository: &str, revision: &str) -> Result<()> {
    let source =
        format!("git+{repository}?rev={revision}#{revision}").parse::<cargo_lock::SourceId>()?;
    let mut local = BTreeMap::new();
    for package in &mut lock.packages {
        if package.source.is_none() {
            package.source = Some(source.clone());
            local.insert(
                (package.name.clone(), package.version.clone()),
                LockedDependency::from(&*package),
            );
        }
    }
    for package in &mut lock.packages {
        for dependency in &mut package.dependencies {
            if dependency.source.is_none()
                && let Some(replacement) =
                    local.get(&(dependency.name.clone(), dependency.version.clone()))
            {
                *dependency = replacement.clone();
            }
        }
    }
    Ok(())
}

/// The lock entries an extracted framework package's own `Cargo.lock` carries
/// at the revision the scaffold pins — the only written record of its resolved
/// graph, since an extracted crate never enters the framework lock.
///
/// An extracted backend's graph can require package identities the `WaterUI`
/// workspace lock does not name — the backend's graph failed the `Water.lock`
/// gate on every edge while Hydrolysis lived out-of-tree (water-rs/cli#197;
/// the in-tree member's graph now resolves from the framework lock itself,
/// #1635). Each pinned repository's lock is annotated with its own pin
/// exactly as the framework lock is annotated with the channel's — member
/// packages become `git+<repo>?rev=<rev>` entries and member edges follow
/// them. Entries naming a framework member, or resolving the framework
/// repository at the extracted crate's own (older) patch pin, are dropped:
/// the channel's lock owns every framework member identity at this
/// revision.
async fn foreign_locked_packages(
    framework_lock: &Lockfile,
    packages: &BTreeMap<String, DependencyDetail>,
    repository: &str,
) -> Result<Vec<cargo_lock::Package>> {
    let member_names: BTreeSet<&str> = framework_lock
        .packages
        .iter()
        .filter(|package| package.source.is_none())
        .map(|package| package.name.as_str())
        .collect();
    let mut known: BTreeSet<(String, String, String)> = framework_lock
        .packages
        .iter()
        .map(|package| {
            (
                package.name.to_string(),
                package.version.to_string(),
                package
                    .source
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string),
            )
        })
        .collect();
    let mut pins = BTreeSet::new();
    for detail in packages.values() {
        let (Some(git), Some(revision)) = (&detail.git, &detail.rev) else {
            continue;
        };
        if canonical_git_url(git) != canonical_git_url(repository) {
            pins.insert((git.clone(), revision.clone()));
        }
    }
    let mut foreign = Vec::new();
    for (git, revision) in pins {
        let slug = repository_slug(&git)?;
        let Some(bytes) = fetch_optional(&format!(
            "https://raw.githubusercontent.com/{slug}/{revision}/Cargo.lock"
        ))
        .await?
        else {
            // An extracted crate that keeps no lock of its own contributes
            // nothing the channel can pin.
            continue;
        };
        let mut lock: Lockfile = std::str::from_utf8(&bytes)?.parse()?;
        annotate_workspace_lock(&mut lock, &git, &revision)?;
        for package in lock.packages {
            if member_names.contains(package.name.as_str()) {
                continue;
            }
            if package.source.as_ref().is_some_and(|source| {
                source.is_git()
                    && canonical_git_url(source.url().as_str()) == canonical_git_url(repository)
            }) {
                continue;
            }
            let key = (
                package.name.to_string(),
                package.version.to_string(),
                package
                    .source
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string),
            );
            if known.insert(key) {
                foreign.push(package);
            }
        }
    }
    Ok(foreign)
}

/// A git URL in the spelling Cargo canonicalizes sources to: the query,
/// fragment, `.git` suffix and trailing slash carry no meaning.
fn canonical_git_url(url: &str) -> &str {
    let url = url.split(['?', '#']).next().unwrap_or_default();
    url.trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
}

/// Whether two URLs name the same git source — `repo?branch=dev`, `repo.git`
/// and `repo` canonicalize to one source, so a `[patch]` table keyed on any
/// of them patches the framework repository itself.
fn same_git_source(source: &str, repository: &str) -> bool {
    canonical_git_url(source) == canonical_git_url(repository)
}

#[cfg(test)]
mod tests {
    use test_fixtures::{
        dev_framework, nightly_framework, package, stable_framework, test_lock,
        write_apple_pathless_checkout, write_local_checkout, write_pre_decoupling_checkout,
    };

    use super::*;

    /// The persisted framework selection, the certified `Water.lock`, and the
    /// generated hydrolysis backend's resolved `cargo metadata` graph, all
    /// captured from a real `water create`/`water build` on the recorded `dev`
    /// revision (#203). The captures under `tests/fixtures/dev_channel/` are
    /// trimmed to the subgraph the predicates exercise by
    /// `tests/fixtures/dev_channel/slim.py` — record a fresh `water create` +
    /// `water build` into a scratch directory and run
    /// `python3 slim.py --capture <dir> --out tests/fixtures/dev_channel`
    /// when `dev` moves on.
    #[test]
    fn a_fresh_dev_channel_project_reports_no_lock_conflicts() {
        #[derive(Deserialize)]
        struct Manifest {
            framework: ResolvedFramework,
        }
        let manifest: Manifest =
            toml::from_str(include_str!("../../tests/fixtures/dev_channel/Water.toml")).unwrap();
        let metadata: cargo_metadata::Metadata = serde_json::from_str(include_str!(
            "../../tests/fixtures/dev_channel/hydrolysis-backend.metadata.json"
        ))
        .unwrap();
        manifest
            .framework
            .validate_dependencies(
                &metadata,
                include_bytes!("../../tests/fixtures/dev_channel/Water.lock"),
            )
            .expect("a fresh dev-channel resolution replaces no locked package");
    }

    /// A project whose resolution carries none of the selected framework
    /// source fails naming the selected revision, the revision the resolved
    /// lock carries for the framework repository — or that it carries none —
    /// and the `water channel` command that re-pins the project (#236).
    #[test]
    fn a_divergent_framework_resolution_names_the_revisions_and_the_command() {
        let repository = framework_repository();
        let selected = 'a'.to_string().repeat(40);
        let resolved = 'b'.to_string().repeat(40);
        let contents = test_lock().to_string();
        let mut framework = dev_framework();
        framework.source = Source::Dev {
            repository: repository.to_owned(),
            revision: selected.clone(),
            lock_sha256: hex::encode(Sha256::digest(contents.as_bytes())),
        };
        let package = |name: &str, version: &str, source: &str| {
            serde_json::json!({
                "name": name,
                "version": version,
                "id": source,
                "source": source,
                "dependencies": [],
                "targets": [],
                "features": {},
                "manifest_path": "/project/Cargo.toml",
                "edition": "2024"
            })
        };
        let metadata = |packages: serde_json::Value| {
            serde_json::from_value::<cargo_metadata::Metadata>(serde_json::json!({
                "version": 1,
                "packages": packages,
                "workspace_members": [],
                "resolve": {"nodes": [], "root": null},
                "workspace_root": "/project",
                "target_directory": "/project/target",
                "workspace_metadata": {}
            }))
            .unwrap()
        };
        // The resolved lock carries the framework repository at another
        // revision than the selection records.
        let error = framework
            .validate_dependencies(
                &metadata(serde_json::json!([package(
                    "waterui",
                    "0.5.2",
                    &format!("git+{repository}?rev={resolved}#{resolved}")
                )])),
                contents.as_bytes(),
            )
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains(&selected), "{message}");
        assert!(message.contains(&resolved), "{message}");
        assert!(message.contains("water channel dev"), "{message}");
        // The resolved lock carries no framework revision at all.
        let error = framework
            .validate_dependencies(
                &metadata(serde_json::json!([package(
                    "serde",
                    "1.0.0",
                    "registry+https://github.com/rust-lang/crates.io-index"
                )])),
                contents.as_bytes(),
            )
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains(&selected), "{message}");
        assert!(message.contains("no framework revision"), "{message}");
        assert!(message.contains("water channel dev"), "{message}");
    }

    /// A `Water.lock` that no longer hashes to the checksum the selection
    /// records fails naming the channel and the selected revision, the
    /// recorded and the found checksums, and the `water channel` command
    /// that writes the pair again — `dev --rev` re-pinning the recorded
    /// revision on dev, a `nightly` re-resolution on nightly (#1814).
    #[test]
    fn a_mismatched_water_lock_names_the_checksums_and_the_command() {
        let contents = test_lock().to_string();
        let found = hex::encode(Sha256::digest(contents.as_bytes()));
        for framework in [dev_framework(), nightly_framework()] {
            let (revision, recorded, command) = match &framework.source {
                Source::Dev {
                    revision,
                    lock_sha256,
                    ..
                } => (
                    revision.clone(),
                    lock_sha256.clone(),
                    format!("water channel dev --rev {revision}"),
                ),
                Source::Nightly {
                    revision,
                    lock_sha256,
                    ..
                } => (
                    revision.clone(),
                    lock_sha256.clone(),
                    "water channel nightly".to_owned(),
                ),
                _ => unreachable!("dev and nightly selections only"),
            };
            let message = framework
                .cargo_lock(contents.as_bytes())
                .unwrap_err()
                .to_string();
            assert!(message.contains(&revision), "{message}");
            assert!(message.contains(&recorded), "{message}");
            assert!(message.contains(&found), "{message}");
            assert!(message.contains(&command), "{message}");
        }
    }

    /// `water create` seeds the generated crate's lock the way the build
    /// does — `seed_lockfile` writes the channel's certified pins over the
    /// project's — so the create-time resolution the scan performs can never
    /// write a `Cargo.lock` whose `accesskit` family splits across two
    /// incompatible generations, the seed `water build` then could not
    /// resolve (#203). Built from the same `seed_lockfile` call the scan
    /// makes; the resolved lock is the fixture a real create + build
    /// recorded, trimmed by `tests/fixtures/dev_channel/slim.py` (see the
    /// sibling test's doc comment for how to re-cut it).
    #[test]
    fn the_create_time_seed_resolves_one_accesskit_generation() {
        #[derive(Deserialize)]
        struct Manifest {
            framework: ResolvedFramework,
        }
        let manifest: Manifest =
            toml::from_str(include_str!("../../tests/fixtures/dev_channel/Water.toml")).unwrap();
        let canonical = manifest
            .framework
            .cargo_lock(include_bytes!(
                "../../tests/fixtures/dev_channel/Water.lock"
            ))
            .unwrap();
        let project: Lockfile = include_str!("../../tests/fixtures/dev_channel/Cargo.lock")
            .parse()
            .unwrap();

        smol::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let lockfile = dir.path().join("project.lock");
            smol::fs::write(&lockfile, project.to_string())
                .await
                .unwrap();
            let crate_dir = dir.path().join("backends/hydrolysis");
            smol::fs::create_dir_all(&crate_dir).await.unwrap();

            crate::templates::seed_lockfile(&crate_dir, &lockfile, Some(&canonical))
                .await
                .unwrap();
            let seed: Lockfile = smol::fs::read_to_string(crate_dir.join("Cargo.lock"))
                .await
                .unwrap()
                .parse()
                .unwrap();
            for pin in &canonical.packages {
                assert!(
                    seed.packages.contains(pin),
                    "the seed must carry the certified identity of {}",
                    pin.name
                );
            }
            // `accesskit_winit` is a name the certified lock does not record:
            // the seed leaves it for the resolution to pick compatibly.
            assert!(
                !seed
                    .packages
                    .iter()
                    .any(|package| package.name.as_str() == "accesskit_winit")
            );

            // The resolution that seed produced: every dependency edge lands
            // on a package the lock records — a family split across
            // generations could not satisfy all of them.
            let resolved: Lockfile =
                include_str!("../../tests/fixtures/dev_channel/hydrolysis-backend.lock")
                    .parse()
                    .unwrap();
            for package in &resolved.packages {
                for dependency in &package.dependencies {
                    assert!(
                        resolved.packages.iter().any(|p| dependency.matches(p)),
                        "{} names {}, which no package in the lock provides",
                        package.name,
                        dependency
                    );
                }
            }
        });
    }

    #[test]
    fn a_second_major_beside_the_locked_one_is_an_addition_not_a_change() {
        let registry = "registry+https://github.com/rust-lang/crates.io-index";
        let allowed: BTreeSet<_> = [("annotate-snippets", "0.12.16"), ("toml", "0.9.5")]
            .into_iter()
            .map(|(name, version)| LockedPackage {
                name: name.to_owned(),
                version: version.to_owned(),
                source: Some(registry.to_owned()),
            })
            .collect();
        let resolved = |name: &str, version: &str| {
            replaces_locked_package(
                &allowed,
                name,
                &semver::Version::parse(version).unwrap(),
                Some(registry),
            )
        };
        assert!(
            !resolved("annotate-snippets", "0.12.16"),
            "the locked entry itself"
        );
        assert!(
            !resolved("annotate-snippets", "0.11.5"),
            "bindgen's 0.11 line coexists with the locked 0.12"
        );
        assert!(
            !resolved("bincode", "2.0.1"),
            "a name the project never locked"
        );
        assert!(
            resolved("toml", "0.9.8"),
            "the locked 0.9.5 moved within its caret range"
        );
        assert!(
            resolved("annotate-snippets", "0.12.20"),
            "the locked 0.12.16 moved"
        );
    }

    /// The generated crate's lock seed is one resolution: a name the
    /// pinned framework lock records resolves only to the identities the
    /// channel certifies — the union seed handed Cargo the project's
    /// divergent `wasm-bindgen`/`js-sys` pair beside the pin and no
    /// resolution then satisfied `Water.lock` (#177) — while a name the
    /// canonical lock does not record keeps every input lock's entries.
    #[test]
    fn the_seed_resolves_a_canonical_name_to_the_pinned_identity() {
        let registry = Some("registry+https://github.com/rust-lang/crates.io-index");
        let lock = |packages| Lockfile {
            packages,
            version: cargo_lock::ResolveVersion::V4,
            root: None,
            metadata: BTreeMap::default(),
            patch: cargo_lock::Patch::default(),
        };
        let canonical = lock(vec![
            package("wasm-bindgen", "0.2.128", registry),
            package("js-sys", "0.3.128", registry),
            package("cc", "1.4.5", registry),
        ]);
        let project = lock(vec![
            package("app", "0.1.0", None),
            package("wasm-bindgen", "0.2.105", registry),
            package("js-sys", "0.3.105", registry),
            package("cc", "1.4.7", registry),
            package("annotate-snippets", "0.12.16", registry),
            package("toml", "0.9.5", registry),
        ]);
        let previous = lock(vec![
            package("wasm-bindgen", "0.2.105", registry),
            package("annotate-snippets", "0.11.5", registry),
            package("bindgen", "0.72.0", registry),
        ]);

        let seed = seed_packages(Some(&canonical), &project, Some(&previous));
        let versions = |name: &str| {
            let mut versions: Vec<_> = seed
                .iter()
                .filter(|package| package.name.as_str() == name)
                .map(|package| package.version.to_string())
                .collect();
            versions.sort();
            versions
        };
        assert_eq!(versions("wasm-bindgen"), ["0.2.128"]);
        assert_eq!(versions("js-sys"), ["0.3.128"]);
        assert_eq!(versions("cc"), ["1.4.5"]);
        assert_eq!(versions("app"), ["0.1.0"]);
        assert_eq!(versions("toml"), ["0.9.5"]);
        assert_eq!(versions("annotate-snippets"), ["0.11.5", "0.12.16"]);
        assert_eq!(versions("bindgen"), ["0.72.0"]);

        // Without a canonical lock — the stable channel — every input
        // entry still seeds, one resolution per recorded identity.
        let seed = seed_packages(None, &project, Some(&previous));
        assert!(seed.iter().any(|package| {
            package.name.as_str() == "wasm-bindgen" && package.version.to_string() == "0.2.105"
        }));
        assert_eq!(seed.iter().filter(|p| p.name.as_str() == "app").count(), 1);
    }

    /// The divergence hydroterm hit: the previous generated lock, the
    /// channel's `Water.lock` and the application's `Cargo.lock` all record
    /// `accesskit`, at `0.25.0`, `0.25.0` and `0.25.1` respectively — three
    /// locks, one caret family, two identities. The union seed kept the
    /// application entry beside the pinned one, and Cargo could not satisfy
    /// both the pinned identity and the edges that named the other version.
    /// The seed must carry only the canonical identity for a name the
    /// channel records.
    #[test]
    fn the_seed_drops_a_projects_divergent_patch_of_a_canonical_name() {
        let registry = Some("registry+https://github.com/rust-lang/crates.io-index");
        let lock = |packages| Lockfile {
            packages,
            version: cargo_lock::ResolveVersion::V4,
            root: None,
            metadata: BTreeMap::default(),
            patch: cargo_lock::Patch::default(),
        };
        let canonical = lock(vec![
            package("accesskit", "0.25.0", registry),
            package("dirs", "6.0.0", registry),
        ]);
        let project = lock(vec![
            package("app", "0.1.0", None),
            package("accesskit", "0.25.1", registry),
            package("dirs", "7.0.0", registry),
        ]);
        let previous = lock(vec![
            package("accesskit", "0.25.0", registry),
            package("aither", "0.12.0", registry),
        ]);

        let seed = seed_packages(Some(&canonical), &project, Some(&previous));
        let versions = |name: &str| {
            let mut versions: Vec<_> = seed
                .iter()
                .filter(|package| package.name.as_str() == name)
                .map(|package| package.version.to_string())
                .collect();
            versions.sort();
            versions
        };
        // A name the channel records resolves to its certified identity
        // alone — the project's divergent 0.25.1 and the previous lock's
        // duplicate 0.25.0 cannot both enter the seed.
        assert_eq!(versions("accesskit"), ["0.25.0"]);
        // The project's second-major addition beside a canonical name is
        // dropped too: canonical owns the name.
        assert_eq!(versions("dirs"), ["6.0.0"]);
        // Names only the previous generated lock knew stay seeded.
        assert_eq!(versions("aither"), ["0.12.0"]);
        assert_eq!(versions("app"), ["0.1.0"]);
    }

    fn snapshot(lock: &Lockfile) -> (ResolvedFramework, Vec<u8>) {
        let bytes = lock.to_string().into_bytes();
        let scaffold = lock
            .packages
            .iter()
            .map(|package| {
                (
                    format!("{}-version", package.name),
                    package.version.to_string(),
                )
            })
            .collect();
        let repository = framework_repository();
        let revision = "a".repeat(40);
        let framework = ResolvedFramework {
            source: Source::Nightly {
                repository: repository.to_owned(),
                revision: revision.clone(),
                tag: "nightly-test".into(),
                lock_sha256: hex::encode(Sha256::digest(&bytes)),
            },
            minimum_cli_version: None,
            rust_version: None,
            metadata: toml::toml! {
                android-min-api-level = 31
                apple-backend-path = "backends/apple"
                hydrolysis-path = "backends/hydrolysis"
            },
            packages: resolve_packages(&scaffold, lock, repository, &revision).unwrap(),
            scaffold,
            experimental_packages: BTreeMap::new(),
            patches: PatchSet::default(),
        };
        (framework, bytes)
    }

    #[test]
    fn cli_requirement_uses_semver_precedence() {
        for (minimum, current, compatible) in [
            ("0.1.4", "0.1.3", false),
            ("0.1.4", "0.1.4", true),
            ("0.1.9", "0.1.10", true),
            ("0.1.4", "0.1.4-rc.1", false),
            ("0.1.4-rc.1", "0.1.4-rc.2", true),
            ("0.1.4+z", "0.1.4+a", true),
            ("0.1.4", "1.0.0", true),
        ] {
            let minimum = minimum.parse().unwrap();
            let current = current.parse().unwrap();
            let update = registry_cli_update(&minimum);
            let result = validate_cli_version(&minimum, &current, &update);
            assert_eq!(result.is_ok(), compatible, "{current} against {minimum}");
            if let Err(error) = result {
                let message = error.to_string();
                assert!(message.contains(&minimum.to_string()));
                assert!(message.contains(&current.to_string()));
                assert!(message.contains(&update));
                assert!(message.contains("water --version"));
            }
        }
    }

    #[test]
    fn cli_requirement_metadata_rejects_invalid_versions() {
        let mut metadata = toml::toml! {
            minimum-cli-version = "0.1.4"
        };
        assert_eq!(
            minimum_cli_version(&metadata).unwrap(),
            Some("0.1.4".parse().unwrap())
        );
        metadata["minimum-cli-version"] = toml::Value::String(">=0.1.4".into());
        assert!(minimum_cli_version(&metadata).is_err());
        assert!(minimum_cli_version(&toml::Table::new()).unwrap().is_none());
    }

    #[test]
    fn android_min_api_level_is_required_framework_metadata() {
        assert_eq!(stable_framework().android_min_api_level().unwrap(), 31);

        let mut missing = stable_framework();
        missing.metadata.remove("android-min-api-level");
        let error = missing.android_min_api_level().unwrap_err().to_string();
        assert!(error.contains("android-min-api-level"), "{error}");
        assert!(error.contains("v0.4.1"), "{error}");

        let mut invalid = stable_framework();
        invalid.metadata["android-min-api-level"] = toml::Value::String("31".to_owned());
        let error = invalid.android_min_api_level().unwrap_err().to_string();
        assert!(error.contains("android-min-api-level"), "{error}");
    }

    #[test]
    fn persisted_cli_requirement_blocks_an_older_cli_with_update_guidance() {
        let mut minimum: cargo_toml::SemVer = env!("CARGO_PKG_VERSION").parse().unwrap();
        minimum.major += 1;
        let mut framework = stable_framework();
        framework.minimum_cli_version = Some(minimum.clone());
        let contents = toml::to_string(&framework).unwrap();
        let framework: ResolvedFramework = toml::from_str(&contents).unwrap();
        let error = framework.validate_cli().unwrap_err().to_string();
        assert!(error.contains(&registry_cli_update(&minimum)));
        assert_eq!(framework.minimum_cli_version, Some(minimum));
    }

    #[test]
    fn snapshot_preserves_independent_package_sources() {
        let backend_revision = "b".repeat(40);
        let backend_source = format!(
            "git+https://example.com/waterui-gtk?rev={backend_revision}#{backend_revision}"
        );
        let lock = Lockfile {
            packages: vec![
                package("waterui", "0.3.0", None),
                package("waterui-gtk", "0.1.0", Some(&backend_source)),
                package(
                    "hydrolysis-m3",
                    "0.1.0",
                    Some("registry+https://github.com/rust-lang/crates.io-index"),
                ),
            ],
            version: cargo_lock::ResolveVersion::V4,
            root: None,
            metadata: BTreeMap::default(),
            patch: cargo_lock::Patch::default(),
        };
        let (framework, _) = snapshot(&lock);
        let persisted = toml::to_string(&framework).unwrap();
        let framework: ResolvedFramework = toml::from_str(&persisted).unwrap();
        assert_eq!(framework.channel(), Some(FrameworkChannel::Nightly));
        assert_eq!(framework.dependency("waterui").rev, Some("a".repeat(40)));
        let backend = framework.dependency("waterui-gtk");
        assert_eq!(
            backend.git.as_deref(),
            Some("https://example.com/waterui-gtk")
        );
        assert_eq!(backend.rev, Some(backend_revision));
        let theme = framework.dependency("hydrolysis-m3");
        assert!(theme.git.is_none());
        assert_eq!(theme.version.unwrap().to_string(), "=0.1.0");
    }

    #[test]
    fn extracted_crate_absent_from_the_lock_resolves_to_its_declared_requirement() {
        // A crate released from its own repository and not consumed by the
        // framework never enters the framework lock — the scaffold's declared
        // requirement is the requirement a dev/nightly resolution pins,
        // whether the workspace names a version or a git revision.
        let lock = Lockfile {
            packages: vec![package("waterui", "0.3.0", None)],
            version: cargo_lock::ResolveVersion::V4,
            root: None,
            metadata: BTreeMap::default(),
            patch: cargo_lock::Patch::default(),
        };
        let gtk_revision = "b".repeat(40);
        let scaffold = BTreeMap::from([
            ("waterui-version".to_string(), "0.3.0".to_string()),
            ("waterui-dew-version".to_string(), "0.2.1".to_string()),
            ("waterui-gtk-version".to_string(), "0.2.0".to_string()),
            (
                "waterui-gtk-git".to_string(),
                "https://github.com/water-rs/gtk-backend".to_string(),
            ),
            ("waterui-gtk-rev".to_string(), gtk_revision.clone()),
        ]);
        let packages =
            resolve_packages(&scaffold, &lock, framework_repository(), &"a".repeat(40)).unwrap();
        assert!(packages["waterui"].git.is_some());
        let dew = &packages["waterui-dew"];
        assert!(dew.git.is_none());
        assert_eq!(dew.version.as_ref().unwrap().to_string(), "=0.2.1");
        let gtk = &packages["waterui-gtk"];
        assert_eq!(
            gtk.git.as_deref(),
            Some("https://github.com/water-rs/gtk-backend")
        );
        assert_eq!(gtk.rev.as_deref(), Some(gtk_revision.as_str()));
        assert_eq!(gtk.version.as_ref().unwrap().to_string(), "^0.2.0");
    }

    #[test]
    fn declared_git_source_must_agree_with_the_lock() {
        // The declared pin is authoritative, but a framework that also builds
        // the crate in-tree must not lock a different commit than it declares.
        let locked_revision = "b".repeat(40);
        let drifted_revision = "c".repeat(40);
        for (lock_revision, expected) in [
            (locked_revision.as_str(), true),
            (drifted_revision.as_str(), false),
        ] {
            let source = format!(
                "git+https://github.com/water-rs/gtk-backend?rev={lock_revision}#{lock_revision}"
            );
            let lock = Lockfile {
                packages: vec![package("waterui-gtk", "0.2.0", Some(&source))],
                version: cargo_lock::ResolveVersion::V4,
                root: None,
                metadata: BTreeMap::default(),
                patch: cargo_lock::Patch::default(),
            };
            let scaffold = BTreeMap::from([
                ("waterui-gtk-version".to_string(), "0.2.0".to_string()),
                (
                    "waterui-gtk-git".to_string(),
                    "https://github.com/water-rs/gtk-backend".to_string(),
                ),
                ("waterui-gtk-rev".to_string(), locked_revision.clone()),
            ]);
            let result =
                resolve_packages(&scaffold, &lock, framework_repository(), &"a".repeat(40));
            assert_eq!(result.is_ok(), expected, "lock revision {lock_revision}");
            if expected {
                assert_eq!(
                    result.unwrap()["waterui-gtk"].rev.as_deref(),
                    Some(locked_revision.as_str())
                );
            }
        }
    }

    #[test]
    fn resolve_packages_matches_the_requirement_against_the_lock() {
        // The scaffold value is a requirement: a declaration satisfied by a
        // newer locked version — `hydrolysis-m3 = "0.2.0"` where the lock
        // carries the pinned 0.2.1 — still resolves the lock candidate's git
        // source instead of pinning `=0.2.0`, a registry release the patch
        // table cannot rescue and the framework never certified.
        let m3_revision = "14a35e3ef69ced36557a3c7ab11d52e0afb53ad5";
        let m3_source = format!(
            "git+https://github.com/water-rs/hydrolysis-m3?rev={m3_revision}#{m3_revision}"
        );
        let lock = Lockfile {
            packages: vec![
                package("waterui", "0.3.0", None),
                package("hydrolysis-m3", "0.2.1", Some(&m3_source)),
            ],
            version: cargo_lock::ResolveVersion::V4,
            root: None,
            metadata: BTreeMap::default(),
            patch: cargo_lock::Patch::default(),
        };
        let scaffold = BTreeMap::from([
            ("waterui-version".to_string(), "0.3.0".to_string()),
            ("hydrolysis-m3-version".to_string(), "0.2.0".to_string()),
        ]);
        let packages =
            resolve_packages(&scaffold, &lock, framework_repository(), &"a".repeat(40)).unwrap();
        let m3 = &packages["hydrolysis-m3"];
        assert_eq!(
            m3.git.as_deref(),
            Some("https://github.com/water-rs/hydrolysis-m3")
        );
        assert_eq!(m3.rev.as_deref(), Some(m3_revision));
    }

    #[test]
    fn stable_dependency_honors_a_declared_git_source() {
        // A persisted stable selection written before `experimental-packages`
        // existed can still carry a `{name}-git`/`{name}-rev` pair in
        // `scaffold`; `dependency` keeps honoring the declared pin.
        let mut framework = stable_framework();
        let revision = "b".repeat(40);
        framework
            .scaffold
            .insert("waterui-gtk-version".to_owned(), "0.1.2".to_owned());
        framework.scaffold.insert(
            "waterui-gtk-git".to_owned(),
            "https://github.com/water-rs/gtk-backend".to_owned(),
        );
        framework
            .scaffold
            .insert("waterui-gtk-rev".to_owned(), revision.clone());
        framework
            .scaffold
            .insert("waterui-dew-version".to_owned(), "0.2.1".to_owned());
        let gtk = framework.dependency("waterui-gtk");
        assert_eq!(
            gtk.git.as_deref(),
            Some("https://github.com/water-rs/gtk-backend")
        );
        assert_eq!(gtk.rev.as_deref(), Some(revision.as_str()));
        assert_eq!(gtk.version.as_ref().unwrap().to_string(), "^0.1.2");
        // A scaffold package declared by version alone still resolves the
        // registry pin.
        let dew = framework.dependency("waterui-dew");
        assert!(dew.git.is_none());
        assert_eq!(dew.version.as_ref().unwrap().to_string(), "=0.2.1");
    }

    #[test]
    fn stable_withholds_the_git_pinned_scaffold_packages() {
        // `waterui-dew`, `waterui-gtk` and `waterui-winui` are git pins, so a
        // stable manifest withholds them — recorded under
        // `experimental-packages`, absent from `scaffold` — and scaffolding
        // one fails naming the package, the channel and the fix.
        let framework = stable_framework();
        for name in ["waterui-dew", "waterui-gtk", "waterui-winui"] {
            let package = &framework.experimental_packages[name];
            assert_eq!(package.rev.len(), 40);
            assert!(!framework.scaffold.contains_key(&format!("{name}-version")));
            assert!(!framework.scaffold.contains_key(&format!("{name}-git")));
            let error = framework
                .require_distributable(name)
                .unwrap_err()
                .to_string();
            assert!(error.contains(name), "{error}");
            assert!(error.contains("stable"), "{error}");
            assert!(error.contains(&package.git), "{error}");
            assert!(error.contains(&package.rev), "{error}");
            // The gate checks the pin, not the registry, so the refusal must
            // not claim the package is unreleased — `waterui-gtk` is on
            // crates.io while the framework still pins it by revision.
            assert!(!error.contains("registry release"), "{error}");
            assert!(error.contains("--channel dev"), "{error}");
            assert!(error.contains("--channel nightly"), "{error}");
        }
        // Registry-backed scaffold packages stay distributable on stable.
        for name in ["waterui", "hydrolysis-m3"] {
            framework
                .require_distributable(name)
                .unwrap_or_else(|error| panic!("{name} must scaffold on stable: {error}"));
        }
    }

    #[test]
    fn dev_and_nightly_distribute_the_experimental_packages() {
        for framework in [dev_framework(), nightly_framework()] {
            for name in ["waterui-dew", "waterui-gtk", "waterui-winui"] {
                framework
                    .require_distributable(name)
                    .unwrap_or_else(|error| panic!("{name} must scaffold off stable: {error}"));
                let dependency = framework.dependency(name);
                assert!(
                    dependency.git.is_some(),
                    "{name} must keep its declared git pin"
                );
                assert_eq!(dependency.rev.as_deref().map(str::len), Some(40));
            }
        }
    }

    #[test]
    fn a_legacy_stable_selection_re_derives_the_withheld_set() {
        // A `Water.toml` written before `experimental-packages` existed
        // keeps the git pins inside `scaffold`; validation restores the
        // split so the withheld packages stay unscaffoldable.
        let mut framework = stable_framework();
        for (name, package) in framework.experimental_packages.clone() {
            framework
                .scaffold
                .insert(format!("{name}-version"), package.version);
            framework
                .scaffold
                .insert(format!("{name}-git"), package.git);
            framework
                .scaffold
                .insert(format!("{name}-rev"), package.rev);
        }
        framework.experimental_packages.clear();

        let framework = framework.validated().expect("fixture validates");
        assert_eq!(framework.experimental_packages.len(), 3);
        assert!(
            framework.require_distributable("waterui-winui").is_err(),
            "a stale `waterui-winui-git` entry must not resurrect the package"
        );
    }

    #[test]
    fn a_stable_certification_must_agree_on_the_withheld_set() {
        let repository = framework_repository();
        let revision = "a".repeat(40);
        let lock_sha256 = "f".repeat(64);
        let metadata = toml::toml! { android-min-api-level = 31 };
        let mut scaffold = BTreeMap::from([
            ("waterui-version".to_owned(), "0.4.1".to_owned()),
            ("waterui-winui-version".to_owned(), "0.1.0".to_owned()),
            (
                "waterui-winui-git".to_owned(),
                "https://github.com/water-rs/waterui-winui".to_owned(),
            ),
            ("waterui-winui-rev".to_owned(), "e".repeat(40)),
        ]);
        let experimental_packages = split_experimental_packages(&mut scaffold);

        let stable_certification = |experimental: BTreeMap<_, _>, scaffold| Certification {
            schema_version: 2,
            channel: FrameworkChannel::Stable,
            repository: "water-rs/waterui".to_owned(),
            revision: revision.clone(),
            tag: "v0.4.1".to_owned(),
            lockfiles: BTreeMap::from([("Cargo.lock".to_owned(), lock_sha256.clone())]),
            submodules: BTreeMap::new(),
            scaffold,
            experimental_packages: experimental,
            metadata: metadata.clone(),
        };
        let source = certified_source(
            &stable_certification(experimental_packages.clone(), scaffold.clone()),
            repository,
            &revision,
            &metadata,
            &scaffold,
            &experimental_packages,
            &lock_sha256,
        )
        .expect("a manifest carrying the withheld set verifies");
        assert!(matches!(source, Source::Stable { .. }));

        // Dropping a git-pinned package without recording it is not a valid
        // stable manifest.
        let error = certified_source(
            &stable_certification(BTreeMap::new(), scaffold.clone()),
            repository,
            &revision,
            &metadata,
            &scaffold,
            &experimental_packages,
            &lock_sha256,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("experimental packages"), "{error}");

        // Neither is recording it while still scaffolding it.
        let mut doubled = scaffold.clone();
        doubled.insert("waterui-winui-version".to_owned(), "0.1.0".to_owned());
        let error = certified_source(
            &stable_certification(experimental_packages.clone(), doubled),
            repository,
            &revision,
            &metadata,
            &scaffold,
            &experimental_packages,
            &lock_sha256,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("waterui-winui-version"), "{error}");
    }

    #[test]
    fn extracted_crate_validation_accepts_only_the_sanctioned_source() {
        // `Water.lock` cannot record an extracted crate, so
        // `validate_dependencies` holds it to its declared pin instead: the
        // exact commit for a git source, the exact version for the registry.
        let gtk_revision = "b".repeat(40);
        let mut framework = stable_framework();
        framework.packages.insert(
            "waterui-gtk".to_owned(),
            DependencyDetail {
                version: Some("0.2.0".parse().unwrap()),
                git: Some("https://github.com/water-rs/gtk-backend".to_owned()),
                rev: Some(gtk_revision.clone()),
                ..Default::default()
            },
        );
        framework.packages.insert(
            "waterui-dew".to_owned(),
            DependencyDetail {
                version: Some("=0.2.1".parse().unwrap()),
                ..Default::default()
            },
        );
        let identity = |name: &str, version: &str, source: String| LockedPackage {
            name: name.to_owned(),
            version: version.to_owned(),
            source: Some(source),
        };
        let gtk_source = |revision: &str| {
            format!("git+https://github.com/water-rs/gtk-backend?rev={revision}#{revision}")
        };
        assert!(framework.sanctioned_source(&identity(
            "waterui-gtk",
            "0.2.0",
            gtk_source(&gtk_revision)
        )));
        // The pinned commit carries whatever version its manifest declares.
        assert!(framework.sanctioned_source(&identity(
            "waterui-gtk",
            "0.2.1",
            gtk_source(&gtk_revision)
        )));
        // A different commit is a different pin.
        let drifted = "c".repeat(40);
        assert!(!framework.sanctioned_source(&identity(
            "waterui-gtk",
            "0.2.0",
            gtk_source(&drifted)
        )));
        // The registry pin holds only its exact version.
        let registry = || "registry+https://github.com/rust-lang/crates.io-index".to_owned();
        assert!(framework.sanctioned_source(&identity("waterui-dew", "0.2.1", registry())));
        assert!(!framework.sanctioned_source(&identity("waterui-dew", "0.2.2", registry())));
    }

    /// The exact requirement a stable channel writes for one scaffold entry.
    ///
    /// Spelling the number out here would put a third copy of it beside the two
    /// the manifest and the workspace already keep in step (#548), and it would
    /// have to be edited on every release.
    fn scaffolded(field: &str) -> String {
        let version = &stable_framework().scaffold[field];
        format!("={version}")
    }

    #[test]
    fn channel_update_preserves_aliases_features_and_unrelated_dependencies() {
        let manifest = toml::toml! {
            [dependencies.ui]
            package = "waterui"
            path = "../waterui"
            default-features = false
            features = ["gpu"]
            [dependencies.serde]
            version = "1"
            features = ["derive"]
            [target."cfg(unix)".build-dependencies]
            waterui-core = "0.2"
        };
        let mut document = toml_edit::ser::to_document(&manifest).unwrap();
        stable_framework()
            .update_manifest(&mut document, &PatchSet::default())
            .unwrap();
        assert_eq!(
            document["dependencies"]["ui"]["package"].as_str(),
            Some("waterui")
        );
        assert!(document["dependencies"]["ui"].get("path").is_none());
        assert_eq!(
            document["dependencies"]["ui"]["version"].as_str(),
            Some(scaffolded("waterui-version").as_str())
        );
        assert_eq!(
            document["dependencies"]["ui"]["default-features"].as_bool(),
            Some(false)
        );
        assert_eq!(
            document["dependencies"]["ui"]["features"][0].as_str(),
            Some("gpu")
        );
        let updated: toml::Value = toml::from_str(&document.to_string()).unwrap();
        assert_eq!(
            updated["dependencies"]["serde"],
            manifest["dependencies"]["serde"]
        );
        assert_eq!(
            updated["target"]["cfg(unix)"]["build-dependencies"]["waterui-core"]["version"]
                .as_str(),
            Some(scaffolded("waterui-core-version").as_str())
        );
    }

    #[test]
    fn channel_update_writes_patches_as_tables_and_clears_stale_ones() {
        let mut document: toml_edit::DocumentMut =
            "[package]\nname = \"app\"\n\n[dependencies]\nwaterui = \"0.3.0\"\n"
                .parse()
                .unwrap();
        let (dev, _) = snapshot(&Lockfile {
            packages: vec![package("waterui", "0.3.0", None)],
            version: cargo_lock::ResolveVersion::V4,
            root: None,
            metadata: BTreeMap::default(),
            patch: cargo_lock::Patch::default(),
        });
        let mut dev = dev;
        let vello: Dependency = toml::from_str::<toml::Value>(
            r#"git = "https://github.com/lexoliu/vello"
rev = "d68d9e9825bcd1ffee762323881c13a2e7a3f639""#,
        )
        .unwrap()
        .try_into()
        .unwrap();
        dev.patches
            .entry("crates-io".into())
            .or_default()
            .insert("vello".into(), vello);
        dev.update_manifest(&mut document, &PatchSet::default())
            .unwrap();
        let rendered = document.to_string();
        assert!(rendered.starts_with("[package]"), "{rendered}");
        assert!(rendered.contains("[patch.crates-io]\n"), "{rendered}");
        assert!(!rendered.contains("\n[patch]\n"), "{rendered}");
        assert_eq!(
            document["patch"]["crates-io"]["vello"]["rev"].as_str(),
            Some("d68d9e9825bcd1ffee762323881c13a2e7a3f639")
        );
        assert_eq!(
            document["dependencies"]["waterui"]["rev"].as_str(),
            Some("a".repeat(40).as_str())
        );

        stable_framework()
            .update_manifest(&mut document, &dev.patches())
            .unwrap();
        let rendered = document.to_string();
        assert!(!rendered.contains("patch"), "{rendered}");
        assert_eq!(
            document["dependencies"]["waterui"]["version"].as_str(),
            Some(scaffolded("waterui-version").as_str())
        );
    }

    #[test]
    fn snapshot_lock_preserves_external_sources_and_rewrites_local_edges() {
        let core = package("waterui-core", "0.3.0", None);
        let mut facade = package("waterui", "0.3.0", None);
        facade.dependencies.push(LockedDependency::from(&core));
        let theme = package(
            "hydrolysis-m3",
            "0.1.0",
            Some("registry+https://github.com/rust-lang/crates.io-index"),
        );
        let lock = Lockfile {
            packages: vec![facade, core, theme.clone()],
            version: cargo_lock::ResolveVersion::V4,
            root: None,
            metadata: BTreeMap::default(),
            patch: cargo_lock::Patch::default(),
        };
        let (framework, bytes) = snapshot(&lock);
        let resolved = framework.cargo_lock(&bytes).unwrap();
        let facade = resolved
            .packages
            .iter()
            .find(|package| package.name.as_str() == "waterui")
            .unwrap();
        let core = resolved
            .packages
            .iter()
            .find(|package| package.name.as_str() == "waterui-core")
            .unwrap();
        assert_eq!(facade.dependencies, vec![LockedDependency::from(core)]);
        assert!(core.source.as_ref().unwrap().is_git());
        assert!(resolved.packages.contains(&theme));
        let mut changed = bytes;
        changed.push(b'\n');
        assert!(
            framework
                .cargo_lock(&changed)
                .unwrap_err()
                .to_string()
                .contains("does not match")
        );
    }

    #[test]
    fn rebase_patches_onto_source_redirects_git_source_dependencies() {
        let mut patches = PatchSet::default();
        let mut crates_io = std::collections::BTreeMap::new();
        crates_io.insert(
            "waterui-core".to_string(),
            Dependency::Detailed(Box::new(DependencyDetail {
                path: Some("core".to_string()),
                ..DependencyDetail::default()
            })),
        );
        crates_io.insert(
            "waterkit-audio".to_string(),
            Dependency::Detailed(Box::new(DependencyDetail {
                path: Some("kit/multimedia/audio".to_string()),
                ..DependencyDetail::default()
            })),
        );
        crates_io.insert(
            "vello".to_string(),
            Dependency::Detailed(Box::new(DependencyDetail {
                git: Some("https://github.com/lexoliu/vello".to_string()),
                rev: Some("5e5f538556be16527f67379b105af82f408b747d".to_string()),
                ..DependencyDetail::default()
            })),
        );
        patches.insert("crates-io".to_string(), crates_io);
        // A fetched table keyed on the framework repository itself — in any
        // spelling Cargo canonicalizes to it — must not survive: a patch may
        // not point at the source it patches.
        patches.insert(
            "https://github.com/water-rs/waterui".to_string(),
            std::collections::BTreeMap::new(),
        );
        patches.insert(
            "https://github.com/water-rs/waterui.git?branch=dev".to_string(),
            std::collections::BTreeMap::new(),
        );

        let submodules = BTreeMap::from([(
            "kit".to_string(),
            SubmodulePin {
                repository: "https://github.com/water-rs/waterkit".to_string(),
                commit: "98c89ee702c5629094030023fb8d55464592d35d".to_string(),
            },
        )]);
        let rebased = rebase_patches_onto_source(
            patches,
            "https://github.com/water-rs/waterui",
            "475b4bb884a5f4e2b1156f1af74c40feaf71fdc1",
            &submodules,
        );

        // The crates-io path entry became a git pin at the channel revision.
        let Dependency::Detailed(core) = &rebased["crates-io"]["waterui-core"] else {
            panic!("a path patch stays a detailed dependency");
        };
        assert!(core.path.is_none());
        assert_eq!(
            core.git.as_deref(),
            Some("https://github.com/water-rs/waterui")
        );
        assert_eq!(
            core.rev.as_deref(),
            Some("475b4bb884a5f4e2b1156f1af74c40feaf71fdc1")
        );

        // A path under a submodule rebases onto the submodule's repository at
        // the pinned commit — the superproject holds a gitlink, not the crate.
        let Dependency::Detailed(audio) = &rebased["crates-io"]["waterkit-audio"] else {
            panic!("a submodule path patch stays a detailed dependency");
        };
        assert_eq!(
            audio.git.as_deref(),
            Some("https://github.com/water-rs/waterkit")
        );
        assert_eq!(
            audio.rev.as_deref(),
            Some("98c89ee702c5629094030023fb8d55464592d35d")
        );

        // Dependencies patched to another source stay untouched, and no
        // repository-source table is synthesized — it would patch a source
        // onto itself.
        let Dependency::Detailed(vello) = &rebased["crates-io"]["vello"] else {
            panic!("a git patch stays a detailed dependency");
        };
        assert_eq!(
            vello.git.as_deref(),
            Some("https://github.com/lexoliu/vello")
        );
        assert!(!rebased.contains_key("https://github.com/water-rs/waterui"));
        assert!(!rebased.contains_key("https://github.com/water-rs/waterui.git?branch=dev"));
    }

    #[test]
    fn coherence_admits_a_submodule_crate_at_either_source() {
        let revision = "a".repeat(40);
        let pin = "98c89ee702c5629094030023fb8d55464592d35d";
        let framework_source = format!("git+{}?rev={revision}#{revision}", framework_repository());
        let pin_source = format!("git+https://github.com/water-rs/waterkit?rev={pin}#{pin}");
        let (mut framework, _) = snapshot(&test_lock());
        framework
            .patches
            .entry("crates-io".into())
            .or_default()
            .insert(
                "waterkit-codec".into(),
                Dependency::Detailed(Box::new(DependencyDetail {
                    git: Some("https://github.com/water-rs/waterkit".into()),
                    rev: Some(pin.into()),
                    ..DependencyDetail::default()
                })),
            );
        framework
            .patches
            .entry("crates-io".into())
            .or_default()
            .insert(
                "waterkit-fs".into(),
                Dependency::Detailed(Box::new(DependencyDetail {
                    git: Some("https://github.com/water-rs/waterkit".into()),
                    rev: Some(pin.into()),
                    ..DependencyDetail::default()
                })),
            );
        let packages = vec![
            // Recorded at the framework's own source — a submodule path dep
            // cargo vendors in-source.
            package("waterkit-codec", "0.1.1", Some(&framework_source)),
            // Recorded at the submodule repository the patch pins it to.
            package("waterkit-fs", "0.1.1", Some(&pin_source)),
            package(
                "serde",
                "1.0.0",
                Some("registry+https://github.com/rust-lang/crates.io-index"),
            ),
        ];
        let allowed = framework.allowed_packages(&packages);
        let identity = |name: &str, source: &str| LockedPackage {
            name: name.to_owned(),
            version: "0.1.1".to_owned(),
            source: Some(source.to_owned()),
        };
        assert!(allowed.contains(&identity("waterkit-codec", &framework_source)));
        assert!(allowed.contains(&identity("waterkit-codec", &pin_source)));
        assert!(allowed.contains(&identity("waterkit-fs", &pin_source)));
        assert!(allowed.contains(&identity("waterkit-fs", &framework_source)));
        // A registry package gains no variants.
        assert_eq!(
            allowed
                .iter()
                .filter(|package| package.name == "serde")
                .count(),
            1
        );
    }

    #[test]
    fn parse_gitmodules_reads_submodule_paths_and_urls() {
        let submodules = parse_gitmodules(
            "[submodule \"utils/nami\"]\n\
             \tpath = utils/nami\n\
             \turl = https://github.com/water-rs/nami.git\n\
             \tbranch = dev\n\
             [submodule \"kit\"]\n\
             \tpath = kit\n\
             \turl = \"https://github.com/water-rs/waterkit.git\"\n",
        );
        assert_eq!(
            submodules,
            BTreeMap::from([
                (
                    "utils/nami".to_string(),
                    "https://github.com/water-rs/nami.git".to_string(),
                ),
                (
                    "kit".to_string(),
                    "https://github.com/water-rs/waterkit.git".to_string(),
                ),
            ])
        );
    }

    fn release(tag: &str, draft: bool, prerelease: bool, published_at: &str) -> Release {
        Release {
            tag_name: tag.to_owned(),
            draft,
            prerelease,
            published_at: Some(published_at.to_owned()),
            assets: vec![ReleaseAsset {
                name: "framework.json".to_owned(),
                browser_download_url: format!(
                    "https://github.com/water-rs/waterui/releases/download/{tag}/framework.json"
                ),
            }],
        }
    }

    #[test]
    fn stable_version_is_the_highest_published_unyanked_bare_semver() {
        let index = concat!(
            r#"{"name":"waterui","vers":"0.4.0","yanked":false}"#,
            "\n",
            // A prerelease is not a stable distribution no matter how recent.
            r#"{"name":"waterui","vers":"0.5.0-rc.1","yanked":false}"#,
            "\n",
            r#"{"name":"waterui","vers":"0.4.1","yanked":false}"#,
            "\n",
            // A yanked version has no release a user should scaffold against.
            r#"{"name":"waterui","vers":"0.9.9","yanked":true}"#,
            "\n",
            // A backport published after a newer version does not outrank it:
            // stable is ordered by version, not by publication order.
            r#"{"name":"waterui","vers":"0.3.9","yanked":false}"#,
            "\n",
        );
        let version = newest_registry_version(index.as_bytes()).unwrap();
        assert_eq!(version.to_string(), "0.4.1");
    }

    #[test]
    fn stable_index_with_nothing_publishable_names_the_other_channels() {
        let index = r#"{"name":"waterui","vers":"0.1.0","yanked":true}"#;
        let error = newest_registry_version(index.as_bytes())
            .unwrap_err()
            .to_string();
        assert!(error.contains("dev or nightly"), "{error}");
    }

    #[test]
    fn sparse_index_paths_follow_the_registry_layout() {
        assert_eq!(
            sparse_index_url("waterui"),
            "https://index.crates.io/wa/te/waterui"
        );
        assert_eq!(sparse_index_url("ab"), "https://index.crates.io/2/ab");
        assert_eq!(sparse_index_url("abc"), "https://index.crates.io/3/a/abc");
    }

    #[test]
    fn stable_release_without_a_manifest_reports_it_predates_publishing() {
        let error = StableReleaseWithoutManifest {
            tag: "v0.4.1".to_owned(),
        }
        .to_string();
        assert!(error.contains("v0.4.1"), "{error}");
        assert!(error.contains("predates manifest publishing"), "{error}");
        assert!(
            error.contains(FIRST_STABLE_MANIFEST_RELEASE),
            "{error} must name the first manifest-carrying stable release"
        );
        assert!(
            error.contains("--channel dev") && error.contains("--channel nightly"),
            "{error} must name the channels that resolve today"
        );
    }

    #[test]
    fn nightly_selection_takes_the_newest_published_prerelease() {
        let releases = vec![
            release("nightly-2025-12-01", false, true, "2025-12-02T00:00:00Z"),
            // Stable tags, drafts and non-prerelease tags are not nightlies.
            release("v0.4.1", false, false, "2025-12-03T00:00:00Z"),
            release("nightly-2025-12-04", true, true, "2025-12-05T00:00:00Z"),
            release("nightly-2025-12-03", false, true, "2025-12-04T00:00:00Z"),
        ];
        let newest = releases
            .into_iter()
            .filter(is_nightly_release)
            .max_by(|left, right| left.published_at.cmp(&right.published_at))
            .unwrap();
        assert_eq!(newest.tag_name, "nightly-2025-12-03");
        assert_eq!(certification_asset(&newest).unwrap().name, "framework.json");
    }

    #[test]
    fn only_github_api_requests_carry_the_token() {
        assert!(
            github_api_token(
                "https://github.com/water-rs/waterui/releases/download/v0.5.0/framework.json"
            )
            .is_none()
        );
        assert!(github_api_token("https://index.crates.io/wa/te/waterui").is_none());
        assert!(
            github_api_token("https://raw.githubusercontent.com/water-rs/waterui/abc/Cargo.toml")
                .is_none()
        );
    }

    fn certification(channel: FrameworkChannel, tag: &str) -> Certification {
        Certification {
            schema_version: 2,
            channel,
            repository: "water-rs/waterui".to_owned(),
            revision: "a".repeat(40),
            tag: tag.to_owned(),
            lockfiles: BTreeMap::from([("Cargo.lock".to_owned(), "f".repeat(64))]),
            submodules: BTreeMap::new(),
            scaffold: BTreeMap::new(),
            experimental_packages: BTreeMap::new(),
            metadata: toml::toml! {
                android-min-api-level = 31
                apple-backend-path = "backends/apple"
                hydrolysis-path = "backends/hydrolysis"
            },
        }
    }

    #[test]
    fn certification_verification_rejects_uncertified_or_mismatched_manifests() {
        let repository = framework_repository();
        let release = release("v0.4.1", false, false, "2025-11-01T00:00:00Z");

        let dev = certification(FrameworkChannel::Dev, "dev");
        assert!(
            verify_certification(&dev, None, repository)
                .unwrap_err()
                .to_string()
                .contains("dev")
        );

        let mut wrong_schema = certification(FrameworkChannel::Stable, "v0.4.1");
        wrong_schema.schema_version = 1;
        assert!(
            verify_certification(&wrong_schema, None, repository)
                .unwrap_err()
                .to_string()
                .contains("schema")
        );
        let nightly_on_a_stable_tag = certification(FrameworkChannel::Nightly, "v0.4.1");
        let error = certifies_channel(&nightly_on_a_stable_tag, FrameworkChannel::Stable)
            .unwrap_err()
            .to_string();
        assert!(error.contains("certifies the nightly channel"), "{error}");
        certifies_channel(&nightly_on_a_stable_tag, FrameworkChannel::Nightly).unwrap();

        // A schema-1 manifest has no `metadata`; the schema is still what the
        // error names, not the field the newer schema happens to require.
        let error = parse_certification(
            br#"{"schema_version": 1, "channel": "nightly", "repository": "water-rs/waterui"}"#,
        )
        .err()
        .expect("a schema-1 manifest is rejected")
        .to_string();
        assert!(
            error.contains("schema version 1 is not supported"),
            "{error}"
        );

        let mut wrong_repository = certification(FrameworkChannel::Stable, "v0.4.1");
        wrong_repository.repository = "water-rs/android-backend".to_owned();
        assert!(
            verify_certification(&wrong_repository, None, repository)
                .unwrap_err()
                .to_string()
                .contains("water-rs/android-backend")
        );

        let wrong_tag = certification(FrameworkChannel::Stable, "v0.4.0");
        assert!(
            verify_certification(&wrong_tag, Some(&release), repository)
                .unwrap_err()
                .to_string()
                .contains("does not match its release")
        );

        // A manifest read from disk has no release; the tag check is skipped.
        verify_certification(&wrong_tag, None, repository).unwrap();

        let stable = certification(FrameworkChannel::Stable, "v0.4.1");
        verify_certification(&stable, Some(&release), repository).unwrap();
    }

    #[test]
    fn manifest_loading_verifies_a_certification_from_disk() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("framework.json");
        let manifest = serde_json::json!({
            "schema_version": 2,
            "channel": "stable",
            "repository": "water-rs/waterui",
            "revision": "a".repeat(40),
            "tag": "v0.4.1",
            "lockfiles": {"Cargo.lock": "f".repeat(64)},
            "submodules": {
                "kit": "c".repeat(40),
            },
            "scaffold": {
                "hydrolysis-path": "backends/hydrolysis",
                "hydrolysis-m3-version": "0.2.0",
                "waterui-dew-version": "0.2.1",
                "waterui-gtk-version": "0.1.2",
                "apple-backend-path": "backends/apple",
                "android-backend-url": "https://github.com/water-rs/android-backend.git",
            },
            "metadata": {
                "minimum-cli-version": "0.1.0",
                "android-min-api-level": 31,
            },
        });
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let repository = framework_repository();
        let certification = smol::block_on(load_manifest(&path, repository)).unwrap();
        assert_eq!(certification.channel, FrameworkChannel::Stable);
        assert_eq!(certification.tag, "v0.4.1");

        std::fs::write(&path, b"not json").unwrap();
        assert!(smol::block_on(load_manifest(&path, repository)).is_err());
    }

    /// The Rust scaffold derivation and `framework_manifest.py`'s must produce
    /// the same table for the same tree — this asserts the Rust side against
    /// the fixture manifest, which carries the framework root manifest's
    /// metadata table and the workspace requirements `scaffold-packages`
    /// names.
    #[test]
    fn framework_scaffold_derives_from_the_framework_manifest() {
        let root: toml::Value = toml::from_str(include_str!(
            "../../tests/fixtures/framework_checkout_manifest.toml"
        ))
        .unwrap();
        let scaffold = framework_scaffold(&root).unwrap();
        let workspace = |name: &str| {
            let dependency = &root["workspace"]["dependencies"][name];
            dependency
                .as_str()
                .or_else(|| dependency.get("version").and_then(toml::Value::as_str))
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            scaffold,
            BTreeMap::from([
                (
                    "hydrolysis-m3-version".to_owned(),
                    workspace("hydrolysis-m3")
                ),
                ("waterui-dew-version".to_owned(), workspace("waterui-dew")),
                (
                    "waterui-dew-git".to_owned(),
                    "https://github.com/water-rs/dew".to_owned()
                ),
                (
                    "waterui-dew-rev".to_owned(),
                    "b64f6759a3ebe7ac621bad84be00fe431f978119".to_owned()
                ),
                ("waterui-gtk-version".to_owned(), workspace("waterui-gtk")),
                (
                    "waterui-gtk-git".to_owned(),
                    "https://github.com/water-rs/gtk-backend".to_owned()
                ),
                (
                    "waterui-gtk-rev".to_owned(),
                    "3162043e618e759bea6d6e52ec75c6ee1273c080".to_owned()
                ),
                (
                    "waterui-winui-version".to_owned(),
                    workspace("waterui-winui")
                ),
                (
                    "waterui-winui-git".to_owned(),
                    "https://github.com/water-rs/waterui-winui".to_owned()
                ),
                (
                    "waterui-winui-rev".to_owned(),
                    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_owned()
                ),
                ("apple-backend-path".to_owned(), "backends/apple".to_owned()),
                (
                    "hydrolysis-path".to_owned(),
                    "backends/hydrolysis".to_owned()
                ),
                (
                    "android-backend-url".to_owned(),
                    "https://github.com/water-rs/android-backend.git".to_owned()
                ),
                ("android-backend-revision".to_owned(), "c".repeat(40)),
                (
                    "hydrolysis-android-host-subdirectory".to_owned(),
                    "backends/hydrolysis/android".to_owned()
                ),
            ])
        );
    }

    #[test]
    fn framework_scaffold_rejects_a_git_package_without_a_revision() {
        let mut root: toml::Value = toml::from_str(include_str!(
            "../../tests/fixtures/framework_checkout_manifest.toml"
        ))
        .unwrap();
        let gtk = &mut root["workspace"]["dependencies"]["waterui-gtk"];
        gtk.as_table_mut()
            .unwrap()
            .insert("branch".to_owned(), toml::Value::String("dev".to_owned()));
        gtk.as_table_mut().unwrap().remove("rev");
        let error = framework_scaffold(&root).unwrap_err();
        assert!(error.to_string().contains("waterui-gtk"), "{error:?}");
    }

    #[test]
    fn framework_scaffold_rejects_a_backend_revision_that_is_not_a_commit() {
        let mut root: toml::Value = toml::from_str(include_str!(
            "../../tests/fixtures/framework_checkout_manifest.toml"
        ))
        .unwrap();
        root["package"]["metadata"]["waterui"]["android-backend-revision"] =
            toml::Value::String("dev".to_owned());
        let error = framework_scaffold(&root).unwrap_err();
        assert!(
            error.to_string().contains("android-backend-revision"),
            "{error:?}"
        );
    }

    #[test]
    fn framework_scaffold_validates_the_member_and_host_paths() {
        for (key, value) in [
            ("hydrolysis-android-host-subdirectory", "../outside"),
            ("hydrolysis-android-host-subdirectory", "/absolute"),
            ("hydrolysis-android-host-subdirectory", ""),
            ("hydrolysis-path", "../outside"),
            ("hydrolysis-path", "/absolute"),
            ("hydrolysis-path", ""),
        ] {
            let mut root: toml::Value = toml::from_str(include_str!(
                "../../tests/fixtures/framework_checkout_manifest.toml"
            ))
            .unwrap();
            root["package"]["metadata"]["waterui"][key] = toml::Value::String(value.to_owned());
            let error = framework_scaffold(&root).unwrap_err();
            assert!(error.to_string().contains(key), "{key}={value}: {error:?}");
        }
    }

    /// The Git member every channel emits — the framework repository at the
    /// selected revision; `stable` resolves the certified release's
    /// provenance — and never a backend repository of its own. `hydrolysis`
    /// resolves through the same shared `member_source` (#1635).
    #[test]
    fn member_sources_pin_the_framework_source_on_every_channel() {
        let repository = framework_repository();
        let revision = 'a'.to_string().repeat(40);
        for framework in [stable_framework(), dev_framework(), nightly_framework()] {
            for member in [APPLE_BACKEND, HYDROLYSIS] {
                let detail = framework.member_source(member).unwrap();
                assert_eq!(detail.git.as_deref(), Some(repository));
                assert_eq!(detail.rev.as_deref(), Some(revision.as_str()));
            }
        }
    }

    #[test]
    fn member_sources_require_the_declared_member_path() {
        for (key, member) in [
            ("apple-backend-path", APPLE_BACKEND),
            ("hydrolysis-path", HYDROLYSIS),
        ] {
            let mut framework = stable_framework();
            framework.metadata.remove(key);
            let error = framework.member_source(member).unwrap_err().to_string();
            assert!(error.contains(key), "{error}");
        }
    }

    /// The stable member-substitution table: every `path` member the root
    /// `[patch.crates-io]` names — and the `waterui` facade — resolves to the
    /// exact version the certified lock records, under the framework's Git
    /// source. The `*-path` members (`waterui-apple`, `hydrolysis`) are never
    /// substituted, and the crates.io table keeps the framework's Git pins
    /// plus a member-source entry for the member the workspace redirects —
    /// the row `hydrolysis-m3`'s `hydrolysis` requirement resolves through
    /// (#1635).
    #[test]
    fn stable_member_substitutions_patch_path_members_to_lock_versions() {
        let root: toml::Value = toml::from_str(
            r#"
[patch.crates-io]
waterui-core = { path = "core" }
waterui-graphics = { path = "components/visual/graphics" }
hydrolysis = { path = "backends/hydrolysis" }
nami = { git = "https://github.com/water-rs/nami", rev = "f1db5017f5d64cb45b9b304c74ff420377258476" }
hydrolysis-m3 = { git = "https://github.com/water-rs/hydrolysis-m3", rev = "d8872e5" }
"#,
        )
        .unwrap();
        let lock = Lockfile {
            packages: vec![
                package("waterui", "0.4.1", None),
                package("waterui-core", "0.5.1", None),
                package("waterui-graphics", "0.5.1", None),
                package("waterui-apple", "0.4.1", None),
                package("hydrolysis", "0.3.1", None),
                package(
                    "nami",
                    "0.11.1",
                    Some(
                        "git+https://github.com/water-rs/nami?rev=f1db5017f5d64cb45b9b304c74ff420377258476#f1db5017",
                    ),
                ),
            ],
            version: cargo_lock::ResolveVersion::V4,
            root: None,
            metadata: BTreeMap::default(),
            patch: cargo_lock::Patch::default(),
        };
        let metadata = toml::toml! {
            apple-backend-path = "backends/apple"
            hydrolysis-path = "backends/hydrolysis"
        };
        let repository = framework_repository();
        let revision = "a".repeat(40);
        let patches =
            stable_member_substitutions(&root, &lock, repository, &revision, &metadata).unwrap();
        let dependencies = &patches[repository];
        let version = |name: &str| match &dependencies[name] {
            Dependency::Detailed(detail) => detail.version.as_ref().unwrap().to_string(),
            other => panic!("{name} resolved to {other:?}"),
        };
        assert_eq!(version("waterui"), "=0.4.1");
        assert_eq!(version("waterui-core"), "=0.5.1");
        assert_eq!(version("waterui-graphics"), "=0.5.1");
        assert!(!dependencies.contains_key("waterui-apple"));
        assert!(!dependencies.contains_key("hydrolysis"));
        assert!(!dependencies.contains_key("nami"));

        // The crates.io table carries the framework's Git pins verbatim —
        // the member graph's registry requirements cannot resolve without
        // them — and redirects the member's own crates.io name to the
        // framework source so `hydrolysis-m3`'s edge joins the member the
        // generated crate links. `waterui-apple` earns no row: the
        // workspace's own table names no crates.io redirect for it.
        let crates_io = &patches["crates-io"];
        let Dependency::Detailed(hydrolysis) = &crates_io["hydrolysis"] else {
            panic!("hydrolysis resolves as a member of the framework source");
        };
        assert_eq!(hydrolysis.git.as_deref(), Some(repository));
        assert_eq!(hydrolysis.rev.as_deref(), Some(revision.as_str()));
        assert!(hydrolysis.path.is_none());
        assert!(!crates_io.contains_key("waterui-apple"));
        let Dependency::Detailed(nami) = &crates_io["nami"] else {
            panic!("the framework's git pins ride verbatim");
        };
        assert_eq!(
            nami.git.as_deref(),
            Some("https://github.com/water-rs/nami")
        );
        let Dependency::Detailed(m3) = &crates_io["hydrolysis-m3"] else {
            panic!("the theme's own pin rides verbatim");
        };
        assert_eq!(
            m3.git.as_deref(),
            Some("https://github.com/water-rs/hydrolysis-m3")
        );

        // A member the lock does not record has no registry identity — the
        // revision cannot carry the native backend on `stable`.
        let mut sparse = lock.clone();
        sparse
            .packages
            .retain(|package| package.name.as_str() != "waterui-graphics");
        let error = stable_member_substitutions(&root, &sparse, repository, &revision, &metadata)
            .unwrap_err()
            .to_string();
        assert!(error.contains("waterui-graphics"), "{error}");

        // A revision declaring no `*-path` member resolves no Git member —
        // nothing needs substituting.
        let patches =
            stable_member_substitutions(&root, &lock, repository, &revision, &toml::Table::new())
                .unwrap();
        assert!(patches.is_empty());
    }

    /// The Hydrolysis Android host is the selected framework source itself:
    /// a channel answers its own `(repository, revision)` — `stable` the
    /// certified release's provenance — a local checkout its own root, and
    /// the subdirectory names the Gradle root inside that source (#1428,
    /// #1635).
    #[test]
    fn the_hydrolysis_android_host_is_the_selected_framework_source() {
        let repository = framework_repository();
        let revision = 'a'.to_string().repeat(40);
        for framework in [stable_framework(), dev_framework(), nightly_framework()] {
            let HydrolysisAndroidHost::Git {
                url,
                revision: resolved,
            } = framework.hydrolysis_android_host().unwrap()
            else {
                panic!("a channel framework resolves its host by Git source");
            };
            assert_eq!(url, repository);
            assert_eq!(resolved, revision);
            assert_eq!(
                framework.hydrolysis_android_host_subdirectory().unwrap(),
                "backends/hydrolysis/android"
            );
        }

        let mut missing = stable_framework();
        missing
            .scaffold
            .remove("hydrolysis-android-host-subdirectory");
        let error = missing
            .hydrolysis_android_host_subdirectory()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("hydrolysis-android-host-subdirectory"),
            "{error}"
        );
    }

    #[test]
    fn declared_apple_backend_path_resolves_without_a_gitlink() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("waterui");
        write_local_checkout(&root);

        let framework = smol::block_on(ResolvedFramework::for_local_checkout(&root)).unwrap();
        assert_eq!(framework.member_path(APPLE_BACKEND), Some("backends/apple"));
        assert!(framework.git_source().is_none());
    }

    /// A checkout from before the backend's return declares no
    /// `apple-backend-path`: it supplies no native Apple backend — nothing
    /// is invented from an old gitlink or a separate repository.
    #[test]
    fn a_checkout_without_apple_backend_path_carries_no_native_backend() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("waterui");
        write_apple_pathless_checkout(&root);

        let framework = smol::block_on(ResolvedFramework::for_local_checkout(&root)).unwrap();
        assert!(framework.member_path(APPLE_BACKEND).is_none());
        assert!(!framework.scaffold.contains_key("apple-backend-revision"));
        assert!(!framework.scaffold.contains_key("apple-backend-version"));
    }

    #[test]
    fn local_checkout_resolves_from_its_own_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("waterui");
        write_local_checkout(&root);
        let framework = smol::block_on(ResolvedFramework::for_local_checkout(&root)).unwrap();
        assert_eq!(framework.channel(), None);
        assert_eq!(
            framework.scaffold_value("hydrolysis-path"),
            "backends/hydrolysis"
        );
        let HydrolysisAndroidHost::Local { root: host_root } =
            framework.hydrolysis_android_host().unwrap()
        else {
            panic!("a local checkout resolves its host in place");
        };
        assert_eq!(host_root, root);
        assert_eq!(
            framework.hydrolysis_android_host_subdirectory().unwrap(),
            "backends/hydrolysis/android"
        );
        assert_eq!(
            framework.scaffold_value("apple-backend-path"),
            "backends/apple"
        );
        assert_eq!(
            framework.scaffold_value("android-backend-revision"),
            "c".repeat(40)
        );
        assert_eq!(framework.scaffold_value("waterui-version"), "0.4.1");
        assert!(framework.git_source().is_none());
    }

    /// A checkout from before the backends left the tree: its manifest
    /// declares no `android-backend-revision`, and the retired `backends/android`
    /// gitlink supplies nothing — the pin comes only from the declared scaffold
    /// fact. The `backends/apple` gitlink a pre-extraction tree carries pins
    /// nothing either — the in-tree member needs no revision, and a checkout that
    /// does not declare `apple-backend-path` has no native Apple backend.
    #[test]
    fn local_checkout_predating_the_revision_declaration_reads_no_gitlink() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("waterui");
        write_pre_decoupling_checkout(&root);

        let framework = smol::block_on(ResolvedFramework::for_local_checkout(&root)).unwrap();
        assert!(framework.member_path(APPLE_BACKEND).is_none());
        assert!(!framework.scaffold.contains_key("apple-backend-revision"));
        assert!(!framework.scaffold.contains_key("android-backend-revision"));
    }

    #[test]
    fn rev_argument_accepts_only_a_commit_hash() {
        assert!(validate_rev("225259c80").is_ok());
        assert!(validate_rev(&"a".repeat(40)).is_ok());
        for rev in ["abc", "main", "dev", "HEAD~1", "", &"a".repeat(41)] {
            assert!(validate_rev(rev).is_err(), "`{rev}` must be rejected");
        }
    }

    #[test]
    fn dev_ancestry_accepts_only_dev_history() {
        let head = "b".repeat(40);
        let pin = "a".repeat(40);
        assert!(ensure_dev_ancestor("ahead", &pin, &head).is_ok());
        assert!(ensure_dev_ancestor("identical", &pin, &head).is_ok());
        for status in ["behind", "diverged", "unknown"] {
            let message = ensure_dev_ancestor(status, &pin, &head)
                .unwrap_err()
                .to_string();
            assert!(message.contains(&pin), "{message} must name the commit");
            assert!(message.contains(&head), "{message} must name the dev head");
            assert!(
                message.contains(status),
                "{message} must name the compare status"
            );
        }
    }

    /// A pinned revision is held to the tip's own promise: the gate decision
    /// accepts only a successful run that names the commit itself — a run on
    /// another sha, a failed conclusion, or a response without runs all fail.
    #[test]
    fn the_gate_decision_requires_a_successful_run_on_the_commit() {
        let pin = "a".repeat(40);
        let other = "b".repeat(40);
        let run = |sha: &str, conclusion: &str| serde_json::json!({"head_sha": sha, "conclusion": conclusion});
        assert!(gate_run_succeeded(
            &serde_json::json!({"workflow_runs": [run(&pin, "success")]}),
            &pin
        ));
        assert!(gate_run_succeeded(
            &serde_json::json!({"workflow_runs": [run(&other, "success"), run(&pin, "success")]}),
            &pin
        ));
        for response in [
            serde_json::json!({"workflow_runs": [run(&pin, "failure")]}),
            serde_json::json!({"workflow_runs": [run(&other, "success")]}),
            serde_json::json!({"workflow_runs": []}),
            serde_json::json!({}),
        ] {
            assert!(
                !gate_run_succeeded(&response, &pin),
                "{response} must not pass the gate for {pin}"
            );
        }
    }

    /// A `--rev` pin beside a certified channel fails inside `resolve` —
    /// before any release is listed — the same refusal the flag layer makes.
    #[test]
    fn a_certified_channel_rejects_a_rev_pin_before_resolving() {
        for channel in [FrameworkChannel::Stable, FrameworkChannel::Nightly] {
            let message = smol::block_on(ResolvedFramework::resolve(channel, Some("225259c80")))
                .unwrap_err()
                .to_string();
            assert!(message.contains("dev"), "{message} must name dev");
        }
    }
}
