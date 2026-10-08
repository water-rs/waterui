//! Android release signing: the `[signing.android]` manifest section and its
//! reconciliation with `water package --unsigned`.
//!
//! The manifest names the keystore — a path relative to the project root —
//! and the key alias. The store and key passwords are read from the
//! environment variables below, never written to the project or into the
//! generated Gradle files; the generated `signingConfig` reads them itself at
//! build time. Debug packaging keeps the Gradle debug keystore, so `water
//! run` is unaffected.

use std::path::{Path, PathBuf};

use eyre::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    platform::{DeviceSigning, PackageOptions},
    project::{Manifest, Project},
    toolchain::Host,
};

/// Environment variable the release keystore's password is read from.
pub const STORE_PASSWORD_ENV: &str = "WATERUI_ANDROID_STORE_PASSWORD";

/// Environment variable the signing key's password is read from.
pub const KEY_PASSWORD_ENV: &str = "WATERUI_ANDROID_KEY_PASSWORD";

/// Environment variable `water package --unsigned` sets on the Gradle build
/// so the generated release `signingConfig` stays unused.
pub const UNSIGNED_ENV: &str = "WATERUI_ANDROID_UNSIGNED";

/// `[signing.android]` in `Water.toml`: the upload key a release package is
/// signed with.
///
/// Unknown keys are rejected: a `store_password` or a typo'd field must fail
/// parsing, never be ignored — one is a secret committed to the project, the
/// other a configuration that silently does nothing.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AndroidSigningConfig {
    /// Keystore file, relative to the project root.
    #[serde(deserialize_with = "deserialize_keystore_field")]
    keystore: PathBuf,
    /// Alias of the signing key inside the keystore.
    #[serde(deserialize_with = "deserialize_key_alias_field")]
    key_alias: String,
}

/// `keystore` and `key_alias` render into Kotlin string literals in the
/// generated Gradle project; a control character is never a legitimate part
/// of either and breaks the literal it lands in.
fn reject_control_chars(field: &str, value: &str) -> Result<()> {
    if let Some(c) = value.chars().find(|c| c.is_control()) {
        bail!(
            "[signing.android] {field} contains a control character \
             (U+{:04X}); control characters are not valid in {field}",
            c as u32
        );
    }
    Ok(())
}

fn deserialize_keystore_field<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    reject_control_chars("keystore", &value).map_err(serde::de::Error::custom)?;
    Ok(PathBuf::from(value))
}

fn deserialize_key_alias_field<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    reject_control_chars("key_alias", &value).map_err(serde::de::Error::custom)?;
    Ok(value)
}

impl AndroidSigningConfig {
    /// Build the configuration programmatically, under the same invariant the
    /// manifest parser enforces: neither value may carry a control character.
    ///
    /// # Errors
    /// Returns an error naming the field whose value is rejected.
    pub fn new(keystore: impl Into<PathBuf>, key_alias: impl Into<String>) -> Result<Self> {
        let keystore = keystore.into();
        let key_alias = key_alias.into();
        reject_control_chars("keystore", &keystore.to_string_lossy())?;
        reject_control_chars("key_alias", &key_alias)?;
        Ok(Self {
            keystore,
            key_alias,
        })
    }

    /// The keystore path as declared, relative to the project root.
    #[must_use]
    pub fn keystore(&self) -> &Path {
        &self.keystore
    }

    /// The signing key's alias inside the keystore.
    #[must_use]
    pub fn key_alias(&self) -> &str {
        &self.key_alias
    }

    /// The keystore location on disk: `project_root` joined to the declared
    /// path, unless the declaration is already absolute.
    #[must_use]
    pub fn keystore_path(&self, project_root: &Path) -> PathBuf {
        project_root.join(&self.keystore)
    }

