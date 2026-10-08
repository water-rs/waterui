//! Configuration only the app knows — a Firebase project, Apple Pay
//! merchants, a Cast receiver — which a crate requests and the app supplies
//! in `Water.toml`.
//!
//! The crate names the value it needs, gated like its other tables:
//!
//! ```toml
//! [[package.metadata.waterui.app-value]]
//! key = "firebase_config"
//! required-feature = "remote"
//! ```
//!
//! The app supplies it:
//!
//! ```toml
//! [app_values]
//! firebase_config = "google-services.json"
//! apple_pay_merchant_ids = ["merchant.com.example.store"]
//! cast_receiver_app_id = "CC1AD845"
//! ```
//!
//! and the CLI places each value where the platform reads it: the Firebase
//! file as the Android module's `google-services.json`, the merchant IDs as
//! the `com.apple.developer.in-app-payments` entitlement, the Cast receiver
//! as the Android string resource [`CAST_RECEIVER_APP_ID_RESOURCE`] — which
//! a crate's `<meta-data resource = "@string/…">` can also point at. A
//! requested value the app does not supply fails packaging, naming the key
//! and the crate that requested it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use askama::Template;
use eyre::Context;
use serde::{Deserialize, Serialize};
use smol::fs;

/// The Android string resource carrying the app's Cast receiver ID.
pub const CAST_RECEIVER_APP_ID_RESOURCE: &str = "waterui_cast_receiver_app_id";

/// Where the Firebase configuration lands in the Gradle module: the file
/// the `com.google.gms.google-services` plugin reads.
const FIREBASE_CONFIG_FILE: &str = "google-services.json";

/// The managed resource file carrying the app-supplied Android strings.
const APP_VALUES_RESOURCE_FILE: &str = "src/main/res/values/waterui_app_values.xml";

/// A value a crate can request from the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppValueKey {
    /// The Firebase Android configuration file (`google-services.json`).
    FirebaseConfig,
    /// The Apple Pay merchant IDs the app processes payments for.
    ApplePayMerchantIds,
    /// The Cast receiver application the app casts to.
    CastReceiverAppId,
}

impl AppValueKey {
    /// The key as `Water.toml` spells it.
    const fn name(self) -> &'static str {
        match self {
            Self::FirebaseConfig => "firebase_config",
            Self::ApplePayMerchantIds => "apple_pay_merchant_ids",
            Self::CastReceiverAppId => "cast_receiver_app_id",
        }
    }

    /// A value of the right shape, for the missing-value error.
    const fn example(self) -> &'static str {
        match self {
            Self::FirebaseConfig => "\"google-services.json\"",
            Self::ApplePayMerchantIds => "[\"merchant.com.example\"]",
            Self::CastReceiverAppId => "\"CC1AD845\"",
        }
    }
}

/// One `[[package.metadata.waterui.app-value]]` entry.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct AppValueRequest {
    /// The value the crate needs.
    pub(super) key: AppValueKey,
    /// Only requested when this cargo feature is enabled on the declaring
    /// crate.
    #[serde(default)]
    pub(super) required_feature: Option<String>,
}

/// The app-supplied values the dependency graph requests, and the crates
/// requesting each.
#[derive(Debug, Default)]
pub struct RequiredAppValues {
    requested: BTreeMap<AppValueKey, Vec<String>>,
}

impl RequiredAppValues {
    /// Records that `crate_name` requests `key`.
    pub(super) fn request(&mut self, crate_name: &str, key: AppValueKey) {
        let crates = self.requested.entry(key).or_default();
        if !crates.iter().any(|name| name == crate_name) {
            crates.push(crate_name.to_owned());
        }
    }

    /// The crates requesting `key`; `None` when none does.
    pub(super) fn requesters(&self, key: AppValueKey) -> Option<&[String]> {
        self.requested.get(&key).map(Vec::as_slice)
    }

