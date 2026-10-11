//! The launch screen: what a platform shows from the tap on the icon until the
//! application's first frame.
//!
//! No Rust runs while a launch screen is visible — the wasm is still
//! downloading, the dylib is still loading — so it is a build-time asset, never
//! a view. `[launch]` in `Water.toml` declares it once; the `water` CLI projects
//! it onto each platform's own mechanism, and the only runtime involvement is
//! the first presented frame that ends it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{AssetRole, BundleManifest, HexColor, ThemeConfig};

/// The `[launch]` section of `Water.toml`.
///
/// The launch artwork is not configured here: like the app icon, it is the
/// single `Launch.*` image at the root of the assets directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchConfig {
    /// Background behind the launch artwork. Defaults to the theme background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<HexColor>,
    /// Background used when the system is in dark appearance. Defaults to
    /// `background`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_dark: Option<HexColor>,
}

/// The light or dark appearance a launch screen is resolved for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    /// Light appearance.
    Light,
    /// Dark appearance.
    Dark,
}

/// A launch screen resolved for staging: the `[launch]` colors filled in from
/// `[theme]`, and the `Launch.*` artwork found at the asset root.
///
/// A `None` background means the platform's own default (the system
/// background on iOS, the window background on Android, the page default on
/// the web); a `None` image means the platform's own default artwork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    background: Option<HexColor>,
    background_dark: Option<HexColor>,
    image: Option<PathBuf>,
}

impl LaunchPlan {
    /// Resolves the launch screen from the manifest's `[launch]` and `[theme]`
    /// sections and the planned assets.
    ///
    /// The light background is `[launch] background`, then `[theme]
    /// background`; the dark background is `[launch] background_dark`, then
    /// the resolved light background.
    #[must_use]
    pub fn resolve(
        launch: Option<&LaunchConfig>,
        theme: Option<&ThemeConfig>,
        manifest: &BundleManifest,
    ) -> Self {
        let background = launch
            .and_then(|launch| launch.background)
            .or_else(|| theme.and_then(|theme| theme.background));
        let background_dark = launch
            .and_then(|launch| launch.background_dark)
            .or(background);
        let image = manifest
            .root_artwork(AssetRole::LaunchImage)
            .map(|asset| asset.source_path.clone());
        Self {
            background,
            background_dark,
            image,
        }
    }

    /// The background for one appearance, or `None` for the platform default.
    #[must_use]
    pub const fn background(&self, scheme: ColorScheme) -> Option<&HexColor> {
        match scheme {
            ColorScheme::Light => self.background.as_ref(),
            ColorScheme::Dark => self.background_dark.as_ref(),
        }
    }

    /// Whether the dark appearance shows a different background than the
    /// light one, which is when a platform needs a second resource.
    #[must_use]
    pub fn has_distinct_dark_background(&self) -> bool {
        self.background_dark != self.background
    }

    /// The `Launch.*` artwork, or `None` for the platform default.
    #[must_use]
    pub const fn image(&self) -> Option<&PathBuf> {
        self.image.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_mount;
    use std::fs;
    use tempfile::tempdir;

    fn color(text: &str) -> HexColor {
        text.parse().unwrap()
    }

    fn manifest_with(files: &[&str]) -> BundleManifest {
        let temp = tempdir().unwrap();
        let assets_root = temp.path().join("assets");
        fs::create_dir_all(&assets_root).unwrap();
        for file in files {
            fs::write(assets_root.join(file), b"png").unwrap();
        }
        // The tempdir is gone after this, which is fine: the plan only carries paths.
        BundleManifest {
            crate_root: temp.path().to_path_buf(),
            assets_root: assets_root.clone(),
            mounts: Vec::new(),
            assets: plan_mount(&assets_root, "").unwrap(),
        }
    }

    #[test]
    fn without_any_section_everything_is_the_platform_default() {
        let plan = LaunchPlan::resolve(None, None, &manifest_with(&[]));
        assert_eq!(plan.background(ColorScheme::Light), None);
        assert_eq!(plan.background(ColorScheme::Dark), None);
        assert_eq!(plan.image(), None);
        assert!(!plan.has_distinct_dark_background());
    }

    #[test]
    fn theme_background_fills_in_both_appearances() {
        let theme = ThemeConfig {
            background: Some(color("#101010")),
            ..ThemeConfig::default()
        };
        let plan = LaunchPlan::resolve(None, Some(&theme), &manifest_with(&[]));
        assert_eq!(plan.background(ColorScheme::Light), Some(&color("#101010")));
        assert_eq!(plan.background(ColorScheme::Dark), Some(&color("#101010")));
        assert!(!plan.has_distinct_dark_background());
    }

    #[test]
    fn launch_section_overrides_theme_and_dark_falls_back_to_light() {
        let theme = ThemeConfig {
            background: Some(color("#101010")),
            ..ThemeConfig::default()
        };
        let launch = LaunchConfig {
            background: Some(color("#0B1E3F")),
            background_dark: None,
        };
        let plan = LaunchPlan::resolve(Some(&launch), Some(&theme), &manifest_with(&[]));
        assert_eq!(plan.background(ColorScheme::Light), Some(&color("#0B1E3F")));
        assert_eq!(plan.background(ColorScheme::Dark), Some(&color("#0B1E3F")));

        let launch = LaunchConfig {
            background: None,
            background_dark: Some(color("#000000")),
        };
        let plan = LaunchPlan::resolve(Some(&launch), Some(&theme), &manifest_with(&[]));
        assert_eq!(plan.background(ColorScheme::Light), Some(&color("#101010")));
        assert_eq!(plan.background(ColorScheme::Dark), Some(&color("#000000")));
        assert!(plan.has_distinct_dark_background());
    }

    #[test]
    fn root_launch_artwork_is_the_image() {
        let plan = LaunchPlan::resolve(None, None, &manifest_with(&["Icon.png", "Launch.png"]));
        assert!(
            plan.image()
                .is_some_and(|path| path.ends_with("Launch.png"))
        );
    }
}
