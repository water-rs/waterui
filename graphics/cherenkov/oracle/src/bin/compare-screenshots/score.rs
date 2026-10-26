use std::{collections::BTreeMap, io, path::Path};

use cherenkov_oracle::{
    F32Image,
    metrics::{
        self, DEFAULT_PPD, FLIP_MAX_LIMIT, FLIP_MEAN_LIMIT, HDR_EXPOSURES, LOCAL_ERROR_LIMIT,
    },
};
use serde::{Deserialize, Serialize};

use super::Error;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub flip_mean: f64,
    pub flip_max: f64,
    pub max_local_error: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            flip_mean: FLIP_MEAN_LIMIT,
            flip_max: FLIP_MAX_LIMIT,
            max_local_error: LOCAL_ERROR_LIMIT,
        }
    }
}

impl Limits {
    fn validate(self) -> Result<Self, Error> {
        if !(0.0..=1.0).contains(&self.flip_mean) || !(0.0..=1.0).contains(&self.flip_max) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "FLIP limits must be finite values in 0..=1",
            )
            .into());
        }
        if !self.max_local_error.is_finite() || self.max_local_error < 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "local error limit must be finite and nonnegative",
            )
            .into());
        }
        Ok(self)
    }

    pub(super) fn accepts(self, score: &Score) -> bool {
        score.flip_mean <= self.flip_mean
            && score.flip_max <= self.flip_max
            && score.max_local_error <= self.max_local_error
    }
}

pub fn load_limits(
    path: &Path,
    platform: &str,
    major: u32,
    example: &str,
) -> Result<Limits, Error> {
    let table: BTreeMap<String, BTreeMap<String, Limits>> =
        serde_json::from_reader(std::fs::File::open(path)?)?;
    for entries in table.values() {
        for limits in entries.values() {
            limits.validate()?;
        }
    }
    let versioned = format!("{platform}@{major}");
    Ok(table
        .get(&versioned)
        .and_then(|entries| entries.get(example))
        .or_else(|| table.get(platform).and_then(|entries| entries.get(example)))
        .copied()
        .unwrap_or_default())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Metric {
    LdrFlip,
    HdrFlip,
}

#[derive(Debug, Serialize)]
pub struct Score {
    pub metric: Metric,
    pub flip_alpha_comparison: &'static str,
    pub flip_mean: f64,
    pub flip_max: f64,
    pub max_local_error: f64,
    pub pixels_per_degree: f64,
    pub hdr_flip_exposure_ev: Vec<f64>,
}

pub fn compare(reference: &F32Image, actual: &F32Image) -> Result<(Score, Vec<u8>), Error> {
    if (reference.width, reference.height) != (actual.width, actual.height) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "image size mismatch: reference {}x{}, actual {}x{}",
                reference.width, reference.height, actual.width, actual.height
            ),
        )
        .into());
    }
    let reference_rgb = metrics::to_linear_p3(reference);
    let actual_rgb = metrics::to_linear_p3(actual);
    // Both mattes are identical for opaque images, so do not repeat FLIP.
    let white = reference
        .pixels
        .iter()
        .chain(&actual.pixels)
        .any(|pixel| pixel[3] < 1.0)
        .then(|| (over_white(reference), over_white(actual)));
    let (reference_range, actual_range) = white
        .as_ref()
        .map_or((&reference_rgb, &actual_rgb), |(r, a)| (r, a));
    let hdr = metrics::is_hdr(reference_range) || metrics::is_hdr(actual_range);
    let width = reference.width as usize;
    let height = reference.height as usize;
    let evaluate = |reference: &[[f64; 3]], actual: &[[f64; 3]]| {
        if hdr {
            metrics::hdr_flip(reference, actual, width, height, DEFAULT_PPD)
        } else {
            metrics::ldr_flip(reference, actual, width, height, DEFAULT_PPD)
        }
    };
    let mut error = evaluate(&reference_rgb, &actual_rgb);
    if let Some((reference_white, actual_white)) = white {
        for (error, white_error) in error
            .iter_mut()
            .zip(evaluate(&reference_white, &actual_white))
        {
            *error = error.max(white_error);
        }
    }
    let score = Score {
        metric: if hdr {
            Metric::HdrFlip
        } else {
            Metric::LdrFlip
        },
        flip_alpha_comparison: "max-over-black-and-white",
        flip_mean: error.iter().sum::<f64>()
            / (f64::from(reference.width) * f64::from(reference.height)),
        flip_max: error.iter().copied().fold(0.0, f64::max),
        max_local_error: metrics::max_local_error(reference, actual),
        pixels_per_degree: DEFAULT_PPD,
        hdr_flip_exposure_ev: if hdr {
            HDR_EXPOSURES.to_vec()
        } else {
            Vec::new()
        },
    };
    let heatmap = metrics::heat_map(&error).into_iter().flatten().collect();
    Ok((score, heatmap))
}

