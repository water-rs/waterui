//! Newtypes and enums describing a project's identity, platforms, and targets.

use std::fmt;
use std::ops::Deref;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Canonical `Cargo` crate name used by a `WaterUI` project and its generated backends.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CrateName(String);

impl CrateName {
    /// Returns the validated crate name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Converts the crate name into a Rust identifier by replacing hyphens with underscores.
    #[must_use]
    pub fn rust_ident(&self) -> RustIdent {
        RustIdent(self.0.replace('-', "_"))
    }

    /// Returns a sibling crate name with the provided suffix appended in Cargo naming style.
    #[must_use]
    pub fn with_suffix(&self, suffix: &str) -> Self {
        Self(format!("{}-{suffix}", self.0))
    }
}

/// The package name of a crate the CLI generates for `project_root`: the
/// project's crate name, the backend suffix, and a short hash of the
/// canonical project root.
///
/// The hash is load-bearing, not cosmetic. Every generated crate compiles
/// into the user's shared Cargo target (`~/.water/build_cache/target`),
/// where Cargo keys dependency artifacts by package id but uplifts the
/// *final* artifact to an unhashed profile-root file — two projects named
/// `demo` would both write `debug/demo-hydrolysis`, last writer wins, and a
/// fingerprint-fresh rebuild of the loser emits nothing, so `water package`
/// would ship the other project's binary. Tagging the package keeps each
/// project's binaries, libraries, and `cargo clean -p` scope distinct while
/// staying stable across opens of the same root.
#[must_use]
pub fn generated_crate_name(
    crate_name: &CrateName,
    suffix: &str,
    project_root: &Path,
) -> CrateName {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(project_root.as_os_str().as_encoded_bytes());
    crate_name.with_suffix(&format!("{suffix}-{}", hex::encode(&digest[..4])))
}

/// The bundle-helper binary a generated crate carries when its backend
/// embeds CEF.
///
/// Derived from the generated package name for the same reason
/// [`generated_crate_name`] exists: its uplifted `debug/<name>` binary in
/// the shared Cargo target must be unique to its project.
#[must_use]
pub fn cef_helper_binary_name(package_name: &str) -> String {
    format!("{package_name}-cef-helper")
}

/// Whether the generated manifests declare the CEF subprocess helper
/// `[[bin]]` for an application whose linked browser engine is `engine`.
///
/// The helper exists to host CEF's subprocesses, so it is declared exactly
/// when the application links the CEF engine crate — a `waterui-chromium`
/// link alone still stages the CEF runtime but produces no helper. The
/// build's `build_binary` call and the packaging lookup gate on this same
/// predicate; widening it asks Cargo for a target the manifest never
/// emitted (`error: no bin target named ...`).
#[must_use]
pub const fn declares_cef_helper(engine: Option<crate::project::ResolvedWebViewBackend>) -> bool {
    matches!(engine, Some(crate::project::ResolvedWebViewBackend::Cef))
}

impl TryFrom<String> for CrateName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            return Err("crate name cannot be empty".to_string());
        }
        Ok(Self(value))
    }
}

impl TryFrom<&str> for CrateName {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_string())
    }
}

impl From<CrateName> for String {
    fn from(value: CrateName) -> Self {
        value.0
    }
}

impl fmt::Display for CrateName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for CrateName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for CrateName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl From<&CrateName> for String {
    fn from(value: &CrateName) -> Self {
        value.0.clone()
    }
}

/// Rust identifier derived from project naming metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RustIdent(String);

