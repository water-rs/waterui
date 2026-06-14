//! Video Player Example - Video playback with selectable sources.
//!
//! This example showcases:
//! - `VideoPlayer` driven by a reactive `Url` source
//! - Buffering / playback status surfaced from `video::Event`
//! - Source switching via selector pills
//!
//! ## Native vs self-rendered (Rust fallback) playback
//!
//! By default the example uses the platform's native video player. Enable the
//! `rust-fallback` feature to route playback through WaterUI's self-rendered
//! pipeline (`GpuSurface` + `waterkit-codec`) and exercise the real HDR10 test
//! sources:
//!
//! ```bash
//! water run -p video-player-example --features rust-fallback
//! ```

use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;

/// A selectable video source.
#[derive(Clone, Copy)]
struct Sample {
    /// Human-readable title shown in the selector and status bar.
    title: &'static str,
    /// Color/dynamic-range profile, surfaced so the fallback HDR path is observable.
    profile: &'static str,
    /// Direct media URL.
    url: &'static str,
}

#[cfg(not(feature = "rust-fallback"))]
const SAMPLES: &[Sample] = &[
    Sample {
        title: "Big Buck Bunny 1MB",
        profile: "SDR / BT.709",
        url: "https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/720/Big_Buck_Bunny_720_10s_1MB.mp4",
    },
    Sample {
        title: "Big Buck Bunny 5MB",
        profile: "SDR / BT.709",
        url: "https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/720/Big_Buck_Bunny_720_10s_5MB.mp4",
    },
    Sample {
        title: "Sintel",
        profile: "SDR / BT.709",
        url: "https://test-videos.co.uk/vids/sintel/mp4/h264/720/Sintel_720_10s_1MB.mp4",
    },
];

#[cfg(feature = "rust-fallback")]
const SAMPLES: &[Sample] = &[
    Sample {
        title: "Big Buck Bunny (SDR)",
        profile: "SDR / BT.709",
        url: "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4",
    },
    Sample {
        title: "Sintel (SDR)",
        profile: "SDR / BT.709",
        url: "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/Sintel.mp4",
    },
    Sample {
        title: "Jellyfin HDR10 1080p 3M",
        profile: "HDR10 / BT.2020 + PQ",
        url: "https://repo.jellyfin.org/test-videos/HDR/HDR10/HEVC/Test%20Jellyfin%201080p%20HEVC%20HDR10%203M.mp4",
    },
    Sample {
        title: "Jellyfin HDR10 1080p 10M",
        profile: "HDR10 / BT.2020 + PQ",
        url: "https://repo.jellyfin.org/test-videos/HDR/HDR10/HEVC/Test%20Jellyfin%201080p%20HEVC%20HDR10%2010M.mp4",
    },
];

/// Self-contained entry: builds the player, status, and source selector.
pub fn demo() -> impl View {
    let selected = Binding::usize(0);
    let status: Binding<Str> = binding("Idle");
    let is_buffering = binding(false);

    let source = selected
        .clone()
        .map(|index| Url::parse(SAMPLES[index].url).expect("sample URL should be valid"));

    let player = VideoPlayer::new(source)
        .show_controls(true)
        .aspect_ratio(AspectRatio::Fit)
        .on_event({
            let status = status.clone();
            let is_buffering = is_buffering.clone();
            let selected = selected.clone();
            move |event| match event {
                video::Event::ReadyToPlay => {
                    is_buffering.set(false);
                    status.set(Str::from_static("Ready"));
                }
                video::Event::Buffering => {
                    is_buffering.set(true);
                    status.set(Str::from_static("Buffering..."));
                }
                video::Event::BufferingEnded => {
                    is_buffering.set(false);
                    status.set(Str::from_static("Playing"));
                }
                video::Event::PictureInPictureChanged { active } => {
                    status.set(if active {
                        Str::from_static("Picture in Picture")
                    } else {
                        Str::from_static("Inline Playback")
                    });
                }
                video::Event::BufferLevel { .. } | video::Event::PlaybackMetrics { .. } => {}
                video::Event::NextRequested => {
                    selected.set((selected.get() + 1) % SAMPLES.len());
                }
                video::Event::PreviousRequested => {
                    selected.set((selected.get() + SAMPLES.len() - 1) % SAMPLES.len());
                }
                video::Event::Ended => {
                    is_buffering.set(false);
                    status.set(Str::from_static("Ended"));
                }
                video::Event::Error { message } => {
                    is_buffering.set(false);
                    status.set(format!("Error: {message}").into());
                }
            }
        });

    player_shell(player, selected, status, is_buffering)
}

/// Static preview frame — keeps `water preview` and Hydrolysis perf tests off the network.
#[preview]
fn video_player_preview() -> impl View {
    let selected = Binding::usize(0);
    let status: Binding<Str> = binding("Preview ready");
    let is_buffering = binding(false);
    let frame = zstack((
        Srgb::BLACK,
        vstack((
            text("Video Preview").title().foreground(Srgb::WHITE),
            text("Static preview frame")
                .caption()
                .foreground(Srgb::WHITE.with_opacity(0.8)),
        ))
        .spacing(12.0),
    ));
    player_shell(frame, selected, status, is_buffering)
}

/// Shared layout: title, player with a buffering overlay, status line, and source pills.
fn player_shell(
    player: impl View,
    selected: Binding<usize>,
    status: Binding<Str>,
    is_buffering: Binding<bool>,
) -> impl View {
    let title = selected.clone().map(|index| SAMPLES[index].title);
    let profile = selected.clone().map(|index| SAMPLES[index].profile);

    let buffering_overlay = vstack((loading(), text("Buffering...").foreground(Srgb::WHITE)))
        .spacing(12.0)
        .background(Srgb::BLACK.with_opacity(0.8))
        .visible(is_buffering.clone());

    let pills = SAMPLES
        .iter()
        .enumerate()
        .map(|(index, sample)| pill_button(sample.title, index, &selected))
        .collect::<Vec<_>>();

    vstack((
        text("WaterUI Video Player").headline(),
        text!("Now Playing: {title}").body(),
        text!("Source Profile: {profile}").footnote(),
        overlay(player, buffering_overlay).height(360.0),
        text!("Status: {status}").footnote(),
        hstack(pills).spacing(12.0),
    ))
    .spacing(12.0)
    .padding()
}

/// Pill-style selection button reflecting the active source.
fn pill_button(label: &'static str, index: usize, selected: &Binding<usize>) -> impl View {
    let is_selected = selected.clone().map(move |s| s == index);
    let selected_for_action = selected.clone();
    let selected_bg_opacity = is_selected.clone().select(1.0, 0.0);
    let idle_bg_opacity = is_selected.select(0.0, 1.0);

    zstack((
        Srgb::WHITE.with_opacity(0.15).opacity(idle_bg_opacity),
        Srgb::WHITE.with_opacity(0.35).opacity(selected_bg_opacity),
        button(label)
            .action(move |State(s): State<Binding<usize>>| s.set(index))
            .state(&selected_for_action),
    ))
}

pub fn app(env: Environment) -> App {
    #[cfg_attr(not(feature = "rust-fallback"), expect(unused_mut))]
    let mut env = env;
    #[cfg(feature = "rust-fallback")]
    video::install_rust_player_hooks(&mut env);
    App::new(demo, env)
}
