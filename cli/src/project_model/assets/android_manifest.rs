//! Android manifest components a crate declares under
//! `[package.metadata.waterui.android]`, and their staging into the
//! generated module's `AndroidManifest.xml`.
//!
//! A crate whose platform code needs an entry inside `<application>` —
//! an `Activity`, a `ContentProvider`, a `Service`, a
//! `BroadcastReceiver`, or an application-level `<meta-data>` flag —
//! declares it for the Android host:
//!
//! ```toml
//! [[package.metadata.waterui.android.provider]]
//! name = "waterkit.clipboard.ClipboardFileProvider"
//! authorities = ["${applicationId}.waterkit.clipboard"]
//! exported = false
//! grant-uri-permissions = true
//! ```
//!
//! Each element kind is its own typed table that rejects unknown keys, so a
//! misspelt attribute fails the scan instead of silently dropping out of the
//! manifest. Values are written verbatim — Gradle's manifest merger resolves
//! placeholders such as `${applicationId}` against the module that consumes
//! the manifest, which for the Hydrolysis preview library is the preview host application.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;
use std::path::Path;

use askama::Template;
use eyre::Context;
use serde::Deserialize;
use smol::fs;

/// Markers bracketing the block [`write_manifest_components`] maintains
/// inside `<application>`. The manifest templates emit the pair empty; the
/// stage fills it.
pub(super) const MANIFEST_COMPONENTS_BEGIN: &str =
    "<!-- begin waterui android manifest components -->";
pub(super) const MANIFEST_COMPONENTS_END: &str = "<!-- end waterui android manifest components -->";

/// An `<activity>` — typically a non-exported trampoline a library owns so
/// a platform API can deliver its result through `onActivityResult`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct Activity {
    /// Fully qualified `Activity` class.
    name: String,
    /// Whether other applications can launch the activity.
    exported: bool,
    /// `android:theme`, such as `@android:style/Theme.Translucent.NoTitleBar`.
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    exclude_from_recents: Option<bool>,
    #[serde(default)]
    launch_mode: Option<String>,
    /// `android:configChanges` flags, rendered `|`-separated.
    #[serde(default)]
    config_changes: Vec<String>,
    #[serde(default)]
    intent_filter: Vec<IntentFilter>,
    #[serde(default)]
    meta_data: Vec<MetaData>,
}

impl Activity {
    /// The `android:configChanges` value, when any flag is declared.
    fn config_changes_attribute(&self) -> Option<String> {
        (!self.config_changes.is_empty()).then(|| self.config_changes.join("|"))
    }
}

/// A `<provider>`: a `ContentProvider` subclass and the authorities it
/// serves.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct Provider {
    /// Fully qualified `ContentProvider` class.
    name: String,
    /// The URI authorities the provider serves, rendered `;`-separated.
    authorities: Vec<String>,
    /// Whether other applications can reach the provider.
    exported: bool,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    grant_uri_permissions: Option<bool>,
    #[serde(default)]
    permission: Option<String>,
    #[serde(default)]
    read_permission: Option<String>,
    #[serde(default)]
    write_permission: Option<String>,
    #[serde(default)]
    process: Option<String>,
    #[serde(default)]
    multiprocess: Option<bool>,
    #[serde(default)]
    init_order: Option<i32>,
    #[serde(default)]
    syncable: Option<bool>,
    #[serde(default)]
    direct_boot_aware: Option<bool>,
    #[serde(default)]
    meta_data: Vec<MetaData>,
}

impl Provider {
    /// The `android:authorities` value.
    fn authorities_attribute(&self) -> String {
        self.authorities.join(";")
    }
}

/// A `<service>`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct Service {
    /// Fully qualified `Service` class.
    name: String,
    /// Whether other applications can bind to or start the service.
    exported: bool,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    permission: Option<String>,
    #[serde(default)]
    process: Option<String>,
    #[serde(default)]
    direct_boot_aware: Option<bool>,
    /// `android:foregroundServiceType` flags, rendered `|`-separated.
    #[serde(default)]
    foreground_service_type: Vec<String>,
    #[serde(default)]
    isolated_process: Option<bool>,
    #[serde(default)]
    stop_with_task: Option<bool>,
    #[serde(default)]
    intent_filter: Vec<IntentFilter>,
    #[serde(default)]
    meta_data: Vec<MetaData>,
}

