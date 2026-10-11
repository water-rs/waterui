//! `projective-quality`: the #84 quality reference.
//!
//! The conformance reference renders a projective layer with the specified
//! model: the conservative density, the mip chain and the 16-tap
//! anisotropic filter. Agreeing with it shows a backend implements the
//! model; it cannot show the model is sharp enough. This command renders
//! the scene twice more in the oracle with successively finer local
//! sources and denser footprint integration
//! ([`Reconstruction::Supersampled`]). It reports how far the two refined
//! renders still differ, which is the convergence, and how far the model
//! is from the finest one, which is the model's quality error.

use std::fmt::Write as _;
use std::path::Path;

use cherenkov_oracle::projective::Reconstruction;
use cherenkov_oracle::{Renderer, metrics};
use cherenkov_scene::Scene;

use crate::BenchError;

/// The two refinement steps: density multiple and samples per pixel axis.
const STEPS: [(u32, u32); 2] = [(2, 4), (4, 8)];

pub(crate) fn run(scenes: &[std::path::PathBuf], out: Option<&Path>) -> Result<(), BenchError> {
    let mut report = String::new();
    let _ = writeln!(
        report,
        "{:<32} {:>12} {:>12} {:>12} {:>12}",
        "scene", "model mean", "model max", "conv mean", "conv max"
    );
    for dir in scenes {
        let scene = Scene::load(dir)?;
        let (w, h) = (scene.width as usize, scene.height as usize);
        let render = |reconstruction| {
            Renderer::new(w, h)
                .with_reconstruction(reconstruction)
                .render(&scene, dir)
        };
        let model = render(Reconstruction::Model)?;
        let [coarse, fine] =
            STEPS.map(|(refine, grid)| render(Reconstruction::Supersampled { refine, grid }));
        let (coarse, fine) = (coarse?, fine?);
        let (quality, _) = metrics::compare(&fine, &model);
        let (convergence, _) = metrics::compare(&fine, &coarse);
        let _ = writeln!(
            report,
            "{:<32} {:>12.6} {:>12.6} {:>12.6} {:>12.6}",
            dir.file_name().unwrap_or_default().to_string_lossy(),
            quality.flip_mean,
            quality.flip_max,
            convergence.flip_mean,
            convergence.flip_max
        );
    }
    match out {
        Some(path) => {
            std::fs::write(path, &report)
                .map_err(|e| BenchError::Engine(format!("write {}: {e}", path.display())))?;
            tracing::info!(out = %path.display(), "projective-quality");
        }
        None => print!("{report}"),
    }
    Ok(())
}
