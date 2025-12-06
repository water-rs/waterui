//! Cross-platform permission definitions and mappings.
//!
//! This module defines abstract permission identifiers that map to platform-specific
//! permissions (iOS Info.plist keys and Android manifest permissions).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Abstract permission identifiers that map to platform-specific permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    // Device Hardware
    Camera,
    Microphone,

    // Location
    Location,
    LocationAlways,
    LocationWhenInUse,

    // Media & Storage
    Photos,
    PhotosAddOnly,
    MediaLibrary,

    // Contacts & Calendar
    Contacts,
    Calendar,
    Reminders,

    // Communication
    Notifications,

    // Network
    Internet,
    Bluetooth,
    BluetoothPeripheral,
    Nfc,
    WifiState,

    // Biometrics & Security
    Biometrics,

    // Sensors
    Motion,
    HealthRead,
    HealthWrite,

    // Other Apple-specific
    SpeechRecognition,
    Siri,
    HomeKit,
    AppleMusic,

    // Android-specific
    ReadExternalStorage,
    WriteExternalStorage,
    Vibrate,
    WakeLock,
    ReceiveBootCompleted,
    ReadPhoneState,
    CallPhone,
    SendSms,
    ReceiveSms,
    ReadSms,
}

impl Permission {
    /// Returns all available permissions.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Camera,
            Self::Microphone,
            Self::Location,
            Self::LocationAlways,
            Self::LocationWhenInUse,
            Self::Photos,
            Self::PhotosAddOnly,
            Self::MediaLibrary,
            Self::Contacts,
            Self::Calendar,
            Self::Reminders,
            Self::Notifications,
            Self::Internet,
            Self::Bluetooth,
            Self::BluetoothPeripheral,
            Self::Nfc,
            Self::WifiState,
            Self::Biometrics,
            Self::Motion,
            Self::HealthRead,
            Self::HealthWrite,
            Self::SpeechRecognition,
            Self::Siri,
            Self::HomeKit,
            Self::AppleMusic,
            Self::ReadExternalStorage,
            Self::WriteExternalStorage,
            Self::Vibrate,
            Self::WakeLock,
            Self::ReceiveBootCompleted,
            Self::ReadPhoneState,
            Self::CallPhone,
            Self::SendSms,
            Self::ReceiveSms,
            Self::ReadSms,
        ]
    }

    /// Returns the kebab-case name of the permission.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Microphone => "microphone",
            Self::Location => "location",
            Self::LocationAlways => "location-always",
            Self::LocationWhenInUse => "location-when-in-use",
            Self::Photos => "photos",
            Self::PhotosAddOnly => "photos-add-only",
            Self::MediaLibrary => "media-library",
            Self::Contacts => "contacts",
            Self::Calendar => "calendar",
            Self::Reminders => "reminders",
            Self::Notifications => "notifications",
            Self::Internet => "internet",
            Self::Bluetooth => "bluetooth",
            Self::BluetoothPeripheral => "bluetooth-peripheral",
            Self::Nfc => "nfc",
            Self::WifiState => "wifi-state",
            Self::Biometrics => "biometrics",
            Self::Motion => "motion",
            Self::HealthRead => "health-read",
            Self::HealthWrite => "health-write",
            Self::SpeechRecognition => "speech-recognition",
            Self::Siri => "siri",
            Self::HomeKit => "homekit",
            Self::AppleMusic => "apple-music",
            Self::ReadExternalStorage => "read-external-storage",
            Self::WriteExternalStorage => "write-external-storage",
            Self::Vibrate => "vibrate",
            Self::WakeLock => "wake-lock",
            Self::ReceiveBootCompleted => "receive-boot-completed",
            Self::ReadPhoneState => "read-phone-state",
            Self::CallPhone => "call-phone",
            Self::SendSms => "send-sms",
            Self::ReceiveSms => "receive-sms",
            Self::ReadSms => "read-sms",
        }
    }

    /// Returns the platform-specific mappings for this permission.
    #[must_use]
    pub fn mapping(&self) -> PermissionMapping {
        match self {
            Self::Camera => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSCameraUsageDescription",
                    default_description: "This app needs camera access for capturing photos and videos.",
                    entitlement: None,
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.CAMERA",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::Microphone => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSMicrophoneUsageDescription",
                    default_description: "This app needs microphone access for audio recording.",
                    entitlement: None,
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.RECORD_AUDIO",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::Location => PermissionMapping {
                ios: Some(vec![
                    IosPermission {
                        info_plist_key: "NSLocationWhenInUseUsageDescription",
                        default_description: "This app needs location access while in use.",
                        entitlement: None,
                    },
                    IosPermission {
                        info_plist_key: "NSLocationAlwaysAndWhenInUseUsageDescription",
                        default_description: "This app needs location access.",
                        entitlement: None,
                    },
                ]),
                android: Some(vec![
                    AndroidPermission {
                        manifest_permission: "android.permission.ACCESS_FINE_LOCATION",
                        dangerous: true,
                        min_sdk: 0,
                        max_sdk: 0,
                    },
                    AndroidPermission {
                        manifest_permission: "android.permission.ACCESS_COARSE_LOCATION",
                        dangerous: true,
                        min_sdk: 0,
                        max_sdk: 0,
                    },
                ]),
            },
            Self::LocationAlways => PermissionMapping {
                ios: Some(vec![
                    IosPermission {
                        info_plist_key: "NSLocationAlwaysUsageDescription",
                        default_description: "This app needs location access even when not in use.",
                        entitlement: None,
                    },
                    IosPermission {
                        info_plist_key: "NSLocationAlwaysAndWhenInUseUsageDescription",
                        default_description: "This app needs location access.",
                        entitlement: None,
                    },
                ]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.ACCESS_BACKGROUND_LOCATION",
                    dangerous: true,
                    min_sdk: 29,
                    max_sdk: 0,
                }]),
            },
            Self::LocationWhenInUse => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSLocationWhenInUseUsageDescription",
                    default_description: "This app needs location access while in use.",
                    entitlement: None,
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.ACCESS_FINE_LOCATION",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::Photos => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSPhotoLibraryUsageDescription",
                    default_description: "This app needs access to your photo library.",
                    entitlement: None,
                }]),
                android: Some(vec![
                    AndroidPermission {
                        manifest_permission: "android.permission.READ_MEDIA_IMAGES",
                        dangerous: true,
                        min_sdk: 33,
                        max_sdk: 0,
                    },
                    AndroidPermission {
                        manifest_permission: "android.permission.READ_MEDIA_VIDEO",
                        dangerous: true,
                        min_sdk: 33,
                        max_sdk: 0,
                    },
                    AndroidPermission {
                        manifest_permission: "android.permission.READ_EXTERNAL_STORAGE",
                        dangerous: true,
                        min_sdk: 0,
                        max_sdk: 32,
                    },
                ]),
            },
            Self::PhotosAddOnly => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSPhotoLibraryAddUsageDescription",
                    default_description: "This app needs permission to save photos to your library.",
                    entitlement: None,
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.WRITE_EXTERNAL_STORAGE",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 28,
                }]),
            },
            Self::MediaLibrary => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSAppleMusicUsageDescription",
                    default_description: "This app needs access to your media library.",
                    entitlement: None,
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.READ_MEDIA_AUDIO",
                    dangerous: true,
                    min_sdk: 33,
                    max_sdk: 0,
                }]),
            },
            Self::Contacts => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSContactsUsageDescription",
                    default_description: "This app needs access to your contacts.",
                    entitlement: None,
                }]),
                android: Some(vec![
                    AndroidPermission {
                        manifest_permission: "android.permission.READ_CONTACTS",
                        dangerous: true,
                        min_sdk: 0,
                        max_sdk: 0,
                    },
                    AndroidPermission {
                        manifest_permission: "android.permission.WRITE_CONTACTS",
                        dangerous: true,
                        min_sdk: 0,
                        max_sdk: 0,
                    },
                ]),
            },
            Self::Calendar => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSCalendarsUsageDescription",
                    default_description: "This app needs access to your calendar.",
                    entitlement: None,
                }]),
                android: Some(vec![
                    AndroidPermission {
                        manifest_permission: "android.permission.READ_CALENDAR",
                        dangerous: true,
                        min_sdk: 0,
                        max_sdk: 0,
                    },
                    AndroidPermission {
                        manifest_permission: "android.permission.WRITE_CALENDAR",
                        dangerous: true,
                        min_sdk: 0,
                        max_sdk: 0,
                    },
                ]),
            },
            Self::Reminders => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSRemindersUsageDescription",
                    default_description: "This app needs access to your reminders.",
                    entitlement: None,
                }]),
                android: None, // Android uses Calendar permissions
            },
            Self::Notifications => PermissionMapping {
                ios: None, // iOS handles via UNUserNotificationCenter, not Info.plist
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.POST_NOTIFICATIONS",
                    dangerous: true,
                    min_sdk: 33,
                    max_sdk: 0,
                }]),
            },
            Self::Internet => PermissionMapping {
                ios: None, // No permission needed on iOS
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.INTERNET",
                    dangerous: false,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::Bluetooth => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSBluetoothAlwaysUsageDescription",
                    default_description: "This app uses Bluetooth to connect to nearby devices.",
                    entitlement: None,
                }]),
                android: Some(vec![
                    AndroidPermission {
                        manifest_permission: "android.permission.BLUETOOTH_CONNECT",
                        dangerous: true,
                        min_sdk: 31,
                        max_sdk: 0,
                    },
                    AndroidPermission {
                        manifest_permission: "android.permission.BLUETOOTH_SCAN",
                        dangerous: true,
                        min_sdk: 31,
                        max_sdk: 0,
                    },
                    AndroidPermission {
                        manifest_permission: "android.permission.BLUETOOTH",
                        dangerous: false,
                        min_sdk: 0,
                        max_sdk: 30,
                    },
                    AndroidPermission {
                        manifest_permission: "android.permission.BLUETOOTH_ADMIN",
                        dangerous: false,
                        min_sdk: 0,
                        max_sdk: 30,
                    },
                ]),
            },
            Self::BluetoothPeripheral => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSBluetoothPeripheralUsageDescription",
                    default_description: "This app uses Bluetooth to communicate with peripherals.",
                    entitlement: None,
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.BLUETOOTH_ADVERTISE",
                    dangerous: true,
                    min_sdk: 31,
                    max_sdk: 0,
                }]),
            },
            Self::Nfc => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NFCReaderUsageDescription",
                    default_description: "This app uses NFC to read tags.",
                    entitlement: Some("com.apple.developer.nfc.readersession.formats"),
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.NFC",
                    dangerous: false,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::WifiState => PermissionMapping {
                ios: None, // iOS uses NEHotspotHelper which requires entitlement
                android: Some(vec![
                    AndroidPermission {
                        manifest_permission: "android.permission.ACCESS_WIFI_STATE",
                        dangerous: false,
                        min_sdk: 0,
                        max_sdk: 0,
                    },
                    AndroidPermission {
                        manifest_permission: "android.permission.CHANGE_WIFI_STATE",
                        dangerous: false,
                        min_sdk: 0,
                        max_sdk: 0,
                    },
                ]),
            },
            Self::Biometrics => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSFaceIDUsageDescription",
                    default_description: "This app uses Face ID for secure authentication.",
                    entitlement: None,
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.USE_BIOMETRIC",
                    dangerous: false,
                    min_sdk: 28,
                    max_sdk: 0,
                }]),
            },
            Self::Motion => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSMotionUsageDescription",
                    default_description: "This app uses motion sensors for activity tracking.",
                    entitlement: None,
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.ACTIVITY_RECOGNITION",
                    dangerous: true,
                    min_sdk: 29,
                    max_sdk: 0,
                }]),
            },
            Self::HealthRead => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSHealthShareUsageDescription",
                    default_description: "This app reads health data to provide personalized insights.",
                    entitlement: Some("com.apple.developer.healthkit"),
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.BODY_SENSORS",
                    dangerous: true,
                    min_sdk: 20,
                    max_sdk: 0,
                }]),
            },
            Self::HealthWrite => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSHealthUpdateUsageDescription",
                    default_description: "This app writes health data to track your progress.",
                    entitlement: Some("com.apple.developer.healthkit"),
                }]),
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.BODY_SENSORS",
                    dangerous: true,
                    min_sdk: 20,
                    max_sdk: 0,
                }]),
            },
            Self::SpeechRecognition => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSSpeechRecognitionUsageDescription",
                    default_description: "This app uses speech recognition for voice commands.",
                    entitlement: None,
                }]),
                android: None, // Android uses RECORD_AUDIO
            },
            Self::Siri => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSSiriUsageDescription",
                    default_description: "This app integrates with Siri for voice control.",
                    entitlement: Some("com.apple.developer.siri"),
                }]),
                android: None,
            },
            Self::HomeKit => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSHomeKitUsageDescription",
                    default_description: "This app controls your HomeKit accessories.",
                    entitlement: Some("com.apple.developer.homekit"),
                }]),
                android: None,
            },
            Self::AppleMusic => PermissionMapping {
                ios: Some(vec![IosPermission {
                    info_plist_key: "NSAppleMusicUsageDescription",
                    default_description: "This app needs access to Apple Music.",
                    entitlement: None,
                }]),
                android: None,
            },
            Self::ReadExternalStorage => PermissionMapping {
                ios: None,
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.READ_EXTERNAL_STORAGE",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 32,
                }]),
            },
            Self::WriteExternalStorage => PermissionMapping {
                ios: None,
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.WRITE_EXTERNAL_STORAGE",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 28,
                }]),
            },
            Self::Vibrate => PermissionMapping {
                ios: None, // No permission needed on iOS
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.VIBRATE",
                    dangerous: false,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::WakeLock => PermissionMapping {
                ios: None,
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.WAKE_LOCK",
                    dangerous: false,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::ReceiveBootCompleted => PermissionMapping {
                ios: None,
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.RECEIVE_BOOT_COMPLETED",
                    dangerous: false,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::ReadPhoneState => PermissionMapping {
                ios: None,
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.READ_PHONE_STATE",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::CallPhone => PermissionMapping {
                ios: None,
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.CALL_PHONE",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::SendSms => PermissionMapping {
                ios: None,
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.SEND_SMS",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::ReceiveSms => PermissionMapping {
                ios: None,
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.RECEIVE_SMS",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
            Self::ReadSms => PermissionMapping {
                ios: None,
                android: Some(vec![AndroidPermission {
                    manifest_permission: "android.permission.READ_SMS",
                    dangerous: true,
                    min_sdk: 0,
                    max_sdk: 0,
                }]),
            },
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl FromStr for Permission {
    type Err = ParsePermissionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "camera" => Ok(Self::Camera),
            "microphone" => Ok(Self::Microphone),
            "location" => Ok(Self::Location),
            "location-always" => Ok(Self::LocationAlways),
            "location-when-in-use" => Ok(Self::LocationWhenInUse),
            "photos" => Ok(Self::Photos),
            "photos-add-only" => Ok(Self::PhotosAddOnly),
            "media-library" => Ok(Self::MediaLibrary),
            "contacts" => Ok(Self::Contacts),
            "calendar" => Ok(Self::Calendar),
            "reminders" => Ok(Self::Reminders),
            "notifications" => Ok(Self::Notifications),
            "internet" => Ok(Self::Internet),
            "bluetooth" => Ok(Self::Bluetooth),
            "bluetooth-peripheral" => Ok(Self::BluetoothPeripheral),
            "nfc" => Ok(Self::Nfc),
            "wifi-state" => Ok(Self::WifiState),
            "biometrics" => Ok(Self::Biometrics),
            "motion" => Ok(Self::Motion),
            "health-read" => Ok(Self::HealthRead),
            "health-write" => Ok(Self::HealthWrite),
            "speech-recognition" => Ok(Self::SpeechRecognition),
            "siri" => Ok(Self::Siri),
            "homekit" => Ok(Self::HomeKit),
            "apple-music" => Ok(Self::AppleMusic),
            "read-external-storage" => Ok(Self::ReadExternalStorage),
            "write-external-storage" => Ok(Self::WriteExternalStorage),
            "vibrate" => Ok(Self::Vibrate),
            "wake-lock" => Ok(Self::WakeLock),
            "receive-boot-completed" => Ok(Self::ReceiveBootCompleted),
            "read-phone-state" => Ok(Self::ReadPhoneState),
            "call-phone" => Ok(Self::CallPhone),
            "send-sms" => Ok(Self::SendSms),
            "receive-sms" => Ok(Self::ReceiveSms),
            "read-sms" => Ok(Self::ReadSms),
            _ => Err(ParsePermissionError(s.to_string())),
        }
    }
}

/// Error returned when parsing an unknown permission name.
#[derive(Debug, Clone)]
pub struct ParsePermissionError(pub String);

impl fmt::Display for ParsePermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown permission: {}", self.0)
    }
}

