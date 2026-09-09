//! Framework channel resolution and persisted dependency selection.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
    str::FromStr,
};

use crate::project::Project;
use cargo_lock::{Dependency as LockedDependency, Lockfile};
use cargo_toml::{Dependency, DependencyDetail, PatchSet};
use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use smol::process::Command;
use zenwave::{Client as _, Method, redirect::FollowRedirect};

/// A framework distribution channel, independent of the Rust toolchain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameworkChannel {
    /// The integration branch, resolved to an exact compilation-checked commit.
    Dev,
    /// An immutable revision certified by the complete nightly suite.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "channel", rename_all = "lowercase")]
enum Source {
    Stable,
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
    scaffold: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    packages: BTreeMap<String, DependencyDetail>,
    #[serde(default, skip_serializing_if = "PatchSet::is_empty")]
    patches: PatchSet,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
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

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct Certification {
    #[serde(default, rename = "minimum-cli-version")]
    minimum_cli_version: Option<cargo_toml::SemVer>,
    schema_version: u32,
    channel: FrameworkChannel,
    repository: String,
    revision: String,
    tag: String,
    lockfiles: BTreeMap<String, String>,
    scaffold: BTreeMap<String, String>,
}

impl ResolvedFramework {
    /// Return the selected distribution channel.
    #[must_use]
    pub const fn channel(&self) -> FrameworkChannel {
        match self.source {
            Source::Stable => FrameworkChannel::Stable,
            Source::Dev { .. } => FrameworkChannel::Dev,
            Source::Nightly { .. } => FrameworkChannel::Nightly,
        }
    }

    pub(crate) fn stable() -> Self {
        Self {
            source: Source::Stable,
            minimum_cli_version: None,
            scaffold: scaffold_metadata(include_str!("../../Cargo.toml"))
                .expect("embedded scaffold metadata is valid"),
            packages: BTreeMap::new(),
            patches: PatchSet::default(),
        }
    }

    pub(crate) fn validate_cli(&self) -> Result<()> {
        if let Some(minimum) = &self.minimum_cli_version {
            let update = match &self.source {
                Source::Stable => registry_cli_update(minimum),
                Source::Dev {
                    repository,
                    revision,
                    ..
                }
                | Source::Nightly {
                    repository,
                    revision,
                    ..
                } => git_cli_update(repository, revision),
            };
            validate_installed_cli(minimum, &update)?;
        }
        Ok(())
    }

