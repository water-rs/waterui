//! The scenario table: the layer tree each name builds and the video
//! spec each video layer produces.

use std::f64::consts::FRAC_PI_2;

use cherenkov::kurbo::{Affine, Rect, RoundedRect, Vec2};
use cherenkov::{Layer, Surface};
use cherenkov_gpu::Gpu;
use cherenkov_gpu::interop::{FrameColor, HdrMetadata};

/// The buffer formats the harness produces.
#[derive(Clone, Copy, Debug)]
pub enum Format {
    /// `Y8Cb8Cr8_420`: 8-bit NV12 or I420, decoded as BT.709 video range.
    Nv12,
    /// `YCbCr_P010`: semi-planar 10-bit in u16s, decoded as BT.2020 PQ.
    P010,
}

/// What one video layer produces.
#[derive(Clone, Copy)]
pub struct Spec {
    /// The buffer format.
    pub format: Format,
    /// `COMPOSER_OVERLAY` in the AHB usage — required for promotion.
    pub overlay: bool,
    /// Acquire through a host-signalled timeline semaphore, which the
    /// plane contract rejects with `SemaphoreAcquire` — the `in-engine`
    /// scenario's forcing mechanism.
    pub timeline: bool,
    /// The frame's decode contract.
    pub color: FrameColor,
    /// Static HDR metadata travelling with a promoted frame.
    pub hdr: HdrMetadata,
}

/// One launch-time scenario, from the intent's `scenario` string extra.
#[derive(Clone, Copy, Debug)]
pub enum Scenario {
    /// Recorded pixels and property tracks for the measured policy matrix.
    Recorded(crate::recorded::Spec),
    /// SDR NV12, promoted.
    Overlay,
    /// P010 BT.2020 PQ with HDR metadata, promoted.
    Hdr,
    /// Video under a clipped, scrolled, scaled parent, promoted with crop.
    Clipped,
    /// Video under a quarter-turned and mirrored parent, promoted with a
    /// buffer transform.
    Quarter,
    /// Video rotated 30° — inexpressible, stays in-engine.
    Rotated,
    /// Video under a rounded-rect clip — inexpressible, stays in-engine.
    Rounded,
    /// Buffer without `COMPOSER_OVERLAY` — `NoOverlayUsage`, in-engine.
    NoOverlay,
    /// Two promotable videos; the second is rejected `Budget(1)`.
    Two,
    /// The `overlay` video with a timeline acquire — `SemaphoreAcquire`,
    /// in-engine, for paired measurement.
    InEngine,
}

impl Scenario {
    /// Parses the `scenario` intent extra; unknown values run `overlay`.
    #[must_use]
    pub fn parse(name: &str) -> Self {
        if let Some(spec) = crate::recorded::Spec::parse(name) {
            return Self::Recorded(spec);
        }
        match name {
            "hdr" => Self::Hdr,
            "clipped" => Self::Clipped,
            "quarter" => Self::Quarter,
            "rotated" => Self::Rotated,
            "rounded" => Self::Rounded,
            "no-overlay" => Self::NoOverlay,
            "two" => Self::Two,
            "in-engine" => Self::InEngine,
            _ => Self::Overlay,
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
            Self::Hdr => "hdr",
            Self::Clipped => "clipped",
            Self::Quarter => "quarter",
            Self::Rotated => "rotated",
            Self::Rounded => "rounded",
            Self::NoOverlay => "no-overlay",
            Self::Two => "two",
            Self::InEngine => "in-engine",
        }
    }

    /// The producers' specs, in paint order (the second `two` video is
    /// the one the budget rejects).
    #[must_use]
    pub fn videos(self) -> Vec<Spec> {
        let nv12 = |overlay: bool| Spec {
            format: Format::Nv12,
            overlay,
            timeline: false,
            color: FrameColor::BT709_VIDEO,
            hdr: HdrMetadata::default(),
        };
        match self {
            Self::Recorded(_) => Vec::new(),
            Self::Overlay | Self::Rotated | Self::Rounded | Self::Clipped | Self::Quarter => {
                vec![nv12(true)]
            }
            Self::Hdr => vec![Spec {
                format: Format::P010,
                color: FrameColor::BT2020_PQ,
                hdr: HdrMetadata {
                    mastering: Some(cherenkov_gpu::interop::MasteringDisplay {
                        // BT.2020 primaries and D65 in CIE xy.
                        red: [0.708, 0.292],
                        green: [0.170, 0.797],
                        blue: [0.131, 0.046],
                        white: [0.3127, 0.3290],
                        max_luminance: 1000.0,
                        min_luminance: 0.0001,
                    }),
                    content_light: Some(cherenkov_gpu::interop::ContentLight {
                        max_content: 1000.0,
                        max_frame_average: 400.0,
                    }),
                },
                ..nv12(true)
            }],
            Self::NoOverlay => vec![nv12(false)],
            Self::InEngine => vec![Spec {
                timeline: true,
                ..nv12(true)
            }],
            Self::Two => vec![nv12(true), nv12(true)],
        }
    }