    /// The app's `supplied` value for `key` when a crate requests it.
    ///
    /// # Errors
    ///
    /// Returns an error naming the key and the requesting crates when a
    /// crate requests `key` and the app does not supply it.
    pub(super) fn resolve<'a, T>(
        &self,
        key: AppValueKey,
        supplied: Option<&'a T>,
    ) -> eyre::Result<Option<&'a T>> {
        let Some(crates) = self.requesters(key) else {
            return Ok(None);
        };
        let Some(value) = supplied else {
            eyre::bail!(
                "crate `{}` needs the app-supplied value `{}`; add `{} = {}` under `[app_values]` in Water.toml",
                crates.join("`, `"),
                key.name(),
                key.name(),
                key.example()
            );
        };
        Ok(Some(value))
    }
}

/// The values the app supplies (`[app_values]` in `Water.toml`).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppValuesConfig {
    /// Path, relative to the project root, of the Firebase Android
    /// configuration file downloaded from the Firebase console.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firebase_config: Option<PathBuf>,
    /// The Apple Pay merchant IDs the app processes payments for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apple_pay_merchant_ids: Option<ApplePayMerchantIds>,
    /// The Cast receiver application ID from the Cast developer console.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cast_receiver_app_id: Option<CastReceiverAppId>,
}

impl AppValuesConfig {
    /// Whether the app supplies no value.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.firebase_config.is_none()
            && self.apple_pay_merchant_ids.is_none()
            && self.cast_receiver_app_id.is_none()
    }
}

/// An Apple Pay merchant ID: `merchant.` followed by a reverse-DNS name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ApplePayMerchantId(String);

impl TryFrom<String> for ApplePayMerchantId {
    type Error = String;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        let name = id.strip_prefix("merchant.").unwrap_or_default();
        if name.is_empty()
            || name.split('.').any(str::is_empty)
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        {
            return Err(format!(
                "Apple Pay merchant ID `{id}` must be `merchant.` followed by a reverse-DNS name, such as `merchant.com.example`"
            ));
        }
        Ok(Self(id))
    }
}

impl From<ApplePayMerchantId> for String {
    fn from(id: ApplePayMerchantId) -> Self {
        id.0
    }
}

/// A non-empty list of distinct Apple Pay merchant IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<ApplePayMerchantId>", into = "Vec<ApplePayMerchantId>")]
pub struct ApplePayMerchantIds(Vec<ApplePayMerchantId>);

impl TryFrom<Vec<ApplePayMerchantId>> for ApplePayMerchantIds {
    type Error = String;

    fn try_from(ids: Vec<ApplePayMerchantId>) -> Result<Self, Self::Error> {
        if ids.is_empty() {
            return Err("`apple_pay_merchant_ids` must list at least one merchant ID".to_owned());
        }
        for (index, id) in ids.iter().enumerate() {
            if ids[..index].contains(id) {
                return Err(format!("`apple_pay_merchant_ids` lists `{}` twice", id.0));
            }
        }
        Ok(Self(ids))
    }
}

impl From<ApplePayMerchantIds> for Vec<ApplePayMerchantId> {
    fn from(ids: ApplePayMerchantIds) -> Self {
        ids.0
    }
}

impl ApplePayMerchantIds {
    /// The IDs as the `com.apple.developer.in-app-payments` entitlement
    /// value.
    pub(super) fn to_plist(&self) -> plist::Value {
        plist::Value::Array(
            self.0
                .iter()
                .map(|id| plist::Value::String(id.0.clone()))
                .collect(),
        )
    }
}

/// A Cast receiver application ID: ASCII letters and digits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CastReceiverAppId(String);

impl TryFrom<String> for CastReceiverAppId {
    type Error = String;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(format!(
                "Cast receiver application ID `{id}` must be the ASCII letters and digits the Cast developer console assigns, such as `CC1AD845`"
            ));
        }
        Ok(Self(id))
    }
}

impl From<CastReceiverAppId> for String {
    fn from(id: CastReceiverAppId) -> Self {
        id.0
    }
}

#[derive(Template)]
#[template(path = "src/templates/android_res/app_values.xml.tpl", escape = "xml")]
struct AppValuesResourceTemplate<'a> {
    cast_receiver_app_id_resource: &'static str,
    cast_receiver_app_id: &'a str,
}

