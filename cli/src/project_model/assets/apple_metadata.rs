//! Entitlements and `Info.plist` keys a crate declares under
//! `[package.metadata.waterui.apple]`, merged into the packaged Apple app.
//!
//! A crate whose Apple side needs a capability the signature must claim, or
//! an `Info.plist` key the system reads, declares it beside its permissions:
//!
//! ```toml
//! [package.metadata.waterui.apple]
//! required-feature = "remote"
//! environment-entitlements = ["aps-environment"]
//!
//! [package.metadata.waterui.apple.entitlements]
//! "com.apple.developer.applesignin" = ["Default"]
//!
//! [package.metadata.waterui.apple.info-plist]
//! UIBackgroundModes = ["remote-notification"]
//! ```
//!
//! Entitlements whose value is the APNs or App Attest environment are not
//! literals: the crate names them in `environment-entitlements` and the CLI
//! writes `development` or `production` from the signing kind, under the key
//! the platform reads. Array values merge as a union — every crate's
//! background modes reach the app — while scalars must agree, and a
//! disagreement names both declarations.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Deserialize;

use super::app_values::{AppValueKey, AppValuesConfig, RequiredAppValues};
use crate::platform::TargetPlatform;

/// Entitlements the signing path derives from the provisioning profile; a
/// crate cannot declare them.
const PROFILE_ENTITLEMENTS: &[&str] = &[
    "application-identifier",
    "com.apple.application-identifier",
    "com.apple.developer.team-identifier",
    "get-task-allow",
];

/// The Apple Pay entitlement, whose merchant IDs only the app knows: a crate
/// requests the `apple_pay_merchant_ids` app value instead of declaring it.
const IN_APP_PAYMENTS: &str = "com.apple.developer.in-app-payments";

/// How the app is signed, which decides the APNs and App Attest environment
/// the signature claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningEnvironment {
    /// A development signature: a development provisioning profile, which
    /// grants the sandbox environments.
    Development,
    /// A distribution signature, which claims the production environments.
    Production,
}

impl SigningEnvironment {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }
}

/// An entitlement whose value is the signing environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum EnvironmentEntitlement {
    /// The APNs environment remote notifications are delivered through.
    ApsEnvironment,
    /// The App Attest environment attestations are issued in.
    AppAttestEnvironment,
}

impl EnvironmentEntitlement {
    /// The entitlement key `platform` reads: macOS namespaces the APNs
    /// environment under `com.apple.developer.`.
    const fn key(self, platform: TargetPlatform) -> &'static str {
        match (self, platform) {
            (Self::ApsEnvironment, TargetPlatform::MacOS) => "com.apple.developer.aps-environment",
            (Self::ApsEnvironment, _) => "aps-environment",
            (Self::AppAttestEnvironment, _) => {
                "com.apple.developer.devicecheck.appattest-environment"
            }
        }
    }

    const ALL: [Self; 2] = [Self::ApsEnvironment, Self::AppAttestEnvironment];
}

/// A literal entitlement key: neither one the provisioning profile supplies
/// nor one whose value is the signing environment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub(super) struct EntitlementKey(String);

impl TryFrom<String> for EntitlementKey {
    type Error = String;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        if key.is_empty() {
            return Err("an entitlement key must not be empty".to_owned());
        }
        if PROFILE_ENTITLEMENTS.contains(&key.as_str()) {
            return Err(format!(
                "entitlement `{key}` comes from the provisioning profile and cannot be declared"
            ));
        }
        if key == IN_APP_PAYMENTS {
            return Err(format!(
                "entitlement `{key}` carries the app's merchant IDs; request the `apple_pay_merchant_ids` app value with `[[package.metadata.waterui.app-value]]` instead"
            ));
        }
        if let Some(environment) = EnvironmentEntitlement::ALL.into_iter().find(|environment| {
            [TargetPlatform::IOS, TargetPlatform::MacOS]
                .into_iter()
                .any(|platform| environment.key(platform) == key)
        }) {
            return Err(format!(
                "entitlement `{key}` follows the signing kind; list `{}` in `environment-entitlements` instead",
                environment.name()
            ));
        }
        Ok(Self(key))
    }
}

impl EnvironmentEntitlement {
    const fn name(self) -> &'static str {
        match self {
            Self::ApsEnvironment => "aps-environment",
            Self::AppAttestEnvironment => "app-attest-environment",
        }
    }
}

