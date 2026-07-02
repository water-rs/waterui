use crate::{Brush, DrawContext, WidgetInteractionState};
use vello::kurbo::{Point, Rect, RoundedRectRadii};
use vello::peniko::Color;

/// Minimum ripple diameter (Material Web `MINIMUM_PRESS_DIAMETER`): small
/// targets still produce a ripple at least this wide.
const RIPPLE_MINIMUM_DIAMETER: f64 = 48.0;

/// MD3 hover state-layer opacity token (8%).
const HOVER_STATE_LAYER_OPACITY: f32 = 0.08;
/// MD3 focus state-layer opacity token (12%).
const FOCUS_STATE_LAYER_OPACITY: f32 = 0.12;
/// MD3 pressed state-layer opacity token (12%).
const PRESSED_STATE_LAYER_OPACITY: f32 = 0.12;

/// The dominant resting state-layer opacity for `state`: the renderer-sampled
/// animated value when one is in flight, otherwise the MD3 token for the
/// active boolean state (focus outranks hover).
fn resolved_state_layer_opacity(state: WidgetInteractionState) -> f32 {
    if state.state_layer_opacity > 0.0 {
        return state.state_layer_opacity;
    }
    if state.focus_visible {
        FOCUS_STATE_LAYER_OPACITY
    } else if state.hovered {
        HOVER_STATE_LAYER_OPACITY
    } else {
        0.0
    }
}

/// The pressed ripple opacity for `state`: the renderer-sampled animated value
/// when one is in flight, otherwise the MD3 pressed token.
fn resolved_press_layer_opacity(state: WidgetInteractionState) -> f32 {
    if state.press_layer_opacity > 0.0 {
        return state.press_layer_opacity;
    }
    if state.pressed {
        PRESSED_STATE_LAYER_OPACITY
    } else {
        0.0
    }
}

/// Initial ripple size as a fraction of the full diameter: a press starts at
/// this scale over the press point and grows to full size as the renderer's
/// press-grow animation advances `press_progress` from 0 to 1.
const RIPPLE_INITIAL_SCALE: f64 = 0.4;

/// The full Material ripple diameter for a target: the surface diagonal,
/// floored at [`RIPPLE_MINIMUM_DIAMETER`]. The ripple is drawn fresh each
/// frame from the sampled interaction state — [`ripple_kinematics`] scales it
/// from the initial fraction up to full size while its center drifts from the
/// press point to the surface center.
pub fn ripple_diameter(bounds: Rect) -> f64 {
    bounds.width().hypot(bounds.height()).max(RIPPLE_MINIMUM_DIAMETER)
}

/// Material ripple kinematics for one frame: given the target's `bounds` and
/// the sampled `state`, returns the ripple's current center and radius.
///
/// The center drifts linearly from the press point (already mapped by the
/// renderer into the same coordinate space as `bounds`; the bounds center when
/// absent) to the bounds center, and the radius grows from
/// [`RIPPLE_INITIAL_SCALE`] of the full [`ripple_diameter`] to the full size,
/// both driven by `press_progress`.
fn ripple_kinematics(bounds: Rect, state: WidgetInteractionState) -> (Point, f64) {
    let progress = f64::from(state.press_progress.clamp(0.0, 1.0));
    let bounds_center = Point::new(
        bounds.width().mul_add(0.5, bounds.x0),
        bounds.height().mul_add(0.5, bounds.y0),
    );
    let origin = state.press_origin.unwrap_or(bounds_center);
    let center = Point::new(
        (bounds_center.x - origin.x).mul_add(progress, origin.x),
        (bounds_center.y - origin.y).mul_add(progress, origin.y),
    );
    let scale = (1.0 - RIPPLE_INITIAL_SCALE).mul_add(progress, RIPPLE_INITIAL_SCALE);
    let radius = ripple_diameter(bounds) * 0.5 * scale;
    (center, radius)
}

/// Draws the resting state layer (hover/focus tint) into `bounds`.
fn draw_state_tint_rounded(
    draw: &mut dyn DrawContext,
    bounds: Rect,
    radii: RoundedRectRadii,
    color: Color,
    state_opacity: f32,
) {
    if state_opacity > 0.0 {
        draw.push_rounded_layer(state_opacity, bounds, radii);
        draw.fill_rounded_rect(bounds, radii, &Brush::from(color));
        draw.pop_layer();
    }
}

