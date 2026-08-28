//! Dirty-region tracking and band scheduling.
//!
//! Embedded displays are bandwidth-bound: a full 320×240 RGB565 frame is
//! ~150 KiB, which caps full-frame refresh around 26 fps on a 40 MHz SPI
//! bus. Dew therefore re-rasterizes and flushes only the device pixels that
//! actually changed. `WaterUI`'s fine-grained reactivity identifies the
//! dirty view subtrees; this module turns their logical-pixel bounds into a
//! minimal set of device-pixel regions, sliced into horizontal bands so the
//! rasterization scratch buffer stays small enough for on-chip SRAM.

use kurbo::Rect;

use crate::display_list::DisplayList;

/// An axis-aligned region in whole device pixels, `x`/`y` at the top-left.
///
/// Regions are always fully contained in the screen; constructors clamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceRegion {
    /// Leftmost column covered by the region.
    pub x: u32,
    /// Topmost row covered by the region.
    pub y: u32,
    /// Width in device pixels; never zero for emitted regions.
    pub width: u32,
    /// Height in device pixels; never zero for emitted regions.
    pub height: u32,
}

impl DeviceRegion {
    /// Converts logical-pixel bounds into a device region by rounding
    /// outward and clamping to a `screen_width` × `screen_height` screen.
    ///
    /// Returns [`None`] when the bounds do not intersect the screen or are
    /// empty/degenerate (NaN, zero area).
    #[must_use]
    pub fn from_logical(bounds: Rect, screen_width: u32, screen_height: u32) -> Option<Self> {
        let x0 = bounds.x0.floor().max(0.0);
        let y0 = bounds.y0.floor().max(0.0);
        let x1 = bounds.x1.ceil().min(f64::from(screen_width));
        let y1 = bounds.y1.ceil().min(f64::from(screen_height));
        if !(x1 > x0 && y1 > y0) {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "values are clamped to [0, screen size] above, so the casts are lossless"
        )]
        Some(Self {
            x: x0 as u32,
            y: y0 as u32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        })
    }

    /// Returns the intersection with another region, or [`None`] when the
    /// regions do not overlap.
    #[must_use]
    pub fn intersect(self, other: Self) -> Option<Self> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.width).min(other.x + other.width);
        let y1 = (self.y + self.height).min(other.y + other.height);
        (x1 > x0 && y1 > y0).then(|| Self {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }

    /// Returns the smallest region covering both `self` and `other`.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = (self.x + self.width).max(other.x + other.width);
        let y1 = (self.y + self.height).max(other.y + other.height);
        Self {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
    }

    /// Number of pixels covered by the region.
    #[must_use]
    pub const fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// Slices dirty regions into horizontal bands bounded by the scratch-buffer
/// budget.
///
/// The scheduler owns no pixels; it only decides *what* to rasterize. Each
/// emitted [`DeviceRegion`] is at most `band_height` rows tall, so the
/// painter's scratch pixmap never exceeds `screen_width × band_height`
/// RGBA8 pixels regardless of how large the dirty area is.
#[derive(Debug, Clone, Copy)]
pub struct BandScheduler {
    screen_width: u32,
    screen_height: u32,
    band_height: u32,
}

impl BandScheduler {
    /// Creates a scheduler for a `screen_width` × `screen_height` screen
    /// with bands at most `band_height` rows tall.
    ///
    /// # Panics
    ///
    /// Panics if any dimension is zero — a zero-sized screen or band cannot
    /// render anything and indicates a wiring bug.
    #[must_use]
    pub fn new(screen_width: u32, screen_height: u32, band_height: u32) -> Self {
        assert!(
            screen_width > 0 && screen_height > 0 && band_height > 0,
            "BandScheduler requires non-zero screen ({screen_width}x{screen_height}) and band height ({band_height})"
        );
        Self {
            screen_width,
            screen_height,
            band_height,
        }
    }

    /// Maximum rows per emitted band, which bounds the scratch pixmap.
    #[must_use]
    pub const fn band_height(&self) -> u32 {
        self.band_height
    }

    /// The full screen as a single dirty region.
    #[must_use]
    pub const fn full_screen(&self) -> DeviceRegion {
        DeviceRegion {
            x: 0,
            y: 0,
            width: self.screen_width,
            height: self.screen_height,
        }
    }

    /// Converts dirty logical-pixel bounds into the band-sliced device
    /// regions that must be re-rasterized, in top-to-bottom order.
    ///
    /// Overlapping dirty rects falling into the same band row are merged.
    /// Disjoint rects remain separate so a small update on each side of a
    /// panel never turns into a nearly full-width transfer.
    #[must_use]
    pub fn schedule(&self, dirty: &[Rect]) -> Vec<DeviceRegion> {
        let regions: Vec<DeviceRegion> = dirty
            .iter()
            .filter_map(|rect| {
                DeviceRegion::from_logical(*rect, self.screen_width, self.screen_height)
            })
            .collect();
        let mut bands = Vec::new();
        let mut row = 0u32;
        while row < self.screen_height {
            let band = DeviceRegion {
                x: 0,
                y: row,
                width: self.screen_width,
                height: self.band_height.min(self.screen_height - row),
            };
            let intersections = regions
                .iter()
                .filter_map(|region| region.intersect(band))
                .collect::<Vec<_>>();
            bands.extend(merge_overlapping(intersections));
            row += self.band_height;
        }
        bands
    }
}

/// Merges regions that overlap or abut into the fewest covering regions.
///
/// Every input comes from one band row, so they already share a bounded row
/// span and merging reduces to a sweep along `x`: sort by left edge, then
/// extend the open region while the next one starts at or before its right
/// edge. That is `O(n log n)` and, more importantly, bounded — the previous
/// restart-on-merge scan was quadratic in the number of dirty rects and
/// degenerated further as merges cascaded.
fn merge_overlapping(mut pending: Vec<DeviceRegion>) -> Vec<DeviceRegion> {
    if pending.len() < 2 {
        return pending;
    }
    pending.sort_unstable_by_key(|region| (region.x, region.y));
    let mut merged: Vec<DeviceRegion> = Vec::with_capacity(pending.len());
    for region in pending {
        match merged.last_mut() {
            Some(open) if horizontally_touching(*open, region) => *open = open.union(region),
            _ => merged.push(region),
        }
    }
    merged.sort_unstable_by_key(|region| (region.y, region.x));
    merged
}

/// Whether `next` starts at or before `open`'s right edge.
///
/// Both regions belong to the same band row and the sweep guarantees `next`
/// starts no earlier than `open`, so horizontal adjacency is the only test
/// needed. Regions that merely touch are merged deliberately: one panel
/// transaction covering both costs less than two address-window setups for
/// pixels that are already adjacent.
const fn horizontally_touching(open: DeviceRegion, next: DeviceRegion) -> bool {
    next.x <= open.x + open.width
}

/// Maps band rows to the display-list commands that can touch them.
///
/// The painter replays a display list into one band at a time. Without an
/// index that is `O(bands × commands)` per frame — every command tested
/// against every band, with the overwhelming majority of tests failing.
/// Bucketing each command into the band rows its bounds span costs one pass
/// over the list and leaves each band visiting only the commands that can
/// possibly appear in it.
///
/// Indices are stored in ascending order within a row, so replaying a row's
/// bucket preserves the list's draw order and therefore its z-order.
#[derive(Debug, Clone)]
pub struct BandIndex {
    band_height: u32,
    rows: Vec<Vec<u32>>,
}

impl BandIndex {
    /// Buckets `list`'s commands into the band rows of a `screen_height`-tall
    /// screen sliced at `band_height`.
    ///
    /// # Panics
    ///
    /// Panics when `band_height` or `screen_height` is zero, or when the list
    /// holds more commands than a `u32` can index.
    #[must_use]
    pub fn build(list: &DisplayList, screen_height: u32, band_height: u32) -> Self {
        assert!(
            band_height > 0 && screen_height > 0,
            "BandIndex requires a positive screen ({screen_height}) and band height ({band_height})"
        );
        let row_count = screen_height.div_ceil(band_height);
        let mut rows =
            vec![Vec::new(); usize::try_from(row_count).expect("band row count must fit usize")];
        for (index, placed) in list.commands().iter().enumerate() {
            let index = u32::try_from(index).expect("display list exceeds u32 command indices");
            let bounds = placed.bounds();
            if !(bounds.width() > 0.0 && bounds.height() > 0.0) {
                continue;
            }
            let first_row = row_of(bounds.y0, band_height, row_count);
            let final_row = row_of(bounds.y1.next_down(), band_height, row_count);
            for row in first_row..=final_row {
                rows[usize::try_from(row).expect("band row must fit usize")].push(index);
            }
        }
        Self { band_height, rows }
    }

    /// Command indices that may touch the band row containing `region`.
    ///
    /// # Panics
    ///
    /// Panics when `region` lies below the screen the index was built for,
    /// which would mean the scheduler and the index disagree about the panel.
    #[must_use]
    pub fn candidates(&self, region: DeviceRegion) -> &[u32] {
        let row = usize::try_from(region.y / self.band_height).expect("band row must fit usize");
        assert!(
            row < self.rows.len(),
            "band region at y={} lies outside the {}-row index",
            region.y,
            self.rows.len()
        );
        &self.rows[row]
    }
}

/// The band row a logical `y` coordinate falls in, clamped to the screen.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the quotient is clamped into [0, row_count) before the cast"
)]
fn row_of(y: f64, band_height: u32, row_count: u32) -> u32 {
    let row = (y.max(0.0) / f64::from(band_height)).floor();
    row.min(f64::from(row_count - 1)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_bounds_round_outward_and_clamp() {
        let region =
            DeviceRegion::from_logical(Rect::new(10.2, -5.0, 20.7, 12.0), 320, 240).unwrap();
        assert_eq!(
            region,
            DeviceRegion {
                x: 10,
                y: 0,
                width: 11,
                height: 12
            }
        );
    }

    #[test]
    fn offscreen_bounds_produce_no_region() {
        assert!(DeviceRegion::from_logical(Rect::new(-30.0, 0.0, -1.0, 10.0), 320, 240).is_none());
        assert!(DeviceRegion::from_logical(Rect::new(0.0, 250.0, 10.0, 260.0), 320, 240).is_none());
        assert!(DeviceRegion::from_logical(Rect::new(5.0, 5.0, 5.0, 5.0), 320, 240).is_none());
    }

    #[test]
    fn dirty_rect_spanning_bands_is_sliced_per_band() {
        let scheduler = BandScheduler::new(320, 240, 16);
        let bands = scheduler.schedule(&[Rect::new(10.0, 12.0, 50.0, 40.0)]);
        assert_eq!(
            bands,
            vec![
                DeviceRegion {
                    x: 10,
                    y: 12,
                    width: 40,
                    height: 4
                },
                DeviceRegion {
                    x: 10,
                    y: 16,
                    width: 40,
                    height: 16
                },
                DeviceRegion {
                    x: 10,
                    y: 32,
                    width: 40,
                    height: 8
                },
            ]
        );
    }

    #[test]
    fn disjoint_dirty_rects_stay_separate_within_a_band() {
        let scheduler = BandScheduler::new(320, 240, 16);
        let bands = scheduler.schedule(&[
            Rect::new(0.0, 0.0, 20.0, 10.0),
            Rect::new(100.0, 4.0, 140.0, 10.0),
        ]);
        assert_eq!(
            bands,
            vec![
                DeviceRegion {
                    x: 0,
                    y: 0,
                    width: 20,
                    height: 10
                },
                DeviceRegion {
                    x: 100,
                    y: 4,
                    width: 40,
                    height: 6
                }
            ]
        );
    }

    #[test]
    fn overlapping_dirty_rects_merge_within_a_band() {
        let scheduler = BandScheduler::new(320, 240, 16);
        let bands = scheduler.schedule(&[
            Rect::new(0.0, 0.0, 20.0, 10.0),
            Rect::new(15.0, 4.0, 40.0, 12.0),
        ]);
        assert_eq!(
            bands,
            vec![DeviceRegion {
                x: 0,
                y: 0,
                width: 40,
                height: 12
            }]
        );
    }

    #[test]
    fn clean_frame_schedules_nothing() {
        let scheduler = BandScheduler::new(320, 240, 16);
        assert!(scheduler.schedule(&[]).is_empty());
    }
}
