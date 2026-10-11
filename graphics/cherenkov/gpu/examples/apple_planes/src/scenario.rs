//! The scenario table: which layer tree each name builds and why
//! `in-engine` stays composited.

use cherenkov::kurbo::Affine;
use cherenkov::{Layer, LayerEdit, Surface};
use cherenkov_gpu::Gpu;

use crate::pattern::{HEIGHT, WIDTH};

/// One launch-time scenario, from the `--scenario` argument.
#[derive(Clone, Copy, Debug)]
pub enum Scenario {
    /// Recorded-content policy matrix shared with Android.
    Recorded(crate::recorded::Spec),
    /// The video alone, filling the screen width — promoted.
    Overlay,
    /// The identical video under an empty layer painted above it at
    /// reduced opacity: `TranslucentAbove` keeps it in the engine while
    /// drawing nothing, so the picture and producer cost are unchanged.
    InEngine,
}

/// The scenario names `parse` accepts, for its error message.
pub const NAMES: &str = "overlay|in-engine|static:side:count:lifetime:plane|engine|animated:side:count:lifetime:plane|engine";

impl Scenario {
    /// Parses the `--scenario` value; unknown names are an error — the
    /// harness refuses to guess.
    ///
    /// # Errors
    /// On a name outside [`NAMES`].
    pub fn parse(name: &str) -> Result<Self, String> {
        if let Some(spec) = crate::recorded::Spec::parse(name) {
            return Ok(Self::Recorded(spec));
        }
        match name {
            "overlay" => Ok(Self::Overlay),
            "in-engine" => Ok(Self::InEngine),
            other => Err(format!("unknown scenario {other:?} (expected {NAMES})")),
        }
    }

    /// The scenario's name as it appears on the log line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Recorded(spec) => {
                if spec.animated {
                    "animated"
                } else {
                    "static"
                }
            }
            Self::Overlay => "overlay",
            Self::InEngine => "in-engine",
        }
    }

    /// The video layer's placement for surface `size`: scaled to fill
    /// the surface's width, centred vertically.
    fn layout(size: (u32, u32)) -> Affine {
        let scale = f64::from(size.0) / f64::from(WIDTH);
        let y = f64::from(HEIGHT).mul_add(-scale, f64::from(size.1)) / 2.0;
        Affine::translate((0.0, y)) * Affine::scale(scale)
    }

    /// Builds the scenario's layer tree under `surface`'s root.
    ///
    /// `overlay` is one layer, scaled to the surface's full width and
    /// centred vertically. `in-engine` is the same, then one empty
    /// layer at 99% opacity pushed on top: painted after the video and
    /// not known to be opaque, it keeps the video in the engine as
    /// `TranslucentAbove` while drawing nothing itself — identical
    /// pixels, identical producer work.
    pub fn build(self, surface: &Surface<Gpu>) -> Built {
        if let Self::Recorded(spec) = self {
            return Built {
                video: None,
                rest: Vec::new(),
                recorded: Some(crate::recorded::Scene::new(surface, spec)),
            };
        }
        let video = surface.layer();
        let mut rest = Vec::new();
        if matches!(self, Self::InEngine) {
            rest.push(surface.layer());
        }
        let root = surface.root();
        surface.update(|tx| {
            tx[&video].transform(Self::layout(surface.size()));
            tx[root].push(&video);
            for shade in &rest {
                tx[shade].opacity(0.99_f32);
                tx[root].push(shade);
            }
        });
        Built {
            video: Some(video),
            rest,
            recorded: None,
        }
    }

    /// Recomputes the video layer's placement after a resize.
    pub fn relayout(self, edit: &mut LayerEdit<Gpu>, size: (u32, u32)) {
        edit.transform(Self::layout(size));
    }
}

/// A built scenario's live layers.
pub struct Built {
    /// The layer the video installs on.
    pub video: Option<Layer>,
    /// Recorded workload when no external-frame producer is used.
    pub recorded: Option<crate::recorded::Scene>,
    /// Layers the tree needs held for the run (a dropped handle queues
    /// its `Remove`): `in-engine`'s shade layer.
    pub rest: Vec<Layer>,
}