pub fn draw_bounded(
    draw: &mut dyn DrawContext,
    bounds: Rect,
    radii: RoundedRectRadii,
    color: Color,
    state: WidgetInteractionState,
) {
    draw_state_tint_rounded(draw, bounds, radii, color, resolved_state_layer_opacity(state));

    let press_opacity = resolved_press_layer_opacity(state);
    if press_opacity == 0.0 {
        return;
    }

    // Solid Material ripple, drawn fresh each frame at its current animated
    // geometry: the retained tree re-encodes the scene every frame while the
    // press animations run, so the sampled press_progress/press_origin drive
    // the growth and drift directly.
    let (center, radius) = ripple_kinematics(bounds, state);
    let brush = Brush::from(color.with_alpha(press_opacity.clamp(0.0, 1.0)));

    draw.push_rounded_layer(1.0, bounds, radii);
    draw.fill_circle(center, radius, &brush);
    draw.pop_layer();
}

pub fn draw_unbounded_circle(
    draw: &mut dyn DrawContext,
    center: Point,
    radius: f64,
    color: Color,
    state: WidgetInteractionState,
) {
    let state_opacity = resolved_state_layer_opacity(state);
    if state_opacity > 0.0 {
        draw.push_layer(state_opacity, None);
        draw.fill_circle(center, radius, &Brush::from(color));
        draw.pop_layer();
    }

    let press_opacity = resolved_press_layer_opacity(state);
    if press_opacity == 0.0 {
        return;
    }
    let bounds = Rect::from_center_size(center, (radius * 2.0, radius * 2.0));
    let (ripple_center, ripple_radius) = ripple_kinematics(bounds, state);
    let brush = Brush::from(color.with_alpha(press_opacity.clamp(0.0, 1.0)));
    draw.push_layer(1.0, None);
    draw.fill_circle(ripple_center, ripple_radius, &brush);
    draw.pop_layer();
}

#[cfg(test)]
mod tests {
    use super::{RIPPLE_MINIMUM_DIAMETER, ripple_diameter};
    use crate::{Brush, DrawContext, WidgetInteractionState, theme::state_layer};
    use std::path::Path;
    use vello::kurbo::{Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, RoundedRectRadii};
    use vello::peniko::Color;
    use waterui_graphics::{
        GpuContext, GpuFrame, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenRenderError,
        OffscreenRenderOutput, OffscreenSize,
    };

    #[test]
    fn material_ripple_diameter_spans_diagonal_with_minimum() {
        // Large surface: ripple diameter is the diagonal so the solid circle
        // covers the whole target at full scale.
        let wide = Rect::new(0.0, 0.0, 100.0, 40.0);
        assert!((ripple_diameter(wide) - 100.0_f64.hypot(40.0)).abs() < 1e-6);

        // Tiny surface: floored at the Material minimum press diameter.
        let tiny = Rect::new(0.0, 0.0, 20.0, 20.0);
        assert_eq!(ripple_diameter(tiny), RIPPLE_MINIMUM_DIAMETER);
    }

    #[test]
    fn material_ripple_animates_from_press_point_to_full_coverage() {
        // Mid-press: the ripple sits between the press point and the bounds
        // center, at a radius between the initial fraction and full size.
        let bounds = Rect::new(0.0, 0.0, 100.0, 40.0);
        let origin = Point::new(10.0, 12.0);
        let mut recorder = RecordingDrawContext::default();
        state_layer::draw_bounded(
            &mut recorder,
            bounds,
            8.0.into(),
            Color::new([1.0, 1.0, 1.0, 1.0]),
            WidgetInteractionState {
                pressed: true,
                press_layer_opacity: 0.12,
                press_origin: Some(origin),
                press_progress: 0.5,
                ..WidgetInteractionState::NONE
            },
        );
        let circle = recorder
            .filled_circle
            .expect("mid-press ripple must fill a solid circle");
        assert_eq!(
            circle.center,
            Point::new(30.0, 16.0),
            "center drifts halfway from the press point to the bounds center"
        );
        let full_radius = ripple_diameter(bounds) * 0.5;
        assert!(
            (circle.radius - full_radius * 0.7).abs() < 1e-6,
            "radius is halfway between the 0.4 initial fraction and full size"
        );
    }

    #[test]
    fn material_ripple_press_layer_is_a_solid_circle() {
        // At full progress the ripple covers the target: a solid circle (no
        // soft-edge gradient) centered in bounds at the full ripple diameter.
        let bounds = Rect::new(0.0, 0.0, 100.0, 40.0);
        let mut recorder = RecordingDrawContext::default();
        state_layer::draw_bounded(
            &mut recorder,
            bounds,
            8.0.into(),
            Color::new([1.0, 1.0, 1.0, 1.0]),
            WidgetInteractionState {
                pressed: true,
                press_layer_opacity: 0.12,
                press_origin: Some(Point::new(10.0, 12.0)),
                press_progress: 1.0,
                ..WidgetInteractionState::NONE
            },
        );
        let circle = recorder
            .filled_circle
            .expect("press ripple must fill a solid circle");
        assert_eq!(circle.center, Point::new(50.0, 20.0), "circle is centered");
        assert!(
            (circle.radius - ripple_diameter(bounds) * 0.5).abs() < 1e-6,
            "circle radius is half the ripple diameter"
        );
        assert!(
            matches!(circle.brush, RecordedBrush::Solid(alpha) if (alpha - 0.12).abs() < 1e-6),
            "ripple is a solid color at the press opacity, not a gradient"
        );
    }

