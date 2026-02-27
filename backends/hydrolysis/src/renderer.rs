use core::f64::consts::TAU;

use nami::Signal;
use waterui::filter::Opacity;
use waterui_backend_core::ViewDispatcher;
use waterui_core::layout::{
    ProposalSize, Rect as LayoutRect, Size as LayoutSize, StretchAxis, SubView,
};
use waterui_core::{AnyView, Environment, Metadata, Native, Retain, Str, View};
use waterui_graphics::color::ResolvedColor;
use waterui_graphics::{GradientType, ResolvedGradient, ResolvedGradientStop};
use waterui_layout::container::FixedContainer;
use waterui_layout::spacer::Spacer;
use waterui_shape::{PathCommand, ResolvedShape};
use waterui_text::TextConfig;

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    pub font_cx: parley::FontContext,
    pub layout_cx: parley::LayoutContext,
}

impl Default for HydroState {
    fn default() -> Self {
        Self {
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
        }
    }
}

/// Render context passed to handlers.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext {
    renderer_ptr: *mut HydrolysisRenderer,
    pub transform: vello::kurbo::Affine,
    pub bounds: vello::kurbo::Rect,
}

impl RenderContext {
    pub(crate) fn with_renderer(renderer: &mut HydrolysisRenderer, bounds: vello::kurbo::Rect) -> Self {
        Self {
            renderer_ptr: renderer as *mut HydrolysisRenderer,
            transform: vello::kurbo::Affine::IDENTITY,
            bounds,
        }
    }

    /// # Safety
    /// The caller guarantees the render context belongs to an active render pass.
    pub unsafe fn renderer(&self) -> &mut HydrolysisRenderer {
        unsafe { &mut *self.renderer_ptr }
    }

    /// # Safety
    /// The caller guarantees the render context belongs to an active render pass.
    pub unsafe fn scene(&self) -> &mut vello::Scene {
        unsafe { &mut (*self.renderer_ptr).scene }
    }

    #[must_use]
    pub fn child(&self, transform: vello::kurbo::Affine, bounds: vello::kurbo::Rect) -> Self {
        Self {
            renderer_ptr: self.renderer_ptr,
            transform: self.transform * transform,
            bounds,
        }
    }
}

/// Core hydrolysis renderer state.
pub struct HydrolysisRenderer {
    dispatcher: ViewDispatcher<HydroState, RenderContext, ()>,
    vello_renderer: vello::Renderer,
    scene: vello::Scene,
}

#[derive(Debug, Clone, Copy)]
struct HydroSubview {
    stretch_axis: StretchAxis,
    intrinsic: LayoutSize,
}

impl HydroSubview {
    fn from_view(view: &AnyView) -> Self {
        Self {
            stretch_axis: view.stretch_axis(),
            intrinsic: estimate_intrinsic_size(view),
        }
    }
}

impl SubView for HydroSubview {
    fn size_that_fits(&self, proposal: ProposalSize) -> LayoutSize {
        let width = if self.stretch_axis.stretches_horizontal() {
            proposal.width.unwrap_or(self.intrinsic.width)
        } else {
            proposal
                .width
                .map_or(self.intrinsic.width, |value| self.intrinsic.width.min(value))
        };

        let height = if self.stretch_axis.stretches_vertical() {
            proposal.height.unwrap_or(self.intrinsic.height)
        } else {
            proposal
                .height
                .map_or(self.intrinsic.height, |value| self.intrinsic.height.min(value))
        };

        LayoutSize::new(width, height)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis
    }

    fn priority(&self) -> i32 {
        0
    }
}

impl core::fmt::Debug for HydroState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroState").finish_non_exhaustive()
    }
}

impl core::fmt::Debug for HydrolysisRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisRenderer")
            .field("dispatcher", &self.dispatcher)
            .finish_non_exhaustive()
    }
}

