use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;

#[derive(Clone, Copy)]
struct Sample {
    title: &'static str,
    profile: &'static str,
    url: &'static str,
}

const SAMPLES: [Sample; 4] = [
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

fn main() -> impl View {
    let selected = Binding::usize(2);
    let status = Binding::container(Str::from("Loading fallback video pipeline..."));
    let is_buffering = Binding::bool(true);
    let source = selected
        .clone()
        .map(move |index| Url::parse(SAMPLES[index].url).expect("sample URL should be valid"));

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
                    status.set(Str::from("Ready"));
                }
                video::Event::Buffering => {
                    is_buffering.set(true);
                    status.set(Str::from("Buffering..."));
                }
                video::Event::BufferingEnded => {
                    is_buffering.set(false);
                    status.set(Str::from("Playing"));
                }
                video::Event::PictureInPictureChanged { active } => {
                    status.set(if active {
                        Str::from("Picture in Picture")
                    } else {
                        Str::from("Inline Playback")
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
                    status.set(Str::from("Ended"));
                }
                video::Event::Error { message } => {
                    is_buffering.set(false);
                    status.set(format!("Error: {message}").into());
                }
            }
        });

    fallback_player_shell(player, selected, status, is_buffering)
}

#[preview]
fn rust_fallback_video_player_preview() -> impl View {
    let selected = Binding::usize(2);
    let status = Binding::container(Str::from("Preview ready"));
    let is_buffering = Binding::bool(false);
    let preview_frame = zstack((
        Srgb::BLACK,
        vstack((
            text("Rust Fallback Video Preview")
                .headline()
                .foreground(Srgb::WHITE),
            text("Static frame for Hydrolysis perf")
                .caption()
                .foreground(Srgb::WHITE.with_opacity(0.8)),
        ))
        .spacing(8.0),
    ));

    fallback_player_shell(preview_frame, selected, status, is_buffering)
}

fn fallback_player_shell(
    player: impl View,
    selected: Binding<usize>,
    status: Binding<Str>,
    is_buffering: Binding<bool>,
) -> impl View {
    let title = selected.clone().map(move |index| SAMPLES[index].title);
    let source_profile = selected.clone().map(move |index| SAMPLES[index].profile);

    let status_overlay = text("Buffering...")
        .body()
        .foreground(Srgb::WHITE)
        .padding_with(EdgeInsets::all(10.0))
        .background(Srgb::BLACK.with_opacity(0.7))
        .visible(is_buffering.clone());

    vstack((
        text("WaterUI Rust Fallback VideoPlayer").headline(),
        text("Self-rendered VideoPlayer (GpuSurface + waterkit-codec) with real HDR test sources.")
            .caption(),
        text!("Now Playing: {title}").body(),
        text!("Expected Source Profile: {source_profile}").footnote(),
        text("Tip: check logs for [VideoFallback] color profile ... hdr=true on HDR sources.")
            .footnote(),
        overlay(player, status_overlay).height(360.0),
        text!("Status: {status}").footnote(),
        hstack((
            button("SDR: BBB")
                .action(|State(selected): State<Binding<usize>>| selected.set(0))
                .state(&selected),
            button("SDR: Sintel")
                .action(|State(selected): State<Binding<usize>>| selected.set(1))
                .state(&selected),
            button("HDR10: 1080p 3M")
                .action(|State(selected): State<Binding<usize>>| selected.set(2))
                .state(&selected),
            button("HDR10: 1080p 10M")
                .action(|State(selected): State<Binding<usize>>| selected.set(3))
                .state(&selected),
        ))
        .spacing(12.0),
    ))
    .spacing(12.0)
    .padding()
}

pub fn app(env: Environment) -> App {
    let mut env = env;
    video::install_rust_player_hooks(&mut env);
    App::new(main, env)
}
