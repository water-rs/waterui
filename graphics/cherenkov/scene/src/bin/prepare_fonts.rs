//! Subsets the full OFL fonts in `scenes/fonts/_full/` down to the corpus
//! texts, writes the results to `scenes/fonts/`, and copies the OFL licence
//! next to them. Run once before `generate-corpus`.

use std::process::ExitCode;

use cherenkov_scene::corpus::{self, FONTS, LICENSE_FILE};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .without_time()
        .init();

    let root = corpus::repo_root();
    let full = corpus::full_fonts_dir(&root);
    let fonts = corpus::fonts_dir(&root);
    if let Err(e) = std::fs::create_dir_all(&fonts) {
        tracing::error!(path = %fonts.display(), %e, "cannot create fonts dir");
        return ExitCode::FAILURE;
    }

    let mut failed = false;
    for spec in FONTS {
        let src = full.join(spec.full_file);
        let dst = fonts.join(spec.subset_file);
        let bytes = match std::fs::read(&src) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(path = %src.display(), %e, "cannot read font");
                failed = true;
                continue;
            }
        };
        match corpus::subset(&bytes) {
            Ok(out) => {
                if let Err(e) = std::fs::write(&dst, &out) {
                    tracing::error!(path = %dst.display(), %e, "cannot write subset");
                    failed = true;
                    continue;
                }
                tracing::info!(
                    file = spec.subset_file,
                    before = bytes.len(),
                    after = out.len(),
                    "subset written"
                );
            }
            Err(e) => {
                tracing::error!(file = spec.full_file, %e, "subsetting failed");
                failed = true;
            }
        }
    }

    let license_src = full.join(LICENSE_FILE);
    if let Err(e) = std::fs::copy(&license_src, fonts.join(LICENSE_FILE)) {
        tracing::error!(path = %license_src.display(), %e, "cannot copy licence");
        failed = true;
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
