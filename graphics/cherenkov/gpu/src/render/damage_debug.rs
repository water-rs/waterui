//! Opt-in damage reports and a drawable overlay, separate from retained pixels.

use cherenkov::{Draw, Picture, SurfaceId, WorkingColor, damage::Region, kurbo::Rect};

use super::{
    composite::demand::Demand,
    lower::{Frame, Source, Target},
};

/// One prepared surface's raster damage. Draw [`Self::overlay`] on a separate
/// diagnostic surface so debug ink does not become input to retained rendering.
#[derive(Debug)]
pub struct Report {
    /// Frame whose prepared surface is being encoded.
    pub frame: cherenkov::FrameId,
    /// Surface whose root outputs were prepared.
    pub surface: SurfaceId,
    /// Surface extent in device pixels.
    pub size: (u32, u32),
    /// Union of actual root-output raster scissors.
    pub damage: Region,
    /// Picture tiles composed directly into the root outputs. Tiles inside
    /// an isolated intermediate use that intermediate's coordinates and are
    /// not represented as surface-space rectangles here.
    pub tiles: Vec<Tile>,
}

/// A retained picture tile visible in a root output.
#[derive(Clone, Copy, Debug)]
pub struct Tile {
    /// Conservative surface-space bounds, clipped to the surface extent.
    pub bounds: Rect,
    /// At least one raster pass writes damaged texels into this tile.
    pub repainted: bool,
}

impl Report {
    /// Diagnostic ink in device-pixel coordinates: cyan damage and orange
    /// outlines around repainted picture tiles. The translucent fill and
    /// one-pixel outline are a diagnostic palette, not scene content.
    #[must_use]
    pub fn overlay(&self) -> Picture {
        Picture::record(|canvas| {
            for rect in self.damage.rectangles() {
                canvas.fill(
                    Rect::new(
                        f64::from(rect.x0()),
                        f64::from(rect.y0()),
                        f64::from(rect.x1()),
                        f64::from(rect.y1()),
                    ),
                    WorkingColor::new([0.0, 1.0, 1.0, 0.25]),
                );
            }
            for tile in &self.tiles {
                if tile.repainted {
                    canvas.stroke(
                        tile.bounds,
                        cherenkov::kurbo::Stroke::new(1.0),
                        WorkingColor::new([1.0, 0.5, 0.0, 1.0]),
                    );
                }
            }
        })
    }
}

pub(super) fn capture(
    frame_id: cherenkov::FrameId,
    surface: SurfaceId,
    size: (u32, u32),
    frame: &Frame,
    demand: &Demand,
) -> Report {
    let mut damage = Region::new(size);
    let mut tiles = Vec::new();
    let viewport = Rect::new(0.0, 0.0, f64::from(size.0), f64::from(size.1));
    for (index, pass) in frame.passes.iter().enumerate() {
        if !matches!(pass.target, Target::Part(_)) {
            continue;
        }
        for rect in demand.pass(index).rectangles() {
            let (x, y, width, height) = rect.xywh();
            damage.add(x, y, width, height);
        }
        for range in &pass.ranges {
            let Some(Source::Projected(key)) = range.source else {
                continue;
            };
            if key.picture.is_none() {
                continue;
            }
            let repainted = frame.passes.iter().enumerate().any(|(writer, pass)| {
                pass.target == Target::Projected(key) && !demand.pass(writer).is_empty()
            });
            for instance in
                &frame.instances[range.instances.start as usize..range.instances.end as usize]
            {
                let [x0, y0, x1, y1] = instance.bounds.map(f64::from);
                tiles.push(Tile {
                    bounds: Rect::new(x0, y0, x1, y1).intersect(viewport),
                    repainted,
                });
            }
        }
    }
    Report {
        frame: frame_id,
        surface,
        size,
        damage,
        tiles,
    }
}
