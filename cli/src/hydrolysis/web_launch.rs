//! The launch screen of a Hydrolysis web page.
//!
//! The browser owns nothing before the wasm arrives, so the page itself is the
//! launch screen: `index.html` carries the background, the artwork and a
//! progress bar inline, painted before the first fetch starts, and
//! `bootstrap.js` takes it down when the Hydrolysis runner reports the first
//! presented frame.

use std::path::Path;

use askama::Template;
use base64::Engine as _;
use eyre::Context as _;
use smol::fs;
use waterui_assets_planner::{ColorScheme, HexColor};

use crate::{assets, project::Project, templates::TemplateContext};

// These live beside, not inside, the scaffolded backend templates: a backend
// project never contains them, and `index.html` needs the built bundle.
const BOOTSTRAP_TEMPLATE: &str = include_str!("../templates/hydrolysis_web/bootstrap.js.tpl");
const STYLE_TEMPLATE: &str = include_str!("../templates/hydrolysis_web/style.css.tpl");

/// Pixel size the launch artwork is rasterized at: three times its 128 CSS
/// pixel box, so it stays sharp on 3x displays.
const ARTWORK_PIXELS: u32 = 384;

/// The wasm bundle wasm-pack emits under `pkg/`, whose size the page is told
/// so its progress bar is determinate however the file is transferred.
const WASM_BUNDLE: &str = "pkg/app_bg.wasm";

#[derive(Template)]
#[template(path = "src/templates/hydrolysis_web/index.html.tpl")]
struct IndexTemplate<'a> {
    ctx: &'a TemplateContext,
    launch: WebLaunch,
}

/// The launch colors for one appearance, as CSS values.
struct WebLaunchScheme {
    background: String,
    foreground: String,
}

impl WebLaunchScheme {
    /// A configured background with a foreground that reads on it, or the
    /// browser's own `Canvas` / `CanvasText` pair, which follows the system
    /// appearance by itself.
    fn for_background(background: Option<&HexColor>) -> Self {
        background.map_or_else(
            || Self {
                background: "Canvas".to_string(),
                foreground: "CanvasText".to_string(),
            },
            |background| Self {
                background: background.to_string(),
                foreground: if background.relative_luminance() > 0.5 {
                    "#000000"
                } else {
                    "#FFFFFF"
                }
                .to_string(),
            },
        )
    }
}

/// Everything the page needs to paint its launch screen before fetching
/// anything.
struct WebLaunch {
    light: WebLaunchScheme,
    /// Present only when the dark appearance shows a different background;
    /// the `Canvas` default already follows the appearance on its own.
    dark: Option<WebLaunchScheme>,
    wasm_bytes: u64,
    artwork_data_uri: String,
}

impl WebLaunch {
    fn resolve(project: &Project, wasm_bytes: u64) -> eyre::Result<Self> {
        let launch = assets::project_launch_assets(project)?;
        let plan = launch.plan();
        let light = WebLaunchScheme::for_background(plan.background(ColorScheme::Light));
        let dark = plan
            .has_distinct_dark_background()
            .then(|| WebLaunchScheme::for_background(plan.background(ColorScheme::Dark)));
        let png = launch.artwork_or_app_icon_png(ARTWORK_PIXELS)?;
        let artwork_data_uri = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        Ok(Self {
            light,
            dark,
            wasm_bytes,
            artwork_data_uri,
        })
    }
}

