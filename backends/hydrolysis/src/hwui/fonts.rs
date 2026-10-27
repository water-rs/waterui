//! What the target reads from a registered font's own tables: its ink
//! bounds, for glyph-run ink, and its variation axes, to turn a glyph run's
//! normalized coordinates into the user-space `FontVariationAxis` values a
//! derived platform `Font` is built with.
//!
//! The tables are read with `read-fonts`; only the inverse of the
//! normalization (fvar, then avar) is computed here, because neither
//! `read-fonts` nor `skrifa` offers it.

use read_fonts::{FontRef, TableProvider};

/// A registered font's metrics and axes.
#[derive(Clone, Debug, PartialEq)]
pub struct FontInfo {
    /// The union of every glyph's bounding box (`head`), in ems, y up:
    /// `[x_min, y_min, x_max, y_max]`.
    pub bounds: [f32; 4],
    axes: Box<[Axis]>,
}

#[derive(Clone, Debug, PartialEq)]
struct Axis {
    tag: u32,
    min: f32,
    default: f32,
    max: f32,
    /// The axis's `avar` segment map, `(from, to)` in normalized space;
    /// empty when the font has no `avar` entry for it.
    segments: Box<[(f32, f32)]>,
}

impl FontInfo {
    /// Reads face `index` of `data`.
    ///
    /// # Errors
    ///
    /// A description when the face or a table it needs does not parse, or
    /// when it maps axes through `avar` version 2, whose normalization
    /// has no inverse.
    pub fn read(data: &[u8], index: u32) -> Result<Self, String> {
        let font =
            FontRef::from_index(data, index).map_err(|error| format!("face {index}: {error}"))?;
        let head = font.head().map_err(|error| format!("head: {error}"))?;
        let em = f32::from(head.units_per_em());
        if em <= 0.0 {
            return Err("head: units per em is zero".to_owned());
        }
        let bounds = [
            f32::from(head.x_min()) / em,
            f32::from(head.y_min()) / em,
            f32::from(head.x_max()) / em,
            f32::from(head.y_max()) / em,
        ];
        let Ok(fvar) = font.fvar() else {
            return Ok(Self {
                bounds,
                axes: Box::default(),
            });
        };
        let records = fvar.axes().map_err(|error| format!("fvar: {error}"))?;
        let mut segments: Vec<Box<[(f32, f32)]>> = vec![Box::default(); records.len()];
        if let Ok(avar) = font.avar() {
            if avar.version().major > 1 {
                return Err("avar version 2 maps coordinates through a variation store, which has no inverse".to_owned());
            }
            for (slot, maps) in segments.iter_mut().zip(avar.axis_segment_maps().iter()) {
                let maps = maps.map_err(|error| format!("avar: {error}"))?;
                *slot = maps
                    .axis_value_maps()
                    .iter()
                    .map(|map| (map.from_coordinate().to_f32(), map.to_coordinate().to_f32()))
                    .collect();
            }
        }
        let axes = records
            .iter()
            .zip(segments)
            .map(|(record, segments)| {
                let min = record.min_value().to_f32();
                Axis {
                    tag: u32::from_be_bytes(record.axis_tag().to_be_bytes()),
                    min,
                    default: record.default_value().to_f32(),
                    max: record.max_value().to_f32().max(min),
                    segments,
                }
            })
            .collect();
        Ok(Self { bounds, axes })
    }

    /// A font with `bounds` and `axes` as `(tag, min, default, max)`, no
    /// `avar`.
    #[cfg(test)]
    pub fn synthetic(bounds: [f32; 4], axes: &[(u32, f32, f32, f32)]) -> Self {
        Self {
            bounds,
            axes: axes
                .iter()
                .map(|&(tag, min, default, max)| Axis {
                    tag,
                    min,
                    default,
                    max,
                    segments: Box::default(),
                })
                .collect(),
        }
    }

    /// Whether the font has variation axes.
    #[cfg(test)]
    #[must_use]
    pub const fn is_variable(&self) -> bool {
        !self.axes.is_empty()
    }

    /// The user-space `(tag, value)` of every axis at the normalized
    /// `F2Dot14` coordinates `coords`, in `fvar` order; an axis past the
    /// end of `coords` sits at its default.
    ///
    /// # Errors
    ///
    /// A description when `coords` names more axes than the font has.
    pub fn user_coordinates(
        &self,
        coords: &[i16],
        out: &mut Vec<(u32, f32)>,
    ) -> Result<(), String> {
        if coords.len() > self.axes.len() {
            return Err(format!(
                "{} variation coordinates for a font with {} axes",
                coords.len(),
                self.axes.len()
            ));
        }
        out.clear();
        for (index, axis) in self.axes.iter().enumerate() {
            let normalized = coords
                .get(index)
                .map_or(0.0, |&coord| f32::from(coord) / 16384.0);
            out.push((axis.tag, axis.user(normalized)));
        }
        Ok(())
    }
}