    /// Builds the scenario's layer tree under `surface`'s root. Returns
    /// `(videos, rest, controls)`: the video layers in `videos()` order,
    /// the parent layers, and the `controls` layer (pushed on top of
    /// everything). The caller keeps all of them — a dropped handle
    /// queues its `Remove` ahead of the ops that attach it.
    pub fn build(
        self,
        surface: &Surface<Gpu>,
        controls: cherenkov::Content,
    ) -> (Vec<Layer>, Vec<Layer>, Layer) {
        let w = f64::from(surface.size().0);
        let h = f64::from(surface.size().1);
        let root = surface.root();
        // Allocated outside the transaction: every handle has to
        // outlive the build (dropping one inside queues its `Remove`
        // ahead of the transaction's `Push` and breaks the committed
        // op stream — a dead layer kills the render thread).
        let overlay_controls = surface.layer();
        let mut videos = Vec::new();
        let mut rest = Vec::new();
        surface.update(|tx| {
            match self {
                Self::Recorded(_) => unreachable!("recorded scenarios use recorded::Scene"),
                Self::Overlay | Self::Hdr | Self::NoOverlay | Self::InEngine => {
                    let video = surface.layer();
                    let scale = ((w - 80.0) / 1920.0).min((h - 320.0) / 1080.0);
                    tx[&video].transform(
                        Affine::translate((40.0, 1080.0f64.mul_add(-scale, h) / 2.0))
                            * Affine::scale(scale),
                    );
                    tx[root].push(&video);
                    videos.push(video);
                }
                Self::Clipped => {
                    let parent = surface.layer();
                    let video = surface.layer();
                    tx[&parent]
                        .transform(Affine::translate((120.0, 320.0)) * Affine::scale(0.42))
                        .clip(Rect::new(0.0, 0.0, 1500.0, 860.0))
                        .scroll_offset(Vec2::new(160.0, 80.0));
                    tx[&parent].push(&video);
                    tx[root].push(&parent);
                    videos.push(video);
                    rest.push(parent);
                }
                Self::Quarter => {
                    let parent = surface.layer();
                    let video = surface.layer();
                    // Quarter-turn clockwise then mirror X, at uniform
                    // scale 0.45 — all plane-expressible.
                    tx[&parent].transform(
                        Affine::translate((w * 0.62, h * 0.42))
                            * Affine::rotate(FRAC_PI_2)
                            * Affine::scale_non_uniform(-0.45, 0.45),
                    );
                    tx[&parent].push(&video);
                    tx[root].push(&parent);
                    videos.push(video);
                    rest.push(parent);
                }
                Self::Rotated => {
                    let video = surface.layer();
                    tx[&video].transform(
                        Affine::translate((w * 0.52, h * 0.35))
                            * Affine::rotate(30f64.to_radians())
                            * Affine::scale(0.4),
                    );
                    tx[root].push(&video);
                    videos.push(video);
                }
                Self::Rounded => {
                    let parent = surface.layer();
                    let video = surface.layer();
                    tx[&parent]
                        .transform(Affine::translate((80.0, 240.0)))
                        .clip(RoundedRect::from_rect(
                            Rect::new(0.0, 0.0, 1400.0, 780.0),
                            48.0,
                        ));
                    tx[&video].transform(Affine::scale(0.66));
                    tx[&parent].push(&video);
                    tx[root].push(&parent);
                    videos.push(video);
                    rest.push(parent);
                }
                Self::Two => {
                    let first = surface.layer();
                    let second = surface.layer();
                    tx[&first].transform(Affine::translate((40.0, h * 0.30)) * Affine::scale(0.30));
                    tx[&second]
                        .transform(Affine::translate((40.0, h * 0.55)) * Affine::scale(0.30));
                    tx[root].push(&first);
                    tx[root].push(&second);
                    videos.push(first);
                    videos.push(second);
                }
            }
            tx[&overlay_controls].content(controls);
            tx[root].push(&overlay_controls);
        });
        (videos, rest, overlay_controls)
    }
}