impl HydrolysisRenderer {
    #[must_use]
    pub fn new(device: &wgpu::Device) -> Self {
        Self::new_with_options(
            device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: std::num::NonZeroUsize::new(1),
                pipeline_cache: None,
            },
        )
    }

    #[must_use]
    pub fn new_with_options(device: &wgpu::Device, options: vello::RendererOptions) -> Self {
        let mut dispatcher = ViewDispatcher::with_state(HydroState::default());
        Self::register_core_handlers(&mut dispatcher);

        let vello_renderer =
            vello::Renderer::new(device, options).expect("failed to create hydrolysis renderer");
        Self {
            dispatcher,
            vello_renderer,
            scene: vello::Scene::new(),
        }
    }

    fn register_core_handlers(dispatcher: &mut ViewDispatcher<HydroState, RenderContext, ()>) {
        dispatcher.register::<Native<()>>(|_state, _ctx, _unit, _env| ());
        dispatcher.register::<Native<Spacer>>(|_state, _ctx, _spacer, _env| ());
        dispatcher.register::<Str>(|_state, _ctx, _str, _env| ());
        dispatcher.register::<Native<TextConfig>>(|_state, _ctx, _text, _env| ());

        dispatcher.register::<Native<FixedContainer>>(Self::render_fixed_container);
        dispatcher.register::<Native<ResolvedColor>>(Self::render_resolved_color);
        dispatcher.register::<Native<ResolvedGradient>>(Self::render_resolved_gradient);
        dispatcher.register::<Native<ResolvedShape>>(Self::render_resolved_shape);

        dispatcher.register::<Metadata<Environment>>(Self::render_environment_metadata);
        dispatcher.register::<Metadata<Retain>>(Self::render_retain_metadata);
        dispatcher.register::<Metadata<Opacity>>(Self::render_opacity_metadata);
    }

    fn render_fixed_container(
        _state: &mut HydroState,
        ctx: RenderContext,
        container: Native<FixedContainer>,
        env: &Environment,
    ) {
        let (layout, children) = container.into_inner().into_inner();
        let subviews: Vec<HydroSubview> = children.iter().map(HydroSubview::from_view).collect();
        let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();

        let proposal = ProposalSize::new(Some(ctx.bounds.width() as f32), Some(ctx.bounds.height() as f32));
        let _ = layout.size_that_fits(proposal, &refs);
        let bounds = LayoutRect::from_size(LayoutSize::new(
            ctx.bounds.width() as f32,
            ctx.bounds.height() as f32,
        ));
        let child_rects = layout.place(bounds, &refs);

        let renderer = unsafe { ctx.renderer() };
        for (child, rect) in children.into_iter().zip(child_rects) {
            let child_transform = vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
            let child_bounds = vello::kurbo::Rect::new(
                0.0,
                0.0,
                f64::from(rect.width()),
                f64::from(rect.height()),
            );
            renderer
                .dispatcher
                .dispatch(child, env, ctx.child(child_transform, child_bounds));
        }
    }

    fn render_resolved_color(
        _state: &mut HydroState,
        ctx: RenderContext,
        color: Native<ResolvedColor>,
        _env: &Environment,
    ) {
        let scene = unsafe { ctx.scene() };
        let brush = resolved_color_to_peniko(color.into_inner());
        scene.fill(vello::peniko::Fill::NonZero, ctx.transform, brush, None, &ctx.bounds);
    }

    fn render_resolved_gradient(
        _state: &mut HydroState,
        ctx: RenderContext,
        gradient: Native<ResolvedGradient>,
        _env: &Environment,
    ) {
        let scene = unsafe { ctx.scene() };
        let brush = resolved_gradient_to_brush(&gradient.into_inner(), ctx.bounds);
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            &brush,
            None,
            &ctx.bounds,
        );
    }

    fn render_resolved_shape(
        _state: &mut HydroState,
        ctx: RenderContext,
        shape: Native<ResolvedShape>,
        _env: &Environment,
    ) {
        let resolved = shape.into_inner();
        let path = resolved_shape_to_path(&resolved, ctx.bounds);
        let fill = resolved_color_to_peniko(resolved.fill);
        let scene = unsafe { ctx.scene() };
        scene.fill(vello::peniko::Fill::NonZero, ctx.transform, fill, None, &path);
    }

    fn render_environment_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Environment>,
        _env: &Environment,
    ) {
        let renderer = unsafe { ctx.renderer() };
        renderer
            .dispatcher
            .dispatch(metadata.content, &metadata.value, ctx);
    }

    fn render_retain_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Retain>,
        env: &Environment,
    ) {
        let retain = metadata.value;
        let renderer = unsafe { ctx.renderer() };
        renderer.dispatcher.dispatch(metadata.content, env, ctx);
        drop(retain);
    }

    fn render_opacity_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Opacity>,
        env: &Environment,
    ) {
        let alpha = metadata.value.value.get();
        let scene = unsafe { ctx.scene() };
        scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            ctx.transform,
            &ctx.bounds,
        );

        let renderer = unsafe { ctx.renderer() };
        renderer.dispatcher.dispatch(metadata.content, env, ctx);
        scene.pop_layer();
    }

    #[must_use]
    pub fn state(&self) -> &HydroState {
        self.dispatcher.state()
    }

    pub fn state_mut(&mut self) -> &mut HydroState {
        self.dispatcher.state_mut()
    }

    #[must_use]
    pub fn scene(&self) -> &vello::Scene {
        &self.scene
    }

    pub fn scene_mut(&mut self) -> &mut vello::Scene {
        &mut self.scene
    }

    pub fn vello_renderer(&mut self) -> &mut vello::Renderer {
        &mut self.vello_renderer
    }

    pub fn dispatcher_mut(&mut self) -> &mut ViewDispatcher<HydroState, RenderContext, ()> {
        &mut self.dispatcher
    }

    pub fn dispatch<V: View>(&mut self, view: V, env: &Environment, bounds: vello::kurbo::Rect) {
        let ctx = RenderContext::with_renderer(self, bounds);
        self.dispatcher.dispatch(view, env, ctx);
    }

    pub fn render_scene_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let params = vello::RenderParams {
            base_color: vello::peniko::Color::TRANSPARENT,
            width,
            height,
            antialiasing_method: vello::AaConfig::Area,
        };
        self.vello_renderer
            .render_to_texture(device, queue, &self.scene, target, &params)
            .expect("hydrolysis renderer: failed to render scene");
    }
}