    /// Check the configuration against the host: the keystore file exists and
    /// both password variables are set and non-empty.
    ///
    /// # Errors
    /// Returns an error naming the missing keystore path or the unset
    /// variable — both are things the user fixes, never substituted.
    pub fn validate(&self, project_root: &Path, host: &Host) -> Result<()> {
        let keystore_path = self.keystore_path(project_root);
        if !keystore_path.is_file() {
            bail!(
                "the keystore [signing.android] names does not exist: {}",
                keystore_path.display()
            );
        }
        for variable in [STORE_PASSWORD_ENV, KEY_PASSWORD_ENV] {
            let set = host
                .env_string(variable)
                .is_some_and(|value| !value.is_empty());
            if !set {
                bail!(
                    "[signing.android] is configured but {variable} is not set. The store and \
                     key passwords are read from the environment, never written to the project:\n\n\
                     export {STORE_PASSWORD_ENV}=<keystore password>\n\
                     export {KEY_PASSWORD_ENV}=<key password>"
                );
            }
        }
        Ok(())
    }
}

/// How a release-variant Gradle package is signed once `--unsigned` and the
/// manifest's `[signing.android]` are reconciled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseSigning {
    /// Signed with the validated `[signing.android]` configuration.
    Signed,
    /// Unsigned: the manifest declares no signing configuration, so Gradle's
    /// unsigned release output is what was asked for.
    Unconfigured,
    /// Unsigned because `--unsigned` suppressed a declared configuration; the
    /// generated Gradle project still carries the `signingConfig`, so the
    /// build is told to skip it through [`UNSIGNED_ENV`].
    Suppressed,
}

/// Reconcile the release signing a package applies.
///
/// `[signing.android]` applies when declared and the package was not asked
/// unsigned; without it a release package is an error naming the section to
/// add — an unsigned APK installs nowhere and an unsigned AAB uploads
/// nowhere, so only an explicit `--unsigned` produces one.
///
/// # Errors
/// - the manifest declares no `[signing.android]` and the package was not
///   asked unsigned;
/// - the declared keystore does not exist;
/// - a password variable is unset or empty.
pub(crate) fn resolve_release_signing(
    manifest: &Manifest,
    project_root: &Path,
    host: &Host,
    signing: DeviceSigning,
) -> Result<ReleaseSigning> {
    match manifest.signing.android.as_ref() {
        Some(config) => match signing {
            DeviceSigning::Unsigned => Ok(ReleaseSigning::Suppressed),
            DeviceSigning::Automatic => {
                config.validate(project_root, host)?;
                Ok(ReleaseSigning::Signed)
            }
        },
        None => match signing {
            DeviceSigning::Unsigned => Ok(ReleaseSigning::Unconfigured),
            DeviceSigning::Automatic => Err(missing_signing_error()),
        },
    }
}

/// The signing decision a packaging operation resolved once, bound to the
/// project and package options it was resolved against.
///
/// Constructing one is [`PreparedSigning::resolve`], which validates the
/// manifest's `[signing.android]` against the host — the keystore probe and
/// the password-env reads happen exactly once per operation, before any Rust
/// builds when the caller prepares ahead. Nothing else can produce one, so a
/// packaging step holding a `Signed` or `Suppressed` plan holds a validated
/// decision rather than a token a caller asserted.
///
/// The packaging step hands its own project root and options back through
/// [`PreparedSigning::release_signing_for`], which re-proves the binding: a
/// plan prepared for one project cannot package another, and options changed
/// after resolution are an error, not a stale decision.
#[derive(Debug)]
pub struct PreparedSigning {
    project_root: PathBuf,
    release_signing: Option<ReleaseSigning>,
    /// The option values the decision was made against; packaging re-checks
    /// them so a plan cannot outlive an options edit.
    debug: bool,
    device_signing: DeviceSigning,
}

