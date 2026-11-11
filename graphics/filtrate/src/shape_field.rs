//! The signed distance field of a filter input's shape, for stages that read
//! [`ShapeInput::Sdf`](filtrate_core::ShapeInput::Sdf).
//!
//! Both executors build it by jump flooding, on the same schedule: a seed
//! step records, at every pixel next to the alpha = 0.5 contour, the offset
//! to the contour; each step propagates the nearest offset across a halving
//! jump distance, with one extra unit step that repairs the flood's rare
//! misses; a resolve step turns the offsets into signed distances. The GPU
//! executor runs the steps as passes (`executor::shape`); the lane executor
//! runs them here, on the window it applies to.

extern crate alloc;

use alloc::vec::Vec;

/// The jump distances of a flood over `size`: the powers of two from half
/// the longest side's next power of two down to one, then one more unit
/// step.
pub fn jumps((width, height): (u32, u32)) -> Vec<i32> {
    let mut jumps = Vec::new();
    let mut jump = width.max(height).next_power_of_two() / 2;
    while jump > 0 {
        jumps.push(i32::try_from(jump).expect("a texture side fits in i32"));
        jump /= 2;
    }
    jumps.push(1);
    jumps
}

/// What a pixel no contour point reached holds: the largest finite `f16`,
/// the GPU field's texel precision (`shaders/shape/resolve.wgsl`), so an
/// input with nothing covered reads the same on both executors.
#[cfg(feature = "lanes")]
const FARTHEST: f32 = 65504.0;

/// The 4-neighbours the seed step looks across, as the GPU seed pass
/// (`shaders/shape/seed.wgsl`) orders them.
#[cfg(feature = "lanes")]
const DIRECTIONS: [[i8; 2]; 4] = [[-1, 0], [1, 0], [0, -1], [0, 1]];

/// One pixel of the flood.
#[cfg(feature = "lanes")]
#[derive(Clone, Copy, Debug)]
struct Cell {
    /// The offset from the pixel's centre to the nearest contour point
    /// found, in pixels.
    offset: [f32; 2],
    /// Whether `offset` holds a contour point.
    found: bool,
    /// Whether the pixel is inside the shape.
    inside: bool,
}

/// The coverage of the window a field is built over: rows `top..top + rows`
/// of a `width` × `height` image.
#[cfg(feature = "lanes")]
struct Coverage<'a> {
    pixels: &'a [[f32; 4]],
    width: usize,
    rows: usize,
    top: usize,
    height: usize,
}

#[cfg(feature = "lanes")]
impl Coverage<'_> {
    /// The alpha at `(x, y)` displaced by `step`. Outside the image nothing
    /// is covered, as outside the GPU's input texture, so content reaching
    /// an image edge ends there. Past a window edge inside the image the
    /// window's edge row extends, as the pass's sampler clamps to it, so a
    /// band seam adds no contour.
    fn alpha(&self, x: usize, y: usize, step: [i8; 2]) -> f32 {
        let Some(x) = x
            .checked_add_signed(isize::from(step[0]))
            .filter(|&x| x < self.width)
        else {
            return 0.0;
        };
        let y = match y.checked_add_signed(isize::from(step[1])) {
            None if self.top == 0 => return 0.0,
            None => 0,
            Some(y) if y >= self.rows && self.top + self.rows >= self.height => return 0.0,
            Some(y) => y.min(self.rows - 1),
        };
        self.pixels[y * self.width + x][3]
    }

    /// The seed step at `(x, y)`: the offset to the contour point nearest
    /// the pixel's centre when the contour passes between it and a
    /// 4-neighbour, taken as the straight line through the crossing linear
    /// interpolation of their coverage puts it at, whose normal is the
    /// coverage gradient.
    fn seed(&self, x: usize, y: usize) -> Cell {
        let alpha = self.alpha(x, y, [0, 0]);
        let inside = alpha > 0.5;
        let mut neighbours = [0.0f32; 4];
        let mut nearest = 2.0f32;
        let mut crossing = [0.0f32; 2];
        for (neighbour, step) in neighbours.iter_mut().zip(DIRECTIONS) {
            let other = self.alpha(x, y, step);
            *neighbour = other;
            if (other > 0.5) != inside {
                // `inside` differs, so `alpha - other` is non-zero.
                let t = (alpha - 0.5) / (alpha - other);
                if t < nearest {
                    nearest = t;
                    crossing = step.map(|component| f32::from(component) * t);
                }
            }
        }
        if nearest > 1.0 {
            return Cell {
                offset: [0.0; 2],
                found: false,
                inside,
            };
        }
        let gradient = [neighbours[1] - neighbours[0], neighbours[3] - neighbours[2]];
        let magnitude = gradient[0].hypot(gradient[1]);
        let offset = if magnitude > 0.0 {
            let normal = gradient.map(|component| component / magnitude);
            let along = crossing[0].mul_add(normal[0], crossing[1] * normal[1]);
            normal.map(|component| component * along)
        } else {
            crossing
        };
        Cell {
            offset,
            found: true,
            inside,
        }
    }
}

