use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct WaterConfig {
    pub project: ProjectConfig,
    pub dependencies: HashMap<String, Dependency>,
    #[serde(default)]
    pub features: FeaturesConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    Simple(String),
    Detailed {
        version: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        features: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        git: Option<String>,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FeaturesConfig {
    #[serde(default)]
    pub default_backend: Option<String>,
    #[serde(default)]
    pub backends: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<String>,
}

impl WaterConfig {
    pub fn new(name: String, authors: Vec<String>) -> Self {
        Self {
            project: ProjectConfig {
                name,
                version: "0.1.0".to_string(),
                authors,
                description: Some("A WaterUI project".to_string()),
                license: Some("MIT OR Apache-2.0".to_string()),
                repository: None,
            },
            dependencies: HashMap::new(),
            features: FeaturesConfig::default(),
        }
    }
    
    pub fn add_dependency(&mut self, name: String, version: String) {
        self.dependencies.insert(name, Dependency::Simple(version));
    }
    
    pub fn to_toml_string(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}