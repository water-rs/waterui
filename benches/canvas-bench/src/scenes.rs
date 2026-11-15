//! Measured scenes for `canvas-bench` (water-rs/waterui#1564).
#![cfg(target_os = "ios")]
//!
//! Scenario 2: one 5,000-point area chart and one ~2,500-path SVG document,
//! each realized once as `Canvas` and once as a single `SceneView`.
//! Scenario 4: a Lucide icon grid (72 pt, 4 columns) realized as Canvas
//! (lucide `Svg`) or as one `SceneView` per icon stroking the same path.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use nami::watcher::BoxWatcherGuard;
use waterui::Identifiable;
use waterui::canvas::{Canvas, Circle, Path, StrokeStyle};
use waterui::color::{Working, WorkingColor, signal_color};
use waterui::component::lazy::Lazy;
use waterui::graphics::backend::draw::kurbo::{self, Affine, BezPath, Stroke};
use waterui::graphics::backend::draw::{Draw, Fixed, Recorder};
use waterui::graphics::backend::{
    NextFrame, SceneContent, SceneInvalidator, SceneView, invalidate_on_change,
};
use waterui::layout::scroll::{ScrollController, scroll};
use waterui::layout::{LazyContainer, stack::VStackLayout};
use waterui::prelude::*;
use waterui::views::ForEach;
use waterui_icons_lucide as lucide;

use crate::harness::{CellSpec, SceneHandles, ScrollDriver};

/// Lucide constructors used by the scenario-4 Canvas grid.
const LUCIDE: [fn() -> lucide::Svg; 8] = [
    lucide::house,
    lucide::settings,
    lucide::heart,
    lucide::star,
    lucide::user,
    lucide::search,
    lucide::bell,
    lucide::mail,
];

/// Matching path data for the SceneView grid.
const LUCIDE_PATHS: [&str; 8] = [
    lucide::HOUSE_PATH,
    lucide::SETTINGS_PATH,
    lucide::HEART_PATH,
    lucide::STAR_PATH,
    lucide::USER_PATH,
    lucide::SEARCH_PATH,
    lucide::BELL_PATH,
    lucide::MAIL_PATH,
];

const CHART_W: f32 = 360.0;
const CHART_H: f32 = 240.0;
const SVG_SIZE: f32 = 360.0;
const SVG_GRID: u32 = 50;
const ICON_PT: f32 = 72.0;
const ICON_COLS: u32 = 4;
const CHART_POINTS: u32 = 5000;

#[derive(Clone, Copy, Identifiable)]
struct RowId {
    #[id]
    id: u32,
}

/// Builds a cell's scene and registers its handles before the view mounts.
#[must_use]
pub fn cell_view(spec: &CellSpec, slot: &Rc<RefCell<Option<SceneHandles>>>) -> AnyView {
    let tint = Binding::container(WorkingColor::new([0.15, 0.45, 0.85, 1.0]));
    let handles = SceneHandles {
        scroll: if spec.scenario == 4 && spec.variant.starts_with("scroll-") {
            ScrollDriver::Controller(ScrollController::new(waterui::layout::Point::new(0.0, 0.0)))
        } else {
            ScrollDriver::None
        },
        animate: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        rows_created: Arc::new(AtomicU64::new(0)),
        producer_calls: Arc::new(AtomicU64::new(0)),
        tint: tint.clone(),
    };
    *slot.borrow_mut() = Some(handles.clone());
    crate::harness::mark(&format!("cell-view-{}", spec.name()));
    match spec.scenario {
        2 => AnyView::new(scenario2(spec, &handles)),
        4 => AnyView::new(scenario4(spec, &handles)),
        other => panic!("unsupported scenario {other}"),
    }
}

fn is_scene(variant: &str) -> bool {
    variant.ends_with("-scene")
}

fn chart_paths() -> (Path, Path) {
    let mut area = Path::builder().move_to(Point::new(0.0, CHART_H));
    let mut line = Path::builder();
    for i in 0..CHART_POINTS {
        #[expect(
            clippy::cast_precision_loss,
            reason = "chart x is a point coordinate from a 5000-sample index"
        )]
        let t = i as f32 / (CHART_POINTS - 1) as f32;
        let x = t * CHART_W;
        let y = CHART_H * 0.55 - (t * 28.0).sin() * 70.0 - (t * 7.0).cos() * 18.0;
        area = area.line_to(Point::new(x, y));
        line = if i == 0 {
            line.move_to(Point::new(x, y))
        } else {
            line.line_to(Point::new(x, y))
        };
    }
    (
        area.line_to(Point::new(CHART_W, CHART_H)).close().build(),
        line.build(),
    )
}