/// An `Info.plist` key. Usage descriptions belong to the permission channel —
/// `[package.metadata.waterui.permissions]` and the app's `Water.toml` text —
/// so a crate cannot set one here.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub(super) struct InfoPlistKey(String);

impl TryFrom<String> for InfoPlistKey {
    type Error = String;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        if key.is_empty() {
            return Err("an Info.plist key must not be empty".to_owned());
        }
        if key.ends_with("UsageDescription") {
            return Err(format!(
                "Info.plist key `{key}` is a usage description; declare the permission under `[package.metadata.waterui.permissions]` and the app supplies its text in Water.toml"
            ));
        }
        Ok(Self(key))
    }
}

/// A declared plist value: the shapes entitlements and the keys crates need
/// take.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub(super) enum DeclaredValue {
    /// `<true/>` / `<false/>`.
    Boolean(bool),
    /// `<integer>`.
    Integer(i64),
    /// `<string>`.
    String(String),
    /// An `<array>` of strings, merged as a union across crates.
    Strings(Vec<String>),
}

impl DeclaredValue {
    fn to_plist(&self) -> plist::Value {
        match self {
            Self::Boolean(value) => plist::Value::Boolean(*value),
            Self::Integer(value) => plist::Value::Integer((*value).into()),
            Self::String(value) => plist::Value::String(value.clone()),
            Self::Strings(values) => plist::Value::Array(
                values
                    .iter()
                    .map(|value| plist::Value::String(value.clone()))
                    .collect(),
            ),
        }
    }
}

/// One crate's `[package.metadata.waterui.apple]` table.
///
/// Every key is the CLI's, so an unknown one is an error rather than a
/// declaration silently dropped from the app.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct AppleMetadata {
    /// Literal entitlements the signature claims.
    #[serde(default)]
    entitlements: BTreeMap<EntitlementKey, DeclaredValue>,
    /// Entitlements whose value is the signing environment.
    #[serde(default)]
    environment_entitlements: BTreeSet<EnvironmentEntitlement>,
    /// Keys merged into the generated `Info.plist`.
    #[serde(default)]
    info_plist: BTreeMap<InfoPlistKey, DeclaredValue>,
    /// Only required when this cargo feature is enabled on the declaring
    /// crate; gates every key of the table.
    #[serde(default)]
    pub(super) required_feature: Option<String>,
}

impl AppleMetadata {
    /// Whether the table declares nothing.
    pub(super) fn is_empty(&self) -> bool {
        self.entitlements.is_empty()
            && self.environment_entitlements.is_empty()
            && self.info_plist.is_empty()
    }
}

/// A merged value and the crates that declared it.
#[derive(Debug, Clone)]
struct Declared {
    crates: Vec<String>,
    value: plist::Value,
}

/// The entitlements and `Info.plist` keys the whole dependency graph
/// declares, after `required-feature` gating.
#[derive(Debug, Default)]
pub struct AppleDeclarations {
    entitlements: BTreeMap<String, Declared>,
    environment: BTreeMap<EnvironmentEntitlement, Vec<String>>,
    info_plist: BTreeMap<String, Declared>,
    /// Values the graph requests from the app's `Water.toml`.
    pub(super) app_values: RequiredAppValues,
}

impl AppleDeclarations {
    /// Places the Apple values the graph requests from the app's `config`:
    /// the Apple Pay merchant IDs become the `in-app-payments` entitlement.
    ///
    /// # Errors
    ///
    /// Returns an error naming the key and the requesting crates when a
    /// requested value is missing from `Water.toml`.
    pub fn supply_app_values(&mut self, config: &AppValuesConfig) -> eyre::Result<()> {
        if let Some(ids) = self.app_values.resolve(
            AppValueKey::ApplePayMerchantIds,
            config.apple_pay_merchant_ids.as_ref(),
        )? {
            let crates = self
                .app_values
                .requesters(AppValueKey::ApplePayMerchantIds)
                .unwrap_or_default()
                .to_vec();
            self.entitlements.insert(
                IN_APP_PAYMENTS.to_owned(),
                Declared {
                    crates,
                    value: ids.to_plist(),
                },
            );
        }
        Ok(())
    }

