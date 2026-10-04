//! Retained layer pixels and measured admission inputs.

use cherenkov::{Command, DisplayList, GlyphStyle, RenderError};
use kurbo::{Affine, Rect, Shape};
use rustc_hash::FxHashMap;
use skrifa::MetadataProvider;

use crate::render::{
    bitmap, glyph,
    prepared::{Op, Outline, ResolvedPaint},
};

/// A local source domain. Its integer origin and dimensions include the
/// rasterizer's antialiasing footprint; native placement removes that offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Domain {
    pub origin: kurbo::Vec2,
    pub size: (u32, u32),
    pub density: f64,
}

impl Domain {
    pub fn raster(self) -> Affine {
        Affine::translate(self.origin) * Affine::scale(self.density.recip())
    }
}

/// Engine-side capture. A native realization retains its own immutable
/// presentation buffer until content changes or promotion ends.
pub struct Capture {
    pub domain: Domain,
    /// Released after native publication; the capture generation remains.
    pub source: Option<(wgpu::Texture, wgpu::TextureView)>,
    pub generation: u64,
    pub dirty: bool,
}

/// The longest readmission wait. A recent period raises the wait up to this
/// many frames; a longer quiet interval is a finished lifetime and resets
/// the wait to the initial two frames instead of becoming the next one.
const QUIET_LIMIT: u64 = 8;

/// Content observations are separate from pixel allocations: ineligible
/// candidates consume no plane memory.
pub struct Observation {
    pub stamp: u64,
    pub resources: (u64, u64),
    pub domain: Option<Domain>,
    pub density: f64,
    pub quiet_frames: u64,
    /// Admission must outlast a recent content period, and is at most eight frames.
    pub quiet_required: u64,
    pub capture: Option<Capture>,
}

impl Capture {
    pub fn bytes(&self) -> u64 {
        self.source.as_ref().map_or(0, |_| {
            u64::from(self.domain.size.0)
                * u64::from(self.domain.size.1)
                * crate::render::texel_bytes(crate::render::TARGET_FORMAT)
        })
    }
}

impl Observation {
    pub const fn new(stamp: u64, resources: (u64, u64)) -> Self {
        Self {
            stamp,
            resources,
            domain: None,
            density: 0.0,
            quiet_frames: 0,
            quiet_required: 2,
            capture: None,
        }
    }

    /// Invalidate pixels while retaining the observed content lifetime.
    pub fn change(&mut self, stamp: u64, resources: (u64, u64)) {
        // A lifetime longer than two admission intervals is evidence that
        // the previous churn ended. Forget that backoff; otherwise track
        // the recent period rather than taking a maximum over all history.
        self.quiet_required = if self.quiet_frames > self.quiet_required.saturating_mul(2) {
            2
        } else {
            self.quiet_frames.saturating_add(1).clamp(2, QUIET_LIMIT)
        };
        self.stamp = stamp;
        self.resources = resources;
        self.quiet_frames = 0;
        self.domain = None;
        self.capture = None;
    }

    /// True only on entry to a new stable interval: bounds are computed once.
    pub fn observe(&mut self, stamp: u64, resources: (u64, u64), density: f64) -> bool {
        let content_changed = self.stamp != stamp || self.resources != resources;
        if !content_changed && self.density.to_bits() == density.to_bits() {
            self.quiet_frames = self.quiet_frames.saturating_add(1);
        } else {
            if content_changed {
                self.change(stamp, resources);
            }
            self.stamp = stamp;
            self.resources = resources;
            self.quiet_frames = 1;
            self.capture = None;
            self.density = density;
            self.domain = None;
        }
        self.quiet_frames == self.quiet_required
    }
}

fn stroke_margin(stroke: &kurbo::Stroke) -> f64 {
    stroke.width.abs() * 0.5 * stroke.miter_limit.max(1.0)
}

fn union(bounds: &mut Option<Rect>, rect: Rect) {
    if !rect.is_zero_area() {
        *bounds = Some(bounds.map_or(rect, |bounds| bounds.union(rect)));
    }
}

