//! Backward pixel demand through the frame's ordered image writes.
//!
//! Regions use each pass's virtual raster coordinates, never texture-local
//! coordinates. A read names its last prior writer so a frozen capture cannot
//! accidentally pull in a later write to the same allocation.

use cherenkov::damage::Region;

use super::plan::{ExecutionPlan, Read, ReadKind};
use crate::render::{
    damage::add,
    lower::{Frame, Target},
};

#[derive(Default)]
pub struct Demand {
    passes: Vec<Region>,
    staging: rustc_hash::FxHashMap<usize, Region>,
}

impl Demand {
    pub fn bytes(&self) -> u64 {
        (self.passes.capacity() * size_of::<Region>()
            + self.staging.capacity() * size_of::<(usize, Region)>()) as u64
            + self
                .staging
                .values()
                .map(|region| region.heap_bytes().0)
                .sum::<u64>()
            + self
                .passes
                .iter()
                .map(|region| region.heap_bytes().0)
                .sum::<u64>()
    }

    /// Diagnostic union of the raster scissors over persistent base-level
    /// outputs. This bounds touched pixels, not fragment invocations.
    pub fn output_pixels(&self, frame: &Frame) -> u64 {
        let mut outputs = rustc_hash::FxHashMap::<Target, Region>::default();
        for (index, pass) in frame.passes.iter().enumerate() {
            if matches!(
                pass.target,
                Target::Part(_) | Target::Plane(_) | Target::Projected(_)
            ) {
                let [x, y, w, h] = pass.region;
                let output = outputs
                    .entry(pass.target)
                    .or_insert_with(|| Region::new((x + w, y + h)));
                for rect in self.pass(index).rectangles() {
                    let (x, y, w, h) = rect.xywh();
                    output.add(x, y, w, h);
                }
            }
        }
        outputs.values().map(Region::pixel_area).sum()
    }

    pub fn raster_pass(&self, index: usize) -> &Region {
        self.staging.get(&index).unwrap_or_else(|| self.pass(index))
    }

    pub fn pass(&self, index: usize) -> &Region {
        &self.passes[index]
    }

    /// Seed the last write of each persistent output, then propagate reads
    /// backward. The caller supplies the pixels that output must replace.
    pub fn update(
        &mut self,
        frame: &Frame,
        plan: &ExecutionPlan,
        mut output: impl FnMut(usize, Target, [u32; 4], &mut Region),
    ) {
        self.reset(frame);
        for (index, pass) in frame.passes.iter().enumerate().rev() {
            if matches!(
                pass.target,
                Target::Part(_) | Target::Plane(_) | Target::Projected(_)
            ) && !frame.passes[index + 1..]
                .iter()
                .any(|later| later.target == pass.target)
            {
                output(index, pass.target, pass.region, &mut self.passes[index]);
            }
        }
        for index in (0..frame.passes.len()).rev() {
            if self.passes[index].is_empty() {
                continue;
            }
            // Enclose before following reads: the pixels between separate
            // changes will also be rasterized and need valid input images.
            add(&mut self.passes[index], [0; 4]);
            // The filter/shadow executors currently produce their whole
            // bounded image. Demand their entire input until their kernels
            // accept a restricted output and its sampling apron.
            if frame.filters.iter().any(|(pass, _)| *pass == index)
                || frame.shadows.iter().any(|(pass, _)| *pass == index)
                || frame.reduces.iter().any(|(pass, _)| *pass == index)
            {
                let [x, y, w, h] = frame.passes[index].region;
                add(&mut self.passes[index], [x, y, w, h]);
            }
            if let Some(read) = &plan.loads[index] {
                self.read(index, read, false, frame);
            }
            if let Some(read) = &plan.backdrops[index] {
                self.read(index, read, false, frame);
            }
            if let Some(read) = &plan.captures[index] {
                let scaled = frame.passes[index]
                    .capture
                    .is_some_and(|capture| capture.resolve.is_some());
                self.read(index, read, scaled, frame);
            }
            for (read, range) in plan.reads[index].iter().zip(&frame.passes[index].ranges) {
                if let Some(read) = read {
                    let whole = read.kind != ReadKind::CorrespondingPixel
                        || !super::plan::corresponds(
                            frame,
                            range,
                            plan.images[read.image.0].region,
                        );
                    self.read(index, read, whole, frame);
                }
            }
        }
        self.prepare_staging(frame);
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "validated capture scale maps clipped target pixels into the device extent"
    )]
    fn prepare_staging(&mut self, frame: &Frame) {
        self.staging.retain(|index, _| {
            frame.passes.get(*index).is_some_and(|pass| {
                crate::render::resolve::of(pass).is_some() && crate::render::resolve::staged(pass)
            })
        });
        for (index, pass) in frame.passes.iter().enumerate() {
            let Some(resolve) =
                crate::render::resolve::of(pass).filter(|_| crate::render::resolve::staged(pass))
            else {
                continue;
            };
            let extent = resolve.extent.into();
            let region = self
                .staging
                .entry(index)
                .or_insert_with(|| Region::new(extent));
            if region.extent() == extent {
                region.clear();
            } else {
                *region = Region::new(extent);
            }
            let scale = f64::from(resolve.scale);
            for rect in self.passes[index].rectangles() {
                let x0 = (f64::from(rect.x0()) / scale).floor() as u32;
                let y0 = (f64::from(rect.y0()) / scale).floor() as u32;
                let x1 = (f64::from(rect.x1()) / scale).ceil() as u32;
                let y1 = (f64::from(rect.y1()) / scale).ceil() as u32;
                region.add(x0, y0, x1 - x0, y1 - y0);
            }
        }
    }

    fn reset(&mut self, frame: &Frame) {
        self.passes.truncate(frame.passes.len());
        for (index, pass) in frame.passes.iter().enumerate() {
            let [x, y, w, h] = pass.region;
            let extent = (x + w, y + h);
            if let Some(region) = self.passes.get_mut(index) {
                if region.extent() == extent {
                    region.clear();
                } else {
                    *region = Region::new(extent);
                }
            } else {
                self.passes.push(Region::new(extent));
            }
        }
    }

    fn read(&mut self, index: usize, read: &Read, whole: bool, frame: &Frame) {
        let Some(writer) = read.writer else {
            // Retained projected images are already valid on entry.
            return;
        };
        assert!(writer < index, "a read must name an earlier writer");
        let (before, after) = self.passes.split_at_mut(index);
        let destination = &mut before[writer];
        let [x, y, w, h] = frame.passes[writer].region;
        if whole {
            add(destination, [x, y, w, h]);
        } else {
            for rect in after[0].rectangles() {
                let x0 = rect.x0().max(x);
                let y0 = rect.y0().max(y);
                let x1 = rect.x1().min(x + w);
                let y1 = rect.y1().min(y + h);
                if x0 < x1 && y0 < y1 {
                    add(destination, [x0, y0, x1 - x0, y1 - y0]);
                }
            }
        }
    }
}

