//! Frame-timing attribution for the shared `cherenkov` front end.
//!
//! GPU timings are returned at the end of the measured window, so compact
//! frame spans trace each [`FrameTiming`] back to its bench frame.

use std::cmp::Ordering;

use cherenkov::{Backend, Engine, FrameStats, FrameTiming, Next, RenderError};

use crate::motion::Clock;
use crate::{BenchError, GpuSample, PassSample};

/// The most renders [`Timings::render_frame`] spends letting motion come
/// to rest.
const SETTLE_FRAMES: u32 = 2000;

/// A contiguous run of engine frames mapped to contiguous bench frames.
#[derive(Debug)]
struct Span {
    first: u64,
    frame: u64,
    len: u64,
}

/// The engine frames whose GPU timings are being collected.
#[derive(Default, Debug)]
pub struct Timings {
    spans: Vec<Span>,
}

impl Timings {
    /// Renders bench frame `frame` at `clock`'s time. With `settle`,
    /// keeps rendering, advancing the clock, until the scene's motion
    /// comes to rest — FLIP compares against the oracle's settled scene.
    /// Every render is recorded as `frame`; GPU timings are returned by
    /// [`Timings::samples`] after `finish_timings`.
    ///
    /// # Errors
    /// A render failure mapped through `render_error`, or
    /// [`BenchError::Engine`] when the motion does not settle within
    /// `SETTLE_FRAMES` renders.
    pub fn render_frame<B: Backend>(
        &mut self,
        engine: &Engine<B>,
        clock: &mut Clock,
        frame: u64,
        settle: bool,
        render_error: fn(RenderError) -> BenchError,
    ) -> Result<(), BenchError> {
        let mut next = engine.render(clock.time()).map_err(render_error)?;
        self.record(frame, &engine.stats());
        if !settle {
            return Ok(());
        }
        for _ in 0..SETTLE_FRAMES {
            if next == Next::Idle {
                return Ok(());
            }
            clock.advance();
            next = engine.render(clock.time()).map_err(render_error)?;
            self.record(frame, &engine.stats());
        }
        if next == Next::Idle {
            return Ok(());
        }
        Err(BenchError::Engine(format!(
            "motion did not settle in {SETTLE_FRAMES} frames"
        )))
    }

    /// Records the engine frame one render drew (if it drew anything) as
    /// bench frame `frame`.
    pub fn record(&mut self, frame: u64, stats: &FrameStats) {
        if let Some(id) = stats.frame {
            let first = id.get();
            if let Some(span) = self.spans.last_mut()
                && span.first + span.len == first
                && span.frame + span.len == frame
            {
                span.len += 1;
            } else {
                self.spans.push(Span {
                    first,
                    frame,
                    len: 1,
                });
            }
        }
    }

    /// Tags each resolved engine frame timing with the bench frame that
    /// rendered it.
    ///
    /// # Panics
    /// When a timing names an engine frame [`Timings::record`] never saw —
    /// the engine times only frames its renders drew.
    #[must_use]
    pub fn samples(&self, timings: Vec<FrameTiming>) -> Vec<GpuSample> {
        timings
            .into_iter()
            .map(|timing| {
                let frame_id = timing.frame.get();
                let span = self
                    .spans
                    .binary_search_by(|span| {
                        if frame_id < span.first {
                            Ordering::Greater
                        } else if frame_id >= span.first + span.len {
                            Ordering::Less
                        } else {
                            Ordering::Equal
                        }
                    })
                    .map(|index| &self.spans[index])
                    .expect("the engine times only frames its renders drew");
                GpuSample {
                    frame: span.frame + (frame_id - span.first),
                    gpu_seconds: timing.gpu_seconds,
                    passes: timing
                        .passes
                        .into_iter()
                        .map(|p| PassSample {
                            name: p.name,
                            width: p.width,
                            height: p.height,
                            format: p.format.to_string(),
                            gpu_seconds: p.gpu_seconds,
                        })
                        .collect(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Timings;
    use cherenkov::{FrameId, FrameStats, FrameTiming};

    fn record(timings: &mut Timings, bench_frame: u64, engine_frame: u64) {
        let stats = FrameStats {
            frame: Some(FrameId::new(engine_frame)),
            ..FrameStats::default()
        };
        timings.record(bench_frame, &stats);
    }

    fn timing(frame: u64) -> FrameTiming {
        FrameTiming {
            frame: FrameId::new(frame),
            gpu_seconds: None,
            passes: Vec::new(),
        }
    }

    #[test]
    fn contiguous_frames_share_a_span() {
        let mut timings = Timings::default();
        record(&mut timings, 0, 10);
        record(&mut timings, 1, 11);
        record(&mut timings, 2, 12);

        let samples = timings.samples(vec![timing(10), timing(11), timing(12)]);
        assert_eq!(
            samples
                .iter()
                .map(|sample| sample.frame)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    #[test]
    fn repeated_settle_renders_keep_the_same_bench_frame() {
        let mut timings = Timings::default();
        record(&mut timings, 3, 20);
        record(&mut timings, 3, 21);
        record(&mut timings, 3, 22);

        let samples = timings.samples(vec![timing(20), timing(21), timing(22)]);
        assert_eq!(
            samples
                .iter()
                .map(|sample| sample.frame)
                .collect::<Vec<_>>(),
            [3, 3, 3]
        );
    }
}