fn estimate_intrinsic_size(view: &AnyView) -> LayoutSize {
    if let Some(text) = view.downcast_ref::<Str>() {
        return LayoutSize::new(text.len() as f32 * 8.0, 20.0);
    }

    if view.is::<Native<TextConfig>>() {
        return LayoutSize::new(120.0, 20.0);
    }

    if view.stretch_axis().stretches_any() {
        return LayoutSize::zero();
    }

    LayoutSize::new(44.0, 44.0)
}

fn resolved_color_to_peniko(color: ResolvedColor) -> vello::peniko::Color {
    let srgb = color.to_srgb_with_headroom();
    vello::peniko::Color::new([srgb.red, srgb.green, srgb.blue, color.opacity])
}

fn resolved_gradient_to_brush(
    gradient: &ResolvedGradient,
    bounds: vello::kurbo::Rect,
) -> vello::peniko::Brush {
    let mut stops: Vec<vello::peniko::ColorStop> = gradient
        .stops
        .iter()
        .map(to_peniko_stop)
        .collect();

    let brush = match gradient.gradient_type {
        GradientType::Linear => {
            let start = resolved_point_to_kurbo(gradient.start_point, bounds);
            let end = resolved_point_to_kurbo(gradient.end_point, bounds);
            vello::peniko::Gradient::new_linear(start, end).with_stops(&*stops)
        }
        GradientType::Radial => {
            let center = resolved_point_to_kurbo(gradient.start_point, bounds);
            let radius_scale = bounds.width().min(bounds.height()) as f32;
            let start_radius = gradient.start_value * radius_scale;
            let end_radius = gradient.end_value * radius_scale;
            vello::peniko::Gradient::new_two_point_radial(
                center,
                start_radius,
                center,
                end_radius,
            )
            .with_stops(&*stops)
        }
        GradientType::Angular => {
            let sweep = gradient.end_value - gradient.start_value;
            let sweep_fraction = f64::from(sweep) / TAU;
            if sweep_fraction < 1.0 {
                let last_color = stops
                    .last()
                    .expect("resolved gradient must contain at least one stop")
                    .color;
                for stop in &mut stops {
                    stop.offset = (f64::from(stop.offset) * sweep_fraction) as f32;
                }
                stops.push(vello::peniko::ColorStop {
                    offset: sweep_fraction as f32,
                    color: last_color,
                });
                stops.push(vello::peniko::ColorStop {
                    offset: 1.0,
                    color: last_color,
                });
            }
            let center = resolved_point_to_kurbo(gradient.start_point, bounds);
            vello::peniko::Gradient::new_sweep(center, gradient.start_value, 0.0)
                .with_stops(&*stops)
        }
        GradientType::Mesh => {
            panic!("resolved mesh gradient must not be dispatched through ResolvedGradient")
        }
    };

    vello::peniko::Brush::Gradient(brush)
}

