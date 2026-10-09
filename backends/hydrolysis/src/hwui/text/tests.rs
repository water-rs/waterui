use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::wire::FAMILY_SEPARATOR;
use super::{
    HwuiTextLayout, PackedRequest, PlatformText, ShapeRequest, TextAlignment, TextLayoutIds,
    TextRun, Utf16Index, decode_reply, unpack_position, unpack_range,
};
use waterui_graphics::draw::TextLayoutId;
use waterui_graphics::draw::kurbo::Rect;

use crate::hwui::HwuiError;
use crate::hwui::buffer::CommandBuffer;
use crate::text::types::{Affinity, TextPosition, TextSelection};

const COMBINING_ACUTE: u16 = 0x0301;

/// A monospace stand-in for the Kotlin provider: one 20 px line, every
/// UTF-16 unit 10 px wide, a combining acute joined to the unit before it.
#[derive(Default)]
struct Mono {
    texts: Mutex<HashMap<u32, Vec<u16>>>,
    requests: Mutex<Vec<PackedRequest>>,
    refuse: bool,
}

fn px(offset: i32) -> f32 {
    f32::from(i16::try_from(offset).expect("test offsets are small")) * 10.0
}

impl Mono {
    fn units(&self, id: u32) -> Result<Vec<u16>, HwuiError> {
        self.texts
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| HwuiError::Text {
                reason: format!("no layout {id}"),
            })
    }

    fn len(&self, id: u32) -> Result<i32, HwuiError> {
        Ok(i32::try_from(self.units(id)?.len()).unwrap())
    }

    /// Whether `offset` falls inside a cluster.
    fn inside(units: &[u16], offset: i32) -> bool {
        let Ok(at) = usize::try_from(offset) else {
            return false;
        };
        at > 0
            && at < units.len()
            && (units[at] == COMBINING_ACUTE
                || char::decode_utf16([units[at - 1]])
                    .next()
                    .is_some_and(|c| c.is_err()))
    }
}