impl RustIdent {
    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RustIdent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for RustIdent {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for RustIdent {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

/// Reverse-DNS application identifier shared across generated platform manifests.
///
/// This is the serialized `[package].bundle_identifier` shape, and it is
/// deliberately platform-neutral: it accepts the union of the characters any
/// supported platform's identifier grammar permits (ASCII alphanumerics,
/// `.`, `-` and `_`), so a manifest parses the same way for an Apple-only
/// project as for an Android-only one. Whether the identifier is actually
/// usable is the platform's own question — an Apple build constructs an
/// [`AppleBundleIdentifier`] from it and an Android build an
/// [`AndroidPackageName`], each rejecting what its platform's grammar
/// forbids before any expensive toolchain work starts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BundleIdentifier(String);

impl BundleIdentifier {
    /// Returns the bundle identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The identifier as an Apple `CFBundleIdentifier`.
    ///
    /// # Errors
    /// Returns an error when the identifier is not valid on Apple platforms —
    /// an underscore being the common case, since Android accepts it and
    /// `CFBundleIdentifier` does not.
    pub fn apple_bundle_identifier(&self) -> Result<AppleBundleIdentifier, String> {
        AppleBundleIdentifier::try_from(self.0.clone())
    }

    /// The identifier as the Android package name — the Java package grammar
    /// Gradle renders `applicationId` and `namespace` from.
    ///
    /// # Errors
    /// Returns an error when the identifier is not a valid Java package
    /// name — a hyphen being the common case, since Apple accepts it and
    /// Android does not.
    pub fn android_package_name(&self) -> Result<AndroidPackageName, String> {
        AndroidPackageName::try_from(self.0.clone())
    }
}

impl TryFrom<String> for BundleIdentifier {
    type Error = String;

    /// The shared lexical shape: non-empty, and every character one a
    /// supported platform's identifier grammar accepts. Structure rules
    /// (segment shapes, leading characters) are platform grammar and are
    /// deliberately not enforced here — an Apple-only project's hyphenated
    /// identifier must not fail Android's rules it will never be checked
    /// against until it targets Android.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            return Err("bundle identifier cannot be empty".to_string());
        }
        if !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        }) {
            return Err(format!(
                "Invalid bundle identifier: '{value}' (only ASCII letters, digits, '.', '-' and '_' \
                 are supported identifier characters)."
            ));
        }
        Ok(Self(value))
    }
}

impl TryFrom<&str> for BundleIdentifier {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_string())
    }
}

impl From<BundleIdentifier> for String {
    fn from(value: BundleIdentifier) -> Self {
        value.0
    }
}

impl fmt::Display for BundleIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for BundleIdentifier {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for BundleIdentifier {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl From<&BundleIdentifier> for String {
    fn from(value: &BundleIdentifier) -> Self {
        value.0.clone()
    }
}

/// Android package name validated against Java package segment rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AndroidPackageName(String);

impl AndroidPackageName {
    /// Returns the Android package name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AndroidPackageName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err(
                "Android package name is empty (set `[package].bundle_identifier` in `Water.toml`)."
                    .to_string(),
            );
        }

        if value.contains('-') {
            return Err(format!(
                "Invalid bundle identifier '{value}' for Android: hyphens are not allowed in a \
Java package name — use '_' instead, in `[package].bundle_identifier` of `Water.toml`."
            ));
        }

        for segment in value.split('.') {
            if segment.is_empty() {
                return Err(format!(
                    "Invalid Android package name: '{value}' (empty segment)."
                ));
            }

            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return Err(format!(
                    "Invalid Android package name: '{value}' (empty segment)."
                ));
            };

            if !(first.is_ascii_alphabetic() || first == '_') {
                return Err(format!(
                    "Invalid Android package name: '{value}' (segment '{segment}' must start with a letter or underscore)."
                ));
            }

            if !chars.all(|character| character.is_ascii_alphanumeric() || character == '_') {
                return Err(format!(
                    "Invalid Android package name: '{value}' (segment '{segment}' contains invalid characters)."
                ));
            }
        }

        Ok(Self(value))
    }
}

impl fmt::Display for AndroidPackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for AndroidPackageName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for AndroidPackageName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl From<AndroidPackageName> for String {
    fn from(value: AndroidPackageName) -> Self {
        value.0
    }
}

/// Apple `CFBundleIdentifier` — the identifier Info.plist, `codesign` and the
/// provisioning pipeline consume.
///
/// Apple's documentation restricts the value to ASCII alphanumeric
/// characters, hyphens and periods; that character set is the whole grammar
/// validated here. In particular Apple's rules carry none of Java's segment
/// structure, so a leading digit or hyphen — invalid in an
/// [`AndroidPackageName`] segment — is accepted here the way
/// `CFBundleIdentifier` accepts it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppleBundleIdentifier(String);

