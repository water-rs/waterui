//! The platform-backed text layout.

use std::fmt;
use std::sync::Arc;
use waterui_graphics::draw::TextLayoutId;

use waterui_graphics::draw::kurbo::Rect;

use crate::hwui::HwuiError;

use super::ids::TextLayoutIds;
use super::index::Utf16Index;
use super::provider::PlatformText;
use super::wire::{ShapeRequest, ShapedMetrics, decode_reply};
use crate::text::types::{Affinity, CARET_WIDTH, LineMetrics, TextPosition, TextSelection};

/// A text layout shaped by the platform.
///
/// Its metrics come from the one shaping reply; caret, hit-test, selection
/// and navigation queries go to the live platform layout. Clones share the
/// platform layout, and dropping the last one queues its release into the
/// next command buffer.
///
/// The methods answer the seam's `TextLayout` one for one, in its byte
/// indices. A query the platform refuses is a broken invariant of the
/// provider — its layout is live as long as this value — and panics
/// naming the layout.
pub struct HwuiTextLayout<P: PlatformText>(Arc<Inner<P>>);

struct Inner<P: PlatformText> {
    platform: Option<Platform<P>>,
    index: Utf16Index,
    metrics: ShapedMetrics,
}

struct Platform<P: PlatformText> {
    id: u32,
    provider: Arc<P>,
    ids: TextLayoutIds,
}

impl<P: PlatformText> Clone for HwuiTextLayout<P> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<P: PlatformText> fmt::Debug for HwuiTextLayout<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HwuiTextLayout")
            .field("id", &self.platform_id())
            .field("metrics", &self.0.metrics)
            .finish_non_exhaustive()
    }
}

impl<P: PlatformText> Drop for Platform<P> {
    fn drop(&mut self) {
        self.ids.release(self.id);
    }
}

impl<P: PlatformText> HwuiTextLayout<P> {
    /// The layout of no text: no lines, no platform layout.
    #[must_use]
    pub fn empty() -> Self {
        Self(Arc::new(Inner {
            platform: None,
            index: Utf16Index::empty(),
            metrics: ShapedMetrics::empty(),
        }))
    }

    /// Shapes `request` on the platform in one call, under a fresh id from
    /// `ids`.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when the request does not pack, the platform
    /// refuses it, or its reply is malformed; [`HwuiError::IdsExhausted`]
    /// when every layout id is live.
    pub fn shape(
        provider: &Arc<P>,
        ids: &TextLayoutIds,
        request: &ShapeRequest<'_>,
    ) -> Result<Self, HwuiError> {
        let index = Utf16Index::new(request.text)?;
        let packed = request.pack(&index)?;
        let id = ids.acquire()?;
        let reply = match provider.shape(id, request.text, &packed) {
            Ok(reply) => reply,
            Err(error) => {
                ids.forget(id);
                return Err(error);
            }
        };
        let platform = Platform {
            id,
            provider: Arc::clone(provider),
            ids: ids.clone(),
        };
        let metrics = decode_reply(&reply)?;
        ids.register(id, metrics.bounds());
        Ok(Self(Arc::new(Inner {
            platform: Some(platform),
            index,
            metrics,
        })))
    }

    /// The platform layout id a frame's `Text` op names; `None` for the
    /// empty layout, which draws nothing.
    #[must_use]
    pub fn platform_id(&self) -> Option<u32> {
        self.0.platform.as_ref().map(|platform| platform.id)
    }

    /// The id `Draw::text` names this layout by; `None` for the empty
    /// layout, which draws nothing.
    #[must_use]
    pub fn layout_id(&self) -> Option<TextLayoutId> {
        self.platform_id()
            .map(|id| TextLayoutId::new(u64::from(id)))
    }