impl PreparedSigning {
    /// Resolve the release signing `project` and `options` require.
    ///
    /// A debug package needs no decision (`release_signing` stays `None`):
    /// it signs with the Gradle debug identity. A release package resolves
    /// and validates once — callers that prepare before the Rust builds
    /// (`water package`, `water run`) surface a missing `[signing.android]`
    /// before compilation; callers that invoke packaging directly resolve
    /// here at the same single validation.
    ///
    /// # Errors
    /// Whatever `resolve_release_signing` rejects: no declared
    /// configuration, a missing keystore, or an unset password variable.
    pub fn resolve(project: &Project, options: &PackageOptions) -> Result<Self> {
        let release_signing = (!options.is_debug())
            .then(|| {
                resolve_release_signing(
                    project.manifest(),
                    project.root(),
                    &Host::current(),
                    options.device_signing(),
                )
            })
            .transpose()?;
        Ok(Self {
            project_root: project.root().to_path_buf(),
            release_signing,
            debug: options.is_debug(),
            device_signing: options.device_signing(),
        })
    }

    /// The decision this plan carries for the packaging step's own
    /// `project_root` and `options`.
    ///
    /// # Errors
    /// - the plan was prepared for a different project root;
    /// - the debug/signature options the package runs with differ from the
    ///   ones the plan was resolved against.
    pub fn release_signing_for(
        &self,
        project_root: &Path,
        options: &PackageOptions,
    ) -> Result<Option<ReleaseSigning>> {
        ensure!(
            self.project_root == project_root,
            "release signing was prepared for {} and cannot package {}",
            self.project_root.display(),
            project_root.display()
        );
        ensure!(
            self.debug == options.is_debug() && self.device_signing == options.device_signing(),
            "package options changed since release signing was resolved; resolve it again"
        );
        Ok(self.release_signing)
    }
}