fn glyph_bounds(
    run: &cherenkov::GlyphRun,
    fonts: &FxHashMap<u64, glyph::FontData>,
) -> Result<Rect, RenderError> {
    let data = fonts
        .get(&run.font.raw())
        .ok_or_else(|| RenderError::Font(format!("unregistered font {}", run.font.raw())))?;
    let font = skrifa::FontRef::from_index(&data.data, data.index)
        .map_err(|error| RenderError::Font(error.to_string()))?;
    let units = f64::from(
        font.metrics(
            skrifa::instance::Size::unscaled(),
            skrifa::instance::LocationRef::default(),
        )
        .units_per_em,
    );
    let scale = f64::from(run.size) / units;
    let coords: Vec<_> = run
        .coords
        .iter()
        .map(|value| skrifa::raw::types::F2Dot14::from_bits(*value))
        .collect();
    let outlines = font.outline_glyphs();
    let mut bounds = None;
    for item in run.glyphs.iter() {
        let transform = Affine::translate((f64::from(item.x), f64::from(item.y)))
            * glyph::checked_transform(item)?;
        if let Some(bitmap_font) = &data.bitmap {
            if let Some(decoded) = bitmap::decode(
                &data.data,
                data.index,
                bitmap_font,
                bitmap_font.select(run.size),
                item.id,
            )? {
                union(
                    &mut bounds,
                    transform.transform_rect_bbox(
                        Affine::scale(f64::from(run.size)).transform_rect_bbox(decoded.em),
                    ),
                );
            }
        } else if let Some(outline) = glyph::outline(&outlines, &coords, item.id)? {
            let mut rect = Affine::scale_non_uniform(scale, -scale)
                .transform_rect_bbox(outline.bounding_box());
            if let GlyphStyle::Stroke(stroke) = &run.style {
                rect = rect.inflate(stroke_margin(stroke), stroke_margin(stroke));
            }
            union(&mut bounds, transform.transform_rect_bbox(rect));
        }
    }
    Ok(bounds.unwrap_or(Rect::ZERO))
}

/// Conservative finite bounds from the actual retained display list. A
/// filter or shader can change without a source edit and is not static.
fn path_bounds(outline: &Outline, list: &DisplayList) -> Rect {
    match outline {
        Outline::Fill { elements, .. } => {
            kurbo::BezPath::from_vec(elements.to_vec()).bounding_box()
        }
        Outline::Stroke { shape, stroke } => shape
            .bounds()
            .inflate(stroke_margin(stroke), stroke_margin(stroke)),
        Outline::Source { command, .. } => match &list.commands()[*command] {
            Command::Fill { shape, .. } => shape.bounds(),
            Command::Stroke { shape, stroke, .. } => shape
                .bounds()
                .inflate(stroke_margin(stroke), stroke_margin(stroke)),
            _ => unreachable!("prepared path source"),
        },
    }
}

fn bounds(
    ops: &[Op],
    list: &DisplayList,
    fonts: &FxHashMap<u64, glyph::FontData>,
) -> Result<Option<Rect>, RenderError> {
    let mut result = None;
    let mut margins = Vec::new();
    for op in ops {
        let (local, rect) = match op {
            Op::Shaped {
                local,
                bounds,
                extra_margin,
                paint,
                ..
            } => {
                if matches!(paint, ResolvedPaint::Shader(_)) {
                    return Ok(None);
                }
                (*local, bounds.inflate(*extra_margin, *extra_margin))
            }
            Op::Shadow {
                local,
                bounds,
                sigma_eff,
                ..
            } => {
                let margin = sigma_eff.mul_add(3.0, 1.0);
                (*local, bounds.inflate(margin, margin))
            }
            Op::Path {
                local,
                outline,
                paint,
                ..
            } => {
                if matches!(paint, ResolvedPaint::Shader(_)) {
                    return Ok(None);
                }
                (*local, path_bounds(outline, list))
            }
            Op::Glyphs { local, run, paint } => {
                if matches!(paint, ResolvedPaint::Shader(_)) {
                    return Ok(None);
                }
                (*local, glyph_bounds(run.get(list), fonts)?)
            }
            Op::BitmapGlyph {
                local,
                font,
                glyph,
                origin,
                size,
            } => {
                let font = &fonts[font];
                let bitmap = font.bitmap.as_ref().expect("prepared bitmap font");
                let Some(decoded) =
                    bitmap::decode(&font.data, font.index, bitmap, bitmap.select(*size), *glyph)?
                else {
                    continue;
                };
                (
                    *local
                        * Affine::translate((f64::from(origin[0]), f64::from(origin[1])))
                        * Affine::scale(f64::from(*size)),
                    decoded.em,
                )
            }
            Op::BeginShadow { parameters, .. } => {
                let margin = 6.0_f64.mul_add(parameters.sigma, parameters.spread.max(0.0));
                let [a, b, c, d, _, _] = parameters.transform.as_coeffs();
                margins.push(kurbo::Vec2::new(a.hypot(c) * margin, b.hypot(d) * margin));
                continue;
            }
            Op::BeginClip { .. } => {
                margins.push(kurbo::Vec2::ZERO);
                continue;
            }
            Op::BeginIsolate { filter, .. } => {
                if filter.is_some() {
                    return Ok(None);
                }
                margins.push(kurbo::Vec2::ZERO);
                continue;
            }
            Op::End => {
                margins.pop().expect("prepared scopes pair");
                continue;
            }
        };
        if rect.is_zero_area() {
            continue;
        }
        let margin: kurbo::Vec2 = margins.iter().copied().sum();
        union(
            &mut result,
            local.transform_rect_bbox(rect).inflate(margin.x, margin.y),
        );
    }
    Ok(result)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "finite positive extents bounded by the device limit"
)]
pub fn domain(
    ops: &[Op],
    list: &DisplayList,
    fonts: &FxHashMap<u64, glyph::FontData>,
    density: f64,
    max: u32,
) -> Result<Option<Domain>, RenderError> {
    if !density.is_finite() || density <= 0.0 {
        return Ok(None);
    }
    let Some(bounds) = bounds(ops, list, fonts)? else {
        return Ok(None);
    };
    if bounds.is_zero_area() {
        return Ok(None);
    }
    let bounds = Affine::scale(density)
        .transform_rect_bbox(bounds)
        .inflate(1.0, 1.0)
        .expand();
    if !bounds.is_finite() || bounds.width() > f64::from(max) || bounds.height() > f64::from(max) {
        return Ok(None);
    }
    let origin = bounds.origin().to_vec2() / density;
    if !origin.is_finite() {
        return Ok(None);
    }
    Ok(Some(Domain {
        origin,
        density,
        size: (bounds.width() as u32, bounds.height() as u32),
    }))
}

