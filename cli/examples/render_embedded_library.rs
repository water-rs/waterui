//! Render the embedded Android library composite for a project, without
//! compiling Rust.
//!
//! ```text
//! cargo run --locked -p waterui-cli --no-default-features --example render_embedded_library -- <project> <out>
//! ```
//!
//! `<project>` is a `WaterUI` project; `<out>` receives the generated Gradle
//! library project `water build` assembles for an embedded project, rendered
//! through the same `render_library` preparation over the Hydrolysis Android
//! host the project's framework selects — for a project on a `waterui_path`,
//! this checkout's own `backends/hydrolysis/android`. The rendered library is
//! written to stdout as JSON: its coordinates, the host version the Gradle
//! tasks publish under, and the tasks themselves. CI runs those tasks and
//! compiles a consumer app against the published coordinates.

use std::io::Write as _;
use std::path::PathBuf;

use eyre::{Context as _, bail};
use waterui_cli::hydrolysis::android::{embedded::render_library, resolve_painter};
use waterui_cli::project::{ManagedBackends, Project};
use waterui_cli::toolchain::Host;

fn main() -> eyre::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let (Some(project_dir), Some(out), None) = (args.next(), args.next(), args.next()) else {
        bail!("usage: render_embedded_library <project> <out>");
    };
    let project_dir = PathBuf::from(project_dir);
    let out = PathBuf::from(out);
    let host = Host::current();

    let library = smol::block_on(async {
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
        render_library(&project, resolve_painter(&project, None), &out).await
    })?;

    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &library).wrap_err("failed to write the library JSON")?;
    stdout
        .write_all(b"\n")
        .wrap_err("failed to write the library JSON")?;
    Ok(())
}
