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
    /// Returns an error when the executable cannot be located, or when the
    /// executable or an explicitly supplied `WATERUI_ASSETS_ROOT` has no
    /// parent directory to hold the fonts beside the assets.
    pub fn application() -> std::io::Result<Self> {
        Self::beside(
            &std::env::current_exe()?,
            std::env::var_os("WATERUI_ASSETS_ROOT").map(PathBuf::from),
        )
    }

    /// The context of `executable`, with `assets_root` overriding the
    /// directory the assets are looked up in.
    fn beside(executable: &Path, assets_root: Option<PathBuf>) -> std::io::Result<Self> {
        let directory = executable.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "the executable path {} has no parent directory",
                    executable.display()
                ),
            )
        })?;
        let mut candidates = vec![
            directory.join("waterui_assets"),
            directory.join("resources/waterui_assets"),
        ];
        if let Some(parent) = directory.parent() {
            candidates.push(parent.join("Resources/waterui_assets"));
        }
        let assets = assets_root
            .or_else(|| candidates.into_iter().find(|path| path.is_dir()))
            .unwrap_or_else(|| directory.join("waterui_assets"));
        let fonts = assets
            .parent()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "the asset root {} has no parent directory",
                        assets.display()
                    ),
                )
            })?
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
    fn an_asset_root_without_a_parent_is_an_error() {
        let error = ResourceContext::beside(Path::new("/app/bin"), Some(PathBuf::from("/")))
            .expect_err("the filesystem root has no directory to hold fonts");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn fonts_sit_beside_the_assets() {
        let context =
            ResourceContext::beside(Path::new("/app/bin"), Some(PathBuf::from("/data/assets")))
                .expect("a rooted asset directory has a parent");
        assert_eq!(context.fonts(), Path::new("/data/fonts"));
    }

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
