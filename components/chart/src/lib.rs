#![doc = "Chart components for `WaterUI` built on top of the graphics canvas."]
#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;

use waterui_color::{Color, Srgb};
use waterui_graphics::{Canvas, PathBuilder, canvas};

mod line;
pub use line::{LineChart, line_chart_path};

mod bar;
pub use bar::{BarChart, bar_rectangles};

/// Shared helpers for chart rendering.
fn ensure_dimensions(width: f32, height: f32) {
    assert!(
        width.is_finite() && width > 0.0,
        "chart width must be positive"
    );
    assert!(
        height.is_finite() && height > 0.0,
        "chart height must be positive"
    );
}

fn to_arc(values: Vec<f32>) -> Arc<[f32]> {
    assert!(!values.is_empty(), "chart data cannot be empty");
    Arc::from(values.into_boxed_slice())
}

fn usize_to_f32(value: usize, context: &str) -> f32 {
    const MAX_PRECISE: usize = 1 << 24;
    assert!(
        value <= MAX_PRECISE,
        "{context} {value} exceeds f32 precision budget"
    );
    #[allow(clippy::cast_precision_loss)]
    {
        value as f32
    }
}

struct ChartCanvas {
    width: f32,
    height: f32,
    background: Option<Srgb>,
}

impl ChartCanvas {
    fn new(width: f32, height: f32, background: Option<Srgb>) -> Self {
        ensure_dimensions(width, height);
        Self {
            width,
            height,
            background,
        }
    }

    fn paint(
        self,
        draw: impl Fn(&mut waterui_graphics::GraphicsContext<'_>) + Send + Sync + 'static,
    ) -> Canvas {
        let Self {
            width,
            height,
            background,
        } = self;

        canvas(move |ctx| {
            if let Some(bg) = &background {
                let bg_color: Color = (*bg).into();
                let rect = PathBuilder::new()
                    .move_to([0.0, 0.0])
                    .line_to([width, 0.0])
                    .line_to([width, height])
                    .line_to([0.0, height])
                    .close()
                    .build();
                ctx.fill(&rect, &bg_color);
            }
            draw(ctx);
        })
        .width(width)
        .height(height)
    }
}
