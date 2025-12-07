use std::{collections::HashMap, sync::LazyLock};

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Permission {
    #[serde(skip)]
    name: String,
    /// You should explain why the permission is needed here.
    description: String,
    note: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Permissions {
    permissions: HashMap<String, PermissionEntry>,
}

impl Permissions {
    /// Check if no permissions are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.permissions.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<Permission> {
        self.permissions.get(name).map(|entry| Permission {
            name: name.to_string(),
            description: entry.description.clone(),
            note: entry.note.clone(),
        })
    }

    pub fn insert(&mut self, permission: Permission) {
        self.permissions.insert(
            permission.name,
            PermissionEntry {
                description: permission.description,
                note: permission.note,
            },
        );
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct PermissionEntry {
    description: String,
    note: Option<String>,
}

impl Permission {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        let name = name.into();
        assert!(
            PERMISSION_MAPPING.contains_key(&name),
            "Permission '{}' is not defined in the mapping",
            name,
        );

        Self {
            name,
            description: description.into(),
            note: None,
        }
    }

    /// Get platform-specific permission details.
    pub fn for_platform(&self, platform: &str) -> &PlatformPermission {
        PERMISSION_MAPPING.get(platform).unwrap()
    }
}

/// Serve for mapping the permission names to platform-specific details.
///
/// See `mapping.toml` for the actual mapping.
#[derive(Debug, Deserialize)]
enum PlatformPermission {
    Ios {
        info_plist_key: String,
        default_description: String,
        entitlement: String,
    },
    Android {
        manifest_permission: String,
        dangerous: bool,
        min_sdk: u32,
        max_sdk: u32,
    },
}

type PlatformPermissionMapping = HashMap<String, PlatformPermission>;

static PERMISSION_MAPPING: LazyLock<PlatformPermissionMapping> = LazyLock::new(load_mapping);

fn load_mapping() -> PlatformPermissionMapping {
    let content = include_str!("mapping.toml");
    toml::from_str(content).expect("Failed to parse permissions mapping")
}
