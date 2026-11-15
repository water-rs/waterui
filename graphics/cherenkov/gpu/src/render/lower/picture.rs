//! Picture-local raster retention, using the shared local-image cache.
mod versions;

use super::{
    Affine, Emission, EmissionStorage, GlyphContext, LayerId, LocalKey, Lowering, Op, Placement,
    Range, Realization, Rect, RenderError, Target,
};
use crate::render::projective::tiles::{Cell, Grid};
use crate::render::projective::{Key, PictureId, PictureImage, Realize};
use cherenkov::lowering::projective::{Homography, LocalImage};
use std::sync::Arc;

/// Picture bounds survive texture eviction and admission retries. Keeping the
/// immutable source alive makes allocation identity safe across replacements.
pub(super) struct PreparedDomain {
    source: cherenkov::Picture,
    bucket: i32,
    source_bytes: u64,
    domain: Option<crate::render::planes::static_layer::Domain>,
    tiling: Option<Tiling>,
    versions: versions::Versions,
}

struct Tile {
    identity: Arc<[i32; 2]>,
    visible: bool,
}

struct Tiling {
    grid: Grid,
    anchor: kurbo::Vec2,
    cells: super::FxHashMap<Cell, Tile>,
    levels: Vec<(u32, u32)>,
    committed: u64,
}

impl PreparedDomain {
    pub(super) fn matches_source(&self, source: &cherenkov::Picture) -> bool {
        std::ptr::eq(self.source.display_list(), source.display_list())
    }

    pub(super) fn source_bytes(&self) -> u64 {
        self.source_bytes
            + self.versions.bytes()
            + self.tiling.as_ref().map_or(0, |tiling| {
                (tiling.cells.capacity() * size_of::<(Cell, Tile)>()
                    + tiling.cells.len() * (2 * size_of::<usize>() + size_of::<[i32; 2]>())
                    + tiling.levels.capacity() * size_of::<(u32, u32)>()) as u64
            })
    }
}

impl<'a> super::PictureInputs<'a> {
    pub fn new(
        (retained, inputs, device): (
            &'a super::FxHashMap<LayerId, Vec<crate::render::projective::Entry>>,
            &'a crate::render::projective::Inputs,
            &'a wgpu::Device,
        ),
        available: u64,
        layers: &super::FxHashMap<LayerId, super::ContentData>,
    ) -> Self {
        let mut candidates = Vec::new();
        for (&layer, content) in layers {
            let Some((ops, _)) = content.retained.current() else {
                continue;
            };
            for (&index, prepared) in &content.storage.picture_domains {
                if !matches!(ops.get(index), Some(Op::BeginPicture { picture, .. }) if prepared.matches_source(picture))
                {
                    continue;
                }
                let Some(tiling) = &prepared.tiling else {
                    continue;
                };
                let count = tiling.cells.values().filter(|tile| tile.visible).count() as u64;
                if count != 0 {
                    candidates.push((layer, prepared, count * tiling.committed));
                }
            }
        }
        let selected = if candidates.is_empty() {
            None
        } else {
            // Geometry preparation saved per retained texture byte, rather
            // than paint order deciding which content receives the budget.
            candidates.sort_by(|a, b| {
                (u128::from(b.1.source_bytes) * u128::from(a.2))
                    .cmp(&(u128::from(a.1.source_bytes) * u128::from(b.2)))
            });
            let mut remaining = available;
            let mut selected = super::FxHashMap::default();
            for (layer, prepared, bytes) in candidates {
                let admitted = bytes <= remaining;
                if admitted {
                    remaining -= bytes;
                }
                let tiling = prepared.tiling.as_ref().expect("ranked grid");
                for tile in tiling.cells.values().filter(|tile| tile.visible) {
                    selected.insert(
                        LocalKey {
                            layer,
                            picture: Some(PictureId::cell(&tile.identity)),
                            bucket: prepared.bucket,
                        },
                        admitted,
                    );
                }
            }
            Some(selected)
        };
        Self {
            retained,
            inputs,
            device,
            available,
            selected,
            realize: Vec::new(),
            composed: Vec::new(),
        }
    }
}

