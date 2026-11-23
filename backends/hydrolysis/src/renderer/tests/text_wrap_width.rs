//! A text breaks lines at the width of its own layout answer
//! (water-rs/waterui#4125): the response rectangle positions the glyphs, and
//! its corners, widened to `f64`, are never the source of the line-break
//! width. A label centred in a wide window starts hundreds of points from the
//! origin, so its far edge `x + width` rounds to a coarser float than the
//! width itself; a width recovered from the corners is then a step short of
//! the measured advance and wraps the last word.

use core::time::Duration;
use std::time::Instant;

use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

const WINDOW_HEIGHT: u32 = 120;
/// Windows wide enough that the centred label's far edge needs coarser floats
/// than its width does.
const WINDOW_WIDTHS: core::ops::RangeInclusive<u32> = 1001..=1016;
/// A size whose advances are not multiples of a power of two.
const FONT_SIZE: f64 = 13.37;
const LABELS: [&str; 3] = ["Menu Style", "Radio Style", "Fruit Basket"];

/// The number of glyph runs the settled frame records: one per laid-out line.
fn drawn_runs(label: &'static str, width: u32) -> usize {
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || AnyView::new(text(label).size(FONT_SIZE))),
        width,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );
    let mut at = Instant::now();
    for _ in 0..4 {
        at += Duration::from_millis(16);
        let _ = runtime.pump_at(false, at);
    }
    runtime.renderer().painted_scene().glyph_runs().count()
}

#[test]
fn a_text_draws_on_one_line_at_every_fractional_origin() {
    let wrapped: Vec<_> = LABELS
        .into_iter()
        .flat_map(|label| WINDOW_WIDTHS.map(move |width| (label, width)))
        .filter(|&(label, width)| drawn_runs(label, width) != 1)
        .collect();
    assert!(
        wrapped.is_empty(),
        "texts that wrapped although they fit, as (label, window width): {wrapped:?}"
    );
}

use std::sync::Mutex;

use crate::hwui::HwuiError;
use crate::hwui::text::{PackedRequest, PlatformText};

#[derive(Default)]
struct WrappingProvider {
    max_widths: Mutex<Vec<f32>>,
}

impl PlatformText for WrappingProvider {
    fn shape(&self, _id: u32, text: &str, request: &PackedRequest) -> Result<Vec<f32>, HwuiError> {
        self.max_widths.lock().unwrap().push(request.max_width());
        let units = text.encode_utf16().count();
        let cols = if request.max_width() < 0.0 {
            units.max(1)
        } else {
            (0..units)
                .take_while(|&n| {
                    f32::from(u16::try_from(n + 1).expect("short test word")) * 10.0
                        <= request.max_width()
                })
                .count()
                .max(1)
        };
        let lines = units.div_ceil(cols).max(1);
        let width = f32::from(u16::try_from(units.min(cols)).expect("short test word")) * 10.0;
        let height = f32::from(u16::try_from(lines * 20).expect("short test word"));
        let mut reply = vec![width, height, -1.0, height + 1.0];
        for _ in 0..lines {
            reply.extend_from_slice(&[width, 20.0, 16.0, 0.0, width]);
        }
        Ok(reply)
    }

    fn caret_rect(&self, _id: u32, _offset: i32, _upstream: bool) -> Result<[f32; 4], HwuiError> {
        Ok([0.0, 0.0, 1.0, 20.0])
    }

    fn hit_test(&self, _id: u32, _x: f32, _y: f32) -> Result<(i32, bool), HwuiError> {
        Ok((0, false))
    }

    fn word_at(&self, _id: u32, _x: f32, _y: f32) -> Result<(i32, i32), HwuiError> {
        Ok((0, 0))
    }

    fn line_at(&self, _id: u32, _x: f32, _y: f32) -> Result<(i32, i32), HwuiError> {
        Ok((0, 0))
    }

    fn snap(&self, _id: u32, offset: i32) -> Result<i32, HwuiError> {
        Ok(offset)
    }

    fn selection_rects(
        &self,
        _id: u32,
        _start: i32,
        _end: i32,
    ) -> Result<Vec<[f32; 4]>, HwuiError> {
        Ok(Vec::new())
    }

    fn previous_visual(&self, _id: u32, offset: i32) -> Result<i32, HwuiError> {
        Ok(offset)
    }

    fn next_visual(&self, _id: u32, offset: i32) -> Result<i32, HwuiError> {
        Ok(offset)
    }
}

#[test]
fn hstack_measures_text_children_under_hwui_text_engine() {
    use std::rc::Rc;
    use std::sync::Arc;

    use waterui_core::layout::ProposalSize;
    use waterui_layout::stack::hstack;

    use crate::hwui::text::TextLayoutIds;
    use crate::renderer::{HydrolysisRenderer, RenderNode};
    use crate::text::{FontFamilyResolution, HwuiTextEngine};

    let provider = Arc::new(WrappingProvider::default());
    let ids = TextLayoutIds::new();
    let mut renderer = HydrolysisRenderer::with_engine(
        Rc::new(MinimalTestTheme::default()),
        HwuiTextEngine::new(Arc::clone(&provider), ids, FontFamilyResolution::Strict),
    );
    let env = test_environment();
    let view = AnyView::new(hstack((text("Thin"), text("Light"), text("Reg"))).spacing(6.0));
    let mut node = RenderNode::build(view, &env, &mut renderer);
    node.prepare_for_measure(&mut renderer);
    let theme = renderer.theme();
    let dimensions = node.measure(
        &mut renderer.state,
        &env,
        &theme,
        ProposalSize::new(Some(400.0), None),
    );
    // "Thin" (40) + "Light" (50) + "Reg" (30) + 2 * 6 spacing = 132 pt on a single 20 pt row.
    assert!(
        (dimensions.size.width - 132.0).abs() < f32::EPSILON,
        "dims={dimensions:?}"
    );
    assert!(
        (dimensions.size.height - 20.0).abs() < f32::EPSILON,
        "dims={dimensions:?}"
    );
    let widths = provider.max_widths.lock().unwrap().clone();
    assert!(
        widths.iter().any(|&w| (w + 1.0).abs() < f32::EPSILON),
        "expected HStack max probe to pack UNBOUNDED (-1.0): {widths:?}"
    );
    assert!(
        widths.iter().all(|w| w.is_finite()),
        "expected only finite or UNBOUNDED wrap widths: {widths:?}"
    );
}
