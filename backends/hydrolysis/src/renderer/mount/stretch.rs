//! The overscroll stretch a scroll node's inner layer draws with: HWUI's
//! stretch shader (`frameworks/base/libs/hwui/effects/StretchEffect.cpp`)
//! as filtrate's [`OverscrollStretch`], the inner layer's filter while the
//! stretch shows.
//!
//! HWUI evaluates the shader in the stretched node's own space, the
//! viewport's `[0, width] × [0, height]`. A layer filter runs on the
//! layer's device pixels, so the filter's frame is the viewport's own space
//! carried to device pixels: the window's display transform, the scroll
//! node's resolved placement and the viewport's origin.
//!
//! One filter is registered per inner layer, on its first stretch, and is
//! kept while the layer lives: each later stretch only rewrites its
//! parameters, so a pull never waits for the filter's pipeline to build.
//! At rest the layer carries no filter, as a node HWUI draws without a
//! stretch is not composed through the shader.

use cherenkov::{Filter, FilterId};
use nami::Binding;
use waterui_graphics::filtrate::filters::OverscrollStretch;
use waterui_graphics::{ParamGuards, Reactive};

use super::target::LayerTarget;

/// A stretch the inner layer draws: its viewport in the scroll node's
/// space and HWUI's stretch direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ShownStretch {
    pub viewport: kurbo::Rect,
    pub direction: kurbo::Vec2,
}

impl ShownStretch {
    /// The filter's parameters when the scroll node's space maps to device
    /// pixels through `world`: the frame `world · translate(origin)`, the
    /// viewport's extent and the direction, in [`OverscrollStretch`]'s
    /// flattened order.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "filtrate parameters are f32"
    )]
    fn params(self, world: kurbo::Affine) -> [f32; 10] {
        let frame = world * kurbo::Affine::translate(self.viewport.origin().to_vec2());
        let [xx, xy, yx, yy, tx, ty] = frame.as_coeffs();
        [
            xx,
            xy,
            yx,
            yy,
            tx,
            ty,
            self.viewport.width(),
            self.viewport.height(),
            self.direction.x,
            self.direction.y,
        ]
        .map(|value| value as f32)
    }
}

/// An inner layer's registered stretch filter and the parameter slots that
/// feed it.
pub(super) struct StretchFilter {
    /// The slots, in [`OverscrollStretch`]'s flattened order.
    slots: [Binding<f32>; 10],
    /// The values last written to `slots`.
    written: [f32; 10],
    /// The stretch the parameters describe and the transform they were
    /// carried through — what a commit that does not re-lower the node
    /// re-resolves.
    shown: Option<(ShownStretch, kurbo::Affine)>,
    filter: Filter,
    _guards: ParamGuards,
}

impl std::fmt::Debug for StretchFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StretchFilter")
            .field("written", &self.written)
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

impl StretchFilter {
    /// Registers the filter for `shown` under `world`.
    pub(super) fn new<T: LayerTarget>(
        host: &T::Host,
        shown: ShownStretch,
        world: kurbo::Affine,
    ) -> Self {
        let written = shown.params(world);
        let slots = written.map(Binding::container);
        let mut guards = ParamGuards::default();
        let params: [Reactive; 10] = std::array::from_fn(|index| guards.bind(slots[index].clone()));
        let [xx, xy, yx, yy, tx, ty, width, height, stretch_x, stretch_y] = params;
        let filter = T::stretch_filter(
            host,
            OverscrollStretch {
                frame: [xx, xy, yx, yy, tx, ty],
                size: [width, height],
                stretch: [stretch_x, stretch_y],
            },
        );
        Self {
            slots,
            written,
            shown: Some((shown, world)),
            filter,
            _guards: guards,
        }
    }

    /// Rewrites the parameters for `shown` under `world`; only the slots
    /// whose value changed notify the engine.
    pub(super) fn show(&mut self, shown: ShownStretch, world: kurbo::Affine) {
        self.shown = Some((shown, world));
        let params = shown.params(world);
        for ((slot, written), value) in self.slots.iter().zip(&mut self.written).zip(params) {
            if written.to_bits() != value.to_bits() {
                *written = value;
                slot.set(value);
            }
        }
    }

    /// Re-resolves the shown stretch under `world`, the scroll node's
    /// device transform at a commit that does not re-lower it; returns the
    /// filter to draw with, `None` at rest.
    pub(super) fn follow(&mut self, world: kurbo::Affine) -> Option<FilterId> {
        let (shown, last) = self.shown?;
        if last != world {
            self.show(shown, world);
        }
        Some(self.filter.id())
    }

    /// Marks the stretch at rest: the layer draws without the filter.
    pub(super) const fn rest(&mut self) {
        self.shown = None;
    }

    pub(super) fn id(&self) -> FilterId {
        self.filter.id()
    }

    /// The parameters last written, in [`OverscrollStretch`]'s order.
    #[cfg(test)]
    pub(super) const fn written(&self) -> [f32; 10] {
        self.written
    }
}

#[cfg(test)]
mod tests {
    use super::ShownStretch;

    /// The frame carries the viewport's own space — origin at its top-left
    /// corner — through the device transform.
    #[test]
    fn the_frame_maps_the_viewports_own_space_to_device_pixels() {
        let shown = ShownStretch {
            viewport: kurbo::Rect::new(10.0, 20.0, 110.0, 220.0),
            direction: kurbo::Vec2::new(0.0, -0.02),
        };
        let world = kurbo::Affine::scale(2.0) * kurbo::Affine::translate((5.0, 7.0));
        assert_eq!(
            shown.params(world),
            [2.0, 0.0, 0.0, 2.0, 30.0, 54.0, 100.0, 200.0, 0.0, -0.02]
        );
    }
}
