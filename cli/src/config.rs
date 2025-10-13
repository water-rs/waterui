use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub package: Package,
    #[serde(default)]
    pub backends: Backends,
    #[serde(default)]
    pub hot_reload: HotReload,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Backends {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swift: Option<Swift>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android: Option<Android>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<Web>,
}

impl Config {
    pub fn load(root: &Path) -> Result<Self> {
        let path = Self::path(root);
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = Self::path(root);
        let contents = toml::to_string_pretty(self)?;
        fs::write(&path, contents).with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn path(root: &Path) -> PathBuf {
        root.join("waterui.toml")
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Package {
    pub name: String,
    pub display_name: String,
    pub bundle_identifier: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Swift {
    #[serde(default = "default_swift_project_path")]
    pub project_path: String,
    pub scheme: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_file: Option<String>,
}

fn default_swift_project_path() -> String {
    "apple".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Android {
    #[serde(default = "default_android_project_path")]
    pub project_path: String,
}

fn default_android_project_path() -> String {
    "android".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Web {
    #[serde(default = "default_web_project_path")]
    pub project_path: String,
}

fn default_web_project_path() -> String {
    "web".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct HotReload {
    /// Additional paths to watch for triggering rebuilds
    #[serde(default)]
    pub watch: Vec<String>,
}

impl Config {
    pub fn new(package: Package) -> Self {
        Self {
            package,
            backends: Backends::default(),
            hot_reload: HotReload::default(),
        }
    }
}