    #[derive(Debug, Clone, Copy)]
    enum RecordedBrush {
        Solid(f32),
        Gradient,
    }

    #[derive(Debug, Clone, Copy)]
    struct RecordedCircle {
        center: Point,
        radius: f64,
        brush: RecordedBrush,
    }

    #[derive(Default)]
    struct RecordingDrawContext {
        filled_circle: Option<RecordedCircle>,
    }

    impl RecordingDrawContext {
        fn record_brush(brush: &Brush) -> RecordedBrush {
            match brush {
                Brush::Solid(color) => RecordedBrush::Solid(color.components[3]),
                Brush::Gradient(_) => RecordedBrush::Gradient,
            }
        }
    }

    impl DrawContext for RecordingDrawContext {
        fn fill_rect(&mut self, _rect: Rect, _brush: &Brush) {}
        fn fill_rounded_rect(&mut self, _rect: Rect, _radii: RoundedRectRadii, _brush: &Brush) {}
        fn stroke_rect(&mut self, _rect: Rect, _brush: &Brush, _width: f64) {}
        fn stroke_rounded_rect(
            &mut self,
            _rect: Rect,
            _radii: RoundedRectRadii,
            _brush: &Brush,
            _width: f64,
        ) {
        }
        fn stroke_line(&mut self, _from: Point, _to: Point, _brush: &Brush, _width: f64) {}
        fn stroke_circle(&mut self, _center: Point, _radius: f64, _brush: &Brush, _width: f64) {}
        fn fill_circle(&mut self, center: Point, radius: f64, brush: &Brush) {
            self.filled_circle = Some(RecordedCircle {
                center,
                radius,
                brush: Self::record_brush(brush),
            });
        }
        fn fill_path(&mut self, _path: &BezPath, _brush: &Brush) {}
        fn stroke_path(&mut self, _path: &BezPath, _brush: &Brush, _width: f64) {}
        fn push_layer(&mut self, _alpha: f32, _clip: Option<&Rect>) {}
        fn push_rounded_layer(&mut self, _alpha: f32, _clip: Rect, _radii: RoundedRectRadii) {}
        fn pop_layer(&mut self) {}
        fn push_transform(&mut self, _affine: Affine) {}
        fn pop_transform(&mut self) {}
    }

