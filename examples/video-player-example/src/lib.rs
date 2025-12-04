//! Video Player Example - Demonstrates WaterUI's video playback capabilities
//!
//! This example showcases:
//! - VideoPlayer with native controls
//! - Overlay for buffering indicator
//! - ProgressView for loading state
//! - Reactive state management for playback events

use waterui::color::Srgb;
use waterui::layout::frame::Frame;
use waterui::layout::stack::{Alignment, HorizontalAlignment};
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::widget::condition::when;

pub fn init() -> Environment {
    Environment::new()
}

/// A video player component with buffering overlay
struct BufferingVideoPlayer {
    video_url: &'static str,
}

impl BufferingVideoPlayer {
    fn new(url: &'static str) -> Self {
        Self { video_url: url }
    }
}

impl View for BufferingVideoPlayer {
    fn body(self, _env: &Environment) -> impl View {
        // Track buffering state
        let is_buffering = binding(false);

        // Create the video player with native controls
        let player = VideoPlayer::new(Video::new(
            url::Url::parse(self.video_url).expect("Invalid video URL"),
        ))
        .show_controls(true)
        .aspect_ratio(AspectRatio::Fit)
        .on_event({
            let is_buffering = is_buffering.clone();
            move |event| match event {
                video::Event::Buffering => {
                    is_buffering.set(true);
                }
                video::Event::BufferingEnded | video::Event::ReadyToPlay => {
                    is_buffering.set(false);
                }
                video::Event::Ended => {
                    // Video finished playing
                }
                video::Event::Error { message } => {
                    // Log error - in a real app you might show an error view
                    #[cfg(debug_assertions)]
                    {
                        let _ = message;
                    }
                }
            }
        });

        // Create buffering overlay with centered spinner
        let buffering_overlay = when(is_buffering, || {
            // When buffering: show a semi-transparent overlay with spinner
            zstack((
                // Semi-transparent background
                spacer().background(Color::from(Srgb::BLACK).with_opacity(0.4)),
                // Centered loading indicator with label
                vstack((
                    loading(),
                    text("Buffering...").foreground(Color::from(Srgb::WHITE)),
                ))
                .spacing(12.0),
            ))
            .alignment(Alignment::Center)
        });

        // Combine player with overlay
        overlay(player, buffering_overlay)
    }
}

pub fn main() -> impl View {
    // Sample video URLs (Big Buck Bunny - open source test videos)
    let sample_videos: [(&str, &str); 3] = [
        (
            "Big Buck Bunny (720p)",
            "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4",
        ),
        (
            "Elephant Dream",
            "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/ElephantsDream.mp4",
        ),
        (
            "Sintel",
            "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/Sintel.mp4",
        ),
    ];

    // Track which video is selected
    let selected_index = binding(0usize);

    // Header section
    let header = vstack((
        text("Video Player Demo").size(28.0),
        "Demonstrating VideoPlayer with buffering overlay",
    ));

    // Video selection buttons
    let selection_buttons = hstack((
        button("Video 1").action({
            let selected = selected_index.clone();
            move || selected.set(0)
        }),
        button("Video 2").action({
            let selected = selected_index.clone();
            move || selected.set(1)
        }),
        button("Video 3").action({
            let selected = selected_index.clone();
            move || selected.set(2)
        }),
    ))
    .spacing(8.0);

    // Now playing title (reactive)
    let now_playing = Dynamic::watch(selected_index.clone(), move |idx| {
        let (title, _) = sample_videos[idx];
        text(title).foreground(theme_color::Accent)
    });

    // Video player (reactive)
    let player = Dynamic::watch(selected_index, move |idx| {
        let (_, url) = sample_videos[idx];
        Frame::new(BufferingVideoPlayer::new(url)).height(300.0)
    });

    // Feature description
    let features = vstack((
        text("Features demonstrated:").bold(),
        "- Native video controls (play/pause, seek, fullscreen)",
        "- Buffering detection with overlay indicator",
        "- Circular ProgressView for loading state",
        "- Dynamic video switching",
        "- Aspect ratio handling (Fit mode)",
    ))
    .alignment(HorizontalAlignment::Leading);

    scroll(
        vstack((
            header,
            Divider,
            spacer_min(16.0),
            text("Select a video:").bold(),
            selection_buttons,
            spacer_min(16.0),
            text("Now Playing:").bold(),
            now_playing,
            spacer_min(8.0),
            player,
            spacer_min(16.0),
            Divider,
            features,
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

waterui_ffi::export!();