impl Axis {
    /// Inverts `avar` (a monotone piecewise-linear map, inverted by
    /// swapping its columns), then `fvar`'s default-anchored normalization.
    fn user(&self, normalized: f32) -> f32 {
        let unmapped = unmap(&self.segments, normalized.clamp(-1.0, 1.0));
        if unmapped < 0.0 {
            unmapped.mul_add(self.default - self.min, self.default)
        } else {
            unmapped.mul_add(self.max - self.default, self.default)
        }
    }
}

/// `value` through the inverse of the segment map `segments`.
fn unmap(segments: &[(f32, f32)], value: f32) -> f32 {
    let Some(&(first_from, first_to)) = segments.first() else {
        return value;
    };
    if value <= first_to {
        return first_from;
    }
    for pair in segments.windows(2) {
        let (from0, to0) = pair[0];
        let (from1, to1) = pair[1];
        if value <= to1 {
            if to1 <= to0 {
                return from1;
            }
            return (value - to0).mul_add((from1 - from0) / (to1 - to0), from0);
        }
    }
    segments.last().map_or(value, |&(from, _)| from)
}

#[cfg(test)]
mod tests {
    use read_fonts::types::Fixed;
    use read_fonts::{FontRef, TableProvider};

    use super::FontInfo;

    fn test_font(name: &str) -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/test-fonts/").to_owned() + name;
        std::fs::read(&path).unwrap_or_else(|error| {
            panic!("{path}: {error}; run `uv run backends/hydrolysis/test-fonts/install.py`")
        })
    }

    #[test]
    fn a_static_font_has_its_head_bounds_and_no_axes() {
        let data = test_font("Roboto-Regular.ttf");
        let info = FontInfo::read(&data, 0).unwrap();
        let font = FontRef::new(&data).unwrap();
        let head = font.head().unwrap();
        let em = f32::from(head.units_per_em());
        assert_eq!(info.bounds[1], f32::from(head.y_min()) / em);
        assert_eq!(info.bounds[3], f32::from(head.y_max()) / em);
        assert!(
            info.bounds[3] > 1.0 || info.bounds[1] < -0.25,
            "{:?}",
            info.bounds
        );
        assert!(!info.is_variable());
        let mut out = Vec::new();
        assert!(info.user_coordinates(&[1], &mut out).is_err());
    }

    /// Normalizing a user value with `read-fonts` (fvar, then avar) and
    /// denormalizing the result gives the value back.
    #[test]
    fn user_coordinates_invert_read_fonts_normalization() {
        let data = test_font("TestVariable-ABC.ttf");
        let info = FontInfo::read(&data, 0).unwrap();
        assert!(info.is_variable());
        let font = FontRef::new(&data).unwrap();
        let fvar = font.fvar().unwrap();
        let avar = font.avar().ok();
        let records = fvar.axes().unwrap();
        let mut out = Vec::new();
        for step in 0..=8 {
            let mut coords = Vec::new();
            let mut expected = Vec::new();
            for (index, record) in records.iter().enumerate() {
                let min = record.min_value().to_f64();
                let max = record.max_value().to_f64();
                let user = (max - min).mul_add(f64::from(step) / 8.0, min);
                let mut normalized = record.normalize(Fixed::from_f64(user));
                if let Some(maps) = avar
                    .as_ref()
                    .and_then(|avar| avar.axis_segment_maps().get(index))
                {
                    normalized = maps.unwrap().apply(normalized);
                }
                coords.push(normalized.to_f2dot14().to_bits());
                expected.push(user);
            }
            info.user_coordinates(&coords, &mut out).unwrap();
            for ((tag, value), (record, user)) in out.iter().zip(records.iter().zip(&expected)) {
                assert_eq!(*tag, u32::from_be_bytes(record.axis_tag().to_be_bytes()));
                let span = record.max_value().to_f64() - record.min_value().to_f64();
                assert!(
                    (f64::from(*value) - user).abs() <= span / 2000.0,
                    "axis {:?}: {value} for {user}",
                    record.axis_tag()
                );
            }
        }
    }
}