impl AppleBundleIdentifier {
    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for AppleBundleIdentifier {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err(
                "Apple bundle identifier is empty (set `[package].bundle_identifier` in `Water.toml`)."
                    .to_string(),
            );
        }
        if !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.'))
        {
            return Err(format!(
                "Invalid bundle identifier '{value}' for Apple platforms: a `CFBundleIdentifier` \
                 accepts only ASCII letters, digits, '-' and '.' — edit `bundle_identifier` in \
                 `Water.toml` (underscores are not allowed)."
            ));
        }
        Ok(Self(value))
    }
}

impl TryFrom<&str> for AppleBundleIdentifier {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_string())
    }
}

impl fmt::Display for AppleBundleIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for AppleBundleIdentifier {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for AppleBundleIdentifier {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl From<AppleBundleIdentifier> for String {
    fn from(value: AppleBundleIdentifier) -> Self {
        value.0
    }
}

/// The `bundle_identifier` a fresh project gets when none is given:
/// `dev.waterui.<name>` with the display name in lower camel case.
///
/// `menu-example`, `menu_example` and `Menu Example` all derive
/// `dev.waterui.menuExample`, an identifier every supported platform's
/// grammar accepts.
///
/// The candidate is validated through each platform's own identifier type,
/// not just the shared lexer: a created project targets every platform, so
/// its default must be valid on all of them. A name that cannot produce
/// such an identifier — a leading digit, a name carrying no ASCII
/// identifier characters at all — is an error naming the failed grammar,
/// never a silently mangled identifier.
///
/// # Errors
/// Returns an error describing which platform grammar rejects the derived
/// identifier; the caller asks for an explicit identifier instead.
pub fn default_bundle_identifier(display_name: &str) -> Result<BundleIdentifier, String> {
    use heck::ToLowerCamelCase as _;

    let candidate = format!("dev.waterui.{}", display_name.to_lower_camel_case());
    let identifier = BundleIdentifier::try_from(candidate)?;
    identifier
        .apple_bundle_identifier()
        .and_then(|_| identifier.android_package_name())
        .map_err(|error| {
            format!(
                "the bundle identifier '{identifier}' derived from the project name \
                 '{display_name}' is not usable: {error} Pass an explicit identifier instead."
            )
        })?;
    Ok(identifier)
}

/// Logical permission keys that scaffold platform-specific manifest entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKey {
    /// Network access.
    Internet,
    /// Camera capture.
    Camera,
    /// Microphone recording.
    Microphone,
    /// Fine-grained location access.
    Location,
    /// Coarse-grained location access.
    CoarseLocation,
    /// Read access to shared storage.
    Storage,
    /// Write access to shared storage.
    WriteStorage,
    /// Photo library access.
    PhotoLibrary,
    /// Contacts access.
    Contacts,
    /// Calendar access.
    Calendars,
    /// Bluetooth access.
    Bluetooth,
    /// Legacy Android Bluetooth administration access.
    BluetoothAdmin,
    /// Vibration access.
    Vibrate,
    /// Wake lock access.
    WakeLock,
}

impl PermissionKey {
    /// Returns the Android manifest permission name for this logical permission when one exists.
    #[must_use]
    pub const fn android_permission_name(self) -> Option<&'static str> {
        match self {
            Self::Internet => Some("android.permission.INTERNET"),
            Self::Camera => Some("android.permission.CAMERA"),
            Self::Microphone => Some("android.permission.RECORD_AUDIO"),
            Self::Location => Some("android.permission.ACCESS_FINE_LOCATION"),
            Self::CoarseLocation => Some("android.permission.ACCESS_COARSE_LOCATION"),
            Self::Storage => Some("android.permission.READ_EXTERNAL_STORAGE"),
            Self::WriteStorage => Some("android.permission.WRITE_EXTERNAL_STORAGE"),
            Self::Bluetooth => Some("android.permission.BLUETOOTH"),
            Self::BluetoothAdmin => Some("android.permission.BLUETOOTH_ADMIN"),
            Self::Vibrate => Some("android.permission.VIBRATE"),
            Self::WakeLock => Some("android.permission.WAKE_LOCK"),
            Self::PhotoLibrary | Self::Contacts | Self::Calendars => None,
        }
    }

