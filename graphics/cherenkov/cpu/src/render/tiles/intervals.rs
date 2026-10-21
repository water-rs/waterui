//! Geometry-local visible intervals and their immutable range index.

use super::{IntegerEdge, ONE, Word};

const BLOCK: usize = 32;

/// A supporting line normalized once, with its visible y interval.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Boundary {
    top_sign: i64,
    pub bottom: i64,
    pub x: f64,
    pub slope: f64,
}

impl Boundary {
    #[expect(
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        reason = "placement integers fit f64; normalization needs no software FMA on targets without it"
    )]
    pub fn new(e: IntegerEdge) -> Self {
        let (x, y, dx, dy, negative) = if e.y0 < e.y1 {
            (e.x0, e.y0, e.x1 - e.x0, e.y1 - e.y0, false)
        } else {
            (e.x1, e.y1, e.x0 - e.x1, e.y0 - e.y1, true)
        };
        let slope = dx as f64 / dy as f64;
        Self {
            top_sign: e.top * 2 + i64::from(negative),
            bottom: e.bottom,
            x: (e.top - y) as f64 * slope + x as f64,
            slope,
        }
    }

    pub const fn top(self) -> i64 {
        self.top_sign >> 1
    }

    pub const fn sign(self) -> i32 {
        1 - 2 * (self.top_sign & 1) as i32
    }

    pub const fn words(self) -> [Word; 4] {
        [
            self.top_sign.cast_unsigned(),
            self.bottom.cast_unsigned(),
            self.x.to_bits(),
            self.slope.to_bits(),
        ]
    }
}

pub const fn leaves(count: usize) -> usize {
    count.div_ceil(BLOCK).next_power_of_two()
}

/// The interval tree and boundary payload share the owner's u64 allocation.
pub fn tree(words: &mut [Word], boundaries: &[Boundary]) {
    let leaves = words.len() / 2;
    words.fill(i64::MIN.cast_unsigned());
    for (i, block) in boundaries.chunks(BLOCK).enumerate() {
        words[leaves + i] = block
            .iter()
            .map(|b| b.bottom)
            .max()
            .unwrap()
            .cast_unsigned();
    }
    for i in (1..leaves).rev() {
        words[i] = words[i * 2]
            .cast_signed()
            .max(words[i * 2 + 1].cast_signed())
            .cast_unsigned();
    }
}

pub fn visit(
    boundaries: &[Boundary],
    tree: &[Word],
    top: i64,
    bottom: i64,
    mut emit: impl FnMut(Boundary),
) {
    let end = boundaries.partition_point(|b| b.top() < bottom);
    visit_node(boundaries, tree, 1, 0, tree.len() / 2, end, top, &mut emit);
}

#[expect(
    clippy::too_many_arguments,
    reason = "implicit tree traversal carries the query and node range"
)]
fn visit_node(
    boundaries: &[Boundary],
    tree: &[Word],
    node: usize,
    first: usize,
    blocks: usize,
    end: usize,
    top: i64,
    emit: &mut impl FnMut(Boundary),
) {
    if first * BLOCK >= end || tree[node].cast_signed() <= top {
        return;
    }
    if blocks == 1 {
        for b in &boundaries[first * BLOCK..((first + 1) * BLOCK).min(end)] {
            if b.bottom > top {
                emit(*b);
            }
        }
    } else {
        let half = blocks / 2;
        visit_node(boundaries, tree, node * 2, first, half, end, top, emit);
        visit_node(
            boundaries,
            tree,
            node * 2 + 1,
            first + half,
            half,
            end,
            top,
            emit,
        );
    }
}

/// Maximum vertical intervals a translated 16-row band can intersect.
pub fn vertical_capacity(boundaries: &[Boundary], ends: &mut Vec<i64>) -> (usize, usize) {
    ends.clear();
    ends.extend(
        boundaries
            .iter()
            .filter(|b| b.slope == 0.0)
            .map(|b| b.bottom),
    );
    ends.sort_unstable();
    let (mut started, mut ended, mut peak) = (0, 0, 0);
    for b in boundaries.iter().filter(|b| b.slope == 0.0) {
        while ended < ends.len() && ends[ended] <= b.top() - 16 * ONE + 1 {
            ended += 1;
        }
        started += 1;
        peak = peak.max(started - ended);
    }
    (peak, ends.len())
}