/// The shape field of `pixels` — premultiplied texels holding rows
/// `top..top + rows` of a `width` × `height` image — as texels whose red
/// channel holds the signed distance, in pixels and negative inside, to the
/// alpha = 0.5 contour; alpha is one.
///
/// A contour beyond a window edge inside the image is not seen: a pixel's
/// distance is the full image's, to the flood's accuracy, while its
/// nearest contour point lies inside the window — within the stage's
/// footprint, for every row the [`CpuFilter`](filtrate_core::CpuFilter)
/// band contract keeps — and otherwise the window's nearest, which is no
/// nearer.
#[cfg(feature = "lanes")]
#[expect(
    clippy::cast_precision_loss,
    reason = "a jump is at most half a window side, well below f32's exact integer range"
)]
pub fn signed_distance(
    pixels: &[[f32; 4]],
    width: usize,
    top: usize,
    height: usize,
) -> Vec<[f32; 4]> {
    let rows = pixels.len() / width;
    let coverage = Coverage {
        pixels,
        width,
        rows,
        top,
        height,
    };
    let mut cells: Vec<Cell> = (0..rows)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| coverage.seed(x, y))
        .collect();
    let mut next = cells.clone();
    let side = |length: usize| u32::try_from(length).expect("a window side fits in u32");
    for jump in jumps((side(width), side(rows))) {
        let jump = isize::try_from(jump).expect("a jump fits in isize");
        for y in 0..rows {
            for x in 0..width {
                let own = cells[y * width + x];
                let mut best = own;
                let mut best_distance = own.offset[0].mul_add(own.offset[0], own.offset[1].powi(2));
                for [dx, dy] in [-1isize, 0, 1]
                    .into_iter()
                    .flat_map(|dy| [-1isize, 0, 1].map(|dx| [dx, dy]))
                    .filter(|&step| step != [0, 0])
                {
                    let (Some(other_x), Some(other_y)) = (
                        x.checked_add_signed(dx * jump).filter(|&x| x < width),
                        y.checked_add_signed(dy * jump).filter(|&y| y < rows),
                    ) else {
                        continue;
                    };
                    let other = cells[other_y * width + other_x];
                    if !other.found {
                        continue;
                    }
                    let offset = [
                        (dx * jump) as f32 + other.offset[0],
                        (dy * jump) as f32 + other.offset[1],
                    ];
                    let squared = offset[0].mul_add(offset[0], offset[1].powi(2));
                    if !best.found || squared < best_distance {
                        best = Cell {
                            offset,
                            found: true,
                            inside: own.inside,
                        };
                        best_distance = squared;
                    }
                }
                next[y * width + x] = best;
            }
        }
        core::mem::swap(&mut cells, &mut next);
    }
    cells
        .into_iter()
        .map(|cell| {
            let distance = if cell.found {
                let magnitude = cell.offset[0].hypot(cell.offset[1]);
                if cell.inside { -magnitude } else { magnitude }
            } else {
                FARTHEST
            };
            [distance, 0.0, 0.0, 1.0]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::jumps;

    #[test]
    fn jumps_halve_from_the_longest_side_and_end_with_a_unit_step() {
        assert_eq!(jumps((1, 1)), [1]);
        assert_eq!(jumps((5, 3)), [4, 2, 1, 1]);
        assert_eq!(jumps((8, 64)), [32, 16, 8, 4, 2, 1, 1]);
    }
}