    /// Merges `crate_name`'s table.
    ///
    /// # Errors
    ///
    /// Returns an error naming both crates when two crates give one key
    /// different scalar values, or values of different shapes.
    pub(super) fn merge(&mut self, crate_name: &str, table: AppleMetadata) -> eyre::Result<()> {
        for (key, value) in table.entitlements {
            merge_declared(
                &mut self.entitlements,
                "entitlement",
                crate_name,
                key.0,
                value.to_plist(),
            )?;
        }
        for environment in table.environment_entitlements {
            self.environment
                .entry(environment)
                .or_default()
                .push(crate_name.to_owned());
        }
        for (key, value) in table.info_plist {
            merge_declared(
                &mut self.info_plist,
                "Info.plist key",
                crate_name,
                key.0,
                value.to_plist(),
            )?;
        }
        Ok(())
    }

    /// Merges the declared `Info.plist` keys into the generated `plist`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the declaring crate when a declared scalar
    /// disagrees with the value the CLI generates for that key.
    pub fn merge_into_info_plist(&self, plist: &mut plist::Dictionary) -> eyre::Result<()> {
        merge_into(
            plist,
            &self.info_plist,
            "Info.plist key",
            "the generated Info.plist",
        )
    }

    /// Merges the declared entitlements into `entitlements` — the project's
    /// `.entitlements` dictionary — for an app signed for `platform` with a
    /// signature of kind `environment`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the declaring crate when a declared scalar
    /// disagrees with the project's entitlements file.
    pub fn merge_into_entitlements(
        &self,
        entitlements: &mut plist::Dictionary,
        platform: TargetPlatform,
        environment: SigningEnvironment,
    ) -> eyre::Result<()> {
        let mut declared = self.entitlements.clone();
        for (entitlement, crates) in &self.environment {
            declared.insert(
                entitlement.key(platform).to_owned(),
                Declared {
                    crates: crates.clone(),
                    value: plist::Value::String(environment.as_str().to_owned()),
                },
            );
        }
        merge_into(
            entitlements,
            &declared,
            "entitlement",
            "the project's entitlements",
        )
    }
}

/// Folds `value` from `crate_name` into `merged`: arrays take the union in
/// declaration order, equal scalars merge, anything else is a conflict.
fn merge_declared(
    merged: &mut BTreeMap<String, Declared>,
    what: &str,
    crate_name: &str,
    key: String,
    value: plist::Value,
) -> eyre::Result<()> {
    match merged.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(Declared {
                crates: vec![crate_name.to_owned()],
                value,
            });
        }
        Entry::Occupied(mut entry) => {
            let key = entry.key().clone();
            let existing = entry.get_mut();
            if !union_into(&mut existing.value, &value) {
                eyre::bail!(
                    "crates `{}` and `{crate_name}` declare {what} `{key}` as {} and {}; the app can carry only one",
                    existing.crates.join("`, `"),
                    DisplayValue(&existing.value),
                    DisplayValue(&value),
                );
            }
            existing.crates.push(crate_name.to_owned());
        }
    }
    Ok(())
}

/// Merges the graph's `declared` keys into a generated dictionary.
fn merge_into(
    target: &mut plist::Dictionary,
    declared: &BTreeMap<String, Declared>,
    what: &str,
    origin: &str,
) -> eyre::Result<()> {
    for (key, declared) in declared {
        match target.get_mut(key) {
            None => {
                target.insert(key.clone(), declared.value.clone());
            }
            Some(existing) => {
                if !union_into(existing, &declared.value) {
                    eyre::bail!(
                        "crate `{}` declares {what} `{key}` as {}, but {origin} sets it to {}",
                        declared.crates.join("`, `"),
                        DisplayValue(&declared.value),
                        DisplayValue(existing),
                    );
                }
            }
        }
    }
    Ok(())
}

/// Unions array `incoming` into array `existing`, or checks two scalars are
/// equal; `false` when the two cannot merge.
fn union_into(existing: &mut plist::Value, incoming: &plist::Value) -> bool {
    match (existing, incoming) {
        (plist::Value::Array(items), plist::Value::Array(more)) => {
            for item in more {
                if !items.contains(item) {
                    items.push(item.clone());
                }
            }
            true
        }
        (existing, incoming) => existing == incoming,
    }
}

/// A plist value as an error message quotes it.
struct DisplayValue<'a>(&'a plist::Value);

