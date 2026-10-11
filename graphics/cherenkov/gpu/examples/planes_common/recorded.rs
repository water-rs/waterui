//! Recorded-content scenarios shared by the native plane harnesses.

use std::time::Duration;

use cherenkov::kurbo::{Affine, Circle, Rect};
use cherenkov::{Curve, Draw, Layer, Surface, WorkingColor};
use cherenkov_gpu::Gpu;

/// A matrix point: buffer side, plane count, content lifetime, and motion.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// Square layer side in device pixels.
    pub side: u32,
    /// Independently captured layers.
    pub count: u32,
    /// Replace the pixels every this many host frames; zero keeps them fixed.
    pub lifetime: u64,
    /// Animate the candidate instead of unrelated engine content.
    pub animated: bool,
    /// Force identical pixels through in-engine composition.
    pub engine: bool,
}

impl Spec {
    /// `static:side:count:lifetime:plane|engine` or `animated:...`.
    /// Invalid matrix points fail before the device starts drawing.
    ///
    /// # Panics
    /// If a recognized scenario has missing, nonnumeric or invalid fields.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        let mut fields = name.split(':');
        let animated = match fields.next()? {
            "static" => false,
            "animated" => true,
            _ => return None,
        };
        let side = fields.next().expect("side").parse().expect("numeric side");
        let count = fields
            .next()
            .expect("count")
            .parse()
            .expect("numeric count");
        let lifetime = fields
            .next()
            .expect("lifetime")
            .parse()
            .expect("numeric lifetime");
        let engine = match fields.next().expect("realization") {
            "plane" => false,
            "engine" => true,
            _ => panic!("realization must be plane or engine"),
        };
        assert!(
            fields.next().is_none() && side > 0 && count > 0,
            "invalid matrix point"
        );
        Some(Self {
            side,
            count,
            lifetime,
            animated,
            engine,
        })
    }
}

/// A retained tile pattern, independent of either platform's frame producers.
pub struct Scene {
    /// The experiment's input, included in the device heartbeat.
    pub spec: Spec,
    /// Candidate identities, used to report the planner's verdicts.
    pub layers: Vec<Layer>,
    pulse: Layer,
    _shade: Option<Layer>,
    /// Host frames since construction, independent of engine submissions.
    pub frames: u64,
}

impl Scene {
    /// Build opaque tiles plus a separate engine-composited progress indicator.
    ///
    /// # Panics
    /// If the requested tiles do not fit the surface at their native size.
    #[must_use]
    pub fn new(surface: &Surface<Gpu>, spec: Spec) -> Self {
        let (width, height) = surface.size();
        assert!(
            spec.side + 64 <= width && spec.side + 160 <= height,
            "matrix point must fit the display without scaling"
        );
        let layers: Vec<_> = (0..spec.count).map(|_| surface.layer()).collect();
        let pulse = surface.layer();
        let shade = spec.engine.then(|| surface.layer());
        surface.update(|tx| {
            // The pulse is painted first, so it cannot reject a candidate
            // through TranslucentAbove. Its pixels are outside every tile.
            tx[surface.root()].push(&pulse);
            tx[&pulse].content(surface.record(|c| {
                c.fill(
                    Rect::new(0., 0., 16., 16.),
                    WorkingColor::new([0.2, 0.4, 0.8, 1.]),
                );
            }));
            for (index, layer) in layers.iter().enumerate() {
                let offset = f64::from(u32::try_from(index).expect("layer index"));
                tx[surface.root()].push(layer);
                tx[layer].transform(Affine::translate((32. + offset, 96. + offset)));
            }
            // Empty content changes no pixels. As in the Apple video
            // harness, its opacity makes promotion ineligible by the
            // normal planner contract, without a runtime renderer switch.
            if let Some(shade) = &shade {
                tx[surface.root()].push(shade);
                tx[shade].opacity(0.99_f32);
            }
        });
        let scene = Self {
            spec,
            layers,
            pulse,
            _shade: shade,
            frames: 0,
        };
        scene.record(surface);
        scene
    }

    fn record(&self, surface: &Surface<Gpu>) {
        let side = f64::from(self.spec.side);
        let generation = self.frames.checked_div(self.spec.lifetime).unwrap_or(0);
        let phase = if generation.is_multiple_of(2) {
            0.0
        } else {
            0.1
        };
        surface.update(|tx| {
            for layer in &self.layers {
                tx[layer].content(surface.record(|c| {
                    c.fill(
                        Rect::new(0., 0., side, side),
                        WorkingColor::new([0.02, 0.03, 0.05, 1.]),
                    );
                    for y in 0..16 {
                        for x in 0..16 {
                            let cell = side / 16.;
                            c.fill(
                                Circle::new(
                                    ((f64::from(x) + 0.5) * cell, (f64::from(y) + 0.5) * cell),
                                    cell * 0.4,
                                ),
                                WorkingColor::new([0.15 + phase, 0.5, 0.3, 1.]),
                            );
                        }
                    }
                }));
            }
        });
    }

    /// Advance the controlled workload. Native animation starts after the
    /// observation frames and runs through a full settled measurement window.
    ///
    /// # Panics
    /// If the layer count exceeds the harness's u32 index range.
    pub fn tick(&mut self, surface: &Surface<Gpu>) {
        self.frames += 1;
        if self.spec.lifetime != 0 && self.frames.is_multiple_of(self.spec.lifetime) {
            self.record(surface);
        }
        if !self.spec.animated || self.frames <= 4 {
            let x = f64::from(u32::try_from(self.frames % 32).expect("bounded position"));
            surface.update(|tx| {
                tx[&self.pulse].content(surface.record(|c| {
                    c.fill(
                        Rect::new(x, 32., x + 16., 48.),
                        WorkingColor::new([0.2, 0.4, 0.8, 1.]),
                    );
                }));
            });
        }
        if self.spec.animated && self.frames == 4 {
            surface.update_animated(Curve::linear(Duration::from_secs(40)), |tx| {
                for (index, layer) in self.layers.iter().enumerate() {
                    let offset = f64::from(u32::try_from(index).expect("layer index"));
                    tx[layer].transform(Affine::translate((56. + offset, 120. + offset)));
                }
            });
        }
    }
}