fn svg_dots() -> Vec<(f32, f32, f32)> {
    let pitch = SVG_SIZE / SVG_GRID as f32;
    let radius = pitch * 0.32;
    let mut dots = Vec::with_capacity((SVG_GRID * SVG_GRID) as usize);
    for row in 0..SVG_GRID {
        for col in 0..SVG_GRID {
            #[expect(
                clippy::cast_precision_loss,
                reason = "grid indices convert to canvas points"
            )]
            {
                dots.push((
                    (col as f32 + 0.5) * pitch,
                    (row as f32 + 0.5) * pitch,
                    radius,
                ));
            }
        }
    }
    dots
}

fn scenario2(spec: &CellSpec, handles: &SceneHandles) -> impl View {
    let tint = handles.tint.clone();
    let scene = is_scene(&spec.variant);
    let chart = spec.variant.starts_with("chart-");
    if chart {
        let (area, line) = chart_paths();
        if scene {
            AnyView::new(
                SceneView::new(ChartContent {
                    area: chart_area_bez(),
                    line: chart_line_bez(),
                    tint,
                    invalidator: None,
                    guard: None,
                })
                .size(CHART_W, CHART_H),
            )
        } else {
            AnyView::new(
                Canvas::new(move |cx| {
                    cx.fill(area, tint.clone());
                    cx.stroke(line, StrokeStyle::new(2.0), tint.clone());
                })
                .intrinsic_size(Size::new(CHART_W, CHART_H)),
            )
        }
    } else {
        let dots = svg_dots();
        if scene {
            AnyView::new(
                SceneView::new(SvgDocContent {
                    dots: dots.clone(),
                    tint,
                    invalidator: None,
                    guard: None,
                })
                .size(SVG_SIZE, SVG_SIZE),
            )
        } else {
            AnyView::new(
                Canvas::new(move |cx| {
                    for &(x, y, r) in &dots {
                        cx.fill(Circle::new(Point::new(x, y), r), tint.clone());
                    }
                })
                .intrinsic_size(Size::new(SVG_SIZE, SVG_SIZE)),
            )
        }
    }
}

fn scenario4(spec: &CellSpec, handles: &SceneHandles) -> impl View {
    let variant = spec.variant.clone();
    let tint = handles.tint.clone();
    let rows_created = handles.rows_created.clone();
    let n = spec.n;
    let row_count = n.div_ceil(ICON_COLS);
    let items: Vec<RowId> = (0..row_count).map(|id| RowId { id }).collect();
    let contents = ForEach::new(items, move |row: RowId| {
        rows_created.fetch_add(1, Ordering::Relaxed);
        let start = row.id * ICON_COLS;
        let end = (start + ICON_COLS).min(n);
        let icons: Vec<AnyView> = (start..end)
            .map(|index| icon_view(&variant, index, &tint))
            .collect();
        AnyView::new(hstack(icons).spacing(8.0).padding_with(4.0))
    });
    match &handles.scroll {
        ScrollDriver::Controller(controller) => AnyView::new(
            scroll(LazyContainer::new(VStackLayout::default(), contents))
                .scroll_controller(controller),
        ),
        ScrollDriver::None => AnyView::new(Lazy::vstack(contents)),
    }
}

fn icon_view(variant: &str, index: u32, tint: &Binding<WorkingColor>) -> AnyView {
    let i = (index as usize) % LUCIDE.len();
    if is_scene(variant) {
        let path = BezPath::from_svg(LUCIDE_PATHS[i]).unwrap_or_else(|_| BezPath::new());
        AnyView::new(
            SceneView::new(IconContent {
                path,
                tint: tint.clone(),
                invalidator: None,
                guard: None,
            })
            .size(ICON_PT, ICON_PT),
        )
    } else {
        let color = signal_color(tint.map(|c: WorkingColor| Color::from(Working(c))));
        AnyView::new(LUCIDE[i]().tint(color).size(ICON_PT, ICON_PT))
    }
}

