//! Video Player Example - Video playback with selectable sources.
//!
//! This example showcases:
//! - `VideoPlayer` driven by a reactive `Url` source
//! - Buffering / playback status surfaced from `video::Event`
//! - Source switching via selector pills
//!
//! ## Native vs self-rendered playback
//!
//! The same SDR and HDR10 sources exercise either the platform-native player or
//! WaterUI's self-rendered pipeline (`GpuSurface` + `waterkit-codec`), depending
//! on the selected backend/runtime policy.

use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::grid::{grid as layout_grid, row as grid_row};
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
    let status: Binding<Str> = binding("Idle");
    let is_buffering = binding(false);
    let items = SAMPLES
        .iter()
        .map(|sample| MediaItem::from(Url::parse(sample.url).expect("sample URL should be valid")))
        .collect::<Vec<_>>();
    let item_ids = items.iter().map(|item| item.id).collect::<Vec<_>>();
    let session = PlaybackSession::new(Playlist::new(items[0].clone(), items.into_iter().skip(1)));
    let controller = session.controller();
    let selected = controller.current_item_index();

    let player = VideoPlayer::new(session)
        .show_controls(true)
        .content_mode(video::ContentMode::Fit)
        .on_event({
            let status = status.clone();
            let is_buffering = is_buffering.clone();
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
                    status.set(Str::from_static("Ready"));
                }
                video::Event::PlaybackStateChanged { playing } => {
                    status.set(if playing {
                        Str::from_static("Playing")
                    } else {
                        Str::from_static("Paused")
                    });
                }
                video::Event::PictureInPictureChanged { active } => {
                    status.set(if active {
                        Str::from_static("Picture in Picture")
                    } else {
                        Str::from_static("Inline Playback")
                    });
                }
                video::Event::ExternalPlaybackChanged { active } => {
                    status.set(if active {
                        Str::from_static("External Playback")
                    } else {
                        Str::from_static("Local Playback")
                    });
                }
                video::Event::PlaybackOutputPathChanged { path } => {
                    status.set(format!("Output: {path:?}").into());
                }
                video::Event::TimedMetadata { .. } => {
                    status.set(Str::from_static("Timed Metadata"));
                }
                video::Event::OfflineDrmKeySetChanged { .. } => {
                    status.set(Str::from_static("Offline DRM Key Updated"));
                }
                video::Event::BufferLevel { .. } | video::Event::PlaybackMetrics { .. } => {}
                video::Event::NextRequested | video::Event::PreviousRequested => {}
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

    player_shell(
        player,
        selected,
        status,
        is_buffering,
        Some((controller, item_ids)),
    )
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
    player_shell(frame, selected.computed(), status, is_buffering, None)
}

/// Shared layout: title, player with a buffering overlay, status line, and source pills.
fn player_shell(
    player: impl View,
    selected: Computed<usize>,
    status: Binding<Str>,
    is_buffering: Binding<bool>,
    playlist: Option<(PlayerController, Vec<MediaItemId>)>,
) -> impl View {
    let title = selected.clone().map(|index| SAMPLES[index].title);
    let profile = selected.clone().map(|index| SAMPLES[index].profile);

    let buffering_overlay = vstack((loading(), text("Buffering...").foreground(Srgb::WHITE)))
        .spacing(12.0)
        .background(Srgb::BLACK.with_opacity(0.8))
        .visible(is_buffering.clone());

    let source_grid = playlist.map(|(controller, ids)| {
        layout_grid(
            3,
            [
                grid_row((
                    pill_button(SAMPLES[0].title, 0, ids[0], &selected, &controller),
                    pill_button(SAMPLES[1].title, 1, ids[1], &selected, &controller),
                    pill_button(SAMPLES[2].title, 2, ids[2], &selected, &controller),
                )),
                grid_row((
                    pill_button(SAMPLES[3].title, 3, ids[3], &selected, &controller),
                    pill_button(SAMPLES[4].title, 4, ids[4], &selected, &controller),
                )),
            ],
        )
        .spacing(12.0)
    });

    vstack((
        text("WaterUI Video Player").headline(),
        text!("Now Playing: {title}").body(),
        text!("Source Profile: {profile}").footnote(),
        overlay(player, buffering_overlay).height(360.0),
        text!("Status: {status}").footnote(),
        source_grid,
    ))
    .spacing(12.0)
    .padding()
}

/// Pill-style selection button reflecting the active source.
fn pill_button(
    label: &'static str,
    index: usize,
    item_id: MediaItemId,
    selected: &Computed<usize>,
    controller: &PlayerController,
) -> impl View {
    let is_selected = selected.clone().map(move |s| s == index);
    let controller = controller.clone();
    let selected_bg_opacity = is_selected.clone().select(1.0, 0.0);
    let idle_bg_opacity = is_selected.select(0.0, 1.0);

    zstack((
        Srgb::WHITE.with_opacity(0.15).opacity(idle_bg_opacity),
        Srgb::WHITE.with_opacity(0.35).opacity(selected_bg_opacity),
        button(label).action(move || {
            controller
                .seek_to_item(item_id)
                .expect("sample item must remain in the player playlist");
        }),
    ))
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