    pub(crate) fn scaffold_value(&self, key: &str) -> &str {
        &self.scaffold[key]
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
        for (source, dependencies) in previous_patches {
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
        let patches = toml_edit::ser::to_document(&self.patches)?;
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

    fn update_dependencies(&self, dependencies: &mut dyn toml_edit::TableLike) -> Result<()> {
        for (name, dependency) in dependencies.iter_mut() {
            let package = dependency
                .get("package")
                .and_then(toml_edit::Item::as_str)
                .unwrap_or(&name)
                .to_owned();
            if !self.scaffold.contains_key(&format!("{package}-version")) {
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
        let source = match &self.source {
            Source::Stable => return Ok(()),
            Source::Dev {
                repository,
                revision,
                ..
            }
            | Source::Nightly {
                repository,
                revision,
                ..
            } => format!("git+{repository}?rev={revision}#{revision}"),
        };
        let locked = self.cargo_lock(contents)?;
        let allowed: BTreeSet<_> = locked.packages.iter().map(LockedPackage::from).collect();
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
            bail!("the project does not resolve its selected framework revision");
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
            if !allowed.contains(&identity) {
                bail!(
                    "framework dependency {} {} differs from Water.lock; select a compatible channel explicitly",
                    identity.name,
                    identity.version
                );
            }
            pending.extend(nodes[&id].dependencies.iter().cloned());
        }
        Ok(())
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
        let lock_path = directory.join("Cargo.lock");
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
        let canonical = if self.channel() == FrameworkChannel::Stable {
            None
        } else {
            Some(smol::fs::read(project.root().join("Water.lock")).await?)
        };
        let mut packages = BTreeMap::new();
        if let Some(previous) = &previous {
            packages.extend(
                previous
                    .packages
                    .iter()
                    .map(|package| (LockedDependency::from(package), package.clone())),
            );
        }
        if let Some(canonical) = &canonical {
            packages.extend(
                self.cargo_lock(canonical)?
                    .packages
                    .into_iter()
                    .map(|package| (LockedDependency::from(&package), package)),
            );
        }
        packages.extend(
            project_lock
                .packages
                .iter()
                .map(|package| (LockedDependency::from(package), package.clone())),
        );
        let allowed: BTreeSet<_> = packages.values().map(LockedPackage::from).collect();
        let mut seed = project_lock;
        seed.packages = packages.into_values().collect();
        smol::fs::write(&lock_path, seed.to_string()).await?;
        let root = directory.to_path_buf();
        let features = features.to_vec();
        let result = async {
            let metadata = smol::unblock(move || {
                cargo_metadata::MetadataCommand::new().current_dir(root)
                    .features(cargo_metadata::CargoOpt::SomeFeatures(features)).exec()
            }).await?;
            validate_resolved_cli(&metadata)?;
            if !allow_new {
                for package in &metadata.packages {
                    if package.source.is_some() && !allowed.contains(&LockedPackage {
                        name: package.name.to_string(),
                        version: package.version.to_string(),
                        source: package.source.as_ref().map(|source| source.repr.clone()),
                    }) {
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

    pub(crate) fn cargo_lock(&self, contents: &[u8]) -> Result<Lockfile> {
        let (repository, revision, expected) = match &self.source {
            Source::Stable => bail!("stable uses the application's Cargo.lock"),
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
        if hex::encode(Sha256::digest(contents)) != *expected {
            bail!("Water.lock does not match the selected framework revision");
        }
        let mut lock: Lockfile = std::str::from_utf8(contents)?.parse()?;
        let source = format!("git+{repository}?rev={revision}#{revision}")
            .parse::<cargo_lock::SourceId>()?;
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
        Ok(lock)
    }

    pub(crate) fn dependency(&self, name: &str) -> DependencyDetail {
        match &self.source {
            Source::Stable => DependencyDetail {
                version: Some(
                    format!("={}", self.scaffold_value(&format!("{name}-version")))
                        .parse()
                        .expect("resolved package version is valid"),
                ),
                ..Default::default()
            },
            Source::Dev { .. } | Source::Nightly { .. } => self.packages[name].clone(),
        }
    }

    pub(crate) async fn resolve(channel: FrameworkChannel) -> Result<(Self, Option<Vec<u8>>)> {
        if channel == FrameworkChannel::Stable {
            return Ok((Self::stable(), None));
        }
        let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches(".git");
        let slug = repository
            .strip_prefix("https://github.com/")
            .ok_or_else(|| eyre!("framework repository must identify its GitHub source"))?;
        let certification = if channel == FrameworkChannel::Nightly {
            Some(latest_certification(slug).await?)
        } else {
            None
        };
        let revision = match &certification {
            Some(certification) => certification.revision.clone(),
            None => resolve_dev(repository, slug).await?,
        };
        validate_revision(&revision)?;
        let base = format!("https://raw.githubusercontent.com/{slug}/{revision}");
        let manifest = fetch(&format!("{base}/Cargo.toml")).await?;
        let root: toml::Value = toml::from_str(std::str::from_utf8(&manifest)?)?;
        let minimum_cli_version = minimum_cli_version(&root)?;
        if let Some(minimum) = &minimum_cli_version {
            validate_installed_cli(minimum, &git_cli_update(repository, &revision))?;
        }
        let lock_bytes = fetch(&format!("{base}/Cargo.lock")).await?;
        let (source, scaffold) = if let Some(certification) = certification {
            if certification.minimum_cli_version != minimum_cli_version {
                bail!("nightly CLI requirement does not match the certified framework manifest");
            }
            let expected = certification
                .lockfiles
                .get("Cargo.lock")
                .ok_or_else(|| eyre!("nightly certification has no dependency lock"))?;
            if hex::encode(Sha256::digest(&lock_bytes)) != *expected {
                bail!("nightly dependency lock does not match its certification");
            }
            (
                Source::Nightly {
                    repository: repository.to_owned(),
                    revision: revision.clone(),
                    tag: certification.tag,
                    lock_sha256: expected.clone(),
                },
                certification.scaffold,
            )
        } else {
            let metadata = fetch(&format!("{base}/cli/Cargo.toml")).await?;
            (
                Source::Dev {
                    repository: repository.to_owned(),
                    revision: revision.clone(),
                    lock_sha256: hex::encode(Sha256::digest(&lock_bytes)),
                },
                scaffold_metadata(std::str::from_utf8(&metadata)?)?,
            )
        };
        for key in Self::stable().scaffold.keys() {
            if !scaffold.contains_key(key) {
                bail!(
                    "framework revision {revision} has no {key} scaffold metadata required by this CLI"
                );
            }
        }
        let mut patches: PatchSet = root
            .get("patch")
            .cloned()
            .map(toml::Value::try_into)
            .transpose()?
            .unwrap_or_default();
        patches.retain(|source, _| source.trim_end_matches(".git") != repository);
        for dependencies in patches.values_mut() {
            for dependency in dependencies.values_mut() {
                if let Dependency::Detailed(detail) = dependency
                    && detail.path.take().is_some()
                {
                    detail.git = Some(repository.to_owned());
                    detail.rev = Some(revision.clone());
                }
            }
        }
        let lock: Lockfile = std::str::from_utf8(&lock_bytes)?.parse()?;
        let packages = resolve_packages(&scaffold, &lock, repository, &revision)?;
        Ok((
            Self {
                source,
                minimum_cli_version,
                scaffold,
                packages,
                patches,
            },
            Some(lock_bytes),
        ))
    }
}

fn minimum_cli_version(manifest: &toml::Value) -> Result<Option<cargo_toml::SemVer>> {
    manifest
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("waterui"))
        .and_then(|waterui| waterui.get("minimum-cli-version"))
        .cloned()
        .map(toml::Value::try_into)
        .transpose()
        .wrap_err("invalid package.metadata.waterui.minimum-cli-version")
}

fn local_cli_update(root: &Path) -> String {
    format!(
        "run `cargo install --path cli --locked` from the WaterUI checkout at {}",
        root.display()
    )
}

fn registry_cli_update(minimum: &cargo_toml::SemVer) -> String {
    format!(
        "cargo install {} --version '>={minimum}' --locked",
        env!("CARGO_PKG_NAME")
    )
}

fn git_cli_update(repository: &str, revision: &str) -> String {
    format!(
        "cargo install {} --git {repository} --rev {revision} --locked",
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
        bail!(
            "This WaterUI framework requires waterui-cli >= {minimum}, but the running CLI is {current}.\nUpdate the CLI: {update}\nThen verify the installed version with `water --version`."
        );
    }
    Ok(())
}

pub(crate) async fn validate_local_cli(root: &Path) -> Result<()> {
    let contents = smol::fs::read_to_string(root.join("Cargo.toml")).await?;
    let manifest = toml::from_str(&contents)?;
    if let Some(minimum) = minimum_cli_version(&manifest)? {
        validate_installed_cli(&minimum, &local_cli_update(root))?;
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
            Some(source) if source.is_git() => git_cli_update(
                source.url().as_str(),
                source
                    .precise()
                    .ok_or_else(|| eyre!("resolved framework has no Git revision"))?,
            ),
            Some(_) => registry_cli_update(&minimum),
            None => local_cli_update(
                package
                    .manifest_path
                    .parent()
                    .expect("package manifest has a parent")
                    .as_std_path(),
            ),
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
        if key == "android-kotlin-version" {
            continue;
        }
        let Some(name) = key.strip_suffix("-version") else {
            continue;
        };
        let candidates: Vec<_> = lock
            .packages
            .iter()
            .filter(|package| {
                package.name.as_str() == name && package.version.to_string() == *version
            })
            .collect();
        let package = match candidates.as_slice() {
            [package] => *package,
            [] => bail!("framework lock has no package for {name} {version}"),
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

fn scaffold_metadata(contents: &str) -> Result<BTreeMap<String, String>> {
    let manifest: toml::Value = toml::from_str(contents)?;
    manifest["package"]["metadata"]["waterui-scaffold"]
        .clone()
        .try_into()
        .wrap_err("invalid framework scaffold metadata")
}

fn validate_revision(revision: &str) -> Result<()> {
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("framework revision must be a full Git commit hash");
    }
    Ok(())
}

async fn fetch(url: &str) -> Result<Vec<u8>> {
    let mut client = FollowRedirect::new(zenwave::raw_client());
    let response = client
        .method(Method::GET, url)?
        .header("User-Agent", env!("CARGO_PKG_NAME"))?
        .await?;
    if !response.status().is_success() {
        bail!(
            "framework resolution returned HTTP {} from {url}",
            response.status()
        );
    }
    Ok(response.into_body().into_bytes().await?.to_vec())
}

async fn resolve_dev(repository: &str, slug: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["ls-remote", repository, "refs/heads/dev"])
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "could not resolve framework dev: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let revision = std::str::from_utf8(&output.stdout)?
        .split_whitespace()
        .next()
        .ok_or_else(|| eyre!("framework repository has no dev branch"))?
        .to_owned();
    validate_revision(&revision)?;
    let response = fetch(&format!("https://api.github.com/repos/{slug}/actions/workflows/dev.yml/runs?branch=dev&head_sha={revision}&status=success&event=push&per_page=1")).await?;
    let runs: serde_json::Value = serde_json::from_slice(&response)?;
    let checked = runs["workflow_runs"].as_array().is_some_and(|runs| {
        runs.iter().any(|run| {
            run["head_sha"].as_str() == Some(&revision) && run["conclusion"] == "success"
        })
    });
    if !checked {
        bail!("framework dev revision {revision} has not passed its compilation gate");
    }
    Ok(revision)
}

async fn latest_certification(slug: &str) -> Result<Certification> {
    let mut releases = Vec::new();
    let mut page = 1;
    loop {
        let bytes = fetch(&format!(
            "https://api.github.com/repos/{slug}/releases?per_page=100&page={page}"
        ))
        .await?;
        let batch: Vec<Release> = serde_json::from_slice(&bytes)?;
        let complete = batch.len() < 100;
        releases.extend(batch.into_iter().filter(|release| {
            release.prerelease && !release.draft && release.tag_name.starts_with("nightly-")
        }));
        if complete {
            break;
        }
        page += 1;
    }
    let release = releases
        .into_iter()
        .max_by(|left, right| left.published_at.cmp(&right.published_at))
        .ok_or_else(|| eyre!("no certified nightly exists; select dev or stable explicitly"))?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == "framework.json")
        .ok_or_else(|| eyre!("nightly {} has no certification manifest", release.tag_name))?;
    let certification: Certification =
        serde_json::from_slice(&fetch(&asset.browser_download_url).await?)?;
    if certification.schema_version != 1
        || certification.channel != FrameworkChannel::Nightly
        || certification.repository != slug
        || certification.tag != release.tag_name
    {
        bail!("nightly certification does not match its release");
    }
    validate_revision(&certification.revision)?;
    if let Some(minimum) = &certification.minimum_cli_version {
        validate_installed_cli(
            minimum,
            &git_cli_update(env!("CARGO_PKG_REPOSITORY"), &certification.revision),
        )?;
    }
    Ok(certification)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, version: &str, source: Option<&str>) -> cargo_lock::Package {
        cargo_lock::Package {
            name: name.parse().unwrap(),
            version: version.parse().unwrap(),
            source: source.map(|source| source.parse().unwrap()),
            checksum: None,
            dependencies: Vec::new(),
            replace: None,
        }
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
        let repository = env!("CARGO_PKG_REPOSITORY").trim_end_matches(".git");
        let revision = "a".repeat(40);
        let framework = ResolvedFramework {
            source: Source::Nightly {
                repository: repository.to_owned(),
                revision: revision.clone(),
                tag: "nightly-test".into(),
                lock_sha256: hex::encode(Sha256::digest(&bytes)),
            },
            minimum_cli_version: None,
            packages: resolve_packages(&scaffold, lock, repository, &revision).unwrap(),
            scaffold,
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
        let mut manifest = toml::toml! {
            [package.metadata.waterui]
            minimum-cli-version = "0.1.4"
        };
        assert_eq!(
            minimum_cli_version(&toml::Value::Table(manifest.clone())).unwrap(),
            Some("0.1.4".parse().unwrap())
        );
        manifest["package"]["metadata"]["waterui"]["minimum-cli-version"] =
            toml::Value::String(">=0.1.4".into());
        assert!(minimum_cli_version(&toml::Value::Table(manifest)).is_err());
        assert!(
            minimum_cli_version(&toml::Value::Table(toml::Table::new()))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn persisted_cli_requirement_blocks_an_older_cli_with_update_guidance() {
        let mut minimum: cargo_toml::SemVer = env!("CARGO_PKG_VERSION").parse().unwrap();
        minimum.major += 1;
        let mut framework = ResolvedFramework::stable();
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
        let backend_source =
            format!("git+https://example.com/hydrolysis?rev={backend_revision}#{backend_revision}");
        let lock = Lockfile {
            packages: vec![
                package("waterui", "0.3.0", None),
                package("hydrolysis", "0.1.0", Some(&backend_source)),
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
        assert_eq!(framework.channel(), FrameworkChannel::Nightly);
        assert_eq!(framework.dependency("waterui").rev, Some("a".repeat(40)));
        let backend = framework.dependency("hydrolysis");
        assert_eq!(
            backend.git.as_deref(),
            Some("https://example.com/hydrolysis")
        );
        assert_eq!(backend.rev, Some(backend_revision));
        let theme = framework.dependency("hydrolysis-m3");
        assert!(theme.git.is_none());
        assert_eq!(theme.version.unwrap().to_string(), "=0.1.0");
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
        ResolvedFramework::stable()
            .update_manifest(&mut document, &PatchSet::default())
            .unwrap();
        assert_eq!(
            document["dependencies"]["ui"]["package"].as_str(),
            Some("waterui")
        );
        assert!(document["dependencies"]["ui"].get("path").is_none());
        assert_eq!(
            document["dependencies"]["ui"]["version"].as_str(),
            Some("=0.3.0")
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
            Some("=0.3.0")
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

        ResolvedFramework::stable()
            .update_manifest(&mut document, &dev.patches())
            .unwrap();
        let rendered = document.to_string();
        assert!(!rendered.contains("patch"), "{rendered}");
        assert_eq!(
            document["dependencies"]["waterui"]["version"].as_str(),
            Some("=0.3.0")
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
}