impl fmt::Display for DisplayValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            plist::Value::String(value) => write!(f, "`{value}`"),
            plist::Value::Boolean(value) => write!(f, "`{value}`"),
            plist::Value::Integer(value) => write!(f, "`{value}`"),
            plist::Value::Array(items) => {
                f.write_str("[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}", DisplayValue(item))?;
                }
                f.write_str("]")
            }
            other => write!(f, "{other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(toml_text: &str) -> AppleMetadata {
        toml::from_str(toml_text).expect("apple table parses")
    }

    fn merged(crates: &[(&str, &str)]) -> eyre::Result<AppleDeclarations> {
        let mut declarations = AppleDeclarations::default();
        for (name, text) in crates {
            declarations.merge(name, table(text))?;
        }
        Ok(declarations)
    }

    fn strings(values: &[&str]) -> plist::Value {
        plist::Value::Array(
            values
                .iter()
                .map(|value| plist::Value::String((*value).to_owned()))
                .collect(),
        )
    }

    const PUSH: &str = r#"
        environment-entitlements = ["aps-environment"]

        [info-plist]
        UIBackgroundModes = ["remote-notification"]
    "#;

    #[test]
    fn aps_environment_follows_the_signing_kind_and_platform() {
        let declarations = merged(&[("waterkit-push", PUSH)]).expect("merge");
        let mut ios = plist::Dictionary::new();
        declarations
            .merge_into_entitlements(
                &mut ios,
                TargetPlatform::IOS,
                SigningEnvironment::Development,
            )
            .expect("merge entitlements");
        assert_eq!(
            ios.get("aps-environment"),
            Some(&plist::Value::String("development".to_owned()))
        );

        let mut macos = plist::Dictionary::new();
        declarations
            .merge_into_entitlements(
                &mut macos,
                TargetPlatform::MacOS,
                SigningEnvironment::Production,
            )
            .expect("merge entitlements");
        assert_eq!(
            macos.get("com.apple.developer.aps-environment"),
            Some(&plist::Value::String("production".to_owned()))
        );
        assert!(!macos.contains_key("aps-environment"), "{macos:?}");
    }

    #[test]
    fn literal_entitlements_reach_the_signature() {
        let declarations = merged(&[(
            "waterkit-auth",
            "[entitlements]\n\"com.apple.developer.applesignin\" = [\"Default\"]\n",
        )])
        .expect("merge");
        let mut entitlements = plist::Dictionary::new();
        declarations
            .merge_into_entitlements(
                &mut entitlements,
                TargetPlatform::IOS,
                SigningEnvironment::Development,
            )
            .expect("merge entitlements");
        assert_eq!(
            entitlements.get("com.apple.developer.applesignin"),
            Some(&strings(&["Default"]))
        );
    }

    #[test]
    fn arrays_union_with_the_generated_info_plist() {
        let declarations = merged(&[
            ("waterkit-push", PUSH),
            (
                "waterkit-peers",
                "[info-plist]\nUIBackgroundModes = [\"remote-notification\", \"fetch\"]\nNSBonjourServices = [\"_waterui._tcp\"]\n",
            ),
        ])
        .expect("merge");
        let mut plist = plist::Dictionary::new();
        plist.insert("UIBackgroundModes".to_owned(), strings(&["audio"]));
        declarations
            .merge_into_info_plist(&mut plist)
            .expect("merge Info.plist");
        assert_eq!(
            plist.get("UIBackgroundModes"),
            Some(&strings(&["audio", "remote-notification", "fetch"]))
        );
        assert_eq!(
            plist.get("NSBonjourServices"),
            Some(&strings(&["_waterui._tcp"]))
        );
    }

    #[test]
    fn conflicting_scalars_name_both_crates() {
        let error = merged(&[
            (
                "waterkit-a",
                "[entitlements]\n\"com.apple.developer.icloud-container-environment\" = \"Development\"\n",
            ),
            (
                "waterkit-b",
                "[entitlements]\n\"com.apple.developer.icloud-container-environment\" = \"Production\"\n",
            ),
        ])
        .expect_err("two values of one entitlement must fail");
        let message = error.to_string();
        assert!(message.contains("`waterkit-a`"), "{message}");
        assert!(message.contains("`waterkit-b`"), "{message}");
        assert!(
            message.contains("`Development`") && message.contains("`Production`"),
            "{message}"
        );
    }

    #[test]
    fn mismatched_shapes_conflict() {
        let error = merged(&[
            (
                "waterkit-a",
                "[info-plist]\nUIBackgroundModes = [\"fetch\"]\n",
            ),
            (
                "waterkit-b",
                "[info-plist]\nUIBackgroundModes = \"fetch\"\n",
            ),
        ])
        .expect_err("an array and a scalar cannot merge");
        assert!(error.to_string().contains("`waterkit-b`"), "{error}");
    }

    #[test]
    fn a_scalar_the_cli_generates_differently_fails() {
        let declarations = merged(&[(
            "rogue",
            "[info-plist]\nCFBundleIdentifier = \"com.example.other\"\n",
        )])
        .expect("merge");
        let mut plist = plist::Dictionary::new();
        plist.insert(
            "CFBundleIdentifier".to_owned(),
            plist::Value::String("dev.waterui.app".to_owned()),
        );
        let message = declarations
            .merge_into_info_plist(&mut plist)
            .expect_err("a crate cannot override the bundle id")
            .to_string();
        assert!(message.contains("`rogue`"), "{message}");
        assert!(message.contains("dev.waterui.app"), "{message}");
    }

    #[test]
    fn identical_declarations_merge() {
        let declarations =
            merged(&[("a", PUSH), ("b", PUSH)]).expect("identical tables merge cleanly");
        let mut plist = plist::Dictionary::new();
        declarations
            .merge_into_info_plist(&mut plist)
            .expect("merge Info.plist");
        assert_eq!(
            plist.get("UIBackgroundModes"),
            Some(&strings(&["remote-notification"]))
        );
    }

    #[test]
    fn requested_merchant_ids_come_from_water_toml() {
        let mut declarations = AppleDeclarations::default();
        declarations
            .app_values
            .request("waterkit-pay", AppValueKey::ApplePayMerchantIds);
        let message = declarations
            .supply_app_values(&AppValuesConfig::default())
            .expect_err("a requested value the app omits must fail")
            .to_string();
        assert!(message.contains("apple_pay_merchant_ids"), "{message}");
        assert!(message.contains("`waterkit-pay`"), "{message}");

        let config: AppValuesConfig =
            toml::from_str("apple_pay_merchant_ids = [\"merchant.com.example\"]\n")
                .expect("[app_values] parses");
        declarations
            .supply_app_values(&config)
            .expect("supplied merchant IDs");
        let mut entitlements = plist::Dictionary::new();
        declarations
            .merge_into_entitlements(
                &mut entitlements,
                TargetPlatform::IOS,
                SigningEnvironment::Development,
            )
            .expect("merge entitlements");
        assert_eq!(
            entitlements.get(IN_APP_PAYMENTS),
            Some(&strings(&["merchant.com.example"]))
        );
    }

    #[test]
    fn unrequested_merchant_ids_stay_out_of_the_signature() {
        let mut declarations = AppleDeclarations::default();
        let config: AppValuesConfig =
            toml::from_str("apple_pay_merchant_ids = [\"merchant.com.example\"]\n")
                .expect("[app_values] parses");
        declarations
            .supply_app_values(&config)
            .expect("nothing is requested");
        let mut entitlements = plist::Dictionary::new();
        declarations
            .merge_into_entitlements(
                &mut entitlements,
                TargetPlatform::IOS,
                SigningEnvironment::Development,
            )
            .expect("merge entitlements");
        assert!(entitlements.is_empty(), "{entitlements:?}");
    }

    #[test]
    fn malformed_tables_fail_to_parse() {
        for text in [
            // An unknown key.
            "entitlement = {}\n",
            // A profile-derived entitlement.
            "[entitlements]\n\"get-task-allow\" = true\n",
            "[entitlements]\n\"application-identifier\" = \"X.dev.app\"\n",
            // The merchant IDs only the app knows.
            "[entitlements]\n\"com.apple.developer.in-app-payments\" = [\"merchant.com.example\"]\n",
            // An environment entitlement as a literal.
            "[entitlements]\n\"aps-environment\" = \"development\"\n",
            "[entitlements]\n\"com.apple.developer.aps-environment\" = \"production\"\n",
            // An unknown environment entitlement.
            "environment-entitlements = [\"icloud-environment\"]\n",
            // A usage description outside the permission channel.
            "[info-plist]\nNSCameraUsageDescription = \"scan\"\n",
            // A value shape no declaration takes.
            "[info-plist]\nUIScene = { a = 1 }\n",
        ] {
            assert!(
                toml::from_str::<AppleMetadata>(text).is_err(),
                "must reject: {text}"
            );
        }
    }
}