impl Service {
    /// The `android:foregroundServiceType` value, when any flag is declared.
    fn foreground_service_type_attribute(&self) -> Option<String> {
        (!self.foreground_service_type.is_empty()).then(|| self.foreground_service_type.join("|"))
    }
}

/// A `<receiver>`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct Receiver {
    /// Fully qualified `BroadcastReceiver` class.
    name: String,
    /// Whether broadcasts from other applications reach the receiver.
    exported: bool,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    permission: Option<String>,
    #[serde(default)]
    process: Option<String>,
    #[serde(default)]
    direct_boot_aware: Option<bool>,
    #[serde(default)]
    intent_filter: Vec<IntentFilter>,
    #[serde(default)]
    meta_data: Vec<MetaData>,
}

/// An `<intent-filter>` on a service or receiver.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct IntentFilter {
    /// `<action>` names; a filter without one matches nothing.
    actions: Vec<String>,
    /// `<category>` names.
    #[serde(default)]
    categories: Vec<String>,
    /// `<data>` elements.
    #[serde(default)]
    data: Vec<IntentData>,
    #[serde(default)]
    priority: Option<i32>,
}

/// One `<data>` element of an intent filter.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct IntentData {
    #[serde(default)]
    scheme: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    path_prefix: Option<String>,
    #[serde(default)]
    path_pattern: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

impl IntentData {
    const fn is_empty(&self) -> bool {
        self.scheme.is_none()
            && self.host.is_none()
            && self.port.is_none()
            && self.path.is_none()
            && self.path_prefix.is_none()
            && self.path_pattern.is_none()
            && self.mime_type.is_none()
    }
}

/// A `<meta-data>` entry: a name bound to exactly one of a literal value or
/// a resource reference.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "RawMetaData")]
pub(super) struct MetaData {
    name: String,
    content: MetaDataContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MetaDataContent {
    /// `android:value`.
    Value(MetaDataValue),
    /// `android:resource`, such as `@xml/file_paths`.
    Resource(String),
}

/// A `<meta-data>` literal. Android reads the attribute back typed — a
/// boolean through `Bundle.getBoolean`, an integer through `getInt` — so the
/// declaration keeps the TOML type rather than forcing everything to a
/// string.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum MetaDataValue {
    Bool(bool),
    Integer(i64),
    String(String),
}

impl fmt::Display for MetaDataValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => value.fmt(f),
            Self::Integer(value) => value.fmt(f),
            Self::String(value) => value.fmt(f),
        }
    }
}

/// The `<meta-data>` table as written, before the value/resource choice is
/// checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMetaData {
    name: String,
    #[serde(default)]
    value: Option<MetaDataValue>,
    #[serde(default)]
    resource: Option<String>,
}

impl TryFrom<RawMetaData> for MetaData {
    type Error = String;

    fn try_from(raw: RawMetaData) -> Result<Self, Self::Error> {
        let content = match (raw.value, raw.resource) {
            (Some(value), None) => MetaDataContent::Value(value),
            (None, Some(resource)) => MetaDataContent::Resource(resource),
            (Some(_), Some(_)) => {
                return Err(format!(
                    "<meta-data> `{}` sets both `value` and `resource`; it carries exactly one",
                    raw.name
                ));
            }
            (None, None) => {
                return Err(format!(
                    "<meta-data> `{}` sets neither `value` nor `resource`",
                    raw.name
                ));
            }
        };
        Ok(Self {
            name: raw.name,
            content,
        })
    }
}

impl MetaData {
    /// The attribute the content renders as: `value` or `resource`.
    const fn attribute(&self) -> &'static str {
        match self.content {
            MetaDataContent::Value(_) => "value",
            MetaDataContent::Resource(_) => "resource",
        }
    }

    /// The rendered attribute value.
    fn content(&self) -> String {
        match &self.content {
            MetaDataContent::Value(value) => value.to_string(),
            MetaDataContent::Resource(resource) => resource.clone(),
        }
    }
}