impl PlatformText for Mono {
    fn shape(&self, id: u32, text: &str, request: &PackedRequest) -> Result<Vec<f32>, HwuiError> {
        if self.refuse {
            return Err(HwuiError::Text {
                reason: "refused".to_owned(),
            });
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        let width = px(i32::try_from(units.len()).unwrap());
        self.texts.lock().unwrap().insert(id, units);
        self.requests.lock().unwrap().push(request.clone());
        let (left, right, top, bottom) = if width > 0.0 {
            (0.0, width, -1.0, 21.0)
        } else {
            (
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
            )
        };
        Ok(vec![
            width, 20.0, top, bottom, width, 20.0, 16.0, left, right,
        ])
    }

    fn caret_rect(&self, id: u32, offset: i32, _upstream: bool) -> Result<[f32; 4], HwuiError> {
        self.units(id)?;
        Ok([px(offset), 0.0, px(offset) + 1.0, 20.0])
    }

    fn hit_test(&self, id: u32, x: f32, _y: f32) -> Result<(i32, bool), HwuiError> {
        let len = self.len(id)?;
        let nearest = (0..=len)
            .min_by(|a, b| (px(*a) - x).abs().total_cmp(&(px(*b) - x).abs()))
            .unwrap();
        Ok((nearest, nearest == len))
    }

    fn word_at(&self, id: u32, _x: f32, _y: f32) -> Result<(i32, i32), HwuiError> {
        Ok((0, self.len(id)?))
    }

    fn line_at(&self, id: u32, _x: f32, _y: f32) -> Result<(i32, i32), HwuiError> {
        Ok((0, self.len(id)?))
    }

    fn snap(&self, id: u32, offset: i32) -> Result<i32, HwuiError> {
        let units = self.units(id)?;
        let mut at = offset;
        while Self::inside(&units, at) {
            at -= 1;
        }
        Ok(at)
    }

    fn selection_rects(&self, id: u32, start: i32, end: i32) -> Result<Vec<[f32; 4]>, HwuiError> {
        self.units(id)?;
        Ok(vec![[px(start), 0.0, px(end), 20.0]])
    }

    fn previous_visual(&self, id: u32, offset: i32) -> Result<i32, HwuiError> {
        let units = self.units(id)?;
        let mut at = (offset - 1).max(0);
        while Self::inside(&units, at) {
            at -= 1;
        }
        Ok(at)
    }

    fn next_visual(&self, id: u32, offset: i32) -> Result<i32, HwuiError> {
        let units = self.units(id)?;
        let len = self.len(id)?;
        let mut at = (offset + 1).min(len);
        while Self::inside(&units, at) {
            at += 1;
        }
        Ok(at)
    }
}

fn run(range: std::ops::Range<usize>) -> TextRun {
    TextRun {
        range,
        family: None,
        size: 14.0,
        weight: 400,
        italic: false,
        underline: false,
        strikethrough: false,
        foreground: None,
        background: None,
        line_height: None,
        letter_spacing: 0.0,
    }
}

fn shape(provider: &Arc<Mono>, ids: &TextLayoutIds, text: &str) -> HwuiTextLayout<Mono> {
    HwuiTextLayout::shape(
        provider,
        ids,
        &ShapeRequest::new(text, &[run(0..text.len())]),
    )
    .unwrap()
}

fn packed(request: &ShapeRequest<'_>) -> Result<PackedRequest, HwuiError> {
    request.pack(&Utf16Index::new(request.text).unwrap())
}

fn position(index: usize) -> TextPosition {
    TextPosition {
        index,
        affinity: Affinity::Downstream,
    }
}

fn releases(ids: &TextLayoutIds) -> Vec<String> {
    let mut buffer = CommandBuffer::new();
    buffer.begin_frame();
    ids.drain_releases(&mut buffer, |_| false).unwrap();
    buffer.take_log().split_off(1)
}

#[test]
fn the_index_maps_bytes_and_utf16_units_both_ways() {
    // a: byte 0, unit 0; é: bytes 1..3, unit 1; 🌊: bytes 3..7, units 2..4.
    let index = Utf16Index::new("aé🌊").unwrap();
    assert_eq!((index.len(), index.utf16_len()), (7, 4));
    let utf16: Vec<i32> = (0..=8).map(|byte| index.utf16(byte)).collect();
    assert_eq!(utf16, [0, 1, 1, 2, 2, 2, 2, 4, 4]);
    let bytes: Vec<usize> = (0..=4).map(|unit| index.byte(unit).unwrap()).collect();
    assert_eq!(bytes, [0, 1, 3, 3, 7]);
    for outside in [-1, 5] {
        let error = index.byte(outside).unwrap_err().to_string();
        assert!(
            error.contains(&format!("offset {outside} in a text of 4 units")),
            "{error}"
        );
    }
    let boundaries: Vec<bool> = (0..=8).map(|byte| index.is_boundary(byte)).collect();
    assert_eq!(
        boundaries,
        [true, true, false, true, false, false, false, true, false]
    );
}

/// The checkpoint index answers what a full per-offset table answers, on a
/// text long enough to span many checkpoints.
#[test]
fn the_index_matches_a_full_table_across_checkpoints() {
    let text: String = (0..200)
        .map(|i| ["a", "é", "\u{1f30a}", "中", "e\u{301}"][i % 5])
        .collect();
    let index = Utf16Index::new(&text).unwrap();
    let mut units = 0;
    let mut by_byte = Vec::new();
    let mut by_unit = Vec::new();
    for (byte, character) in text.char_indices() {
        by_byte.extend(std::iter::repeat_n(units, character.len_utf8()));
        by_unit.extend(std::iter::repeat_n(byte, character.len_utf16()));
        units += i32::try_from(character.len_utf16()).unwrap();
    }
    by_byte.push(units);
    by_unit.push(text.len());
    assert_eq!(index.utf16_len(), units);
    for (byte, expected) in by_byte.iter().enumerate() {
        assert_eq!(index.utf16(byte), *expected, "byte {byte}");
    }
    for (unit, expected) in (0..).zip(&by_unit) {
        assert_eq!(index.byte(unit).unwrap(), *expected, "unit {unit}");
    }
    assert!(index.byte(units + 1).is_err());
    let ascii = Utf16Index::new("plain").unwrap();
    assert_eq!((ascii.utf16(3), ascii.byte(5).unwrap()), (3, 5));
    assert!(ascii.byte(6).is_err());
}

#[test]
fn a_shape_sends_utf16_runs_and_reads_the_reply() {
    let mono = Arc::new(Mono::default());
    let ids = TextLayoutIds::new();
    let text = "é🌊a";
    let mut styled = run(0..text.len());
    styled.family = Some("Inter".to_owned());
    styled.weight = 700;
    styled.italic = true;
    styled.foreground = Some(0x1020_3040_5060_7080);
    styled.letter_spacing = 1.4;
    let runs = [styled];
    let layout = HwuiTextLayout::shape(
        &mono,
        &ids,
        &ShapeRequest {
            locale: "zh-TW",
            max_width: Some(120.0),
            max_lines: Some(2),
            ellipsis: true,
            alignment: TextAlignment::Center,
            right_to_left: true,
            strict_families: true,
            ..ShapeRequest::new(text, &runs)
        },
    )
    .unwrap();

    let request = mono.requests.lock().unwrap()[0].clone();
    let size = i32::from_ne_bytes(14.0_f32.to_bits().to_ne_bytes());
    let spacing = i32::from_ne_bytes((1.4_f32 / 14.0).to_bits().to_ne_bytes());
    assert_eq!(
        request.spans,
        [
            0,
            4,
            0,
            size,
            700,
            1 | 8,
            0x5060_7080,
            0x1020_3040,
            0,
            0,
            0,
            spacing
        ]
    );
    assert_eq!(request.families, ["Inter"]);
    assert_eq!(request.locale, "zh-TW");
    assert_eq!(
        (request.span_count, request.max_width, request.max_lines),
        (1, 120.0, 2)
    );
    // Centred, right to left, ellipsized, strict.
    assert_eq!(request.paragraph, 1 | 4 | 8 | 16);

    assert_eq!(layout.platform_id(), Some(0));
    assert_eq!(layout.line_count(), 1);
    assert_eq!(layout.height(), 20.0);
    let line = layout.line_metrics(0);
    assert_eq!(
        (line.advance, line.line_height, line.baseline),
        (40.0, 20.0, 16.0)
    );
    assert_eq!(layout.ink_extent(None), Some((0.0, 40.0)));
    assert_eq!(layout.ink_extent(Some(0)), None);
    let bounds = Rect::new(0.0, -1.0, 40.0, 21.0);
    assert_eq!(layout.bounds(), bounds);
    assert_eq!(ids.bounds(0), Some(bounds));
    assert_eq!(layout.layout_id(), Some(TextLayoutId::new(0)));
}

#[test]
fn queries_answer_in_byte_indices() {
    let provider = Arc::new(Mono::default());
    let ids = TextLayoutIds::new();
    // é: bytes 0..2, unit 0; 🌊: bytes 2..6, units 1..3; a: byte 6, unit 3.
    let layout = shape(&provider, &ids, "é🌊a");

    assert_eq!(layout.hit_test(31.0, 5.0), position(6));
    assert_eq!(
        layout.hit_test(99.0, 5.0),
        TextPosition {
            index: 7,
            affinity: Affinity::Upstream
        }
    );
    assert_eq!(layout.caret_rect(position(2)).x0, 10.0);
    assert_eq!(layout.caret_rect(position(6)).x0, 30.0);
    let word = layout.word_at(0.0, 0.0);
    assert_eq!(
        (word.anchor, word.focus.index, word.focus.affinity),
        (position(0), 7, Affinity::Upstream)
    );
    assert_eq!(layout.line_at(0.0, 0.0), word);

    let mut rects = Vec::new();
    layout.selection_rects(
        TextSelection {
            anchor: position(6),
            focus: position(2),
        },
        |rect| rects.push(rect),
    );
    assert_eq!(rects.len(), 1);
    assert_eq!((rects[0].x0, rects[0].x1), (10.0, 30.0));
    layout.selection_rects(TextSelection::collapsed(position(2)), |_| {
        panic!("a caret covers nothing")
    });
}

#[test]
fn a_selection_snaps_each_end_to_a_cluster_boundary() {
    let provider = Arc::new(Mono::default());
    let ids = TextLayoutIds::new();
    // e: byte 0; U+0301: bytes 1..3, joined to the e; x: byte 3.
    let layout = shape(&provider, &ids, "e\u{301}x");
    let selection = layout.selection(position(3), position(1));
    assert_eq!(
        selection,
        TextSelection {
            anchor: position(3),
            focus: position(0)
        }
    );
}

#[test]
fn visual_moves_follow_the_platform_and_collapse_like_the_seam() {
    let provider = Arc::new(Mono::default());
    let ids = TextLayoutIds::new();
    let layout = shape(&provider, &ids, "é🌊a");
    let caret = TextSelection::collapsed(position(6));

    assert_eq!(
        layout.previous_visual(caret, false),
        TextSelection::collapsed(position(2))
    );
    assert_eq!(
        layout.previous_visual(caret, true),
        TextSelection {
            anchor: position(6),
            focus: position(2)
        }
    );
    assert_eq!(
        layout.next_visual(caret, false),
        TextSelection::collapsed(TextPosition {
            index: 7,
            affinity: Affinity::Upstream
        })
    );
    let range = TextSelection {
        anchor: position(6),
        focus: position(2),
    };
    assert_eq!(
        layout.previous_visual(range, false),
        TextSelection::collapsed(position(2))
    );
    assert_eq!(
        layout.next_visual(range, false),
        TextSelection::collapsed(position(6))
    );
}

#[test]
fn dropping_the_last_clone_releases_through_the_next_buffer() {
    let provider = Arc::new(Mono::default());
    let ids = TextLayoutIds::new();
    let layout = shape(&provider, &ids, "abc");
    let clone = layout.clone();
    drop(layout);
    assert_eq!(releases(&ids), [] as [String; 0]);
    drop(clone);
    assert_eq!(shape(&provider, &ids, "next").platform_id(), Some(1));
    assert_eq!(
        releases(&ids),
        ["ReleaseTextLayout id=0", "ReleaseTextLayout id=1"]
    );
    assert_eq!(shape(&provider, &ids, "held back").platform_id(), Some(2));
    assert_eq!(releases(&ids), ["ReleaseTextLayout id=2"]);
    // The free list hands back the most recently recycled id first.
    assert_eq!(shape(&provider, &ids, "reused").platform_id(), Some(1));
}

#[test]
fn a_refused_shape_registers_nothing_and_queues_no_release() {
    let refusing = Arc::new(Mono {
        refuse: true,
        ..Mono::default()
    });
    let ids = TextLayoutIds::new();
    let runs = [run(0..3)];
    let request = ShapeRequest::new("abc", &runs);
    let error = HwuiTextLayout::shape(&refusing, &ids, &request).unwrap_err();
    assert!(error.to_string().contains("refused"), "{error}");
    assert_eq!(releases(&ids), [] as [String; 0]);
    assert_eq!(ids.acquire().unwrap(), 0);
}

#[test]
fn a_run_off_a_character_boundary_is_refused_before_the_platform() {
    let mono = Arc::new(Mono::default());
    let ids = TextLayoutIds::new();
    for runs in [
        vec![run(0..1)],
        vec![run(0..3)],
        vec![TextRun {
            size: 0.0,
            ..run(0..2)
        }],
    ] {
        let request = ShapeRequest::new("é", &runs);
        assert!(HwuiTextLayout::shape(&mono, &ids, &request).is_err());
    }
    assert!(mono.requests.lock().unwrap().is_empty());
    assert_eq!(ids.acquire().unwrap(), 0);
}

#[test]
fn the_empty_layout_answers_without_the_platform() {
    let layout = HwuiTextLayout::<Mono>::empty();
    let end = TextPosition {
        index: 0,
        affinity: Affinity::Upstream,
    };
    assert_eq!(
        (layout.platform_id(), layout.line_count(), layout.height()),
        (None, 0, 0.0)
    );
    assert_eq!(layout.ink_extent(None), None);
    assert_eq!(layout.hit_test(5.0, 5.0), end);
    assert_eq!(layout.word_at(5.0, 5.0), TextSelection::collapsed(end));
    assert_eq!(layout.caret_rect(end).width(), 1.0);
    assert_eq!(
        layout.previous_visual(TextSelection::collapsed(end), false),
        TextSelection::collapsed(end)
    );
}

#[test]
fn a_malformed_reply_is_refused() {
    for reply in [
        &[][..],
        &[10.0, 20.0, 0.0],
        &[f32::NAN, 20.0, 0.0, 1.0],
        &[10.0, -1.0, 0.0, 1.0],
        &[10.0, 20.0, 5.0, 1.0],
        &[10.0, 20.0, 0.0, 1.0, 1.0],
        &[
            10.0,
            20.0,
            0.0,
            1.0,
            10.0,
            20.0,
            16.0,
            0.0,
            f32::NEG_INFINITY,
        ],
        &[10.0, 20.0, 0.0, 1.0, 10.0, 20.0, 16.0, 5.0, 1.0],
        &[10.0, 20.0, 0.0, 1.0, f32::INFINITY, 20.0, 16.0, 0.0, 1.0],
    ] {
        assert!(decode_reply(reply).is_err(), "{reply:?}");
    }
    let empty = (f32::INFINITY, f32::NEG_INFINITY);
    let blank = decode_reply(&[
        0.0, 20.0, empty.0, empty.1, 0.0, 20.0, 16.0, empty.0, empty.1,
    ])
    .unwrap();
    assert_eq!((&*blank.ink, blank.vertical_ink), (&[None][..], None));
    assert_eq!(blank.bounds(), Rect::new(0.0, 0.0, 0.0, 20.0));
}

/// The same packings the Kotlin `TextWireTest` builds.
#[test]
fn positions_and_ranges_unpack_as_the_provider_packs_them() {
    assert_eq!(unpack_position(11).unwrap(), (5, true));
    assert_eq!(unpack_position(10).unwrap(), (5, false));
    assert!(unpack_position(1 << 40).is_err());
    assert_eq!(unpack_range(12_884_901_897), (3, 9));
    assert_eq!(unpack_range(0x7fff_ffff_0000_0000), (i32::MAX, 0));
}

/// CSS family lists cross as their names; quoted names stay verbatim and
/// the `system-ui`/`ui-*` generics become the Android families they name.
#[test]
fn family_lists_pack_as_their_names() {
    let text = "abc";
    let lists = [
        (
            "Roboto, 'Noto Sans CJK SC',  sans-serif",
            "Roboto|Noto Sans CJK SC|sans-serif",
        ),
        (
            "\"system-ui\", system-ui, ui-monospace",
            "system-ui|sans-serif|monospace",
        ),
        ("  Fira   Sans ,serif", "Fira Sans|serif"),
        ("'It\\'s', x", "It's|x"),
    ];
    for (css, names) in lists {
        let runs = [TextRun {
            family: Some(css.to_owned()),
            ..run(0..3)
        }];
        let request = packed(&ShapeRequest::new(text, &runs)).unwrap();
        assert_eq!(request.families.len(), 1);
        assert_eq!(
            request.families[0].replace(FAMILY_SEPARATOR, "|"),
            names,
            "{css}"
        );
    }
    let shared = TextRun {
        family: Some("Inter, serif".to_owned()),
        ..run(0..1)
    };
    let runs = [
        shared.clone(),
        TextRun {
            range: 1..2,
            family: None,
            ..shared.clone()
        },
        TextRun {
            range: 2..3,
            ..shared
        },
    ];
    let request = packed(&ShapeRequest::new(text, &runs)).unwrap();
    assert_eq!(request.families.len(), 1);
    let families: Vec<i32> = request.spans.chunks(12).map(|span| span[2]).collect();
    assert_eq!(families, [0, -1, 0]);
    for malformed in ["", "Inter,", ", serif", "'open", "Inter 'x'", "a\u{7}b"] {
        let runs = [TextRun {
            family: Some(malformed.to_owned()),
            ..run(0..3)
        }];
        let error = packed(&ShapeRequest::new(text, &runs)).unwrap_err();
        assert!(
            error.to_string().contains("font family list"),
            "{malformed:?}: {error}"
        );
    }
}

#[test]
fn line_limits_and_the_ellipsis_pack_or_are_refused() {
    let runs = [run(0..3)];
    let plain = packed(&ShapeRequest::new("abc", &runs)).unwrap();
    assert_eq!(
        (plain.max_lines, plain.paragraph, plain.locale.as_str()),
        (-1, 0, "")
    );
    let limited = packed(&ShapeRequest {
        max_lines: Some(3),
        ..ShapeRequest::new("abc", &runs)
    })
    .unwrap();
    assert_eq!((limited.max_lines, limited.paragraph), (3, 0));
    for (max_lines, ellipsis) in [(Some(0), false), (None, true)] {
        let request = ShapeRequest {
            max_lines,
            ellipsis,
            ..ShapeRequest::new("abc", &runs)
        };
        assert!(packed(&request).is_err(), "{max_lines:?} {ellipsis}");
    }
}