/// The error a release package without `[signing.android]` fails with: the
/// exact section to add, the variables to export, and the way out.
fn missing_signing_error() -> eyre::Error {
    eyre::eyre!(
        "Android release packaging requires a signing configuration, and Water.toml declares none.\n\
         Add the upload key to the manifest:\n\n\
         [signing.android]\n\
         keystore = \"release.keystore\"   # relative to the project root\n\
         key_alias = \"upload\"\n\n\
         The store and key passwords are read from the environment, never written to the project:\n\n\
         export {STORE_PASSWORD_ENV}=<keystore password>\n\
         export {KEY_PASSWORD_ENV}=<key password>\n\n\
         For a deliberately unsigned package, pass --unsigned to `water package` \
         (`water run` signs release builds unconditionally)."
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{
        AndroidSigningConfig, KEY_PASSWORD_ENV, ReleaseSigning, STORE_PASSWORD_ENV,
        resolve_release_signing,
    };
    use crate::{
        platform::DeviceSigning,
        project::{Manifest, Package},
        project_types::BundleIdentifier,
        toolchain::Host,
    };

    fn manifest_with_signing(keystore: &str) -> Manifest {
        let text = format!(
            "[package]\n\
             name = \"Test\"\n\
             bundle_identifier = \"dev.waterui.test\"\n\n\
             [signing.android]\n\
             keystore = \"{keystore}\"\n\
             key_alias = \"upload\"\n"
        );
        Manifest::parse(&text).expect("manifest with [signing.android] parses")
    }

    fn manifest_without_signing() -> Manifest {
        Manifest::new(Package {
            name: "Test".to_string(),
            bundle_identifier: BundleIdentifier::try_from("dev.waterui.test")
                .expect("bundle identifier"),
            assets_path: "assets".to_string(),
            accessory: false,
            embedded: false,
        })
    }

    fn host_with_passwords() -> Host {
        Host::new(
            Vec::<PathBuf>::new(),
            [
                (STORE_PASSWORD_ENV, "store-secret"),
                (KEY_PASSWORD_ENV, "key-secret"),
            ],
        )
    }

    #[test]
    fn signing_section_parses_from_the_manifest() {
        let manifest = manifest_with_signing("keys/release.keystore");
        let signing = manifest
            .signing
            .android
            .as_ref()
            .expect("[signing.android] is present");
        assert_eq!(signing.keystore(), Path::new("keys/release.keystore"));
        assert_eq!(signing.key_alias(), "upload");
    }

    #[test]
    fn manifest_without_signing_has_no_android_entry() {
        let manifest = manifest_without_signing();
        assert!(manifest.signing.android.is_none());
        assert!(manifest.signing.is_empty());
    }

    #[test]
    fn release_without_signing_config_fails_naming_the_section() {
        let manifest = manifest_without_signing();
        let error = resolve_release_signing(
            &manifest,
            PathBuf::from("/tmp/project").as_path(),
            &host_with_passwords(),
            DeviceSigning::Automatic,
        )
        .expect_err("a release package without signing must fail");
        let message = format!("{error}");
        assert!(message.contains("[signing.android]"), "{message}");
        assert!(message.contains("key_alias"), "{message}");
        assert!(message.contains(STORE_PASSWORD_ENV), "{message}");
        assert!(message.contains(KEY_PASSWORD_ENV), "{message}");
        assert!(message.contains("--unsigned"), "{message}");
    }

    #[test]
    fn unsigned_without_signing_config_is_unconfigured() {
        let manifest = manifest_without_signing();
        let signing = resolve_release_signing(
            &manifest,
            PathBuf::from("/tmp/project").as_path(),
            &host_with_passwords(),
            DeviceSigning::Unsigned,
        )
        .expect("unsigned without a section resolves");
        assert_eq!(signing, ReleaseSigning::Unconfigured);
    }

    #[test]
    fn unsigned_suppresses_a_declared_config_without_touching_it() {
        // No keystore on disk and no passwords: --unsigned must not validate
        // the configuration it suppresses — the whole point is a host that
        // lacks them.
        let manifest = manifest_with_signing("release.keystore");
        let host = Host::new(Vec::<PathBuf>::new(), Vec::<(&str, &str)>::new());
        let signing = resolve_release_signing(
            &manifest,
            PathBuf::from("/tmp/project").as_path(),
            &host,
            DeviceSigning::Unsigned,
        )
        .expect("--unsigned resolves without the keystore or passwords");
        assert_eq!(signing, ReleaseSigning::Suppressed);
    }

    #[test]
    fn signing_resolves_when_keystore_and_passwords_exist() {
        let dir = tempdir().expect("temp dir");
        let keystore = dir.path().join("release.keystore");
        std::fs::write(&keystore, b"not a real keystore").expect("write keystore");
        let manifest = manifest_with_signing("release.keystore");
        let signing = resolve_release_signing(
            &manifest,
            dir.path(),
            &host_with_passwords(),
            DeviceSigning::Automatic,
        )
        .expect("valid signing resolves");
        assert_eq!(signing, ReleaseSigning::Signed);
    }

    #[test]
    fn signing_fails_when_the_keystore_is_missing() {
        let dir = tempdir().expect("temp dir");
        let manifest = manifest_with_signing("release.keystore");
        let error = resolve_release_signing(
            &manifest,
            dir.path(),
            &host_with_passwords(),
            DeviceSigning::Automatic,
        )
        .expect_err("a missing keystore must fail");
        let message = format!("{error}");
        assert!(message.contains("release.keystore"), "{message}");
    }

    #[test]
    fn signing_fails_naming_an_unset_password_variable() {
        let dir = tempdir().expect("temp dir");
        std::fs::write(dir.path().join("release.keystore"), b"ks").expect("write keystore");
        let manifest = manifest_with_signing("release.keystore");
        let host = Host::new(
            Vec::<PathBuf>::new(),
            [(STORE_PASSWORD_ENV, "store-secret")],
        );
        let error = resolve_release_signing(&manifest, dir.path(), &host, DeviceSigning::Automatic)
            .expect_err("an unset password variable must fail");
        let message = format!("{error}");
        assert!(message.contains(KEY_PASSWORD_ENV), "{message}");
    }

    #[test]
    fn signing_fails_on_an_empty_password_variable() {
        let dir = tempdir().expect("temp dir");
        std::fs::write(dir.path().join("release.keystore"), b"ks").expect("write keystore");
        let manifest = manifest_with_signing("release.keystore");
        let host = Host::new(
            Vec::<PathBuf>::new(),
            [(STORE_PASSWORD_ENV, "store-secret"), (KEY_PASSWORD_ENV, "")],
        );
        let error = resolve_release_signing(&manifest, dir.path(), &host, DeviceSigning::Automatic)
            .expect_err("an empty password variable must fail");
        let message = format!("{error}");
        assert!(message.contains(KEY_PASSWORD_ENV), "{message}");
    }

    #[test]
    fn a_password_key_in_signing_android_fails_to_parse() {
        let text = "[package]\n\
                    name = \"Test\"\n\
                    bundle_identifier = \"dev.waterui.test\"\n\n\
                    [signing.android]\n\
                    keystore = \"release.keystore\"\n\
                    key_alias = \"upload\"\n\
                    store_password = \"never-on-disk\"\n";
        let error = Manifest::parse(text).expect_err("a password in the manifest must be rejected");
        let message = format!("{error}");
        assert!(message.contains("store_password"), "{message}");
    }

    #[test]
    fn an_unknown_key_in_signing_fails_to_parse() {
        let text = "[package]\n\
                    name = \"Test\"\n\
                    bundle_identifier = \"dev.waterui.test\"\n\n\
                    [signing]\n\
                    andriod = { keystore = \"release.keystore\", key_alias = \"upload\" }\n";
        let error = Manifest::parse(text).expect_err("a typo'd signing key must be rejected");
        let message = format!("{error}");
        assert!(message.contains("andriod"), "{message}");
    }

    #[test]
    fn a_control_character_in_the_keystore_fails_to_parse() {
        let text = "[package]\n\
                    name = \"Test\"\n\
                    bundle_identifier = \"dev.waterui.test\"\n\n\
                    [signing.android]\n\
                    keystore = \"release\\tkeystore\"\n\
                    key_alias = \"upload\"\n";
        let error = Manifest::parse(text).expect_err("a control character must be rejected");
        let message = format!("{error}");
        assert!(message.contains("keystore"), "{message}");
        assert!(message.contains("control character"), "{message}");
    }

    #[test]
    fn a_control_character_in_the_key_alias_fails_to_parse() {
        let text = "[package]\n\
                    name = \"Test\"\n\
                    bundle_identifier = \"dev.waterui.test\"\n\n\
                    [signing.android]\n\
                    keystore = \"release.keystore\"\n\
                    key_alias = \"up\\nload\"\n";
        let error = Manifest::parse(text).expect_err("a control character must be rejected");
        let message = format!("{error}");
        assert!(message.contains("key_alias"), "{message}");
        assert!(message.contains("control character"), "{message}");
    }

    #[test]
    fn keystore_path_resolves_relative_to_the_project_root() {
        let config = AndroidSigningConfig::new("keys/release.keystore", "upload")
            .expect("a valid programmatic config");
        assert_eq!(
            config.keystore_path(Path::new("/tmp/app")),
            PathBuf::from("/tmp/app/keys/release.keystore")
        );
    }

    #[test]
    fn a_programmatic_config_rejects_control_characters() {
        let error = AndroidSigningConfig::new("release\u{7}keystore", "upload")
            .expect_err("a control character in the keystore must fail");
        assert!(format!("{error}").contains("keystore"), "{error}");
        let error = AndroidSigningConfig::new("release.keystore", "up\nload")
            .expect_err("a control character in the alias must fail");
        assert!(format!("{error}").contains("key_alias"), "{error}");
    }

    #[test]
    fn a_prepared_plan_applies_to_its_own_project_and_options() {
        let options = crate::platform::PackageOptions::development();
        let plan = super::PreparedSigning {
            project_root: PathBuf::from("/tmp/app"),
            release_signing: Some(ReleaseSigning::Signed),
            debug: options.is_debug(),
            device_signing: options.device_signing(),
        };
        let signing = plan
            .release_signing_for(Path::new("/tmp/app"), &options)
            .expect("the bound project and options accept the plan");
        assert_eq!(signing, Some(ReleaseSigning::Signed));
    }

    #[test]
    fn a_prepared_plan_rejects_a_different_project() {
        let options = crate::platform::PackageOptions::development();
        let plan = super::PreparedSigning {
            project_root: PathBuf::from("/tmp/app"),
            release_signing: Some(ReleaseSigning::Signed),
            debug: options.is_debug(),
            device_signing: options.device_signing(),
        };
        let error = plan
            .release_signing_for(Path::new("/tmp/other-app"), &options)
            .expect_err("a plan must not package a project it was not resolved for");
        assert!(format!("{error}").contains("/tmp/other-app"), "{error}");
    }

    #[test]
    fn a_prepared_plan_rejects_options_changed_after_resolution() {
        let options = crate::platform::PackageOptions::development();
        let plan = super::PreparedSigning {
            project_root: PathBuf::from("/tmp/app"),
            release_signing: Some(ReleaseSigning::Signed),
            debug: options.is_debug(),
            device_signing: options.device_signing(),
        };
        for changed in [
            options.clone().with_device_signing(DeviceSigning::Unsigned),
            options.with_debug(false),
        ] {
            assert!(
                plan.release_signing_for(Path::new("/tmp/app"), &changed)
                    .is_err(),
                "options changed after resolution must not reuse the plan"
            );
        }
    }

    #[test]
    fn prepared_signing_resolves_and_validates_for_a_real_project() {
        let dir = tempdir().expect("temp dir");
        let root = dir.path().join("water-example");
        let project = smol::block_on(crate::project::Project::create(
            &root,
            crate::project::CreateOptions {
                name: "Water Example".to_string(),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.waterexample")
                    .expect("bundle identifier"),
                waterui_path: None,
                channel: None,
                framework_manifest: None,
                framework: Some(crate::framework::test_fixtures::stable_framework()),
                framework_lock: None,
                author: "Test".to_string(),
                web: None,
            },
        ))
        .expect("project creation must succeed");
        // The scaffold declares no [signing.android]: a release package must
        // fail in resolve() — once, before any build output exists — rather
        // than producing an unsigned artifact.
        let release = crate::platform::PackageOptions::packaging(
            crate::platform::PackageAudience::Development,
            false,
        );
        let error = super::PreparedSigning::resolve(&project, &release)
            .expect_err("a release package without [signing.android] must fail");
        assert!(format!("{error}").contains("[signing.android]"), "{error}");
        // A debug package resolves with no decision: the Gradle debug
        // identity signs it.
        let debug = crate::platform::PackageOptions::development();
        let plan =
            super::PreparedSigning::resolve(&project, &debug).expect("debug needs no decision");
        assert_eq!(
            plan.release_signing_for(project.root(), &debug)
                .expect("the bound options apply"),
            None
        );

        // The plan is bound to its project: packaging a second project with
        // it is an error at the API boundary, not a transplant.
        let other_dir = tempdir().expect("temp dir");
        let other_root = other_dir.path().join("other-app");
        let other = smol::block_on(crate::project::Project::create(
            &other_root,
            crate::project::CreateOptions {
                name: "Other App".to_string(),
                bundle_identifier: BundleIdentifier::try_from("dev.waterui.otherapp")
                    .expect("bundle identifier"),
                waterui_path: None,
                channel: None,
                framework_manifest: None,
                framework: Some(crate::framework::test_fixtures::stable_framework()),
                framework_lock: None,
                author: "Test".to_string(),
                web: None,
            },
        ))
        .expect("second project creation must succeed");
        assert!(
            plan.release_signing_for(other.root(), &debug).is_err(),
            "a plan must not package a project it was not resolved for"
        );
        // And options mutated after resolution are stale, not silently
        // re-applied.
        let changed = debug.with_device_signing(DeviceSigning::Unsigned);
        assert!(
            plan.release_signing_for(project.root(), &changed).is_err(),
            "options changed after resolution must not reuse the plan"
        );
    }
}
