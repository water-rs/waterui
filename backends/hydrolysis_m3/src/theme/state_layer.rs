use crate::{Brush, DrawContext, WidgetInteractionState};
use vello::kurbo::{Point, Rect, RoundedRectRadii};
use vello::peniko::{Color, Gradient};

const RIPPLE_INITIAL_ORIGIN_SCALE: f64 = 0.2;
const RIPPLE_PADDING: f64 = 10.0;
const RIPPLE_SOFT_EDGE_MINIMUM_SIZE: f64 = 75.0;
const RIPPLE_SOFT_EDGE_CONTAINER_RATIO: f64 = 0.35;
const RIPPLE_SOFT_EDGE_WIDTH: f64 = 70.0;
const RIPPLE_SOFT_EDGE_MINIMUM_SOLID_STOP: f32 = 0.65;

#[derive(Debug, Clone, Copy, PartialEq)]
struct RippleGeometry {
    center: Point,
    radius: f64,
    solid_stop: f32,
}

fn ripple_geometry(bounds: Rect, origin: Point, progress: f64) -> RippleGeometry {
    let progress = progress.clamp(0.0, 1.0);
    let center = Point::new(
        bounds.x0 + bounds.width() * 0.5,
        bounds.y0 + bounds.height() * 0.5,
    );
    let max_dimension = bounds.width().max(bounds.height());
    let initial_size = max_dimension * RIPPLE_INITIAL_ORIGIN_SCALE;
    let soft_edge_size =
        (RIPPLE_SOFT_EDGE_CONTAINER_RATIO * max_dimension).max(RIPPLE_SOFT_EDGE_MINIMUM_SIZE);
    let final_size = bounds.width().hypot(bounds.height()) + RIPPLE_PADDING + soft_edge_size;
    let size = initial_size + (final_size - initial_size) * progress;
    let radius = (size * 0.5).max(1.0);
    let ripple_center = Point::new(
        origin.x + (center.x - origin.x) * progress,
        origin.y + (center.y - origin.y) * progress,
    );
    let solid_stop = ((radius - RIPPLE_SOFT_EDGE_WIDTH) / radius)
        .max(f64::from(RIPPLE_SOFT_EDGE_MINIMUM_SOLID_STOP))
        .clamp(0.0, 1.0) as f32;

    RippleGeometry {
        center: ripple_center,
        radius,
        solid_stop,
    }
}

fn ripple_brush(color: Color, geometry: RippleGeometry) -> Brush {
    Brush::from(
        Gradient::new_radial(geometry.center, geometry.radius as f32)
            .with_stops([(geometry.solid_stop, color), (1.0, color.with_alpha(0.0))]),
    )
}

pub(crate) fn draw_bounded(
    draw: &mut dyn DrawContext,
    bounds: Rect,
    radii: RoundedRectRadii,
    color: Color,
    state: WidgetInteractionState,
) {
    let state_opacity = state.state_layer_opacity();
    if state_opacity > 0.0 {
        draw.push_rounded_layer(state_opacity, bounds, radii);
        draw.fill_rounded_rect(bounds, radii, &Brush::from(color));
        draw.pop_layer();
    }

    let press_opacity = state.press_layer_opacity();
    let Some(origin) = state.press_origin else {
        return;
    };
    if press_opacity == 0.0 {
        return;
    }

    let progress = f64::from(state.press_progress.clamp(0.0, 1.0));
    let ripple = ripple_geometry(bounds, origin, progress);
    let brush = ripple_brush(color, ripple);

    draw.push_rounded_layer(press_opacity, bounds, radii);
    draw.fill_circle(ripple.center, ripple.radius, &brush);
    draw.pop_layer();
}

pub(crate) fn draw_unbounded_circle(
    draw: &mut dyn DrawContext,
    center: Point,
    radius: f64,
    color: Color,
    state: WidgetInteractionState,
) {
    let state_opacity = state.state_layer_opacity();
    if state_opacity > 0.0 {
        draw.push_layer(state_opacity, None);
        draw.fill_circle(center, radius, &Brush::from(color));
        draw.pop_layer();
    }

    let press_opacity = state.press_layer_opacity();
    if press_opacity == 0.0 {
        return;
    }
    let progress = f64::from(state.press_progress.clamp(0.0, 1.0));
    let bounds = Rect::from_center_size(center, (radius * 2.0, radius * 2.0));
    let origin = state.press_origin.unwrap_or(center);
    let ripple = ripple_geometry(bounds, origin, progress);
    let brush = ripple_brush(color, ripple);
    draw.push_layer(press_opacity, None);
    draw.fill_circle(ripple.center, ripple.radius, &brush);
    draw.pop_layer();
}

#[cfg(test)]
mod tests {
    use super::{
        RIPPLE_SOFT_EDGE_MINIMUM_SOLID_STOP, RippleGeometry, ripple_brush, ripple_geometry,
    };
    use crate::{Brush, DrawContext, WidgetInteractionState, theme::state_layer};
    use std::path::Path;
    use vello::kurbo::{Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, RoundedRectRadii};
    use vello::peniko::{Color, GradientKind};
    use waterui_graphics::{
        GpuContext, GpuFrame, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenRenderError,
        OffscreenRenderOutput, OffscreenSize,
    };

    #[test]
    fn material_ripple_geometry_moves_from_origin_to_center() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 40.0);
        let origin = Point::new(10.0, 12.0);

        let start = ripple_geometry(bounds, origin, 0.0);
        let end = ripple_geometry(bounds, origin, 1.0);

        assert_eq!(start.center, origin);
        assert_eq!(end.center, Point::new(50.0, 20.0));
        assert!(end.radius > start.radius);
    }

    #[test]
    fn material_ripple_brush_uses_soft_edge_radial_gradient() {
        let geometry = RippleGeometry {
            center: Point::new(12.0, 16.0),
            radius: 120.0,
            solid_stop: RIPPLE_SOFT_EDGE_MINIMUM_SOLID_STOP,
        };
        let brush = ripple_brush(Color::new([1.0, 0.0, 0.0, 1.0]), geometry);

        let Brush::Gradient(gradient) = brush else {
            panic!("Material ripple press layer must use a radial gradient brush");
        };
        let GradientKind::Radial(position) = gradient.kind else {
            panic!("Material ripple press layer must use a radial gradient");
        };

        assert_eq!(position.end_center, geometry.center);
        assert_eq!(position.end_radius, geometry.radius as f32);
        assert_eq!(gradient.stops[0].offset, geometry.solid_stop);
        assert_eq!(gradient.stops[1].offset, 1.0);
        assert_eq!(gradient.stops[1].color.components[3], 0.0);
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

    waterui_graphics::impl_gpu_subview!(RippleVisualRenderer);

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
        let output_path = Path::new("target/hydrolysis-m3-visual/material-ripple-soft-edge.png");
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
