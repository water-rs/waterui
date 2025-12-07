#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AndroidBackend {
    #[serde(
        default = "default_android_project_path",
        skip_serializing_if = "is_default_android_project_path"
    )]
    project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
}

#[must_use]
pub fn default_android_project_path() -> String {
    "android".to_string()
}

fn is_default_android_project_path(s: &str) -> bool {
    s == "android"
}
