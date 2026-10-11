//! `external-cost` correctness (#168): the external-frame path and the
//! copy-and-convert path must agree on the composited output. Renders
//! one 1080p SDR frame through each and asserts the oracle pair
//! tolerance (`flip_mean <= 0.05`, `max_local_error <= 0.25`) — the same
//! check a scene render vs the oracle passes.

#![cfg(all(feature = "cherenkov", target_vendor = "apple"))]

use cherenkov_bench::cli::{ExternalPath, ExternalSize, ExternalTransfer};
use cherenkov_bench::external_cost;
use cherenkov_oracle::F32Image;

const fn pixels(image: Vec<[f32; 4]>) -> F32Image {
    F32Image {
        width: 1920,
        height: 1080,
        pixels: image,
    }
}

fn assert_paths_match(transfer: ExternalTransfer) {
    let external =
        external_cost::composite_frame(ExternalPath::External, ExternalSize::P1080, transfer, 0)
            .expect("path e composites one frame");
    let copied =
        external_cost::composite_frame(ExternalPath::Copy, ExternalSize::P1080, transfer, 0)
            .expect("path c composites one frame");
    let (metrics, _map) = cherenkov_oracle::metrics::compare(&pixels(external), &pixels(copied));
    assert!(
        metrics.flip_mean <= 0.05 && metrics.max_local_error <= 0.25,
        "path e vs path c ({transfer:?}): flip_mean {} max_local_error {}",
        metrics.flip_mean,
        metrics.max_local_error
    );
}

#[test]
fn external_matches_copy_convert() {
    assert_paths_match(ExternalTransfer::Sdr);
}

#[test]
fn external_matches_copy_convert_pq() {
    assert_paths_match(ExternalTransfer::Pq);
}