impl std::error::Error for ParsePermissionError {}

/// iOS Info.plist permission entry.
#[derive(Debug, Clone)]
pub struct IosPermission {
    /// The `NSUsageDescription` key (e.g., "`NSCameraUsageDescription`")
    pub info_plist_key: &'static str,
    /// Default usage description if none provided
    pub default_description: &'static str,
    /// Optional entitlement key (e.g., for `HealthKit`)
    pub entitlement: Option<&'static str>,
}

/// Android manifest permission entry.
#[derive(Debug, Clone)]
pub struct AndroidPermission {
    /// The permission name (e.g., "android.permission.CAMERA")
    pub manifest_permission: &'static str,
    /// Whether this is a dangerous permission requiring runtime request
    pub dangerous: bool,
    /// Minimum API level required (0 = all)
    pub min_sdk: u32,
    /// Maximum API level (0 = no limit)
    pub max_sdk: u32,
}

/// Platform-specific permission mappings for an abstract permission.
#[derive(Debug, Clone)]
pub struct PermissionMapping {
    pub ios: Option<Vec<IosPermission>>,
    pub android: Option<Vec<AndroidPermission>>,
}

/// Resolved permission with custom description.
#[derive(Debug, Clone)]
pub struct ResolvedPermission {
    pub permission: Permission,
    pub description: Option<String>,
}