/// One crate's manifest component declarations, as its
/// `[package.metadata.waterui.android]` table carries them.
#[derive(Debug, Default)]
pub(super) struct DeclaredComponents {
    pub(super) activities: Vec<Activity>,
    pub(super) providers: Vec<Provider>,
    pub(super) services: Vec<Service>,
    pub(super) receivers: Vec<Receiver>,
    pub(super) meta_data: Vec<MetaData>,
}

/// A merged entry and the crate that declared it first.
#[derive(Debug)]
struct Declared<T> {
    crate_name: String,
    item: T,
}

/// The manifest components the whole dependency graph declares, keyed by
/// `android:name` per element kind — the identity Gradle's manifest merger
/// uses.
///
/// The same declaration arriving from two crates (two versions of one crate
/// in the graph, say) is kept once; two different declarations under one
/// name, or one provider authority claimed by two providers, cannot both
/// ship and fail the merge with an error naming both crates.
#[derive(Debug, Default)]
pub struct ManifestComponents {
    activities: BTreeMap<String, Declared<Activity>>,
    providers: BTreeMap<String, Declared<Provider>>,
    services: BTreeMap<String, Declared<Service>>,
    receivers: BTreeMap<String, Declared<Receiver>>,
    meta_data: BTreeMap<String, Declared<MetaData>>,
    /// Provider authority → (crate, provider class) that claims it.
    authorities: BTreeMap<String, (String, String)>,
}

impl ManifestComponents {
    /// Merges `crate_name`'s declarations.
    ///
    /// # Errors
    ///
    /// Returns an error when a declaration is malformed — a relative class
    /// name, a provider without authorities, an intent filter without an
    /// action, an empty `<data>` — or conflicts with one already merged.
    pub(super) fn merge(
        &mut self,
        crate_name: &str,
        declared: DeclaredComponents,
    ) -> eyre::Result<()> {
        for activity in declared.activities {
            check_class_name(crate_name, "activity", &activity.name)?;
            check_intent_filters(
                crate_name,
                "activity",
                &activity.name,
                &activity.intent_filter,
            )?;
            check_meta_data(crate_name, "activity", &activity.name, &activity.meta_data)?;
            insert(
                &mut self.activities,
                "activity",
                crate_name,
                activity.name.clone(),
                activity,
            )?;
        }
        for provider in declared.providers {
            check_class_name(crate_name, "provider", &provider.name)?;
            eyre::ensure!(
                !provider.authorities.is_empty(),
                "{crate_name} declares Android <provider> `{}` without authorities",
                provider.name
            );
            for authority in &provider.authorities {
                match self.authorities.entry(authority.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert((crate_name.to_owned(), provider.name.clone()));
                    }
                    Entry::Occupied(entry) => {
                        let (other_crate, other_provider) = entry.get();
                        eyre::ensure!(
                            *other_provider == provider.name,
                            "Android provider authority `{authority}` is claimed by <provider> `{other_provider}` from crate `{other_crate}` and by <provider> `{}` from crate `{crate_name}`; an authority belongs to one provider",
                            provider.name
                        );
                    }
                }
            }
            check_meta_data(crate_name, "provider", &provider.name, &provider.meta_data)?;
            insert(
                &mut self.providers,
                "provider",
                crate_name,
                provider.name.clone(),
                provider,
            )?;
        }
        for service in declared.services {
            check_class_name(crate_name, "service", &service.name)?;
            check_intent_filters(crate_name, "service", &service.name, &service.intent_filter)?;
            check_meta_data(crate_name, "service", &service.name, &service.meta_data)?;
            insert(
                &mut self.services,
                "service",
                crate_name,
                service.name.clone(),
                service,
            )?;
        }
        for receiver in declared.receivers {
            check_class_name(crate_name, "receiver", &receiver.name)?;
            check_intent_filters(
                crate_name,
                "receiver",
                &receiver.name,
                &receiver.intent_filter,
            )?;
            check_meta_data(crate_name, "receiver", &receiver.name, &receiver.meta_data)?;
            insert(
                &mut self.receivers,
                "receiver",
                crate_name,
                receiver.name.clone(),
                receiver,
            )?;
        }
        for entry in declared.meta_data {
            insert(
                &mut self.meta_data,
                "meta-data",
                crate_name,
                entry.name.clone(),
                entry,
            )?;
        }
        Ok(())
    }

    /// Renders the managed `<application>` block, markers included.
    pub(super) fn render_block(&self) -> eyre::Result<String> {
        fn items<T>(map: &BTreeMap<String, Declared<T>>) -> Vec<&T> {
            map.values().map(|declared| &declared.item).collect()
        }
        let block = ManifestComponentsTemplate {
            begin: MANIFEST_COMPONENTS_BEGIN,
            end: MANIFEST_COMPONENTS_END,
            activities: items(&self.activities),
            providers: items(&self.providers),
            services: items(&self.services),
            receivers: items(&self.receivers),
            meta_data: items(&self.meta_data),
        }
        .render()
        .wrap_err("rendering the Android manifest components block")?;
        Ok(block.trim_end().to_owned())
    }
}

