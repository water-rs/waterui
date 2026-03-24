use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CrateName(String);

impl CrateName {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn rust_ident(&self) -> RustIdent {
        RustIdent(self.0.replace('-', "_"))
    }

    #[must_use]
    pub fn with_suffix(&self, suffix: &str) -> Self {
        Self(format!("{}-{suffix}", self.0))
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RustIdent(String);

impl RustIdent {
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BundleIdentifier(String);

impl BundleIdentifier {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn android_package_name(&self) -> Result<AndroidPackageName, String> {
        AndroidPackageName::try_from(self.0.clone())
    }
}

impl TryFrom<String> for BundleIdentifier {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            return Err("bundle identifier cannot be empty".to_string());
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AndroidPackageName(String);

impl AndroidPackageName {
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
                "Invalid Android package name: '{value}' (hyphens are not allowed). \
Set `[package].bundle_identifier` in `Water.toml` to a valid Java package name (e.g. replace '-' with '_')."
            ));
        }

        for segment in value.split('.') {
            if segment.is_empty() {
                return Err(format!("Invalid Android package name: '{value}' (empty segment)."));
            }

            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return Err(format!("Invalid Android package name: '{value}' (empty segment)."));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKey {
    Internet,
    Camera,
    Microphone,
    Location,
    CoarseLocation,
    Storage,
    WriteStorage,
    PhotoLibrary,
    Contacts,
    Calendars,
    Bluetooth,
    BluetoothAdmin,
    Vibrate,
    WakeLock,
}

impl PermissionKey {
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
}
