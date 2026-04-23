//! Video Player Example - Immersive video playback demo
//!
//! This example showcases:
//! - VideoPlayer with native controls
//! - Overlay for buffering indicator
//! - Immersive full-screen layout
//! - Reactive state management

use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::binding;

fn main() -> impl View {
    // Sample video URLs (Big Buck Bunny - open source test videos)
    let sample_videos = [
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
    let selected_index = Binding::usize(0);

    // Track buffering state
    let is_buffering = binding(false);

    // Create reactive video URL
    let video_url = selected_index.clone().map(move |idx| {
        let (_, url_str) = sample_videos[idx];
        Url::parse(url_str).expect("Invalid video URL")
    });

    // Buffering overlay
    let buffering_overlay = vstack((loading(), text("Buffering...").foreground(Srgb::WHITE)))
        .spacing(12.0)
        .background(Srgb::BLACK.with_opacity(0.8))
        .visable(is_buffering.clone());

    // Video player - immersive full screen with Fill aspect ratio
    // VideoPlayer now takes a Url directly (not a Video data source)
    let player = VideoPlayer::new(video_url)
        .show_controls(true)
        .aspect_ratio(AspectRatio::Fill)
        .on_event({
            let selected_index = selected_index.clone();
            move |event| match event {
                video::Event::Buffering => is_buffering.set(true),
                video::Event::BufferingEnded | video::Event::ReadyToPlay => is_buffering.set(false),
                video::Event::PictureInPictureChanged { .. }
                | video::Event::BufferLevel { .. }
                | video::Event::PlaybackMetrics { .. } => {}
                video::Event::NextRequested => {
                    selected_index.set((selected_index.get() + 1) % sample_videos.len());
                }
                video::Event::PreviousRequested => {
                    selected_index.set(
                        (selected_index.get() + sample_videos.len() - 1) % sample_videos.len(),
                    );
                }
                video::Event::Ended | video::Event::Error { .. } => is_buffering.set(false),
            }
        });

    // Video with buffering overlay
    let video_layer = overlay(player, buffering_overlay);

    let title_signal = selected_index.clone().map(move |idx| sample_videos[idx].0);

    // Bottom controls overlay
    let controls_overlay = vstack((
        spacer(),
        // Bottom panel
        vstack((
            // Current video title
            text!("{title_signal}")
                .title()
                .bold()
                .foreground(Srgb::WHITE),
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
        .background(Srgb::BLACK.with_opacity(0.6)),
    ));

    // Stack everything
    zstack((video_layer, controls_overlay)).ignore_safe_area(EdgeSet::ALL)
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

/// Pill-style selection button
fn pill_button(label: &'static str, index: usize, selected: &Binding<usize>) -> impl View {
    let is_selected = selected.clone().map(move |s| s == index);
    let selected_for_action = selected.clone();
    let bg = is_selected.select(
        Srgb::WHITE.with_opacity(0.35),
        Srgb::WHITE.with_opacity(0.15),
    );

    button(label)
        .action(move |State(s): State<Binding<usize>>| s.set(index))
        .state(&selected_for_action)
        .foreground(Srgb::WHITE)
        .background(bg.computed())
}