/// Writes `index.html`, `bootstrap.js` and `style.css` into a site whose
/// `pkg/` bundle has already been built.
///
/// # Errors
///
/// Fails when the bundle is missing, the launch assets cannot be resolved, or
/// a file cannot be rendered or written.
pub(super) async fn write_web_shell(project: &Project, site_root: &Path) -> eyre::Result<()> {
    let wasm_path = site_root.join(WASM_BUNDLE);
    let wasm_bytes = fs::metadata(&wasm_path)
        .await
        .wrap_err_with(|| {
            format!(
                "Hydrolysis web bundle missing at {}; the shell is written after wasm-pack",
                wasm_path.display()
            )
        })?
        .len();

    let app_name = project
        .manifest()
        .package
        .name
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect::<String>();
    let ctx = TemplateContext::for_project_manifest(
        project.manifest(),
        project.crate_name().clone(),
        app_name,
        &project.resolved_framework().await?,
    );
    let launch = WebLaunch::resolve(project, wasm_bytes)?;
    let index = IndexTemplate { ctx: &ctx, launch }
        .render()
        .map_err(|error| eyre::eyre!("Failed to render hydrolysis index template: {error}"))?;

    fs::write(site_root.join("index.html"), index).await?;
    fs::write(site_root.join("bootstrap.js"), BOOTSTRAP_TEMPLATE).await?;
    fs::write(site_root.join("style.css"), STYLE_TEMPLATE).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_types::{BundleIdentifier, CrateName};

    fn hex(text: &str) -> HexColor {
        text.parse().unwrap()
    }

    #[test]
    fn unconfigured_background_uses_the_system_canvas_pair() {
        let scheme = WebLaunchScheme::for_background(None);
        assert_eq!(scheme.background, "Canvas");
        assert_eq!(scheme.foreground, "CanvasText");
    }

    #[test]
    fn foreground_contrasts_with_the_configured_background() {
        let dark = WebLaunchScheme::for_background(Some(&hex("#0B1E3F")));
        assert_eq!(
            (dark.background.as_str(), dark.foreground.as_str()),
            ("#0B1E3F", "#FFFFFF")
        );
        let light = WebLaunchScheme::for_background(Some(&hex("#F5F5F7")));
        assert_eq!(
            (light.background.as_str(), light.foreground.as_str()),
            ("#F5F5F7", "#000000")
        );
    }

    #[test]
    fn index_inlines_the_launch_screen_and_the_dark_override() {
        let manifest = crate::project::Manifest::new(crate::project::Package {
            package_type: crate::project::PackageType::Playground,
            name: "Demo <App>".to_string(),
            bundle_identifier: BundleIdentifier::try_from("dev.waterui.demo").unwrap(),
            assets_path: "assets".to_string(),
            accessory: false,
        });
        let ctx = TemplateContext::for_project_manifest(
            &manifest,
            CrateName::try_from("demo").unwrap(),
            "Demo",
            &crate::framework::test_fixtures::stable_framework(),
        );
        let html = IndexTemplate {
            ctx: &ctx,
            launch: WebLaunch {
                light: WebLaunchScheme::for_background(Some(&hex("#F5F5F7"))),
                dark: Some(WebLaunchScheme::for_background(Some(&hex("#000000")))),
                wasm_bytes: 4_242,
                artwork_data_uri: "data:image/png;base64,AAAA".to_string(),
            },
        }
        .render()
        .unwrap();

        assert!(html.contains("--waterui-launch-background: #F5F5F7;"));
        assert!(html.contains("@media (prefers-color-scheme: dark)"));
        assert!(html.contains("--waterui-launch-background: #000000;"));
        assert!(html.contains("data-wasm-bytes=\"4242\""));
        assert!(html.contains("src=\"data:image/png;base64,AAAA\""));
        assert!(html.contains("<title>Demo &#60;App&#62;</title>"), "{html}");
        assert!(html.contains("id=\"waterui-launch\""));
        assert!(html.contains("<canvas id=\"waterui-canvas\">"));
    }

    #[test]
    fn index_omits_the_dark_block_when_nothing_differs() {
        let manifest = crate::project::Manifest::new(crate::project::Package {
            package_type: crate::project::PackageType::Playground,
            name: "Demo".to_string(),
            bundle_identifier: BundleIdentifier::try_from("dev.waterui.demo").unwrap(),
            assets_path: "assets".to_string(),
            accessory: false,
        });
        let ctx = TemplateContext::for_project_manifest(
            &manifest,
            CrateName::try_from("demo").unwrap(),
            "Demo",
            &crate::framework::test_fixtures::stable_framework(),
        );
        let html = IndexTemplate {
            ctx: &ctx,
            launch: WebLaunch {
                light: WebLaunchScheme::for_background(None),
                dark: None,
                wasm_bytes: 0,
                artwork_data_uri: String::new(),
            },
        }
        .render()
        .unwrap();
        assert!(html.contains("--waterui-launch-background: Canvas;"));
        assert!(!html.contains("prefers-color-scheme: dark)"));
    }
}