/// Places the Android values the graph `required` requests from `config`
/// into the Gradle module at `module_dir`; relative paths in `config`
/// resolve against `project_root`.
///
/// Both destinations are managed: a value no crate requests any longer is
/// removed. The embedded AAR is a library, so the Firebase configuration —
/// which the host application's `google-services` plugin reads — is the
/// host's to carry, and the stage names the requesting crates instead.
///
/// # Errors
///
/// Returns an error naming the key when a requested value is missing, and
/// when the Firebase configuration cannot be read or is not JSON.
pub(super) async fn stage_android_app_values(
    module_dir: &Path,
    project_root: &Path,
    required: &RequiredAppValues,
    config: &AppValuesConfig,
) -> eyre::Result<()> {
    let firebase_destination = module_dir.join(FIREBASE_CONFIG_FILE);
    let firebase =
        required.resolve(AppValueKey::FirebaseConfig, config.firebase_config.as_ref())?;
    match firebase {
        Some(path) => {
            let source = project_root.join(path);
            let contents = fs::read(&source).await.wrap_err_with(|| {
                format!(
                    "reading the Firebase configuration `firebase_config` names at {}",
                    source.display()
                )
            })?;
            serde_json::from_slice::<serde_json::Value>(&contents).wrap_err_with(|| {
                format!(
                    "the Firebase configuration at {} is not JSON; download `{FIREBASE_CONFIG_FILE}` from the Firebase console",
                    source.display()
                )
            })?;
            super::super::templates::write_file_if_changed(&firebase_destination, &contents)
                .await
                .wrap_err_with(|| format!("writing {}", firebase_destination.display()))?;
        }
        None => remove_managed_file(&firebase_destination).await?,
    }

    let resource = module_dir.join(APP_VALUES_RESOURCE_FILE);
    match required.resolve(
        AppValueKey::CastReceiverAppId,
        config.cast_receiver_app_id.as_ref(),
    )? {
        Some(id) => {
            let rendered = AppValuesResourceTemplate {
                cast_receiver_app_id_resource: CAST_RECEIVER_APP_ID_RESOURCE,
                cast_receiver_app_id: &id.0,
            }
            .render()
            .wrap_err("rendering the app-supplied Android resources")?;
            if let Some(parent) = resource.parent() {
                fs::create_dir_all(parent).await?;
            }
            super::super::templates::write_file_if_changed(&resource, rendered.as_bytes())
                .await
                .wrap_err_with(|| format!("writing {}", resource.display()))?;
        }
        None => remove_managed_file(&resource).await?,
    }
    Ok(())
}

