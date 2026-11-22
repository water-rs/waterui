//! The `#1812` acceptance scene: GPU content hosted under the HWUI painter.
//!
//! One stage per run, built from the same pieces for the GPU video player and
//! the GPU vector map so the two read side by side: a band painted *beneath*
//! the GPU view that the view partly covers, the GPU view itself clipped to a
//! rounded rectangle, and a card painted *above* it. The stage sits inside a
//! vertical scroll between rows of text, so scrolling cuts the GPU view with
//! the scroll viewport's clip.

use waterui::graphics::color::Srgb;
use waterui::layout::frame::Frame;
use waterui::layout::{PinConstraints, PositionExt as _};
use waterui::prelude::*;
use waterui::shape::RoundedRectangle;

/// Height of the stage the GPU view sits in.
const STAGE_HEIGHT: f32 = 320.0;

fn row(index: usize) -> impl View {
    Frame::new(text!("Row {index}", index = index.to_string()))
        .max_width(f32::INFINITY)
        .height(96.0)
}

/// A band painted beneath the GPU view, the GPU view clipped to a rounded
/// rectangle partly over it, and a card above both.
fn stage(gpu: impl View) -> impl View {
    Frame::new(absolute((
        Frame::new(text("BENEATH the GPU view"))
            .max_width(f32::INFINITY)
            .height(120.0)
            .background(Srgb::from_hex("#D81B60"))
            .pin(PinConstraints::new().leading(0.0).trailing(0.0).top(0.0)),
        gpu.clip(RoundedRectangle::new(0.08)).pin(
            PinConstraints::new()
                .leading(24.0)
                .trailing(24.0)
                .top(72.0)
                .bottom(0.0),
        ),
        Frame::new(text("ABOVE the GPU view").foreground(Srgb::from_hex("#FFFFFF")))
            .width(200.0)
            .height(56.0)
            .background(Srgb::from_hex("#1E88E5"))
            .pin(
                PinConstraints::new()
                    .leading(40.0)
                    .bottom(24.0)
                    .width(200.0)
                    .height(56.0),
            ),
    )))
    .height(STAGE_HEIGHT)
}

/// The scene around `gpu`: a heading and a scroll holding the stage between
/// rows.
pub fn view(title: &'static str, gpu: impl View) -> impl View {
    vstack((
        text(title).title(),
        scroll(
            vstack((
                row(0),
                row(1),
                row(2),
                row(3),
                row(4),
                stage(gpu),
                row(5),
                row(6),
                row(7),
                row(8),
                row(9),
            ))
            .spacing(0.0),
        ),
    ))
    .padding()
}

/// The GPU video player on a source the device plays.
#[cfg(feature = "gpu-video-probe")]
pub fn video() -> impl View {
    use waterui::video::{PlaybackSession, Playlist, Url, VideoPlayer, video::ContentMode};

    /// The 10 s H.264 clip the video-player example opens with.
    const SOURCE: &str =
        "https://test-videos.co.uk/vids/bigbuckbunny/mp4/h264/720/Big_Buck_Bunny_720_10s_1MB.mp4";
    let url = Url::parse(SOURCE).expect("the probe's source URL is valid");
    view(
        "GPU video player",
        VideoPlayer::new(PlaybackSession::new(Playlist::single(url)))
            .show_controls(true)
            .content_mode(ContentMode::Fit),
    )
}

/// The GPU vector map over Manhattan, drawn from the example's tile style.
#[cfg(feature = "gpu-map-probe")]
pub fn map() -> impl View {
    use waterui::reactive::binding;
    use waterui_map::{Coordinate, Map, MapStyle, Region};

    let manhattan =
        Coordinate::from_degrees(40.7580, -73.9855).expect("the probe's coordinate is valid");
    view(
        "GPU vector map",
        Map::new(binding::<Region>(Region::new(manhattan, 0.030, 0.050))).style(MapStyle::Standard),
    )
}

/// The environment the map realization draws from: the tile style and the
/// installed GPU hook.
#[cfg(feature = "gpu-map-probe")]
pub fn install_map(env: &mut Environment) {
    env.insert(waterui_map_gpu::MapGpuOptions::new(waterui_url::Url::new(
        "https://tiles.openfreemap.org/styles/positron",
    )));
    waterui_map_gpu::install(env);
}