    /// Returns the generated Info.plist usage-description key for this permission when iOS requires one.
    #[must_use]
    pub const fn ios_plist_key(self) -> Option<&'static str> {
        match self {
            Self::Microphone => Some("INFOPLIST_KEY_NSMicrophoneUsageDescription"),
            Self::Camera => Some("INFOPLIST_KEY_NSCameraUsageDescription"),
            Self::Location => Some("INFOPLIST_KEY_NSLocationWhenInUseUsageDescription"),
            Self::PhotoLibrary => Some("INFOPLIST_KEY_NSPhotoLibraryUsageDescription"),
            Self::Contacts => Some("INFOPLIST_KEY_NSContactsUsageDescription"),
            Self::Calendars => Some("INFOPLIST_KEY_NSCalendarsUsageDescription"),
            Self::Bluetooth => Some("INFOPLIST_KEY_NSBluetoothAlwaysUsageDescription"),
            Self::Internet
            | Self::CoarseLocation
            | Self::Storage
            | Self::WriteStorage
            | Self::BluetoothAdmin
            | Self::Vibrate
            | Self::WakeLock => None,
        }
    }

    /// Returns the raw macOS Info.plist usage-description keys for this permission.
    #[must_use]
    pub const fn macos_usage_description_keys(self) -> &'static [&'static str] {
        match self {
            Self::Microphone => &["NSMicrophoneUsageDescription"],
            Self::Camera => &["NSCameraUsageDescription"],
            Self::Location => &[
                "NSLocationUsageDescription",
                "NSLocationWhenInUseUsageDescription",
            ],
            Self::PhotoLibrary => &["NSPhotoLibraryUsageDescription"],
            Self::Contacts => &["NSContactsUsageDescription"],
            Self::Calendars => &["NSCalendarsUsageDescription"],
            Self::Bluetooth => &["NSBluetoothAlwaysUsageDescription"],
            Self::Internet
            | Self::CoarseLocation
            | Self::Storage
            | Self::WriteStorage
            | Self::BluetoothAdmin
            | Self::Vibrate
            | Self::WakeLock => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        AndroidPackageName, BundleIdentifier, CrateName, default_bundle_identifier,
        generated_crate_name,
    };

    /// Two projects that share a crate name (`water create demo` twice, a
    /// copied project, two checkouts) generate identically suffixed crates —
    /// and in the shared per-user Cargo target the uplifted profile-root
    /// artifact is unhashed, so identical package names would overwrite each
    /// other's binary. The root tag keeps them apart.
    #[test]
    fn generated_crate_names_carry_the_project_root_tag() {
        let demo = CrateName::try_from("demo").expect("crate name");
        let first = generated_crate_name(&demo, "hydrolysis", Path::new("/work/first/demo"));
        let second = generated_crate_name(&demo, "hydrolysis", Path::new("/work/second/demo"));
        assert_ne!(
            first, second,
            "same-named projects at different roots must not share a generated package name"
        );
        assert!(
            first.as_str().starts_with("demo-hydrolysis-"),
            "the tag appends to the conventional suffix name: {first}"
        );
        assert_eq!(
            first,
            generated_crate_name(&demo, "hydrolysis", Path::new("/work/first/demo")),
            "the tag is stable across opens of one root"
        );
        assert_ne!(
            first,
            generated_crate_name(&demo, "gtk4", Path::new("/work/first/demo")),
            "the suffix still separates a project's own generated crates"
        );
    }

    /// The serialized field is platform-neutral: every character a supported
    /// platform's identifier grammar accepts — hyphens for Apple, underscores
    /// for Android — parses, and only characters no platform supports fail.
    #[test]
    fn bundle_identifier_accepts_the_union_of_platform_characters() {
        for identifier in [
            "dev.waterui.liquid-glass",
            "dev.waterui.liquid_glass",
            "dev.waterui.1glass",
        ] {
            assert!(
                BundleIdentifier::try_from(identifier).is_ok(),
                "{identifier} uses only supported identifier characters"
            );
        }
        for identifier in ["", "dev waterui", "dev.waterui.app@host", "dev/waterui"] {
            assert!(
                BundleIdentifier::try_from(identifier).is_err(),
                "{identifier} carries no platform-supported shape"
            );
        }
    }

    /// Apple grammar is `CFBundleIdentifier`'s: ASCII alphanumerics, hyphens
    /// and periods — hyphens a Java package name rejects are valid, while an
    /// underscore Android accepts is rejected with a platform-named error.
    /// Apple's rules carry no segment or leading-character structure.
    #[test]
    fn apple_bundle_identifier_applies_apples_grammar() {
        let hyphenated = BundleIdentifier::try_from("dev.waterui.liquid-glass")
            .expect("hyphens are shared-lexical");
        assert_eq!(
            hyphenated
                .apple_bundle_identifier()
                .expect("Apple accepts hyphens")
                .as_str(),
            "dev.waterui.liquid-glass"
        );
        let leading_digit = BundleIdentifier::try_from("dev.waterui.1glass").unwrap();
        assert!(
            leading_digit.apple_bundle_identifier().is_ok(),
            "Apple's CFBundleIdentifier grammar has no leading-character rule"
        );

        let underscored = BundleIdentifier::try_from("dev.waterui.liquid_glass").unwrap();
        let error = underscored.apple_bundle_identifier().unwrap_err();
        assert!(
            error.contains("Apple") && error.contains('_'),
            "the rejection names the platform and the character: {error}"
        );
    }

    /// Android grammar is Java package segments: underscores pass, hyphens
    /// and digit-led segments are rejected before any Gradle or SDK work.
    #[test]
    fn android_package_name_applies_java_package_grammar() {
        let underscored = BundleIdentifier::try_from("dev.waterui.liquid_glass")
            .expect("underscores are shared-lexical");
        assert_eq!(
            underscored
                .android_package_name()
                .expect("Android accepts underscores"),
            AndroidPackageName::try_from("dev.waterui.liquid_glass".to_string()).unwrap()
        );

        let hyphenated = BundleIdentifier::try_from("dev.waterui.liquid-glass").unwrap();
        let error = hyphenated.android_package_name().unwrap_err();
        assert!(
            error.contains("Android") && error.contains("hyphens are not allowed"),
            "the rejection names the platform and the rule: {error}"
        );

        let leading_digit = BundleIdentifier::try_from("dev.waterui.1glass").unwrap();
        assert!(
            leading_digit.android_package_name().is_err(),
            "Java package segments may not lead with a digit"
        );
        let empty_segment = BundleIdentifier::try_from("dev.waterui..glass").unwrap();
        assert!(empty_segment.android_package_name().is_err());
    }

    /// The generated default is valid on every supported platform: ordinary
    /// multiword names in any separator style derive the same lower-camel
    /// identifier.
    #[test]
    fn default_bundle_identifier_is_valid_on_every_platform() {
        for name in ["Menu Example", "menu-example", "menu_example"] {
            let identifier = default_bundle_identifier(name).expect("a usable default");
            assert_eq!(identifier.as_str(), "dev.waterui.menuExample");
        }
    }

    /// A name that cannot derive an identifier valid on every supported
    /// platform — digit-led segments are fine for `CFBundleIdentifier` but
    /// not a Java package name, and a name carrying no identifier
    /// characters derives nothing at all — fails naming the platform
    /// grammar that rejects it, never a silently mangled or constant
    /// identifier.
    #[test]
    fn default_bundle_identifier_rejects_names_without_a_common_valid_default() {
        let digit_led = default_bundle_identifier("3D Printer").unwrap_err();
        assert!(digit_led.contains("Android"), "{digit_led}");
        for name in ["", "___", "…"] {
            assert!(
                default_bundle_identifier(name).is_err(),
                "{name} cannot name a bundle identifier"
            );
        }
    }
}