fn chart_y(t: f64) -> f64 {
    f64::from(CHART_H) * 0.55 - (t * 28.0).sin() * 70.0 - (t * 7.0).cos() * 18.0
}

fn chart_area_bez() -> BezPath {
    let mut bez = BezPath::new();
    bez.move_to((0.0, f64::from(CHART_H)));
    for i in 0..CHART_POINTS {
        #[expect(
            clippy::cast_precision_loss,
            reason = "chart x is a point coordinate from a 5000-sample index"
        )]
        let t = f64::from(i) / f64::from(CHART_POINTS - 1);
        bez.line_to((t * f64::from(CHART_W), chart_y(t)));
    }
    bez.line_to((f64::from(CHART_W), f64::from(CHART_H)));
    bez.close_path();
    bez
}

fn chart_line_bez() -> BezPath {
    let mut bez = BezPath::new();
    for i in 0..CHART_POINTS {
        #[expect(
            clippy::cast_precision_loss,
            reason = "chart x is a point coordinate from a 5000-sample index"
        )]
        let t = f64::from(i) / f64::from(CHART_POINTS - 1);
        let x = t * f64::from(CHART_W);
        let y = chart_y(t);
        if i == 0 {
            bez.move_to((x, y));
        } else {
            bez.line_to((x, y));
        }
    }
    bez
}

struct ChartContent {
    area: BezPath,
    line: BezPath,
    tint: Binding<WorkingColor>,
    invalidator: Option<SceneInvalidator>,
    guard: Option<BoxWatcherGuard>,
}

impl SceneContent for ChartContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut waterui::graphics::backend::RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> NextFrame {
        let color = self.tint.snapshot();
        recorder.fill(Fixed(self.area.clone()), Fixed(color));
        recorder.stroke(
            Fixed(self.line.clone()),
            Fixed(Stroke::new(2.0)),
            Fixed(color),
        );
        NextFrame::Idle
    }

    fn rebuild_for_engine(&mut self) {}

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.guard = None;
        self.invalidator = invalidator.clone();
        if let Some(invalidator) = &invalidator {
            self.guard = Some(invalidate_on_change(invalidator, &self.tint));
        }
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(Size::new(CHART_W, CHART_H))
    }
}

struct SvgDocContent {
    dots: Vec<(f32, f32, f32)>,
    tint: Binding<WorkingColor>,
    invalidator: Option<SceneInvalidator>,
    guard: Option<BoxWatcherGuard>,
}

impl SceneContent for SvgDocContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut waterui::graphics::backend::RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> NextFrame {
        let color = self.tint.snapshot();
        for &(x, y, r) in &self.dots {
            recorder.fill(
                Fixed(kurbo::Circle::new(
                    kurbo::Point::new(f64::from(x), f64::from(y)),
                    f64::from(r),
                )),
                Fixed(color),
            );
        }
        NextFrame::Idle
    }

    fn rebuild_for_engine(&mut self) {}

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.guard = None;
        self.invalidator = invalidator.clone();
        if let Some(invalidator) = &invalidator {
            self.guard = Some(invalidate_on_change(invalidator, &self.tint));
        }
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(Size::new(SVG_SIZE, SVG_SIZE))
    }
}

struct IconContent {
    path: BezPath,
    tint: Binding<WorkingColor>,
    invalidator: Option<SceneInvalidator>,
    guard: Option<BoxWatcherGuard>,
}

impl SceneContent for IconContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut waterui::graphics::backend::RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> NextFrame {
        let color = self.tint.snapshot();
        let scale = f64::from(ICON_PT) / 24.0;
        recorder.transform(Fixed(Affine::scale(scale)), |recorder| {
            recorder.stroke(
                Fixed(self.path.clone()),
                Fixed(Stroke::new(2.0)),
                Fixed(color),
            );
        });
        NextFrame::Idle
    }

    fn rebuild_for_engine(&mut self) {}

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.guard = None;
        self.invalidator = invalidator.clone();
        if let Some(invalidator) = &invalidator {
            self.guard = Some(invalidate_on_change(invalidator, &self.tint));
        }
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(Size::new(ICON_PT, ICON_PT))
    }
}