    struct VelloTestDrawContext<'a> {
        scene: &'a mut vello::Scene,
    }

    impl VelloTestDrawContext<'_> {
        fn fill_shape(&mut self, shape: &impl vello::kurbo::Shape, brush: &Brush) {
            match brush {
                Brush::Solid(color) => self.scene.fill(
                    vello::peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    color,
                    None,
                    shape,
                ),
                Brush::Gradient(gradient) => self.scene.fill(
                    vello::peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    gradient,
                    None,
                    shape,
                ),
            }
        }

        fn stroke_shape(&mut self, shape: &impl vello::kurbo::Shape, brush: &Brush, width: f64) {
            let stroke = vello::kurbo::Stroke::new(width);
            match brush {
                Brush::Solid(color) => {
                    self.scene
                        .stroke(&stroke, Affine::IDENTITY, color, None, shape);
                }
                Brush::Gradient(gradient) => {
                    self.scene
                        .stroke(&stroke, Affine::IDENTITY, gradient, None, shape);
                }
            }
        }
    }

    impl DrawContext for VelloTestDrawContext<'_> {
        fn fill_rect(&mut self, rect: Rect, brush: &Brush) {
            self.fill_shape(&rect, brush);
        }

        fn fill_rounded_rect(&mut self, rect: Rect, radii: RoundedRectRadii, brush: &Brush) {
            self.fill_shape(&RoundedRect::from_rect(rect, radii), brush);
        }

        fn stroke_rect(&mut self, rect: Rect, brush: &Brush, width: f64) {
            self.stroke_shape(&rect, brush, width);
        }

        fn stroke_rounded_rect(
            &mut self,
            rect: Rect,
            radii: RoundedRectRadii,
            brush: &Brush,
            width: f64,
        ) {
            self.stroke_shape(&RoundedRect::from_rect(rect, radii), brush, width);
        }

        fn stroke_line(&mut self, from: Point, to: Point, brush: &Brush, width: f64) {
            self.stroke_shape(&Line::new(from, to), brush, width);
        }

        fn stroke_circle(&mut self, center: Point, radius: f64, brush: &Brush, width: f64) {
            self.stroke_shape(&Circle::new(center, radius), brush, width);
        }

        fn fill_circle(&mut self, center: Point, radius: f64, brush: &Brush) {
            self.fill_shape(&Circle::new(center, radius), brush);
        }

        fn fill_path(&mut self, path: &BezPath, brush: &Brush) {
            self.fill_shape(path, brush);
        }

        fn stroke_path(&mut self, path: &BezPath, brush: &Brush, width: f64) {
            self.stroke_shape(path, brush, width);
        }

        fn push_layer(&mut self, alpha: f32, clip: Option<&Rect>) {
            let clip = clip
                .copied()
                .unwrap_or(Rect::new(-1.0e9, -1.0e9, 1.0e9, 1.0e9));
            self.scene.push_layer(
                vello::peniko::Fill::NonZero,
                vello::peniko::BlendMode::default(),
                alpha,
                Affine::IDENTITY,
                &clip,
            );
        }

        fn push_rounded_layer(&mut self, alpha: f32, clip: Rect, radii: RoundedRectRadii) {
            let clip = RoundedRect::from_rect(clip, radii);
            self.scene.push_layer(
                vello::peniko::Fill::NonZero,
                vello::peniko::BlendMode::default(),
                alpha,
                Affine::IDENTITY,
                &clip,
            );
        }

        fn pop_layer(&mut self) {
            self.scene.pop_layer();
        }

        fn push_transform(&mut self, _affine: Affine) {
            panic!("ripple visual test does not use transforms");
        }

        fn pop_transform(&mut self) {
            panic!("ripple visual test does not use transforms");
        }
    }

    struct RippleVisualRenderer {
        renderer: Option<vello::Renderer>,
        scene: vello::Scene,
    }

    impl RippleVisualRenderer {
        fn new() -> Self {
            Self {
                renderer: None,
                scene: vello::Scene::new(),
            }
        }
    }

    impl GpuView for RippleVisualRenderer {
        #[expect(
            clippy::future_not_send,
            reason = "GpuView runs on the render thread; this future borrows the non-Send `GpuContext`/`Environment`"
        )]
        async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
            self.renderer = Some(
                vello::Renderer::new(
                    ctx.device,
                    vello::RendererOptions {
                        use_cpu: false,
                        antialiasing_support: vello::AaSupport::area_only(),
                        num_init_threads: std::num::NonZeroUsize::new(1),
                        pipeline_cache: None,
                    },
                )
                .expect("failed to create ripple visual vello renderer"),
            );
        }

        fn render(&mut self, frame: &mut GpuFrame) {
            self.scene.reset();
            let bounds = Rect::new(24.0, 24.0, 216.0, 96.0);
            let mut draw = VelloTestDrawContext {
                scene: &mut self.scene,
            };
            draw.fill_rounded_rect(
                bounds,
                20.0.into(),
                &Brush::from(Color::new([0.40, 0.31, 0.64, 1.0])),
            );
            state_layer::draw_bounded(
                &mut draw,
                bounds,
                20.0.into(),
                Color::new([1.0, 1.0, 1.0, 1.0]),
                WidgetInteractionState {
                    pressed: true,
                    press_layer_opacity: 0.30,
                    press_origin: Some(Point::new(56.0, 48.0)),
                    press_progress: 0.72,
                    ..WidgetInteractionState::NONE
                },
            );

            self.renderer
                .as_mut()
                .expect("ripple visual renderer was not set up")
                .render_to_texture(
                    frame.device,
                    frame.queue,
                    &self.scene,
                    &frame.view,
                    &vello::RenderParams {
                        base_color: Color::WHITE,
                        width: frame.width,
                        height: frame.height,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .expect("ripple visual vello render failed");
        }
    }


    fn skip_without_gpu(
        result: Result<OffscreenRenderOutput, OffscreenRenderError>,
    ) -> Option<OffscreenRenderOutput> {
        match result {
            Ok(output) => Some(output),
            Err(OffscreenRenderError::NoAdapter) => None,
            Err(error) => panic!("ripple visual offscreen render failed: {error}"),
        }
    }

    #[test]
    #[ignore = "writes a visual acceptance PNG for direct image review"]
    fn material_ripple_visual_snapshot() {
        let mut env = waterui_core::Environment::new();
        let Some(output) = skip_without_gpu(
            GpuSurface::new(RippleVisualRenderer::new()).render_offscreen(
                OffscreenRenderConfig::new(
                    OffscreenSize::try_from_pixels(240, 120).expect("static size must be valid"),
                )
                .format(vello::wgpu::TextureFormat::Rgba8Unorm),
                &mut env,
            ),
        ) else {
            return;
        };
        let output_path = Path::new("target/hydrolysis-m3-visual/material-ripple-solid.png");
        std::fs::create_dir_all(
            output_path
                .parent()
                .expect("ripple visual output path must have parent"),
        )
        .expect("failed to create ripple visual output directory");
        output
            .save_png(output_path)
            .expect("failed to save ripple visual output");
    }
}