    /// The box this layout's node draws in, its ink included.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.0.metrics.bounds()
    }

    /// The number of lines.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.0.metrics.lines.len()
    }

    /// Line `line`'s metrics.
    ///
    /// # Panics
    ///
    /// When `line` is not below [`HwuiTextLayout::line_count`].
    #[must_use]
    pub fn line_metrics(&self, line: usize) -> LineMetrics {
        self.0.metrics.lines[line]
    }

    /// The layout height.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.0.metrics.height
    }

    /// The horizontal ink extent of the first `max_lines` lines (every line
    /// for `None`); `None` when they carry no ink.
    #[must_use]
    pub fn ink_extent(&self, max_lines: Option<usize>) -> Option<(f32, f32)> {
        let ink = &self.0.metrics.ink;
        ink[..max_lines.map_or(ink.len(), |lines| lines.min(ink.len()))]
            .iter()
            .flatten()
            .copied()
            .reduce(|(left, right), (l, r)| (left.min(l), right.max(r)))
    }

    /// A selection from `anchor` to `focus`, each snapped to the cluster
    /// boundary at or before it.
    #[must_use]
    pub fn selection(&self, anchor: TextPosition, focus: TextPosition) -> TextSelection {
        TextSelection {
            anchor: self.snap(anchor),
            focus: self.snap(focus),
        }
    }

    /// The position a point hits.
    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> TextPosition {
        self.ask(|provider, id| provider.hit_test(id, x, y))
            .map_or_else(
                || self.end(),
                |(offset, upstream)| self.position(offset, upstream),
            )
    }

    /// The word under a point.
    #[must_use]
    pub fn word_at(&self, x: f32, y: f32) -> TextSelection {
        self.ask(|provider, id| provider.word_at(id, x, y))
            .map_or_else(
                || TextSelection::collapsed(self.end()),
                |range| self.range(range),
            )
    }

    /// The line under a point.
    #[must_use]
    pub fn line_at(&self, x: f32, y: f32) -> TextSelection {
        self.ask(|provider, id| provider.line_at(id, x, y))
            .map_or_else(
                || TextSelection::collapsed(self.end()),
                |range| self.range(range),
            )
    }

    /// The caret rectangle at `at`, [`CARET_WIDTH`] wide.
    #[must_use]
    pub fn caret_rect(&self, at: TextPosition) -> Rect {
        let offset = self.0.index.utf16(at.index);
        self.ask(|provider, id| provider.caret_rect(id, offset, at.affinity == Affinity::Upstream))
            .map_or_else(|| Rect::new(0.0, 0.0, f64::from(CARET_WIDTH), 0.0), rect)
    }

    /// Calls `each` with every rectangle covering `selection`; a collapsed
    /// selection covers nothing.
    pub fn selection_rects(&self, selection: TextSelection, mut each: impl FnMut(Rect)) {
        if selection.is_collapsed() {
            return;
        }
        let anchor = self.0.index.utf16(selection.anchor.index);
        let focus = self.0.index.utf16(selection.focus.index);
        let (start, end) = (anchor.min(focus), anchor.max(focus));
        for bounds in self
            .ask(|provider, id| provider.selection_rects(id, start, end))
            .unwrap_or_default()
        {
            each(rect(bounds));
        }
    }

    /// The selection one cluster to the left: a non-collapsed selection
    /// collapses to its visually earlier end unless `extend`.
    #[must_use]
    pub fn previous_visual(&self, selection: TextSelection, extend: bool) -> TextSelection {
        if !selection.is_collapsed() && !extend {
            let (anchor, focus) = self.geometry(selection);
            let earlier = if (anchor.y0, anchor.x0) < (focus.y0, focus.x0) {
                selection.anchor
            } else {
                selection.focus
            };
            return TextSelection::collapsed(earlier);
        }
        let offset = self.0.index.utf16(selection.focus.index);
        let moved = self.ask(|provider, id| provider.previous_visual(id, offset));
        self.moved(selection, moved, extend)
    }

    /// The selection one cluster to the right: a non-collapsed selection
    /// collapses to its visually later end unless `extend`.
    #[must_use]
    pub fn next_visual(&self, selection: TextSelection, extend: bool) -> TextSelection {
        if !selection.is_collapsed() && !extend {
            let (anchor, focus) = self.geometry(selection);
            let later = if (anchor.y0, anchor.x0) > (focus.y0, focus.x0) {
                selection.anchor
            } else {
                selection.focus
            };
            return TextSelection::collapsed(later);
        }
        let offset = self.0.index.utf16(selection.focus.index);
        let moved = self.ask(|provider, id| provider.next_visual(id, offset));
        self.moved(selection, moved, extend)
    }

    /// Runs a query on the live platform layout; `None` for the empty
    /// layout.
    fn ask<R>(&self, query: impl FnOnce(&P, u32) -> Result<R, HwuiError>) -> Option<R> {
        let platform = self.0.platform.as_ref()?;
        Some(
            query(&*platform.provider, platform.id)
                .unwrap_or_else(|error| panic!("HWUI text layout {}: {error}", platform.id)),
        )
    }

    /// The byte offset of a platform offset; one outside the text is a
    /// broken invariant of the provider.
    fn byte(&self, offset: i32) -> usize {
        self.0.index.byte(offset).unwrap_or_else(|error| {
            let id = self
                .platform_id()
                .map_or_else(|| "(empty)".to_owned(), |id| id.to_string());
            panic!("HWUI text layout {id}: {error}")
        })
    }

    fn snap(&self, at: TextPosition) -> TextPosition {
        let offset = self.0.index.utf16(at.index);
        let snapped = self
            .ask(|provider, id| provider.snap(id, offset))
            .unwrap_or(0);
        TextPosition {
            index: self.byte(snapped),
            affinity: at.affinity,
        }
    }

    fn position(&self, offset: i32, upstream: bool) -> TextPosition {
        TextPosition {
            index: self.byte(offset),
            affinity: if upstream {
                Affinity::Upstream
            } else {
                Affinity::Downstream
            },
        }
    }

    fn end(&self) -> TextPosition {
        TextPosition::in_text(self.0.index.len(), self.0.index.len())
    }

    fn range(&self, (start, end): (i32, i32)) -> TextSelection {
        TextSelection {
            anchor: self.position(start, false),
            focus: self.position(end, true),
        }
    }

    fn geometry(&self, selection: TextSelection) -> (Rect, Rect) {
        (
            self.caret_rect(selection.anchor),
            self.caret_rect(selection.focus),
        )
    }

    fn moved(&self, selection: TextSelection, moved: Option<i32>, extend: bool) -> TextSelection {
        let focus = moved.map_or(selection.focus, |offset| {
            TextPosition::in_text(self.byte(offset), self.0.index.len())
        });
        if extend {
            TextSelection {
                anchor: selection.anchor,
                focus,
            }
        } else {
            TextSelection::collapsed(focus)
        }
    }
}

fn rect([left, top, right, bottom]: [f32; 4]) -> Rect {
    Rect::new(
        f64::from(left),
        f64::from(top),
        f64::from(right),
        f64::from(bottom),
    )
}