/// Forward changes through ordered image writes. Unlike backward demand, this
/// records what changed, not all inputs needed to reproduce those pixels.
#[derive(Default)]
pub struct Changes {
    regions: Demand,
}

impl Changes {
    pub fn bytes(&self) -> u64 {
        self.regions.bytes()
    }

    pub fn pass(&self, index: usize) -> &Region {
        self.regions.pass(index)
    }

    pub fn update(
        &mut self,
        frame: &Frame,
        plan: &ExecutionPlan,
        mut changed: impl FnMut(usize, &mut Region),
    ) {
        self.regions.reset(frame);
        for index in 0..frame.passes.len() {
            changed(index, &mut self.regions.passes[index]);
            add(&mut self.regions.passes[index], [0; 4]);
            for read in [&plan.loads[index], &plan.backdrops[index]]
                .into_iter()
                .flatten()
            {
                self.read(index, read, false, frame);
            }
            if let Some(read) = &plan.captures[index] {
                let scaled = frame.passes[index]
                    .capture
                    .is_some_and(|capture| capture.resolve.is_some());
                self.read(index, read, scaled, frame);
            }
            for (read, range) in plan.reads[index].iter().zip(&frame.passes[index].ranges) {
                if let Some(read) = read {
                    let whole = read.kind != ReadKind::CorrespondingPixel
                        || !super::plan::corresponds(
                            frame,
                            range,
                            plan.images[read.image.0].region,
                        );
                    self.read(index, read, whole, frame);
                }
            }
            if !self.pass(index).is_empty()
                && (frame.filters.iter().any(|(pass, _)| *pass == index)
                    || frame.shadows.iter().any(|(pass, _)| *pass == index)
                    || frame.reduces.iter().any(|(pass, _)| *pass == index))
            {
                let [x, y, w, h] = frame.passes[index].region;
                add(&mut self.regions.passes[index], [x, y, w, h]);
            }
        }
    }

    fn read(&mut self, index: usize, read: &Read, whole: bool, frame: &Frame) {
        let Some(writer) = read.writer else {
            // Changes to externally retained images are seeded by their owner.
            return;
        };
        assert!(writer < index, "a read must name an earlier writer");
        let (before, after) = self.regions.passes.split_at_mut(index);
        let source = &before[writer];
        let destination = &mut after[0];
        let [x, y, w, h] = frame.passes[index].region;
        if whole {
            if !source.is_empty() {
                add(destination, [x, y, w, h]);
            }
        } else {
            for rect in source.rectangles() {
                let x0 = rect.x0().max(x);
                let y0 = rect.y0().max(y);
                let x1 = rect.x1().min(x + w);
                let y1 = rect.y1().min(y + h);
                if x0 < x1 && y0 < y1 {
                    add(destination, [x0, y0, x1 - x0, y1 - y0]);
                }
            }
        }
    }
}
