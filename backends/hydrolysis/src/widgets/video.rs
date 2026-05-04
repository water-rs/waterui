use std::rc::Rc;

#[cfg(feature = "accessibility")]
use accesskit::{Node as AccessibilityNode, Role as AccessibilityNodeRole};
use nami::{Binding, SignalExt};
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityChildren;
use waterui_controls::{button, slider::slider};
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{AnyView, Environment, Native};
use waterui_graphics::Gradient;
use waterui_graphics::color::Srgb;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::{hstack, vstack};
use waterui_text::Text;
use waterui_video::{VideoConfig, VideoPlayerConfig, Volume};

use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    local_shared, measure_view_dimensions_with_proposal, measure_view_intrinsic,
    normalize_view_for_render, transformed_rect,
};

const VIDEO_SURFACE_HEIGHT: f32 = 160.0;
const VIDEO_PLAYBACK_RATE_MIN: f64 = 0.25;
const VIDEO_PLAYBACK_RATE_MAX: f64 = 4.0;

fn video_dimensions_from_proposal(proposal: ProposalSize, intrinsic: LayoutSize) -> ViewDimensions {
    ViewDimensions::new(LayoutSize::new(
        proposal
            .width
            .filter(|width| width.is_finite())
            .unwrap_or(intrinsic.width)
            .max(0.0),
        proposal
            .height
            .filter(|height| height.is_finite())
            .unwrap_or(intrinsic.height)
            .max(0.0),
    ))
}

struct HydroVideoPlayerState {
    is_playing: Binding<bool>,
    progress: Binding<f64>,
}

impl core::fmt::Debug for HydroVideoPlayerState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroVideoPlayerState")
            .finish_non_exhaustive()
    }
}

impl HydroVideoPlayerState {
    fn new() -> Self {
        Self {
            is_playing: Binding::bool(false),
            progress: Binding::f64(0.0),
        }
    }
}

fn hydro_video_player_state(env: &Environment) -> Rc<HydroVideoPlayerState> {
    local_shared(env, HydroVideoPlayerState::new)
}

fn muted_binding(volume: &Binding<Volume>) -> Binding<bool> {
    Binding::mapping(volume, |current| current.is_sign_negative(), {
        move |volume_binding, requested: bool| {
            let magnitude = volume_binding.get().abs().max(0.1);
            volume_binding.set(if requested { -magnitude } else { magnitude });
        }
    })
}

fn volume_level_binding(volume: &Binding<Volume>) -> Binding<f64> {
    Binding::mapping(
        volume,
        |current| f64::from(current.abs().clamp(0.0, 1.0)),
        {
            let volume = volume.clone();
            move |volume_binding, requested: f64| {
                let next = requested.clamp(0.0, 1.0) as f32;
                volume_binding.set(if volume.get().is_sign_negative() {
                    -next
                } else {
                    next
                });
            }
        },
    )
}

fn playback_rate_binding(playback_rate: &Binding<f32>) -> Binding<f64> {
    Binding::mapping(
        playback_rate,
        |current| {
            f64::from(current.clamp(
                VIDEO_PLAYBACK_RATE_MIN as f32,
                VIDEO_PLAYBACK_RATE_MAX as f32,
            ))
        },
        move |playback_rate_binding, requested: f64| {
            playback_rate_binding
                .set(requested.clamp(VIDEO_PLAYBACK_RATE_MIN, VIDEO_PLAYBACK_RATE_MAX) as f32);
        },
    )
}

fn video_surface_view() -> impl waterui_core::View {
    spacer()
        .height(VIDEO_SURFACE_HEIGHT)
        .background(Srgb::BLACK)
}

fn raw_video_content(env: &Environment) -> AnyView {
    let content = AnyView::new(
        Gradient::linear(
            vec![(0.0, Srgb::BLACK.resolve()), (1.0, Srgb::BLACK.resolve())],
            [0.0, 0.0],
            [1.0, 1.0],
        )
        .height(VIDEO_SURFACE_HEIGHT),
    );
    normalize_view_for_render(content, env)
}

fn video_player_content(config: &VideoPlayerConfig, env: &Environment) -> AnyView {
    let state = hydro_video_player_state(env);
    let is_playing = state.is_playing.clone();
    let progress = state.progress.clone();
    let muted = muted_binding(&config.volume);
    let volume_level = volume_level_binding(&config.volume);
    let playback_rate = playback_rate_binding(&config.playback_rate);
    let previous = button("Previous")
        .action(|| {})
        .visible(config.has_previous.clone());
    let next = button("Next")
        .action(|| {})
        .visible(config.has_next.clone());

    let play_pause = {
        let is_playing_for_label = is_playing.clone();
        let is_playing_for_action = is_playing.clone();
        button(Text::display(
            is_playing_for_label.map(|playing| if playing { "Pause" } else { "Play" }),
        ))
        .action(move || is_playing_for_action.set(!is_playing_for_action.get()))
    };

    let mute_button = {
        let muted_for_label = muted.clone();
        let muted_for_action = muted.clone();
        button(Text::display(
            muted_for_label.map(|is_muted| if is_muted { "Unmute" } else { "Mute" }),
        ))
        .action(move || muted_for_action.set(!muted_for_action.get()))
    };

    let timeline = slider(&progress).label("Timeline");
    let volume = slider(&volume_level).label("Volume");
    let speed = slider(&playback_rate)
        .range(VIDEO_PLAYBACK_RATE_MIN..=VIDEO_PLAYBACK_RATE_MAX)
        .label("Speed");

    normalize_view_for_render(
        AnyView::new(
            vstack((
                video_surface_view(),
                hstack((previous, play_pause, mute_button, next)).spacing(8.0),
                timeline,
                volume,
                speed,
            ))
            .spacing(8.0),
        ),
        env,
    )
}

impl HydroNativeView for Native<VideoConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, _view: Self, env: &Environment) {
        let render_env = env.extending(AccessibilityChildren::ExcludeDescendants);
        ctx.dispatch_in_rect(&render_env, raw_video_content(&render_env), ctx.bounds);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        state: &mut HydroState,
        _view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        measure_view_dimensions_with_proposal(&raw_video_content(env), proposal, state, env)
    }

    fn accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        _view: &Self,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let _ = renderer.register_accessibility_node(
                AccessibilityNode::new(
                    renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Image),
                ),
                transformed_rect(ctx.hit_transform, ctx.bounds),
                env,
                None,
            );
        }
    }
}

impl HydroNativeView for Native<VideoPlayerConfig> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        ctx.dispatch_in_rect(env, video_player_content(view.as_inner(), env), ctx.bounds);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_view_intrinsic(&video_player_content(view.as_inner(), env), state, env)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let content = video_player_content(view.as_inner(), env);
        let intrinsic = measure_view_intrinsic(&content, state, env);
        video_dimensions_from_proposal(proposal, intrinsic)
    }

    fn accessibility_is_render_driven() -> bool {
        true
    }
}