/// Inserts `item` under `name`, keeping an identical earlier declaration and
/// failing on a different one.
fn insert<T: PartialEq>(
    map: &mut BTreeMap<String, Declared<T>>,
    kind: &str,
    crate_name: &str,
    name: String,
    item: T,
) -> eyre::Result<()> {
    match map.entry(name) {
        Entry::Vacant(entry) => {
            entry.insert(Declared {
                crate_name: crate_name.to_owned(),
                item,
            });
        }
        Entry::Occupied(entry) => {
            let existing = entry.get();
            eyre::ensure!(
                existing.item == item,
                "crates `{}` and `{crate_name}` declare Android <{kind}> `{}` differently; the manifest can carry only one",
                existing.crate_name,
                entry.key()
            );
        }
    }
    Ok(())
}

/// A component class must be fully qualified: a relative `.Name` resolves
/// against the generated application's namespace, which no crate knows.
fn check_class_name(crate_name: &str, kind: &str, name: &str) -> eyre::Result<()> {
    eyre::ensure!(
        !name.is_empty() && !name.starts_with('.') && name.contains('.'),
        "{crate_name} declares Android <{kind}> `{name}`: the class name must be fully qualified"
    );
    Ok(())
}

fn check_intent_filters(
    crate_name: &str,
    kind: &str,
    name: &str,
    filters: &[IntentFilter],
) -> eyre::Result<()> {
    for filter in filters {
        eyre::ensure!(
            !filter.actions.is_empty(),
            "{crate_name} declares an <intent-filter> without actions on Android <{kind}> `{name}`; it would match nothing"
        );
        eyre::ensure!(
            !filter.data.iter().any(IntentData::is_empty),
            "{crate_name} declares an empty <data> in an <intent-filter> on Android <{kind}> `{name}`"
        );
    }
    Ok(())
}

fn check_meta_data(
    crate_name: &str,
    kind: &str,
    name: &str,
    entries: &[MetaData],
) -> eyre::Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        eyre::ensure!(
            seen.insert(entry.name.as_str()),
            "{crate_name} declares <meta-data> `{}` twice on Android <{kind}> `{name}`",
            entry.name
        );
    }
    Ok(())
}

/// The managed block inside `<application>`, rendered with XML escaping so
/// a declared value cannot break out of its attribute.
#[derive(Template)]
#[template(
    path = "src/templates/android_manifest/application_components.xml.tpl",
    escape = "xml"
)]
struct ManifestComponentsTemplate<'a> {
    begin: &'static str,
    end: &'static str,
    activities: Vec<&'a Activity>,
    providers: Vec<&'a Provider>,
    services: Vec<&'a Service>,
    receivers: Vec<&'a Receiver>,
    meta_data: Vec<&'a MetaData>,
}

/// Rewrites the managed components block inside `<application>` of
/// `module_dir/src/main/AndroidManifest.xml`. The manifest templates ship the
/// marker pair; a manifest missing it was hand-edited or generated before
/// this mechanism and is an error. An empty set leaves the pair empty, so a
/// component whose crate left the graph stops shipping.
pub(super) async fn write_manifest_components(
    module_dir: &Path,
    components: &ManifestComponents,
) -> eyre::Result<()> {
    let manifest_path = module_dir.join("src/main/AndroidManifest.xml");
    let existing = fs::read_to_string(&manifest_path)
        .await
        .wrap_err_with(|| format!("reading module manifest {}", manifest_path.display()))?;
    let block = components.render_block()?;
    let body = super::splice_managed_block(
        &existing,
        &manifest_path,
        MANIFEST_COMPONENTS_BEGIN,
        MANIFEST_COMPONENTS_END,
        &block,
    )?
    .ok_or_else(|| {
        eyre::eyre!(
            "{} has no managed components block ({MANIFEST_COMPONENTS_BEGIN} / {MANIFEST_COMPONENTS_END}); the manifest template must emit the marker pair inside `<application>`",
            manifest_path.display()
        )
    })?;

    super::super::templates::write_file_if_changed(&manifest_path, body.as_bytes())
        .await
        .map_err(Into::into)
}

