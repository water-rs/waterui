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
//! WaterUI's self-rendered pipeline (`GpuContentView` + `waterkit-codec`), depending
//! on the selected backend/runtime policy.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use waterui::ContentLedger;
use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::grid::{grid as layout_grid, row as grid_row};
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::video::{
    MediaItem, MediaItemId, PlaybackSession, PlayerController, Playlist, Url, VideoPlayer,
};
use waterui::widget::condition::when;

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
    let item_ids = items.iter().map(|item| item.get_id()).collect::<Vec<_>>();
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
                _ => {}
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

/// Real player for the first sample URL.
///
/// The capture waits until the player reports ready, so the image shows the
/// player, its controls, and its first frame.
#[preview]
fn video_player_preview() -> impl View {
    PreviewPlayer
}

/// The player a preview mounts, opened from [`SAMPLES`] rather than a stand-in.
struct PreviewPlayer;

impl View for PreviewPlayer {
    fn body(self, env: &Environment) -> impl View {
        let url = Url::parse(SAMPLES[0].url).expect("sample URL should be valid");
        let status: Binding<Str> = binding("Opening");
        let owed = Rc::new(RefCell::new(ContentLedger::owe::<Self>(
            env,
            url.as_str().to_owned(),
        )));
        let player = VideoPlayer::new(PlaybackSession::new(Playlist::single(url)))
            .show_controls(true)
            .content_mode(video::ContentMode::Fit)
            .on_event({
                let status = status.clone();
                move |event| match event {
                    video::Event::ReadyToPlay => {
                        if let Some(owed) = owed.borrow_mut().take() {
                            owed.fulfil();
                        }
                        status.set(Str::from_static("Ready"));
                    }
                    video::Event::Error { message } => {
                        if let Some(owed) = owed.borrow_mut().take() {
                            owed.fail(PlaybackFailed(message.clone()));
                        }
                        status.set(format!("Error: {message}").into());
                    }
                    video::Event::Buffering => {
                        status.set(Str::from_static("Buffering..."));
                    }
                    video::Event::BufferingEnded => {
                        status.set(Str::from_static("Ready"));
                    }
                    video::Event::PlaybackStateChanged { playing } => {
                        status.set(if playing {
                            Str::from_static("Playing")
                        } else {
                            Str::from_static("Paused")
                        });
                    }
                    _ => {}
                }
            });

        // The buffering overlay is a repeating animation. A capture cannot
        // settle while one is mounted, so the preview leaves it off and
        // reports buffering through the status line.
        player_shell(
            player,
            Binding::usize(0).computed(),
            status,
            binding(false),
            None,
        )
    }
}

/// Playback error reported to a capture when the sample fails to open.
#[derive(Debug)]
struct PlaybackFailed(String);

impl fmt::Display for PlaybackFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PlaybackFailed {}

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

    // Mount the indicator only while buffering. `.visible(false)` would keep
    // the indeterminate animation running, and a capture cannot settle on it.
    let buffering_overlay = when(is_buffering.clone(), || {
        vstack((loading(), text("Buffering...").foreground(Srgb::WHITE)))
            .spacing(12.0)
            .background(Srgb::BLACK.with_opacity(0.8))
    });

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
        // A `Fit` player is as tall as its source's aspect ratio makes it at
        // the width it is given.
        overlay(player, buffering_overlay).max_width(640.0),
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
    let is_selected = selected.clone().equal_to(index);
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
