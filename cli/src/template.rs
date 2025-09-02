use crate::cargo_config::CargoToml;
use anyhow::Result;
use std::fs;
use std::path::Path;

const LIB_RS_TEMPLATE: &str = include_str!("../templates/lib.rs");
const GITIGNORE_TEMPLATE: &str = include_str!("../templates/gitignore");

pub fn create_project_structure(path: &Path, name: &str) -> Result<()> {
    fs::create_dir_all(path.join("src"))?;
    
    // Write lib.rs from template
    fs::write(path.join("src/lib.rs"), LIB_RS_TEMPLATE)?;
    
    // Generate Cargo.toml using serde
    let cargo_toml = CargoToml::new(name.to_string());
    fs::write(path.join("Cargo.toml"), cargo_toml.to_toml_string()?)?;
    
    // Write .gitignore from template
    fs::write(path.join(".gitignore"), GITIGNORE_TEMPLATE)?;
    
    Ok(())
}