/// Asserts a scaffolded manifest carries the empty marker pair inside
/// `<application>`, where [`write_manifest_components`] stages the
/// components.
#[cfg(test)]
#[track_caller]
pub fn assert_component_markers_inside_application(manifest: &str) {
    let position = |needle: &str| {
        manifest
            .find(needle)
            .unwrap_or_else(|| panic!("manifest lacks `{needle}`: {manifest}"))
    };
    let application = position("<application");
    let begin = position(MANIFEST_COMPONENTS_BEGIN);
    let end = position(MANIFEST_COMPONENTS_END);
    let close = position("</application>");
    assert!(
        application < begin && begin < end && end < close,
        "the component markers must sit inside <application>: {manifest}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// The `[package.metadata.waterui.android]` component tables of one
    /// crate, parsed from TOML the way `cargo metadata` hands them over.
    fn declared(toml_text: &str) -> DeclaredComponents {
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case", deny_unknown_fields)]
        struct Tables {
            #[serde(default)]
            activity: Vec<Activity>,
            #[serde(default)]
            provider: Vec<Provider>,
            #[serde(default)]
            service: Vec<Service>,
            #[serde(default)]
            receiver: Vec<Receiver>,
            #[serde(default)]
            meta_data: Vec<MetaData>,
        }
        let tables: Tables = toml::from_str(toml_text).expect("component tables parse");
        DeclaredComponents {
            activities: tables.activity,
            providers: tables.provider,
            services: tables.service,
            receivers: tables.receiver,
            meta_data: tables.meta_data,
        }
    }

    const CLIPBOARD: &str = r#"
[[provider]]
name = "waterkit.clipboard.ClipboardFileProvider"
authorities = ["${applicationId}.waterkit.clipboard"]
exported = false
grant-uri-permissions = true
"#;

    const PUSH: &str = r#"
[[activity]]
name = "waterkit.wallet.SavePassesActivity"
exported = false
theme = "@android:style/Theme.Translucent.NoTitleBar"
exclude-from-recents = true
config-changes = ["orientation", "screenSize"]

[[service]]
name = "waterkit.push.PushMessagingService"
exported = false
intent-filter = [{ actions = ["com.google.firebase.MESSAGING_EVENT"] }]

[[receiver]]
name = "waterkit.geofence.GeofenceReceiver"
exported = true
permission = "com.google.android.gms.permission.ACTIVITY_RECOGNITION"
intent-filter = [{ actions = ["waterkit.geofence.TRANSITION"], categories = ["android.intent.category.DEFAULT"], data = [{ scheme = "geo", host = "a&b" }], priority = 5 }]

[[receiver.meta-data]]
name = "waterkit.geofence.LABEL"
value = "fence \"<one>\""

[[meta-data]]
name = "firebase_messaging_installation_id_enabled"
value = true

[[meta-data]]
name = "com.google.android.gms.version"
resource = "@integer/google_play_services_version"
"#;

    fn merged(crates: &[(&str, &str)]) -> eyre::Result<ManifestComponents> {
        let mut components = ManifestComponents::default();
        for (crate_name, tables) in crates {
            components.merge(crate_name, declared(tables))?;
        }
        Ok(components)
    }

    /// Every element kind renders as the Android XML it names, in a stable
    /// order, with values escaped for their attribute and the
    /// `${applicationId}` placeholder left for Gradle's manifest merger.
    #[test]
    fn components_render_into_the_managed_block() {
        let components =
            merged(&[("waterkit-push", PUSH), ("waterkit-clipboard", CLIPBOARD)]).expect("merge");
        let block = components.render_block().expect("render");
        let expected = r#"<!-- begin waterui android manifest components -->
        <activity
            android:name="waterkit.wallet.SavePassesActivity"
            android:exported="false"
            android:theme="@android:style/Theme.Translucent.NoTitleBar"
            android:excludeFromRecents="true"
            android:configChanges="orientation|screenSize" />
        <provider
            android:name="waterkit.clipboard.ClipboardFileProvider"
            android:authorities="${applicationId}.waterkit.clipboard"
            android:exported="false"
            android:grantUriPermissions="true" />
        <service
            android:name="waterkit.push.PushMessagingService"
            android:exported="false">
            <intent-filter>
                <action android:name="com.google.firebase.MESSAGING_EVENT" />
            </intent-filter>
        </service>
        <receiver
            android:name="waterkit.geofence.GeofenceReceiver"
            android:exported="true"
            android:permission="com.google.android.gms.permission.ACTIVITY_RECOGNITION">
            <intent-filter android:priority="5">
                <action android:name="waterkit.geofence.TRANSITION" />
                <category android:name="android.intent.category.DEFAULT" />
                <data android:scheme="geo" android:host="a&#38;b" />
            </intent-filter>
            <meta-data android:name="waterkit.geofence.LABEL" android:value="fence &#34;&#60;one&#62;&#34;" />
        </receiver>
        <meta-data android:name="com.google.android.gms.version" android:resource="@integer/google_play_services_version" />
        <meta-data android:name="firebase_messaging_installation_id_enabled" android:value="true" />
        <!-- end waterui android manifest components -->"#;
        assert_eq!(block, expected);
    }

    /// No declarations leave the marker pair empty.
    #[test]
    fn no_components_render_an_empty_block() {
        let block = ManifestComponents::default()
            .render_block()
            .expect("render");
        assert_eq!(
            block,
            format!("{MANIFEST_COMPONENTS_BEGIN}\n        {MANIFEST_COMPONENTS_END}")
        );
    }

    /// The same declaration from two crates — two versions of one crate in
    /// the graph — ships once.
    #[test]
    fn identical_declarations_merge_into_one() {
        let components = merged(&[
            ("waterkit-clipboard", CLIPBOARD),
            ("waterkit-clipboard-legacy", CLIPBOARD),
        ])
        .expect("identical declarations merge");
        assert_eq!(components.providers.len(), 1);
        let block = components.render_block().expect("render");
        assert_eq!(block.matches("<provider").count(), 1, "{block}");
    }

    /// Two different declarations under one `android:name` cannot both ship:
    /// the error names both crates.
    #[test]
    fn conflicting_declarations_name_both_crates() {
        let exported = CLIPBOARD.replace("exported = false", "exported = true");
        let error = merged(&[
            ("waterkit-clipboard", CLIPBOARD),
            ("other-clipboard", &exported),
        ])
        .expect_err("a conflicting provider must fail");
        let message = error.to_string();
        assert!(message.contains("`waterkit-clipboard`"), "{message}");
        assert!(message.contains("`other-clipboard`"), "{message}");
        assert!(
            message.contains("waterkit.clipboard.ClipboardFileProvider"),
            "{message}"
        );

        let flag_off = PUSH.replace("value = true", "value = false");
        let error = merged(&[("waterkit-push", PUSH), ("other-push", &flag_off)])
            .expect_err("a conflicting meta-data value must fail");
        let message = error.to_string();
        assert!(message.contains("`waterkit-push`"), "{message}");
        assert!(message.contains("`other-push`"), "{message}");
    }

    /// One authority served by two providers is a conflict even though the
    /// providers' names differ.
    #[test]
    fn an_authority_claimed_by_two_providers_fails() {
        let other = CLIPBOARD.replace("ClipboardFileProvider", "OtherFileProvider");
        let error = merged(&[("waterkit-clipboard", CLIPBOARD), ("other", &other)])
            .expect_err("a shared authority must fail");
        let message = error.to_string();
        assert!(
            message.contains("${applicationId}.waterkit.clipboard"),
            "{message}"
        );
        assert!(message.contains("`waterkit-clipboard`"), "{message}");
        assert!(message.contains("`other`"), "{message}");
    }

    /// Declarations that cannot mean what they say fail the merge.
    #[test]
    fn malformed_declarations_fail() {
        for (tables, needle) in [
            (
                CLIPBOARD.replace(
                    "waterkit.clipboard.ClipboardFileProvider",
                    ".ClipboardFileProvider",
                ),
                "fully qualified",
            ),
            (
                CLIPBOARD.replace(r#"["${applicationId}.waterkit.clipboard"]"#, "[]"),
                "without authorities",
            ),
            (
                PUSH.replace(
                    r#"actions = ["com.google.firebase.MESSAGING_EVENT"]"#,
                    "actions = []",
                ),
                "without actions",
            ),
            (
                PUSH.replace(r#"{ scheme = "geo", host = "a&b" }"#, "{}"),
                "empty <data>",
            ),
        ] {
            let error = merged(&[("crate", &tables)]).expect_err(needle);
            assert!(error.to_string().contains(needle), "{needle}: {error}");
        }
    }

    /// A `<meta-data>` carries exactly one of a value and a resource, and a
    /// misspelt attribute is a parse error rather than a silently dropped
    /// one.
    #[test]
    fn malformed_tables_fail_to_parse() {
        for tables in [
            "[[meta-data]]\nname = \"x\"\nvalue = 1\nresource = \"@xml/x\"\n",
            "[[meta-data]]\nname = \"x\"\n",
            &CLIPBOARD.replace("grant-uri-permissions", "grant-uri-permission"),
        ] {
            #[derive(Debug, Deserialize)]
            #[serde(rename_all = "kebab-case", deny_unknown_fields)]
            struct Tables {
                #[serde(default)]
                #[expect(dead_code, reason = "parsed only to observe the error")]
                provider: Vec<Provider>,
                #[serde(default)]
                #[expect(dead_code, reason = "parsed only to observe the error")]
                meta_data: Vec<MetaData>,
            }
            toml::from_str::<Tables>(tables).expect_err(tables);
        }
    }

    /// Staging writes the block between the markers a generated manifest
    /// ships, and restaging an empty set removes what an earlier stage left.
    #[test]
    fn staging_rewrites_the_managed_block() {
        let root = tempdir().expect("temp root");
        let module_dir = root.path().join("app");
        let manifest = module_dir.join("src/main/AndroidManifest.xml");
        std::fs::create_dir_all(manifest.parent().expect("manifest dir")).expect("manifest dir");
        let scaffolded = format!(
            "<manifest>\n    <application>\n        {MANIFEST_COMPONENTS_BEGIN}\n        {MANIFEST_COMPONENTS_END}\n    </application>\n</manifest>\n"
        );
        std::fs::write(&manifest, &scaffolded).expect("manifest");

        let components = merged(&[("waterkit-clipboard", CLIPBOARD)]).expect("merge");
        smol::block_on(write_manifest_components(&module_dir, &components)).expect("stage");
        let staged = std::fs::read_to_string(&manifest).expect("staged manifest");
        assert!(
            staged.contains(
                "        <provider\n            android:name=\"waterkit.clipboard.ClipboardFileProvider\""
            ),
            "{staged}"
        );
        assert!(
            staged.ends_with("    </application>\n</manifest>\n"),
            "{staged}"
        );

        smol::block_on(write_manifest_components(
            &module_dir,
            &ManifestComponents::default(),
        ))
        .expect("restage");
        assert_eq!(
            std::fs::read_to_string(&manifest).expect("restaged manifest"),
            scaffolded
        );
    }

    /// A manifest without the marker pair was hand-edited or predates the
    /// mechanism: staging fails rather than dropping the components.
    #[test]
    fn a_manifest_without_the_markers_fails() {
        let root = tempdir().expect("temp root");
        let manifest = root.path().join("src/main/AndroidManifest.xml");
        std::fs::create_dir_all(manifest.parent().expect("manifest dir")).expect("manifest dir");
        std::fs::write(&manifest, "<manifest>\n    <application />\n</manifest>\n")
            .expect("manifest");

        let error = smol::block_on(write_manifest_components(
            root.path(),
            &ManifestComponents::default(),
        ))
        .expect_err("a manifest without markers must fail");
        assert!(
            error.to_string().contains("managed components block"),
            "{error}"
        );
    }
}
