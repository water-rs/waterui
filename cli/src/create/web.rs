use std::{collections::HashMap, path::Path};

use color_eyre::eyre::Result;

use super::template;

pub fn create_web_assets(project_dir: &Path, display_name: &str) -> Result<()> {
    let mut context = HashMap::new();
    context.insert("APP_DISPLAY_NAME", display_name.to_string());

    let templates = &template::TEMPLATES_DIR;
    let web_dir = templates
        .get_dir("web")
        .expect("web template directory must exist");
    template::process_template_directory(web_dir, &project_dir.join("web"), &context)
}
