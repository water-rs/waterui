//! Render the `water preview --platform android` host composite against this
//! checkout's own Hydrolysis Android host, without a device.
//!
//! ```text
//! cargo run --locked -p waterui-cli --example render_preview_host -- <project> <out>
//! ```
//!
//! `<project>` is a `WaterUI` project whose framework resolution supplies the
//! generated app's `minSdk` and JDK level; `<out>` receives the composite
//! Gradle project, whose `:app:assembleDebug` then compiles the generated
//! app against `backends/hydrolysis/android`. It is the same
//! `render_preview_host` call `water preview` makes for the managed host —
//! CI assembles its output to check the generated app's Kotlin and Gradle
//! wiring.

use std::path::{Path, PathBuf};

use eyre::{Context as _, bail};
use waterui_cli::hydrolysis::android::render_preview_host;
use waterui_cli::project::{ManagedBackends, Project};

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

    smol::block_on(async {
        let (host_project_dir, out, project) = futures_util::try_join!(
            async {
                smol::fs::canonicalize(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../backends/hydrolysis/android"),
                )
                .await
                .wrap_err("the in-tree hydrolysis android host must exist")
            },
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
                Project::open(&project_dir, ManagedBackends::NONE)
                    .await
                    .wrap_err_with(|| {
                        format!("failed to open the project {}", project_dir.display())
                    })
            },
        )?;
        render_preview_host(&project, &host_project_dir, &out, VERSION_CODE).await
    })
}
