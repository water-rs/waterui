//! Stable picture-texel cells and the apron their reconstruction reads.

use cherenkov::lowering::projective::{Homography, footprint};
use kurbo::{Affine, Rect};

/// A cell is identified in picture texels, independently of its placement
/// on the surface. Scrolling changes the visible cells, not their identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Cell {
    pub x: i32,
    pub y: i32,
}

/// Every texture origin and size is aligned to the coarsest sampled mip.
/// Consequently adjacent textures reconstruct the same global texel grid.
#[derive(Clone, Copy, Debug)]
pub(super) struct Grid {
    span: [u32; 2],
    apron: [u32; 2],
    level: u32,
}

impl Grid {
    /// Plans cells for an affine destination-to-picture-texel transform.
    /// `span` is the requested cell interior; `limit` is the device's
    /// maximum texture dimension, including the filtering apron.
    /// An unrepresentable footprint is not admitted to the raster cache.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "finite nonnegative dimensions are bounded by the device limit before conversion"
    )]
    pub fn new(inverse: Affine, span: [u32; 2], limit: u32) -> Option<Self> {
        let filter = footprint(&Homography::affine(inverse), [0.0, 0.0])?;
        let level = filter.lod.ceil();
        let alignment = level.exp2();
        if !alignment.is_finite() || alignment > f64::from(limit) {
            return None;
        }
        // The outermost tap is (N - 1)/2 steps from the centre. A
        // bilinear sample of an area-averaged mip reaches at most one
        // whole mip texel farther in each axis, in base-level texels.
        let radius = filter
            .step
            .map(|step| f64::mul_add(step.abs() * (f64::from(filter.taps) - 1.0), 0.5, alignment));
        let apron = radius.map(|r| (r / alignment).ceil() * alignment);
        let span = span.map(|s| (f64::from(s) / alignment).ceil() * alignment);
        for axis in 0..2 {
            if span[axis] == 0.0 || 2.0f64.mul_add(apron[axis], span[axis]) > f64::from(limit) {
                return None;
            }
        }
        Some(Self {
            span: span.map(|s| s as u32),
            apron: apron.map(|s| s as u32),
            level: level as u32,
        })
    }

    /// All cells intersecting `visible` in picture texels. The iterator
    /// stores only two ranges; a large picture never allocates a dense grid.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "cell coordinates are range-checked before conversion"
    )]
    pub fn cells(self, visible: Rect) -> Option<impl Iterator<Item = Cell>> {
        let lo = [
            (visible.x0 / f64::from(self.span[0])).floor(),
            (visible.y0 / f64::from(self.span[1])).floor(),
        ];
        let hi = [
            (visible.x1 / f64::from(self.span[0])).ceil(),
            (visible.y1 / f64::from(self.span[1])).ceil(),
        ];
        if !visible.is_finite()
            || lo.iter().any(|v| *v < f64::from(i32::MIN))
            || hi.iter().any(|v| *v > f64::from(i32::MAX))
        {
            return None;
        }
        let x_end = if visible.width() <= 0.0 || visible.height() <= 0.0 {
            lo[0]
        } else {
            hi[0]
        };
        let x = lo[0] as i32..x_end as i32;
        let y = lo[1] as i32..hi[1] as i32;
        Some(y.flat_map(move |y| x.clone().map(move |x| Cell { x, y })))
    }

    /// Half-open interior: each picture sample belongs to exactly one cell.
    pub fn interior(self, cell: Cell) -> Rect {
        let x = f64::from(cell.x) * f64::from(self.span[0]);
        let y = f64::from(cell.y) * f64::from(self.span[1]);
        Rect::new(
            x,
            y,
            x + f64::from(self.span[0]),
            y + f64::from(self.span[1]),
        )
    }

    /// Rasterized area, including neighboring content required by filtering.
    pub fn raster(self, cell: Cell) -> Rect {
        self.interior(cell)
            .inflate(f64::from(self.apron[0]), f64::from(self.apron[1]))
    }

    /// Only the mip levels this affine reconstruction can sample. Texture
    /// dimensions divide exactly at every level, without odd-edge rescaling.
    pub fn levels(self) -> Vec<(u32, u32)> {
        let width = self.span[0] + 2 * self.apron[0];
        let height = self.span[1] + 2 * self.apron[1];
        (0..=self.level)
            .map(|level| (width >> level, height >> level))
            .collect()
    }

    /// A changed pose can keep the existing pixels when its taps fit the
    /// retained apron and mip range. Reducing the footprint never shrinks
    /// a live texture merely to expand it again on the next animation step.
    pub fn supports(self, inverse: Affine) -> bool {
        let Some(required) = Self::new(inverse, self.span, u32::MAX) else {
            return false;
        };
        required.level <= self.level
            && required.apron[0] <= self.apron[0]
            && required.apron[1] <= self.apron[1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_cells_use_a_stable_negative_and_positive_grid() {
        let grid = Grid::new(Affine::IDENTITY, [64, 32], 1024).unwrap();
        let visible = Rect::new(-64.0, -0.25, 64.0, 32.0);
        assert_eq!(
            grid.cells(visible).unwrap().collect::<Vec<_>>(),
            [
                Cell { x: -1, y: -1 },
                Cell { x: 0, y: -1 },
                Cell { x: -1, y: 0 },
                Cell { x: 0, y: 0 },
            ]
        );
        let translated = Grid::new(Affine::translate((0.25, -17.5)), [64, 32], 1024).unwrap();
        let cell = Cell { x: -1, y: 0 };
        assert_eq!(grid.raster(cell), translated.raster(cell));
        assert_eq!(grid.levels(), translated.levels());
        assert_eq!(
            grid.cells(Rect::new(0.25, 0.5, 0.25, 12.0))
                .unwrap()
                .count(),
            0
        );
        assert!(Grid::new(Affine::IDENTITY, [64, 32], 64).is_none());
        assert!(grid.supports(Affine::translate((0.75, -0.25))));
        assert!(!grid.supports(Affine::scale(4.0)));
        let minified = Grid::new(Affine::scale(4.0), [64, 32], 1024).unwrap();
        assert!(minified.supports(Affine::scale(1.2)));
    }

    #[test]
    fn mip_aprons_cover_rotated_anisotropic_taps_on_both_sides_of_a_seam() {
        for inverse in [
            Affine::IDENTITY,
            Affine::scale(1.2),
            Affine::rotate(0.7) * Affine::scale_non_uniform(31.0, 1.3),
            Affine::rotate(-0.4) * Affine::scale_non_uniform(1.1, 64.0),
            Affine::new([2.0, 0.3, 7.0, 1.5, 0.25, -0.75]),
        ] {
            let grid = Grid::new(inverse, [64, 32], 1024).unwrap();
            let footprint = footprint(&Homography::affine(inverse), [0.0, 0.0]).unwrap();
            let levels = grid.levels();
            let alignment = f64::from(1_u32 << grid.level);
            for cell in [Cell { x: -1, y: 0 }, Cell { x: 0, y: 0 }] {
                let core = grid.interior(cell);
                let raster = grid.raster(cell);
                assert_eq!(raster.x0 % alignment, 0.0);
                assert_eq!(raster.y0 % alignment, 0.0);
                for (level, &(w, h)) in levels.iter().enumerate() {
                    assert_eq!((w << level, h << level), levels[0]);
                }
                for center in [core.origin(), (core.x1, core.y1).into()] {
                    for tap in 0..footprint.taps {
                        let offset =
                            0.5f64.mul_add(-f64::from(footprint.taps), f64::from(tap) + 0.5);
                        let x = footprint.step[0].mul_add(offset, center.x);
                        let y = footprint.step[1].mul_add(offset, center.y);
                        assert!(x - alignment >= raster.x0 && x + alignment <= raster.x1);
                        assert!(y - alignment >= raster.y0 && y + alignment <= raster.y1);
                    }
                }
            }
        }
    }
}
