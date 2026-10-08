//! Render the `water preview --platform android` host composite for a
//! project, without a device.
//!
//! ```text
//! cargo run --locked -p waterui-cli --no-default-features --example render_preview_host -- <project> <out>
//! ```
//!
//! `<project>` is a `WaterUI` project; `<out>` receives the composite Gradle
//! project. The library resolves the Hydrolysis Android host the project's
//! framework selects — for a project on a `waterui_path`, this checkout's
//! own `backends/hydrolysis/android` — exactly as `water preview` does,
//! along with the generated app's `minSdk` and JDK level. It is the same
//! `render_preview_host` preparation `water preview` renders the managed
//! host with; CI assembles the output's `:app:assembleDebug` to check the
//! generated app's Kotlin and Gradle wiring.

use std::path::PathBuf;

use eyre::{Context as _, bail};
use waterui_cli::hydrolysis::android::render_preview_host;
use waterui_cli::project::{ManagedBackends, Project};
use waterui_cli::toolchain::Host;

/// The `versionCode` the rendered app carries: CI assembles it and never
/// installs it, so no device compares it against an installed host.
const VERSION_CODE: u32 = 1;

fn main() -> eyre::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let (Some(project_dir), Some(out), None) = (args.next(), args.next(), args.next()) else {
        bail!("usage: render_preview_host <project> <out>");
    };
    let project_dir = PathBuf::from(project_dir);
    let out = PathBuf::from(out);
    let host = Host::current();

    smol::block_on(async {
        let (out, project) = futures_util::try_join!(
            async {
                smol::fs::create_dir_all(&out)
                    .await
                    .wrap_err_with(|| format!("failed to create {}", out.display()))?;
                smol::fs::canonicalize(&out)
                    .await
                    .wrap_err_with(|| format!("failed to canonicalize {}", out.display()))
            },
            async {
                let project_dir =
                    smol::fs::canonicalize(&project_dir)
                        .await
                        .wrap_err_with(|| {
                            format!("failed to canonicalize {}", project_dir.display())
                        })?;
                Project::open(&host, &project_dir, ManagedBackends::NONE)
                    .await
                    .wrap_err_with(|| {
                        format!("failed to open the project {}", project_dir.display())
                    })
            },
        )?;
        render_preview_host(&project, &host, &out, VERSION_CODE).await
    })
}
