use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct CargoToml {
    pub package: Package,
    pub dependencies: HashMap<String, Dependency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<Features>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub edition: String,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct Features {
    pub default: Vec<String>,
}

impl CargoToml {
    pub fn new(name: String) -> Self {
        let mut dependencies = HashMap::new();
        dependencies.insert(
            "waterui".to_string(),
            Dependency::Detailed {
                version: "0.1.0".to_string(),
                features: None,
                path: None,
                git: None,
            },
        );
        dependencies.insert(
            "nami".to_string(),
            Dependency::Detailed {
                version: "0.1.0".to_string(),
                features: None,
                path: None,
                git: None,
            },
        );

        Self {
            package: Package {
                name,
                version: "0.1.0".to_string(),
                edition: "2021".to_string(),
            },
            dependencies,
            features: Some(Features {
                default: vec![],
            }),
        }
    }

    pub fn to_toml_string(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}