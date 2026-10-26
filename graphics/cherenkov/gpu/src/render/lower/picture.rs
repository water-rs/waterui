//! Picture-local raster retention, using the shared local-image cache.
use super::{
    Affine, Emission, EmissionStorage, GlyphContext, LayerId, LocalKey, Lowering, Op, Placement,
    Range, Realization, Rect, RenderError, Target,
};
use crate::render::projective::{Key, PictureImage, Realize};
use cherenkov::lowering::projective::{Homography, LocalImage, mip_levels};

/// Picture bounds survive texture eviction and admission retries. Keeping the
/// immutable source alive makes allocation identity safe across replacements.
pub(super) struct PreparedDomain {
    source: cherenkov::Picture,
    bucket: i32,
    format: wgpu::TextureFormat,
    visible: bool,
    domain: Option<crate::render::planes::static_layer::Domain>,
}

impl PreparedDomain {
    pub(super) fn matches_source(&self, source: &cherenkov::Picture) -> bool {
        std::ptr::eq(self.source.display_list(), source.display_list())
    }

    pub(super) fn source_bytes(&self) -> u64 {
        self.source
            .display_list()
            .heap_bytes(cherenkov::lowering::memory::command_bytes)
    }
}

impl<'a> super::PictureInputs<'a> {
    pub fn new(
        retained: &'a super::FxHashMap<LayerId, Vec<crate::render::projective::Entry>>,
        inputs: &'a crate::render::projective::Inputs,
        available: u64,
        layers: &super::FxHashMap<LayerId, super::ContentData>,
    ) -> Self {
        let mut candidates = Vec::new();
        for (&layer, content) in layers {
            let Some((ops, _)) = content.retained.current() else {
                continue;
            };
            for (&index, prepared) in &content.storage.picture_domains {
                if !prepared.visible {
                    continue;
                }
                if !matches!(ops.get(index), Some(Op::BeginPicture { picture, .. }) if prepared.matches_source(picture))
                {
                    continue;
                }
                let Some(domain) = prepared.domain else {
                    continue;
                };
                let key = LocalKey {
                    layer,
                    picture: Some(std::ptr::from_ref(prepared.source.display_list()) as usize),
                    bucket: prepared.bucket,
                    format: prepared.format,
                };
                let mut size = domain.size;
                let mut bytes = 0;
                loop {
                    bytes += crate::render::texel_bytes(key.format)
                        * u64::from(size.0)
                        * u64::from(size.1);
                    if size == (1, 1) {
                        break;
                    }
                    size = ((size.0 / 2).max(1), (size.1 / 2).max(1));
                }
                candidates.push((key, prepared.source_bytes(), bytes));
            }
        }
        let selected = if candidates.is_empty() {
            None
        } else {
            // Geometry preparation saved per retained texture byte, rather
            // than paint order deciding which content receives the budget.
            candidates.sort_by(|a, b| {
                (u128::from(b.1) * u128::from(a.2)).cmp(&(u128::from(a.1) * u128::from(b.2)))
            });
            let mut remaining = available;
            let mut selected = super::FxHashSet::default();
            for (key, _, bytes) in candidates {
                if bytes <= remaining && selected.insert(key) {
                    remaining -= bytes;
                }
            }
            Some(selected)
        };
        Self {
            retained,
            inputs,
            available,
            selected,
        }
    }
}

struct PicturePlan {
    image: LocalImage,
    key: LocalKey,
    cache: Key,
    current: bool,
    interop: u64,
    domain: crate::render::planes::static_layer::Domain,
}

