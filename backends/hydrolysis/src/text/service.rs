//! The text service: the engine plus the caches the render path and layout
//! measurement share.
//!
//! Shaping is by far the heaviest part of measuring a text leaf — tens of
//! microseconds against the tens of nanoseconds every other leaf costs — so
//! it is the one measure that genuinely must be cached. The service owns that
//! layout cache plus the encoded glyph-scene cache; the engine it was built
//! with owns every cache that keys on the engine's own types.

use std::sync::{Arc, Mutex};

use lru::LruCache;
use waterui_core::layout::{Size as LayoutSize, VerticalAlignment, ViewDimensions};

use crate::renderer::{FrameWorkCounters, Recording};

use super::engine::{TailMark, TextEngine, TextLayout};
use super::input::{ResolvedTextLayoutInput, TextLayoutCacheKey};

/// Upper bound on retained shaped layouts.
///
/// Shaping is keyed by content *and* fitted width, so one text leaf mints an
/// entry per distinct proposal a container probes it with, and reactive text — a
/// clock, a counter, a field's value — mints one per distinct string. Unbounded,
/// the cache grows for the life of the process. This holds several times a dense
/// screen's working set, so steady-state UI still never misses, while a long
/// session evicts what it has stopped drawing.
const TEXT_LAYOUT_CACHE_CAPACITY: usize = 4096;

/// Cache identity for an encoded glyph scene: the shaped layout it draws plus
/// the line limit and tail truncation applied while drawing.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextSceneCacheKey {
    layout: TextLayoutCacheKey,
    max_lines: Option<usize>,
    tail_ellipsis: bool,
}

/// Thread-safe text shaping service shared by the render path and layout
/// measurement. The engine does the shaping; the service caches it.
pub struct TextService<E: TextEngine> {
    /// The session's text engine — a shared handle the service was built
    /// with.
    engine: E,
    /// Shared layout cache — one source of truth for the render path and
    /// measurement. Bounded at [`TEXT_LAYOUT_CACHE_CAPACITY`], evicting
    /// least-recently-shaped entries. Layouts are cheap handles, so a hit
    /// hands out a clone instead of copying the glyph runs.
    layouts: Mutex<LruCache<TextLayoutCacheKey, E::Layout>>,
    /// Encoded glyph scenes, keyed by the shaped layout's identity plus the
    /// draw-time line limit. The render path redraws every text leaf on every
    /// frame (whole-scene re-encode), and re-emitting glyph runs — font
    /// resolution, per-glyph iteration, run encoding — dominates a text leaf's
    /// flush cost. A fragment is encoded once at the local origin and appended
    /// under the frame's transform, so a scrolled or animated frame pays one
    /// encoding copy per text instead of a full glyph-run walk.
    scenes: Mutex<LruCache<TextSceneCacheKey, Arc<Recording>>>,
}

impl<E: TextEngine> TextService<E> {
    pub(crate) fn new(engine: E) -> Self {
        Self {
            engine,
            layouts: Mutex::new(LruCache::new(
                core::num::NonZeroUsize::new(TEXT_LAYOUT_CACHE_CAPACITY)
                    .expect("text layout cache capacity must be non-zero"),
            )),
            scenes: Mutex::new(LruCache::new(
                core::num::NonZeroUsize::new(TEXT_LAYOUT_CACHE_CAPACITY)
                    .expect("text scene cache capacity must be non-zero"),
            )),
        }
    }

    /// Shape `input` into a layout, reusing the shared cache.
    ///
    /// The layout is shared rather than copied: every consumer only reads it,
    /// and copying glyph runs on every probe is the bulk of a cache hit's cost.
    pub(crate) fn shape(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
    ) -> E::Layout {
        if input.is_empty() {
            return self.engine.empty_layout();
        }
        self.shape_cached(input, max_width)
    }

    /// Shape `input` as an editable field's content. Unlike [`Self::shape`],
    /// an empty input lays out the one empty line a caret in it occupies, in
    /// the input's default style, so an empty field measures as tall as a
    /// filled one.
    pub(crate) fn shape_editable(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
    ) -> E::Layout {
        // `shape` answers an empty input without the cache, so an empty
        // input's key only ever holds its editable line.
        self.shape_cached(input, max_width)
    }

    fn shape_cached(&self, input: &ResolvedTextLayoutInput, max_width: Option<f32>) -> E::Layout {
        let cache_key = input.cache_key(max_width);
        if let Some(layout) = self
            .layouts
            .lock()
            .expect("text layout cache mutex must not be poisoned")
            .get(&cache_key)
        {
            return layout.clone();
        }

        let layout = self.engine.shape(input, max_width);

        self.layouts
            .lock()
            .expect("text layout cache mutex must not be poisoned")
            .put(cache_key, layout.clone());
        layout
    }