fn over_white(image: &F32Image) -> Vec<[f64; 3]> {
    image
        .pixels
        .iter()
        .map(|pixel| {
            let background = 1.0 - f64::from(pixel[3]);
            [pixel[0], pixel[1], pixel[2]].map(|c| f64::from(c) + background)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cherenkov_oracle::{F32Image, metrics::LOCAL_ERROR_LIMIT};

    use super::{Limits, Metric, compare, load_limits};

    fn solid(value: f32) -> F32Image {
        F32Image {
            width: 32,
            height: 32,
            pixels: vec![[value, value, value, 1.0]; 32 * 32],
        }
    }

    #[test]
    fn uniform_images_are_valid_comparison_inputs() {
        let image = solid(0.25);
        let (mut score, _) = compare(&image, &image).unwrap();
        assert!(Limits::default().accepts(&score), "{score:?}");
        score.max_local_error = 2.0 * LOCAL_ERROR_LIMIT;
        assert!(!Limits::default().accepts(&score), "{score:?}");
    }

    #[test]
    fn transparent_black_is_not_opaque_black() {
        let reference = solid(0.0);
        let mut actual = reference.clone();
        actual.pixels.fill([0.0; 4]);
        let (score, _) = compare(&reference, &actual).unwrap();
        assert!(!Limits::default().accepts(&score), "{score:?}");
    }

    #[test]
    fn a_changed_bottom_band_is_not_a_platform_exemption() {
        let reference = solid(1.0);
        let mut actual = reference.clone();
        actual.pixels[24 * 32..].fill([0.0, 0.0, 0.0, 1.0]);
        let (score, _) = compare(&reference, &actual).unwrap();
        assert!(!Limits::default().accepts(&score), "{score:?}");
    }

    #[test]
    fn hdr_on_either_side_selects_hdr_flip() {
        let reference = solid(1.0);
        let actual = solid(4.0);
        let (score, _) = compare(&reference, &actual).unwrap();
        assert!(matches!(score.metric, Metric::HdrFlip));
        assert!(!Limits::default().accepts(&score), "{score:?}");
    }

    #[test]
    fn dimensions_are_an_input_error() {
        let reference = solid(0.25);
        let mut actual = solid(0.25);
        actual.width = 16;
        actual.pixels.truncate(16 * 32);
        assert!(compare(&reference, &actual).is_err());
    }

    #[test]
    fn typed_limits_resolve_version_then_platform_then_shared_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limits.json");
        let config = serde_json::json!({
            "ios": {"sample": {"flip_mean": 0.1, "flip_max": 0.2, "max_local_error": LOCAL_ERROR_LIMIT}},
            "ios@27": {"sample": {"flip_mean": 0.3, "flip_max": 0.4, "max_local_error": LOCAL_ERROR_LIMIT}}
        });
        serde_json::to_writer(std::fs::File::create(&path).unwrap(), &config).unwrap();
        let versioned = load_limits(&path, "ios", 27, "sample").unwrap();
        assert!((versioned.flip_mean - 0.3).abs() < 1e-12);
        let platform = load_limits(&path, "ios", 26, "sample").unwrap();
        assert!((platform.flip_max - 0.2).abs() < 1e-12);
        let default = load_limits(&path, "ios", 27, "other").unwrap();
        assert!((default.flip_mean - Limits::default().flip_mean).abs() < 1e-12);
    }

    #[test]
    fn old_pixel_fractions_are_not_reinterpreted_as_flip_limits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limits.json");
        let config = serde_json::json!({"ios": {"sample": 0.09}});
        serde_json::to_writer(std::fs::File::create(&path).unwrap(), &config).unwrap();
        assert!(load_limits(&path, "ios", 27, "sample").is_err());
    }

    #[test]
    fn an_invalid_unused_limit_is_not_silently_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limits.json");
        let config = serde_json::json!({
            "ios": {"other": {"flip_mean": -0.1, "flip_max": 0.2, "max_local_error": LOCAL_ERROR_LIMIT}}
        });
        serde_json::to_writer(std::fs::File::create(&path).unwrap(), &config).unwrap();
        assert!(load_limits(&path, "ios", 27, "sample").is_err());
        assert!(
            Limits {
                flip_mean: f64::NAN,
                flip_max: 0.1,
                ..Limits::default()
            }
            .validate()
            .is_err()
        );
    }
}
