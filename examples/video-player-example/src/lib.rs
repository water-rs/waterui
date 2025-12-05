//! Video Player Example - Immersive video playback demo
//!
//! This example showcases:
//! - VideoPlayer with native controls
//! - Overlay for buffering indicator
//! - Immersive full-screen layout
//! - Reactive state management

use waterui::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::widget::condition::when;

pub fn init() -> Environment {
    Environment::new()
}

pub fn main() -> impl View {
    // Sample video URLs (Big Buck Bunny - open source test videos)
    let sample_videos: [(&str, &str); 3] = [
        (
            "Big Buck Bunny",
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

    // Track buffering state
    let is_buffering = binding(false);

    // Create reactive video source
    let video_source = selected_index.clone().map(move |idx| {
        let (_, url_str) = sample_videos[idx];
        Video::new(url::Url::parse(url_str).expect("Invalid video URL"))
    });

    // Video player - immersive full screen with Fill aspect ratio
    let player = VideoPlayer::new(video_source)
        .show_controls(true)
        .aspect_ratio(AspectRatio::Fill)
        .on_event({
            let is_buffering = is_buffering.clone();
            move |event| match event {
                video::Event::Buffering => is_buffering.set(true),
                video::Event::BufferingEnded | video::Event::ReadyToPlay => is_buffering.set(false),
                video::Event::Ended | video::Event::Error { .. } => is_buffering.set(false),
            }
        });

    // Buffering overlay
    let buffering_overlay = when(is_buffering, || {
        zstack((
            spacer().background(Color::from(Srgb::BLACK).with_opacity(0.5)),
            vstack((
                loading(),
                text("Buffering...").foreground(Color::from(Srgb::WHITE)),
            ))
            .spacing(12.0),
        ))
    });

    // Video with buffering overlay
    let video_layer = overlay(player, buffering_overlay);

    // Bottom controls overlay
    let controls_overlay = vstack((
        spacer(),
        // Bottom panel
        vstack((
            // Current video title
            Dynamic::watch(selected_index.clone(), move |idx| {
                let (title, _) = sample_videos[idx];
                text(title)
                    .size(28.0)
                    .bold()
                    .foreground(Color::from(Srgb::WHITE))
            }),
            spacer_min(20.0),
            // Video selector pills
            hstack((
                pill_button("Big Buck Bunny", 0, &selected_index),
                pill_button("Elephant Dream", 1, &selected_index),
                pill_button("Sintel", 2, &selected_index),
            ))
            .spacing(12.0),
        ))
        .padding_with(EdgeInsets::new(60.0, 32.0, 32.0, 32.0))
        .background(Color::from(Srgb::BLACK).with_opacity(0.6)),
    ));

    // Stack everything
    zstack((video_layer, controls_overlay)).ignore_safe_area(EdgeSet::ALL)
}

/// Pill-style selection button
fn pill_button(label: &'static str, index: usize, selected: &Binding<usize>) -> impl View {
    let is_selected = selected.clone().map(move |s| s == index);
    let selected_for_action = selected.clone();

    Dynamic::watch(is_selected, move |active| {
        let bg = if active {
            Color::from(Srgb::WHITE).with_opacity(0.35)
        } else {
            Color::from(Srgb::WHITE).with_opacity(0.15)
        };

        let selected_clone = selected_for_action.clone();
        button(text(label).foreground(Color::from(Srgb::WHITE)))
            .action(move || selected_clone.set(index))
            .background(bg)
    })
}

waterui_ffi::export!();