    /// Shape `input` with at most `max_lines` laid-out lines, truncating the
    /// last allowed line to a trailing ellipsis when the text does not fit.
    /// The truncated line is respelled — its kept clusters plus the marker —
    /// and re-shaped, so the layout's widest line is the honest laid-out
    /// width the leaf reports, and the drawn line is cached and encoded
    /// through the same paths as any other.
    pub(crate) fn shape_limited(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        max_lines: Option<usize>,
    ) -> E::Layout {
        let layout = self.shape(input, max_width);
        let Some(limit) = max_lines.filter(|limit| *limit > 0) else {
            return layout;
        };
        let limit = core::num::NonZeroUsize::new(limit)
            .expect("a filtered non-zero line limit must be non-zero");
        self.engine
            .truncate_tail(layout, input, max_width, limit, |i, w| self.shape(i, w))
    }

    /// Horizontal ink extent of `layout` at the draw-time line limit — what
    /// the measured-frame contract widens by and the record path shifts by.
    pub(crate) fn ink_extent(
        &self,
        layout: &E::Layout,
        max_lines: Option<usize>,
    ) -> Option<(f32, f32)> {
        self.engine.ink_extent(layout, max_lines)
    }

    /// Compute view dimensions (size plus first/last baselines) from a shaped
    /// layout. Pure; safe to call on any thread.
    pub(crate) fn dimensions(
        &self,
        layout: &E::Layout,
        max_lines: Option<usize>,
    ) -> ViewDimensions {
        let line_count = layout.line_count();
        if line_count == 0 {
            return ViewDimensions::new(LayoutSize::zero());
        }

        let mut width = 0.0_f32;
        let mut height = 0.0_f32;
        let mut first_baseline = None;
        let mut last_baseline = None;

        for index in 0..line_count {
            if max_lines.is_some_and(|limit| index >= limit) {
                break;
            }
            let metrics = layout.line_metrics(index);
            width = width.max(metrics.advance);
            height += metrics.line_height;
            if first_baseline.is_none() {
                first_baseline = Some(metrics.baseline);
            }
            last_baseline = Some(metrics.baseline);
        }

        // Advance is the pen distance; ink may extend past it on either edge.
        // The frame covers `[-ink_min, ink_max]` when ink overhangs so the
        // record path's matching shift lands the painted extent inside the
        // view rect.
        if let Some((ink_min, ink_max)) = self.engine.ink_extent(layout, max_lines) {
            width = width.max(ink_max) - ink_min.min(0.0);
        }

        let mut dimensions = ViewDimensions::new(LayoutSize::new(width, height));
        if let Some(first_baseline) = first_baseline {
            dimensions.set_vertical(VerticalAlignment::FirstBaseline, first_baseline);
        }
        if let Some(last_baseline) = last_baseline {
            dimensions.set_vertical(VerticalAlignment::LastBaseline, last_baseline);
        }
        dimensions
    }

    /// The encoded glyph scene for `input` at `max_width`, drawn with its
    /// `tail` treatment, at the local origin (identity transform). A miss
    /// shapes through [`Self::shape`] — or [`Self::shape_limited`] for
    /// [`TailMark::Ellipsis`] — and records once; a hit returns the shared
    /// fragment so the caller only pays a transformed append into the frame's
    /// scene. A truncated layout carries its respelled background spans, so
    /// recording reads the backgrounds the drawn text was actually shaped
    /// with.
    pub(crate) fn record(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        tail: TailMark,
        counters: &mut FrameWorkCounters,
    ) -> Arc<Recording> {
        let (max_lines, tail_ellipsis) = tail.parts();
        let key = TextSceneCacheKey {
            layout: input.cache_key(max_width),
            max_lines,
            tail_ellipsis,
        };
        if let Some(scene) = self
            .scenes
            .lock()
            .expect("text scene cache mutex must not be poisoned")
            .get(&key)
        {
            return Arc::clone(scene);
        }
        let layout = if tail_ellipsis {
            self.shape_limited(input, max_width, max_lines)
        } else {
            self.shape(input, max_width)
        };
        let mut scene = Recording::new();
        if layout.line_count() > 0 {
            let x_shift = self
                .engine
                .ink_extent(&layout, max_lines)
                .map_or(0.0, |(ink_min, _)| -ink_min.min(0.0));
            self.engine
                .record(&layout, max_lines, x_shift, &mut scene, counters);
        }
        let scene = Arc::new(scene);
        self.scenes
            .lock()
            .expect("text scene cache mutex must not be poisoned")
            .put(key, Arc::clone(&scene));
        scene
    }
}

impl<E: TextEngine> core::fmt::Debug for TextService<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextService").finish_non_exhaustive()
    }
}