fn resolved_point_to_kurbo(point: [f32; 2], bounds: vello::kurbo::Rect) -> vello::kurbo::Point {
    vello::kurbo::Point::new(
        f64::from(point[0]) * bounds.width(),
        f64::from(point[1]) * bounds.height(),
    )
}

fn to_peniko_stop(stop: &ResolvedGradientStop) -> vello::peniko::ColorStop {
    vello::peniko::ColorStop {
        offset: stop.position,
        color: resolved_color_to_peniko(stop.color).into(),
    }
}

fn resolved_shape_to_path(shape: &ResolvedShape, bounds: vello::kurbo::Rect) -> vello::kurbo::BezPath {
    let width = bounds.width();
    let height = bounds.height();
    let mut path = vello::kurbo::BezPath::new();
    let mut has_current = false;

    for command in &shape.commands {
        match command {
            PathCommand::MoveTo { x, y } => {
                path.move_to(vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height));
                has_current = true;
            }
            PathCommand::LineTo { x, y } => {
                if !has_current {
                    panic!("PathCommand::LineTo requires an active current point");
                }
                path.line_to(vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height));
            }
            PathCommand::QuadTo { cx, cy, x, y } => {
                if !has_current {
                    panic!("PathCommand::QuadTo requires an active current point");
                }
                path.quad_to(
                    vello::kurbo::Point::new(f64::from(*cx) * width, f64::from(*cy) * height),
                    vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                if !has_current {
                    panic!("PathCommand::CubicTo requires an active current point");
                }
                path.curve_to(
                    vello::kurbo::Point::new(f64::from(*c1x) * width, f64::from(*c1y) * height),
                    vello::kurbo::Point::new(f64::from(*c2x) * width, f64::from(*c2y) * height),
                    vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => {
                let center_x = f64::from(*cx) * width;
                let center_y = f64::from(*cy) * height;
                let radius_x = f64::from(*rx) * width;
                let radius_y = f64::from(*ry) * height;
                let start = f64::from(*start);
                let step = f64::from(*sweep) / 32.0;

                let start_point = vello::kurbo::Point::new(
                    center_x + radius_x * start.cos(),
                    center_y + radius_y * start.sin(),
                );
                if has_current {
                    path.line_to(start_point);
                } else {
                    path.move_to(start_point);
                    has_current = true;
                }

                let mut angle = start;
                for _ in 0..32 {
                    angle += step;
                    path.line_to(vello::kurbo::Point::new(
                        center_x + radius_x * angle.cos(),
                        center_y + radius_y * angle.sin(),
                    ));
                }
            }
            PathCommand::Close => {
                path.close_path();
                has_current = false;
            }
        }
    }

    path
}