impl Lowering<'_> {
    /// Retains source-over pictures in their own stable texel coordinates.
    /// Fractional or shaped outer clips remain distributed over the original
    /// commands; grouping their coverage would change overlapping edges.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounds are clamped to the raster"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "one admission decision computes local bounds, identity and budget together"
    )]
    fn plan_picture(
        &mut self,
        picture: &cherenkov::Picture,
        local: Affine,
        source: &cherenkov::DisplayList,
        ops: &[Op],
        (layer, domains, index): (LayerId, &mut super::FxHashMap<usize, PreparedDomain>, usize),
        glyphs: &GlyphContext<'_>,
    ) -> Result<Option<PicturePlan>, RenderError> {
        let Some(context) = self.pictures.as_ref() else {
            return Ok(None);
        };
        if self.depth != 0 || self.current_space() != cherenkov::BlendSpace::Linear {
            return Ok(None);
        }
        if self.clip.is_some_and(|clip| {
            clip.mask.is_some()
                || clip
                    .aligned_rect
                    .is_none_or(|r| [r.x0, r.y0, r.x1, r.y1].iter().any(|v| v.fract() != 0.0))
        }) {
            return Ok(None);
        }
        let to_parent = self.transform * local;
        let [a, b, c, d, _, _] = to_parent.as_coeffs();
        // The largest singular value measures scale without charging rotation
        // as magnification. Match the raster's f32 precision before bucketing,
        // so roundoff in an orthogonal transform cannot double its density.
        let (xx, yy, xy) = (
            b.mul_add(b, a * a),
            d.mul_add(d, c * c),
            b.mul_add(d, a * c),
        );
        let scale = ((xx + yy + (xx - yy).hypot(2.0 * xy)) * 0.5).sqrt();
        // Half-octave buckets bound excess texel area to 2x while preserving
        // pixels across small scale changes. Whole octaves can charge 4x.
        let density = (f64::from(scale as f32).log2() * 2.0).ceil();
        let density = (density * 0.5).exp2();
        let format = if crate::render::content_range::of_ops(ops, glyphs.images)
            == crate::render::present::ColorRange::HighDynamicRange
        {
            wgpu::TextureFormat::Rgba16Float
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        };
        let key = LocalKey {
            layer,
            picture: Some(std::ptr::from_ref(picture.display_list()) as usize),
            bucket: crate::render::projective::bucket(density),
            format,
        };
        let entry = context
            .retained
            .get(&layer)
            .and_then(|entries| entries.iter().find(|entry| entry.matches(key)));
        let pending = self.picture_realize.iter().find(|image| image.key == key);
        let snapshot = entry
            .and_then(|entry| entry.picture.as_ref())
            .or_else(|| pending.and_then(|image| image.picture.as_ref()));
        let domain = if let Some(snapshot) = snapshot {
            snapshot.domain
        } else {
            // Composite pictures keep their children independently reusable when
            // their placement changes or the enclosing recording is replaced.
            // Groups may depend on the destination and keep their own path.
            if ops.iter().any(|op| {
                matches!(
                    op,
                    Op::BeginPicture { .. } | Op::BeginIsolate { .. } | Op::BeginShadow { .. }
                )
            }) {
                return Ok(None);
            }
            let cached = domains
                .get(&index)
                .filter(|entry| entry.bucket == key.bucket && entry.matches_source(picture));
            let domain = if let Some(cached) = cached {
                cached.domain
            } else {
                let domain = crate::render::planes::static_layer::domain_in(
                    local.inverse(),
                    ops,
                    source,
                    glyphs.fonts,
                    density,
                    context.inputs.limits.max_dimension,
                )?;
                domains.insert(
                    index,
                    PreparedDomain {
                        source: picture.clone(),
                        bucket: key.bucket,
                        format,
                        visible: true,
                        domain,
                    },
                );
                domain
            };
            let Some(domain) = domain else {
                return Ok(None);
            };
            domain
        };
        let raster = domain.raster();
        let forward = to_parent * raster;
        let bounds = forward
            .transform_rect_bbox(Rect::new(
                0.0,
                0.0,
                f64::from(domain.size.0),
                f64::from(domain.size.1),
            ))
            .expand()
            .intersect(Rect::new(
                0.0,
                0.0,
                f64::from(self.width),
                f64::from(self.height),
            ));
        let prepared = domains.entry(index).or_insert_with(|| PreparedDomain {
            source: picture.clone(),
            bucket: key.bucket,
            format,
            visible: true,
            domain: Some(domain),
        });
        prepared.bucket = key.bucket;
        prepared.format = format;
        prepared.domain = Some(domain);
        prepared.visible = !bounds.is_zero_area();
        if !prepared.visible {
            return Ok(None);
        }
        let mut image = LocalImage {
            density,
            local_to_texel: raster.inverse(),
            size: domain.size,
            levels: Vec::new(),
            inverse: Homography::affine(forward.inverse()),
            forward: Homography::affine(forward),
            to_parent: Homography::affine(to_parent),
            bounds: [
                bounds.x0 as u32,
                bounds.y0 as u32,
                bounds.x1 as u32,
                bounds.y1 as u32,
            ],
        };
        let cache = Key::new(&image, 0, context.inputs.replacements);
        let current = pending.is_some()
            || (!context.inputs.stale.contains(&key)
                && entry.is_some_and(|entry| entry.key == Some(cache)));
        if context
            .selected
            .as_ref()
            .is_some_and(|selected| !selected.contains(&key))
        {
            return Ok(None);
        }
        let already_composed = self.picture_composed.contains(&key);
        let bytes = if current {
            entry.map_or_else(
                || {
                    pending
                        .expect("current image is retained or pending")
                        .bytes()
                },
                crate::render::projective::Entry::bytes,
            )
        } else {
            image.levels = mip_levels(image.size);
            crate::render::projective::mip_bytes(format, &image.levels)
        };
        if !already_composed && bytes > context.available {
            return Ok(None);
        }
        let interop = context.inputs.interop;
        if !already_composed {
            self.pictures.as_mut().expect("picture inputs").available -= bytes;
            self.picture_composed.push(key);
        }
        Ok(Some(PicturePlan {
            image,
            key,
            cache,
            current,
            interop,
            domain,
        }))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one retained operation range and its source"
    )]
    pub(super) fn cached_picture(
        &mut self,
        picture: &cherenkov::Picture,
        local: Affine,
        source: &cherenkov::DisplayList,
        ops: &[Op],
        emissions: &mut [Realization<Emission>],
        storage: &mut EmissionStorage,
        (layer, range): (LayerId, Range<usize>),
        glyphs: &GlyphContext<'_>,
    ) -> Result<Option<bool>, RenderError> {
        let Some(PicturePlan {
            image,
            key,
            cache,
            current,
            interop,
            domain,
        }) = self.plan_picture(
            picture,
            local,
            source,
            &ops[range.clone()],
            (layer, &mut storage.picture_domains, range.start - 1),
            glyphs,
        )?
        else {
            return Ok(None);
        };
        if !current {
            let saved = (
                self.transform,
                self.width,
                self.height,
                self.clip,
                self.member_outer_clip,
                self.root_target,
                self.semantic_target,
                self.animating,
            );
            let target = self.current_target();
            self.finish_pass();
            self.set_clip(None);
            self.member_outer_clip = None;
            self.root_target = Target::Projected(key);
            self.semantic_target = self.root_target;
            self.transform = image.local_to_texel * local.inverse();
            self.animating = false;
            self.raster(image.size);
            self.begin_pass(self.root_target, Some([0.0; 4]));
            let (filters, passes) = (self.frame.filters.len(), self.frame.passes.len());
            // The outer image already retains nested pictures' pixels.
            let context = self.pictures.take();
            let result = self.ops(source, ops, emissions, storage, (layer, range), glyphs);
            self.pictures = context;
            self.finish_pass();
            let deps = crate::render::projective::Deps::of(self.frame, filters, passes, interop);
            self.push_mips(self.frame.passes.len() - 1, key);
            (
                self.transform,
                self.width,
                self.height,
                _,
                self.member_outer_clip,
                self.root_target,
                self.semantic_target,
                self.animating,
            ) = saved;
            self.set_clip(saved.3);
            self.begin_pass(target, None);
            result?;
            self.picture_realize.push(Realize {
                key,
                cache,
                levels: image.levels,
                picture: Some(PictureImage {
                    source: picture.clone(),
                    domain,
                }),
                deps,
            });
        }
        self.projective_draws += 1;
        self.emit_projected(
            &Placement {
                key,
                inverse: image.inverse,
                to_parent: image.to_parent,
                bounds: image.bounds,
                density: image.density,
            },
            1.0,
            cherenkov::BlendMode::Normal,
        );
        Ok(Some(!current))
    }
}