impl ResolvedPermission {
    /// Generate iOS Info.plist entries for this permission.
    #[must_use]
    pub fn ios_plist_entries(&self) -> Vec<(String, String)> {
        let mapping = self.permission.mapping();
        let Some(ios_perms) = mapping.ios else {
            return Vec::new();
        };

        ios_perms
            .iter()
            .map(|p| {
                let desc = self
                    .description
                    .as_deref()
                    .unwrap_or(p.default_description);
                (p.info_plist_key.to_string(), desc.to_string())
            })
            .collect()
    }

    /// Generate Android manifest permission entries for this permission.
    #[must_use]
    pub fn android_manifest_entries(&self) -> Vec<String> {
        let mapping = self.permission.mapping();
        let Some(android_perms) = mapping.android else {
            return Vec::new();
        };

        android_perms
            .iter()
            .map(|p| {
                let mut entry =
                    format!("    <uses-permission android:name=\"{}\"", p.manifest_permission);
                if p.min_sdk > 0 && p.max_sdk > 0 {
                    entry.push_str(&format!(
                        " android:minSdkVersion=\"{}\" android:maxSdkVersion=\"{}\"",
                        p.min_sdk, p.max_sdk
                    ));
                } else if p.max_sdk > 0 {
                    entry.push_str(&format!(" android:maxSdkVersion=\"{}\"", p.max_sdk));
                }
                entry.push_str(" />");
                entry
            })
            .collect()
    }
}

/// Generate iOS Info.plist permission entries from a list of resolved permissions.
#[must_use]
pub fn generate_ios_plist_entries(permissions: &[ResolvedPermission]) -> String {
    let mut entries = String::new();
    for perm in permissions {
        for (key, desc) in perm.ios_plist_entries() {
            entries.push_str(&format!(
                "    <key>{key}</key>\n    <string>{desc}</string>\n"
            ));
        }
    }
    entries
}

/// Generate Android manifest permission entries from a list of resolved permissions.
#[must_use]
pub fn generate_android_manifest_entries(permissions: &[ResolvedPermission]) -> String {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    for perm in permissions {
        for entry in perm.android_manifest_entries() {
            // Deduplicate by permission name
            let key = entry.clone();
            if seen.insert(key) {
                entries.push(entry);
            }
        }
    }

    entries.join("\n")
}
