//! Visitors over a filter's reactive parameters, and the offsetting
//! wrappers [`Chain`](crate::Chain) uses to place its second half.

use crate::{
    AuxImage, ColorStage, FilterParam, ImageVisitor, Placed, SpatialStage, StageCollector,
};

/// Sink for [`Filter::visit_signals`](crate::Filter::visit_signals).
pub trait SignalVisitor {
    /// Visits the parameter at `param_index` in the flattened
    /// [`Filter::Params`](crate::Filter::Params).
    fn visit<P: FilterParam + ?Sized>(&mut self, param_index: usize, param: &P);
}

/// Shifts every visited parameter index by `base`.
pub struct OffsetVisitor<'a, V: SignalVisitor + ?Sized> {
    inner: &'a mut V,
    base: usize,
}

impl<'a, V: SignalVisitor + ?Sized> OffsetVisitor<'a, V> {
    pub(crate) const fn new(inner: &'a mut V, base: usize) -> Self {
        Self { inner, base }
    }
}

impl<V: SignalVisitor + ?Sized> SignalVisitor for OffsetVisitor<'_, V> {
    fn visit<P: FilterParam + ?Sized>(&mut self, param_index: usize, param: &P) {
        self.inner.visit(self.base + param_index, param);
    }
}

/// Shifts every visited image index by `base`.
pub struct OffsetImages<'a, V: ImageVisitor + ?Sized> {
    inner: &'a mut V,
    base: usize,
}

impl<'a, V: ImageVisitor + ?Sized> OffsetImages<'a, V> {
    pub(crate) const fn new(inner: &'a mut V, base: usize) -> Self {
        Self { inner, base }
    }
}

impl<V: ImageVisitor + ?Sized> ImageVisitor for OffsetImages<'_, V> {
    fn visit<I: AuxImage + ?Sized>(&mut self, index: usize, image: &I) {
        self.inner.visit(self.base + index, image);
    }
}

/// Places every collected stage after `params` parameters and `images`
/// images.
pub struct OffsetCollector<'a, C: StageCollector + ?Sized> {
    inner: &'a mut C,
    params: usize,
    images: usize,
}

impl<'a, C: StageCollector + ?Sized> OffsetCollector<'a, C> {
    pub(crate) const fn new(inner: &'a mut C, params: usize, images: usize) -> Self {
        Self {
            inner,
            params,
            images,
        }
    }
}

impl<C: StageCollector + ?Sized> StageCollector for OffsetCollector<'_, C> {
    fn color(&mut self, stage: Placed<ColorStage>) {
        self.inner.color(stage.shifted(self.params, self.images));
    }

    fn spatial(&mut self, stage: Placed<SpatialStage>) {
        self.inner.spatial(stage.shifted(self.params, self.images));
    }
}