/// Removes a managed file an earlier stage wrote; absent is the goal state.
async fn remove_managed_file(path: &Path) -> eyre::Result<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).wrap_err_with(|| format!("removing {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> AppValuesConfig {
        toml::from_str(text).expect("[app_values] parses")
    }

    fn requiring(keys: &[AppValueKey]) -> RequiredAppValues {
        let mut required = RequiredAppValues::default();
        for key in keys {
            required.request("waterkit-capability", *key);
        }
        required
    }

    fn stage(
        module: &Path,
        project: &Path,
        required: &RequiredAppValues,
        config: &AppValuesConfig,
    ) -> eyre::Result<()> {
        smol::block_on(stage_android_app_values(module, project, required, config))
    }

    #[test]
    fn a_missing_requested_value_names_the_key_and_crate() {
        let module = tempfile::tempdir().expect("module dir");
        let project = tempfile::tempdir().expect("project dir");
        for key in [AppValueKey::FirebaseConfig, AppValueKey::CastReceiverAppId] {
            let message = stage(
                module.path(),
                project.path(),
                &requiring(&[key]),
                &AppValuesConfig::default(),
            )
            .expect_err("a requested value the app omits must fail")
            .to_string();
            assert!(message.contains(key.name()), "{message}");
            assert!(message.contains("`waterkit-capability`"), "{message}");
            assert!(message.contains("[app_values]"), "{message}");
        }
    }

    #[test]
    fn supplied_values_are_placed_and_unrequested_ones_removed() {
        let module = tempfile::tempdir().expect("module dir");
        let project = tempfile::tempdir().expect("project dir");
        let firebase = br#"{"project_info":{"project_id":"probe"}}"#;
        std::fs::write(project.path().join("google-services.json"), firebase)
            .expect("firebase fixture");
        let supplied = config(
            "firebase_config = \"google-services.json\"\ncast_receiver_app_id = \"CC1AD845\"\n",
        );

        stage(
            module.path(),
            project.path(),
            &requiring(&[AppValueKey::FirebaseConfig, AppValueKey::CastReceiverAppId]),
            &supplied,
        )
        .expect("supplied values stage");
        assert_eq!(
            std::fs::read(module.path().join(FIREBASE_CONFIG_FILE)).expect("staged firebase"),
            firebase
        );
        let resource = std::fs::read_to_string(module.path().join(APP_VALUES_RESOURCE_FILE))
            .expect("staged resource");
        assert!(
            resource.contains(
                "<string name=\"waterui_cast_receiver_app_id\" translatable=\"false\">CC1AD845</string>"
            ),
            "{resource}"
        );

        // Supplied but no longer requested: the managed files go away.
        stage(
            module.path(),
            project.path(),
            &RequiredAppValues::default(),
            &supplied,
        )
        .expect("an unrequested value stages nothing");
        assert!(!module.path().join(FIREBASE_CONFIG_FILE).exists());
        assert!(!module.path().join(APP_VALUES_RESOURCE_FILE).exists());
    }

    #[test]
    fn an_unreadable_or_malformed_firebase_config_fails() {
        let module = tempfile::tempdir().expect("module dir");
        let project = tempfile::tempdir().expect("project dir");
        let required = requiring(&[AppValueKey::FirebaseConfig]);
        let supplied = config("firebase_config = \"google-services.json\"\n");
        let missing = stage(module.path(), project.path(), &required, &supplied)
            .expect_err("a named file that does not exist must fail");
        assert!(
            format!("{missing:#}").contains("google-services.json"),
            "{missing:#}"
        );

        std::fs::write(project.path().join("google-services.json"), "not json")
            .expect("malformed fixture");
        let malformed = stage(module.path(), project.path(), &required, &supplied)
            .expect_err("a file that is not JSON must fail");
        assert!(malformed.to_string().contains("is not JSON"), "{malformed}");
    }

    #[test]
    fn app_values_round_trip_through_water_toml() {
        let text = "firebase_config = \"config/google-services.json\"\napple_pay_merchant_ids = [\"merchant.com.example\", \"merchant.com.example.eu\"]\ncast_receiver_app_id = \"CC1AD845\"\n";
        let parsed = config(text);
        assert_eq!(toml::to_string(&parsed).expect("serialize"), text);
        assert!(!parsed.is_empty());
        assert!(config("").is_empty());
    }

    #[test]
    fn malformed_app_values_fail_to_parse() {
        for text in [
            "firebase = \"google-services.json\"\n",
            "apple_pay_merchant_ids = []\n",
            "apple_pay_merchant_ids = [\"com.example\"]\n",
            "apple_pay_merchant_ids = [\"merchant.\"]\n",
            "apple_pay_merchant_ids = [\"merchant.com..example\"]\n",
            "apple_pay_merchant_ids = [\"merchant.com.example\", \"merchant.com.example\"]\n",
            "cast_receiver_app_id = \"\"\n",
            "cast_receiver_app_id = \"CC1A<D845\"\n",
        ] {
            assert!(
                toml::from_str::<AppValuesConfig>(text).is_err(),
                "must reject: {text}"
            );
        }
    }

    #[test]
    fn malformed_requests_fail_to_parse() {
        for text in [
            "key = \"firebase-config\"\n",
            "key = \"google_maps_key\"\n",
            "key = \"firebase_config\"\nfeature = \"remote\"\n",
        ] {
            assert!(
                toml::from_str::<AppValueRequest>(text).is_err(),
                "must reject: {text}"
            );
        }
        let request: AppValueRequest =
            toml::from_str("key = \"cast_receiver_app_id\"\nrequired-feature = \"cast\"\n")
                .expect("a well-formed request parses");
        assert_eq!(request.key, AppValueKey::CastReceiverAppId);
        assert_eq!(request.required_feature.as_deref(), Some("cast"));
    }
}
