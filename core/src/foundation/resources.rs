//! Resource directories owned by one application environment.

use std::path::{Path, PathBuf};

/// The resource roots of one native application or embedded instance.
///
/// Install this value into the instance's [`crate::Environment`] before
/// constructing its app. Asset handles resolve against this value; no
/// process-global resource root is changed when another instance starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceContext {
    assets: PathBuf,
    fonts: PathBuf,
}

impl ResourceContext {
    /// Own the directories supplied by a native host.
    #[must_use]
    pub fn new(assets: impl Into<PathBuf>, fonts: impl Into<PathBuf>) -> Self {
        Self {
            assets: assets.into(),
            fonts: fonts.into(),
        }
    }

    /// Resources adjacent to an application's executable or main bundle.
    ///
    /// The environment override is captured once at the application boundary;
    /// subsequent asset resolution reads only the owned context. An app with
    /// no resources can start without either directory existing.
    ///
    /// # Errors
    /// Returns an error when the executable cannot be located.
    ///
    /// # Panics
    /// Panics if an executable or explicitly supplied asset root has no parent directory.
    pub fn application() -> std::io::Result<Self> {
        let executable = std::env::current_exe()?;
        let directory = executable.parent().expect("executable has a parent");
        let mut candidates = vec![
            directory.join("waterui_assets"),
            directory.join("resources/waterui_assets"),
        ];
        if let Some(parent) = directory.parent() {
            candidates.push(parent.join("Resources/waterui_assets"));
        }
        let assets = std::env::var_os("WATERUI_ASSETS_ROOT")
            .map(PathBuf::from)
            .or_else(|| candidates.into_iter().find(|path| path.is_dir()))
            .unwrap_or_else(|| directory.join("waterui_assets"));
        let fonts = assets
            .parent()
            .expect("asset root has a parent")
            .join("fonts");
        Ok(Self::new(assets, fonts))
    }

    /// Root containing the main assets and named bundle mounts.
    #[must_use]
    pub fn assets(&self) -> &Path {
        &self.assets
    }

    /// Directory of fonts staged for this instance.
    #[must_use]
    pub fn fonts(&self) -> &Path {
        &self.fonts
    }

    /// The resource context explicitly installed by the host.
    ///
    /// # Panics
    /// Panics if the host constructed the view tree without a resource context.
    #[must_use]
    pub fn from_environment(env: &crate::Environment) -> &Self {
        env.get()
            .expect("host must install ResourceContext before constructing the app")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separate_environments_keep_their_resource_roots() {
        let mut first = crate::Environment::new();
        first.insert(ResourceContext::new("/first/assets", "/first/fonts"));
        let mut second = first.clone();
        second.insert(ResourceContext::new("/second/assets", "/second/fonts"));
        assert_eq!(
            ResourceContext::from_environment(&first).assets(),
            Path::new("/first/assets")
        );
        assert_eq!(
            ResourceContext::from_environment(&second).fonts(),
            Path::new("/second/fonts")
        );
    }
}