#[cfg(test)]
mod tests {
    use super::Observation;

    #[test]
    fn quiet_required_stays_bounded_after_a_long_interval() {
        let mut entry = Observation::new(1, (0, 0));
        let mut stamp = 1u64;
        for _ in 0..48 {
            let ceiling = entry.quiet_required.saturating_mul(2);
            while entry.quiet_frames < ceiling {
                entry.observe(stamp, (0, 0), 1.);
            }
            stamp += 1;
            entry.observe(stamp, (0, 0), 1.);
            assert!(entry.quiet_required <= super::QUIET_LIMIT);
        }
        assert_eq!(entry.quiet_required, super::QUIET_LIMIT);
        for _ in 0..10_000 {
            entry.observe(stamp, (0, 0), 1.);
        }
        stamp += 1;
        entry.observe(stamp, (0, 0), 1.);
        assert_eq!(entry.quiet_required, 2);
    }

    #[test]
    fn long_static_lifetime_does_not_delay_readmission() {
        for resources_changed in [false, true] {
            let mut entry = Observation::new(1, (0, 0));
            for _ in 0..1000 {
                entry.observe(1, (0, 0), 1.);
            }
            let (stamp, resources) = if resources_changed {
                (1, (1, 0))
            } else {
                (2, (0, 0))
            };
            assert!(!entry.observe(stamp, resources, 1.));
            assert!(entry.observe(stamp, resources, 1.));
        }
    }

    #[test]
    fn capture_domain_rejects_invalid_density_and_origin() {
        let picture = cherenkov::Picture::record(|_| {});
        let ops = [super::Op::Shadow {
            local: kurbo::Affine::IDENTITY,
            ambient: kurbo::Affine::IDENTITY,
            shape: crate::render::instance::Shape::rect([16., 16.]),
            bounds: kurbo::Rect::new(0., 0., 32., 32.),
            sigma_eff: 0.,
            color: [1.; 4],
        }];
        for density in [0., -1., f64::NAN, f64::INFINITY, f64::MIN_POSITIVE / 16.] {
            assert_eq!(
                super::domain(
                    &ops,
                    picture.display_list(),
                    &rustc_hash::FxHashMap::default(),
                    density,
                    4096
                )
                .unwrap(),
                None,
                "density {density}"
            );
        }
        let domain = super::domain(
            &ops,
            picture.display_list(),
            &rustc_hash::FxHashMap::default(),
            1.25,
            4096,
        )
        .unwrap()
        .unwrap();
        assert_eq!(domain.density, 1.25);
        assert!(domain.raster().is_finite());
    }

    #[test]
    fn periodic_content_must_outlast_its_previous_lifetime() {
        let mut entry = Observation::new(1, (0, 0));
        assert!(!entry.observe(1, (0, 0), 1.25));
        assert!(entry.observe(1, (0, 0), 1.25));
        assert!(!entry.observe(1, (0, 0), 1.25));
        for stamp in 2..5 {
            for _ in 0..3 {
                assert!(!entry.observe(stamp, (0, 0), 1.25));
            }
        }
        assert!(entry.observe(4, (0, 0), 1.25));
        assert!(!entry.observe(4, (0, 0), 1.25));
        assert_eq!(entry.density, 1.25);
    }
}