struct PicturePlan {
    image: LocalImage,
    key: LocalKey,
    cache: Key,
    current: bool,
    interop: u64,
    grid: Grid,
    anchor: kurbo::Vec2,
    /// The bytes the driver commits for the image, which admission counted.
    bytes: u64,
    revisions: Arc<[u64]>,
    cell: Arc<[i32; 2]>,
    tile: crate::render::projective::TilePlacement,
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
    ) -> Result<Option<Vec<PicturePlan>>, RenderError> {
        let Some(context) = self.pictures.as_ref() else {
            return Ok(None);
        };
        if self.depth != 0
            || self.clip.is_some_and(|clip| {
                clip.mask.is_some()
                    || clip
                        .aligned_rect
                        .is_none_or(|r| [r.x0, r.y0, r.x1, r.y1].iter().any(|v| v.fract() != 0.0))
            })
        {
            return Ok(None);
        }
        // Destination-dependent scopes keep their existing lowering path.
        if ops.iter().any(|op| {
            matches!(
                op,
                Op::BeginPicture { .. } | Op::BeginIsolate { .. } | Op::BeginShadow { .. }
            )
        }) {
            return Ok(None);
        }
        let space = self.current_space();
        let to_parent = self.transform * local;
        let [a, b, c, d, _, _] = to_parent.as_coeffs();
        let (xx, yy, xy) = (
            b.mul_add(b, a * a),
            d.mul_add(d, c * c),
            b.mul_add(d, a * c),
        );
        let scale = ((xx + yy + (xx - yy).hypot(2.0 * xy)) * 0.5).sqrt();
        // Half-octave buckets bound excess texel area to 2x.
        let density = ((f64::from(scale as f32).log2() * 2.0).ceil() * 0.5).exp2();
        let bucket = crate::render::projective::bucket(density);
        let prepared = domains.entry(index).or_insert_with(|| PreparedDomain {
            source: picture.clone(),
            bucket,
            source_bytes: picture
                .display_list()
                .heap_bytes(cherenkov::lowering::memory::command_bytes),
            domain: None,
            tiling: None,
            versions: versions::Versions::default(),
        });
        if prepared.versions.revisions().is_empty() || !prepared.matches_source(picture) {
            prepared.versions.update(ops, local.inverse(), index + 1);
        }
        if !prepared.matches_source(picture) || prepared.bucket != bucket {
            if prepared.bucket != bucket {
                prepared.tiling = None;
            }
            prepared.source = picture.clone();
            prepared.source_bytes = picture
                .display_list()
                .heap_bytes(cherenkov::lowering::memory::command_bytes);
            prepared.bucket = bucket;
            prepared.domain = None;
        }
        if prepared.domain.is_none() {
            // Only individual textures obey the device dimension limit.
            prepared.domain = crate::render::planes::static_layer::domain_in(
                local.inverse(),
                ops,
                source,
                glyphs.fonts,
                density,
                u32::MAX,
            )?;
        }
        let Some(domain) = prepared.domain else {
            return Ok(None);
        };
        let source_bounds = Rect::from_origin_size(
            (domain.origin * density).to_point(),
            (f64::from(domain.size.0), f64::from(domain.size.1)),
        );
        let inverse = Affine::scale(density) * to_parent.inverse();
        let viewport = Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height));
        let viewport = self
            .clip
            .and_then(|clip| clip.aligned_rect)
            .map_or(viewport, |clip| viewport.intersect(clip));
        let visible = inverse
            .transform_rect_bbox(viewport)
            .intersect(source_bounds);
        if visible.is_zero_area() {
            if let Some(tiling) = &mut prepared.tiling {
                for tile in tiling.cells.values_mut() {
                    tile.visible = false;
                }
            }
            return Ok(Some(Vec::new()));
        }
        if prepared.tiling.is_none() {
            let entries = context.retained.get(&layer).map_or_default(Vec::as_slice);
            let compatible = |entry: &&crate::render::projective::Entry| {
                entry.bucket == bucket
                    && entry.picture.as_ref().is_some_and(|snapshot| {
                        std::ptr::eq(snapshot.source.display_list(), picture.display_list())
                            && snapshot.grid.supports(inverse)
                    })
            };
            if let Some(entry) = entries.iter().find(compatible) {
                let snapshot = entry.picture.as_ref().expect("matched picture");
                let mut cells = super::FxHashMap::default();
                for image in entries
                    .iter()
                    .filter(compatible)
                    .filter_map(|entry| entry.picture.as_ref())
                {
                    if image.grid == snapshot.grid && image.anchor == snapshot.anchor {
                        let identity = &image.cell;
                        cells.insert(
                            Cell {
                                x: identity[0],
                                y: identity[1],
                            },
                            Tile {
                                identity: Arc::clone(identity),
                                visible: false,
                            },
                        );
                    }
                }
                prepared.tiling = Some(Tiling {
                    grid: snapshot.grid,
                    anchor: snapshot.anchor,
                    cells,
                    levels: snapshot.grid.levels(),
                    committed: entry.bytes(),
                });
            }
        }
        if prepared
            .tiling
            .as_ref()
            .is_none_or(|tiling| !tiling.grid.supports(inverse))
        {
            let Some(grid) = Grid::for_view(
                inverse,
                source_bounds,
                visible,
                context.inputs.limits.max_dimension,
            ) else {
                return Ok(None);
            };
            let levels = grid.levels();
            let committed = crate::render::resident::texture_bytes(
                context.device,
                &crate::render::projective::descriptor(&levels),
            )?;
            prepared.tiling = Some(Tiling {
                grid,
                anchor: domain.origin * density,
                cells: super::FxHashMap::default(),
                levels,
                committed,
            });
        }
        let tiling = prepared.tiling.as_mut().expect("planned grid");
        for tile in tiling.cells.values_mut() {
            tile.visible = false;
        }
        let Some(cells) = tiling.grid.cells(visible - tiling.anchor) else {
            return Ok(None);
        };
        let mut plans = Vec::new();
        let mut required = 0_u64;
        let mut admitted = true;
        for cell in cells {
            let recorded = tiling.cells.entry(cell).or_insert_with(|| Tile {
                identity: Arc::new([cell.x, cell.y]),
                visible: false,
            });
            recorded.visible = true;
            let key = LocalKey {
                layer,
                picture: Some(PictureId::cell(&recorded.identity)),
                bucket,
            };
            let core = tiling.grid.interior(cell) + tiling.anchor;
            let raster = tiling.grid.raster(cell) + tiling.anchor;
            let local_to_texel =
                Affine::translate(-raster.origin().to_vec2()) * Affine::scale(density);
            let forward = to_parent * local_to_texel.inverse();
            let bounds = inverse
                .inverse()
                .transform_rect_bbox(core.intersect(source_bounds))
                .expand()
                .intersect(viewport);
            let image = LocalImage {
                density,
                local_to_texel,
                size: tiling.levels[0],
                levels: tiling.levels.clone(),
                inverse: Homography::affine(inverse),
                forward: Homography::affine(forward),
                to_parent: Homography::affine(to_parent),
                bounds: [
                    bounds.x0 as u32,
                    bounds.y0 as u32,
                    bounds.x1 as u32,
                    bounds.y1 as u32,
                ],
            };
            let cache = Key::new(&image, space, 0, context.inputs.replacements);
            let entry = context
                .retained
                .get(&layer)
                .and_then(|entries| entries.iter().find(|entry| entry.matches(key)));
            let pending = context.realize.iter().find(|image| image.key == key);
            let snapshot_matches = |snapshot: &PictureImage| {
                std::ptr::eq(snapshot.source.display_list(), picture.display_list())
            };
            let current = pending.is_some_and(|image| {
                image.cache == cache && image.picture.as_ref().is_some_and(snapshot_matches)
            }) || (!context.inputs.stale.contains(&key)
                && entry.is_some_and(|entry| {
                    entry.key == Some(cache) && entry.picture.as_ref().is_some_and(snapshot_matches)
                }));
            let bytes = tiling.committed;
            if !context.composed.contains(&key) {
                required = required.saturating_add(bytes);
            }
            // A newly exposed cell has no prior admission ranking. The
            // frame's remaining budget still applies to it below.
            if entry.is_some()
                && context
                    .selected
                    .as_ref()
                    .is_some_and(|set| set.get(&key) == Some(&false))
            {
                admitted = false;
            }
            plans.push(PicturePlan {
                image,
                key,
                cache,
                current,
                interop: context.inputs.interop,
                grid: tiling.grid,
                anchor: tiling.anchor,
                bytes,
                revisions: prepared.versions.revisions(),
                cell: Arc::clone(&recorded.identity),
                tile: crate::render::projective::TilePlacement {
                    interior: core,
                    origin: raster.origin(),
                },
            });
        }
        // A snapshot owns every resident cell's identity. Once neither a
        // snapshot nor this frame needs it, the sparse grid can release it.
        tiling
            .cells
            .retain(|_, tile| tile.visible || Arc::strong_count(&tile.identity) > 1);
        if !admitted || required > context.available {
            return Ok(None);
        }
        let context = self.pictures.as_mut().expect("picture inputs");
        context.available -= required;
        for plan in &plans {
            if !context.composed.contains(&plan.key) {
                context.composed.push(plan.key);
            }
        }
        Ok(Some(plans))
    }

    /// Lowers the picture scope opened at `ops[open]`: its retained image
    /// when admission accepts one, its commands inline otherwise. Returns the
    /// index after the scope's end and whether anything was realized.
    ///
    /// Kept out of line so the per-op dispatch loop keeps the code shape it
    /// has without pictures: picture-free content pays nothing for the arm
    /// (`bench/scripts/ir_gate.py`).
    #[inline(never)]
    pub(super) fn picture_ops(
        &mut self,
        source: &cherenkov::DisplayList,
        ops: &[Op],
        emissions: &mut [Realization<Emission>],
        storage: &mut EmissionStorage,
        (layer, open): (LayerId, usize),
        glyphs: &GlyphContext<'_>,
    ) -> Result<(usize, bool), RenderError> {
        let Op::BeginPicture {
            picture,
            local,
            end,
        } = &ops[open]
        else {
            unreachable!("picture_ops opens at a picture scope")
        };
        let (body, next) = (open + 1..*end as usize, *end as usize + 1);
        if let Some(realized) = self.cached_picture(
            picture,
            *local,
            source,
            ops,
            emissions,
            storage,
            (layer, body.clone()),
            glyphs,
        )? {
            return Ok((next, realized));
        }
        let changed = self.ops(source, ops, emissions, storage, (layer, body), glyphs)?;
        Ok((next, changed))
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
        let Some(plans) = self.plan_picture(
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
        let mut changed = false;
        for plan in plans {
            changed |= self.realize_picture_tile(
                picture,
                local,
                source,
                ops,
                emissions,
                storage,
                (layer, range.clone()),
                glyphs,
                plan,
            )?;
        }
        Ok(Some(changed))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one retained operation range and its tile plan"
    )]
    fn realize_picture_tile(
        &mut self,
        picture: &cherenkov::Picture,
        local: Affine,
        source: &cherenkov::DisplayList,
        ops: &[Op],
        emissions: &mut [Realization<Emission>],
        storage: &mut EmissionStorage,
        (layer, range): (LayerId, Range<usize>),
        glyphs: &GlyphContext<'_>,
        plan: PicturePlan,
    ) -> Result<bool, RenderError> {
        let PicturePlan {
            image,
            key,
            cache,
            current,
            interop,
            grid,
            anchor,
            bytes,
            revisions,
            cell,
            tile,
        } = plan;
        if !current {
            let saved = (
                self.transform,
                self.width,
                self.height,
                self.clip,
                self.member_outer_clip,
                self.root_target,
                self.root_space,
                self.semantic_target,
                self.animating,
            );
            let target = self.current_target();
            self.finish_pass();
            self.set_clip(None);
            self.member_outer_clip = None;
            self.root_target = Target::Projected(key);
            self.root_space = cache.space;
            self.semantic_target = self.root_target;
            self.transform = image.local_to_texel * local.inverse();
            self.animating = false;
            self.raster(image.size);
            self.begin_pass(self.root_target, Some([0.0; 4]));
            let (filters, passes) = (self.frame.filters.len(), self.frame.passes.len());
            // The outer image already retains nested pictures' pixels.
            let context = self.pictures.take();
            let first_source = self.frame.command_sources.len();
            let first_op = range.start;
            let result = self.ops(source, ops, emissions, storage, (layer, range), glyphs);
            for source in &self.frame.command_sources[first_source..] {
                for (offset, command) in self.frame.commands[source.commands.clone()]
                    .iter_mut()
                    .enumerate()
                {
                    command.revision = revisions[source.first_op + offset - first_op];
                }
            }
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
                self.root_space,
                self.semantic_target,
                self.animating,
            ) = saved;
            self.set_clip(saved.3);
            self.begin_pass(target, None);
            result?;
            self.pictures
                .as_mut()
                .expect("a planned picture has inputs")
                .realize
                .push(Realize {
                    key,
                    cache,
                    bytes,
                    levels: image.levels,
                    picture: Some(PictureImage {
                        source: picture.clone(),
                        grid,
                        anchor,
                        cell,
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
                tile: Some(tile),
            },
            1.0,
            cherenkov::BlendMode::Normal,
            cache.space,
        );
        Ok(!current)
    }
}
