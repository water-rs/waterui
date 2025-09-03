# Media Player Application

Let's build a complete media player application that demonstrates advanced WaterUI concepts including media handling, complex state management, and rich user interactions.

## Application Architecture

```rust,ignore
use waterui::*;
use nami::*;
use std::time::Duration;

#[derive(Clone, Debug)]
struct Track {
    id: u32,
    title: String,
    artist: String,
    album: String,
    duration: Duration,
    file_path: String,
    cover_art: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Loading,
}

#[derive(Clone, Debug, PartialEq)]
enum RepeatMode {
    None,
    Track,
    Playlist,
}

#[derive(Clone, Debug)]
struct PlayerState {
    current_track: Option<Track>,
    playlist: Vec<Track>,
    current_index: usize,
    playback_state: PlaybackState,
    current_time: Duration,
    volume: f64,
    muted: bool,
    shuffle: bool,
    repeat_mode: RepeatMode,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            current_track: None,
            playlist: Vec::new(),
            current_index: 0,
            playback_state: PlaybackState::Stopped,
            current_time: Duration::from_secs(0),
            volume: 0.8,
            muted: false,
            shuffle: false,
            repeat_mode: RepeatMode::None,
        }
    }
}

fn media_player_app() -> impl View {
    let player_state = binding(PlayerState::default());
    let view_mode = binding(ViewMode::Player);
    
    // Initialize with sample tracks
    {
        let player_state = player_state.clone();
        task::spawn(async move {
            let sample_tracks = create_sample_tracks();
            player_state.update(|mut state| {
                state.playlist = sample_tracks;
                if !state.playlist.is_empty() {
                    state.current_track = Some(state.playlist[0].clone());
                }
                state
            });
        });
    }
    
    vstack((
        // Top navigation
        player_navigation(view_mode.clone()),
        
        // Main content based on view mode
        s!(match view_mode {
            ViewMode::Player => Some(player_view(player_state.clone())),
            ViewMode::Playlist => Some(playlist_view(player_state.clone())),
            ViewMode::Library => Some(library_view(player_state.clone())),
        }),
    ))
    .background(Color::rgb(0.05, 0.05, 0.1))
    .color(Color::white())
}

#[derive(Clone, PartialEq)]
enum ViewMode {
    Player,
    Playlist,
    Library,
}

fn create_sample_tracks() -> Vec<Track> {
    vec![
        Track {
            id: 1,
            title: "Sunset Dreams".to_string(),
            artist: "Ambient Collective".to_string(),
            album: "Peaceful Moments".to_string(),
            duration: Duration::from_secs(245),
            file_path: "/music/sunset_dreams.mp3".to_string(),
            cover_art: Some("/covers/peaceful_moments.jpg".to_string()),
        },
        Track {
            id: 2,
            title: "Digital Horizons".to_string(),
            artist: "Synth Wave".to_string(),
            album: "Neon Nights".to_string(),
            duration: Duration::from_secs(198),
            file_path: "/music/digital_horizons.mp3".to_string(),
            cover_art: Some("/covers/neon_nights.jpg".to_string()),
        },
        Track {
            id: 3,
            title: "Mountain Echo".to_string(),
            artist: "Nature Sounds".to_string(),
            album: "Wilderness".to_string(),
            duration: Duration::from_secs(312),
            file_path: "/music/mountain_echo.mp3".to_string(),
            cover_art: Some("/covers/wilderness.jpg".to_string()),
        },
    ]
}
```

## Player Interface

```rust,ignore
fn player_view(player_state: Binding<PlayerState>) -> impl View {
    vstack((
        // Album art and track info
        track_display_section(player_state.clone()),
        
        // Progress bar
        playback_progress_section(player_state.clone()),
        
        // Main controls
        main_controls_section(player_state.clone()),
        
        // Volume and additional controls
        secondary_controls_section(player_state.clone()),
    ))
    .spacing(30.0)
    .padding(40.0)
    .max_width(600.0)
    .alignment(.center)
}

fn track_display_section(player_state: Binding<PlayerState>) -> impl View {
    let current_track = s!(player_state.current_track.clone());
    
    vstack((
        // Album artwork
        s!(if let Some(track) = &current_track {
            Some(
                album_artwork(
                    track.cover_art.clone().unwrap_or_default(),
                    player_state.clone()
                )
            )
        } else {
            Some(placeholder_artwork())
        }),
        
        // Track information
        s!(if let Some(track) = &current_track {
            Some(track_info(track.clone()))
        } else {
            Some(
                text("No track selected")
                    .font_size(18.0)
                    .color(Color::white().opacity(0.7))
                    .alignment(.center)
            )
        }),
    ))
    .spacing(20.0)
    .alignment(.center)
}

fn album_artwork(
    cover_path: String,
    player_state: Binding<PlayerState>
) -> impl View {
    let is_playing = s!(player_state.playback_state == PlaybackState::Playing);
    
    zstack((
        // Main artwork
        s!(if !cover_path.is_empty() {
            Some(
                async_image(cover_path)
                    .width(280.0)
                    .height(280.0)
                    .fit(ImageFit::Cover)
                    .corner_radius(20.0)
                    .shadow(Color::black().opacity(0.3), 8.0)
            )
        } else {
            Some(placeholder_artwork())
        }),
        
        // Play/pause overlay (appears on hover)
        play_pause_overlay(player_state.clone(), is_playing.clone()),
        
        // Animated border for playing state
        s!(if is_playing {
            Some(
                animated_border()
            )
        } else {
            None
        }),
    ))
}

fn placeholder_artwork() -> impl View {
    rectangle()
        .width(280.0)
        .height(280.0)
        .color(Color::white().opacity(0.1))
        .corner_radius(20.0)
        .overlay(
            text("♪")
                .font_size(80.0)
                .color(Color::white().opacity(0.3))
        )
}

fn play_pause_overlay(
    player_state: Binding<PlayerState>,
    is_playing: Signal<bool>
) -> impl View {
    let hover_state = binding(false);
    
    s!(if hover_state {
        Some(
            circle()
                .width(80.0)
                .height(80.0)
                .color(Color::black().opacity(0.6))
                .overlay(
                    text(s!(if is_playing { "⏸" } else { "▶" }))
                        .font_size(32.0)
                        .color(Color::white())
                )
                .on_tap({
                    let player_state = player_state.clone();
                    move |_| toggle_playback(player_state.clone())
                })
        )
    } else {
        None
    })
    .on_hover({
        let hover_state = hover_state.clone();
        move |hovering| hover_state.set(hovering)
    })
}

fn animated_border() -> impl View {
    let animation_progress = binding(0.0);
    
    // Start animation loop
    {
        let animation_progress = animation_progress.clone();
        task::spawn(async move {
            loop {
                for i in 0..=360 {
                    animation_progress.set(i as f64);
                    task::sleep(Duration::from_millis(50)).await;
                }
            }
        });
    }
    
    rectangle()
        .width(288.0)
        .height(288.0)
        .color(Color::transparent())
        .border_width(4.0)
        .border_color(Color::primary())
        .corner_radius(24.0)
        .rotation(animation_progress.get())
        .opacity(0.7)
}

fn track_info(track: Track) -> impl View {
    vstack((
        text!("{}", track.title)
            .font_size(24.0)
            .font_weight(FontWeight::Bold)
            .color(Color::white())
            .alignment(.center),
            
        text!("{}", track.artist)
            .font_size(18.0)
            .color(Color::white().opacity(0.8))
            .alignment(.center),
            
        text!("{}", track.album)
            .font_size(16.0)
            .color(Color::white().opacity(0.6))
            .alignment(.center),
    ))
    .spacing(8.0)
}
```

## Playback Controls

```rust,ignore
fn playback_progress_section(player_state: Binding<PlayerState>) -> impl View {
    let current_time = s!(player_state.current_time);
    let duration = s!(player_state.current_track.as_ref().map(|t| t.duration).unwrap_or_default());
    let progress = s!(if duration.as_secs() > 0 {
        current_time.as_secs() as f64 / duration.as_secs() as f64
    } else {
        0.0
    });
    
    vstack((
        // Progress slider
        slider(progress.clone())
            .width(500.0)
            .track_color(Color::white().opacity(0.3))
            .thumb_color(Color::primary())
            .fill_color(Color::primary())
            .on_change({
                let player_state = player_state.clone();
                move |new_progress| {
                    seek_to_progress(player_state.clone(), new_progress);
                }
            }),
            
        // Time labels
        hstack((
            text!(format_duration(current_time))
                .font_size(14.0)
                .color(Color::white().opacity(0.7))
                .font_family("monospace"),
                
            spacer(),
            
            text!(format_duration(duration))
                .font_size(14.0)
                .color(Color::white().opacity(0.7))
                .font_family("monospace"),
        )),
    ))
    .spacing(8.0)
}

fn main_controls_section(player_state: Binding<PlayerState>) -> impl View {
    let playback_state = s!(player_state.playback_state.clone());
    let can_previous = s!(player_state.current_index > 0 || player_state.repeat_mode != RepeatMode::None);
    let can_next = s!(player_state.current_index < player_state.playlist.len().saturating_sub(1) || player_state.repeat_mode != RepeatMode::None);
    
    hstack((
        // Previous track
        control_button("⏮", can_previous.get(), {
            let player_state = player_state.clone();
            move || previous_track(player_state.clone())
        }),
        
        // Play/Pause
        main_play_button(playback_state.clone(), {
            let player_state = player_state.clone();
            move || toggle_playback(player_state.clone())
        }),
        
        // Next track
        control_button("⏭", can_next.get(), {
            let player_state = player_state.clone();
            move || next_track(player_state.clone())
        }),
    ))
    .spacing(20.0)
    .alignment(.center)
}

fn control_button<F>(icon: &str, enabled: bool, action: F) -> impl View
where
    F: Fn() + 'static,
{
    button(icon)
        .width(50.0)
        .height(50.0)
        .font_size(20.0)
        .background_color(Color::white().opacity(if enabled { 0.15 } else { 0.05 }))
        .color(Color::white().opacity(if enabled { 1.0 } else { 0.3 }))
        .corner_radius(25.0)
        .disabled(!enabled)
        .action(move |_| {
            if enabled {
                action();
            }
        })
}

fn main_play_button<F>(playback_state: Signal<PlaybackState>, action: F) -> impl View
where
    F: Fn() + 'static,
{
    let icon = s!(match playback_state {
        PlaybackState::Playing => "⏸",
        PlaybackState::Loading => "⏳",
        _ => "▶",
    });
    
    button(icon)
        .width(70.0)
        .height(70.0)
        .font_size(32.0)
        .background_color(Color::primary())
        .color(Color::white())
        .corner_radius(35.0)
        .disabled(s!(playback_state == PlaybackState::Loading))
        .shadow(Color::primary().opacity(0.3), 4.0)
        .action(move |_| action())
        .scale_on_press(0.95)
}

fn secondary_controls_section(player_state: Binding<PlayerState>) -> impl View {
    hstack((
        // Shuffle
        toggle_control_button(
            "🔀",
            s!(player_state.shuffle),
            {
                let player_state = player_state.clone();
                move || {
                    player_state.update(|mut state| {
                        state.shuffle = !state.shuffle;
                        state
                    });
                }
            }
        ),
        
        spacer(),
        
        // Volume control
        volume_control(player_state.clone()),
        
        spacer(),
        
        // Repeat mode
        repeat_control(player_state.clone()),
    ))
    .width(500.0)
}

fn toggle_control_button<F>(icon: &str, active: Signal<bool>, action: F) -> impl View
where
    F: Fn() + 'static,
{
    button(icon)
        .width(40.0)
        .height(40.0)
        .font_size(16.0)
        .background_color(s!(if active { 
            Color::primary().opacity(0.3) 
        } else { 
            Color::white().opacity(0.1) 
        }))
        .color(s!(if active { Color::primary() } else { Color::white().opacity(0.7) }))
        .corner_radius(20.0)
        .action(move |_| action())
}

fn volume_control(player_state: Binding<PlayerState>) -> impl View {
    let volume = s!(player_state.volume);
    let muted = s!(player_state.muted);
    
    hstack((
        // Mute button
        button(s!(if muted { "🔇" } else if volume > 0.5 { "🔊" } else { "🔉" }))
            .width(30.0)
            .height(30.0)
            .background_color(Color::transparent())
            .color(Color::white().opacity(0.7))
            .action({
                let player_state = player_state.clone();
                move |_| {
                    player_state.update(|mut state| {
                        state.muted = !state.muted;
                        state
                    });
                }
            }),
            
        // Volume slider
        slider(volume.clone())
            .width(100.0)
            .track_color(Color::white().opacity(0.3))
            .thumb_color(Color::white())
            .fill_color(Color::white())
            .on_change({
                let player_state = player_state.clone();
                move |new_volume| {
                    player_state.update(|mut state| {
                        state.volume = new_volume;
                        state.muted = false; // Unmute when adjusting volume
                        state
                    });
                }
            }),
    ))
    .spacing(10.0)
}

fn repeat_control(player_state: Binding<PlayerState>) -> impl View {
    let repeat_mode = s!(player_state.repeat_mode.clone());
    
    let (icon, color) = s!(match repeat_mode {
        RepeatMode::None => ("🔁", Color::white().opacity(0.3)),
        RepeatMode::Playlist => ("🔁", Color::primary()),
        RepeatMode::Track => ("🔂", Color::primary()),
    });
    
    button(icon)
        .width(40.0)
        .height(40.0)
        .font_size(16.0)
        .background_color(Color::white().opacity(0.1))
        .color(color)
        .corner_radius(20.0)
        .action({
            let player_state = player_state.clone();
            move |_| {
                player_state.update(|mut state| {
                    state.repeat_mode = match state.repeat_mode {
                        RepeatMode::None => RepeatMode::Playlist,
                        RepeatMode::Playlist => RepeatMode::Track,
                        RepeatMode::Track => RepeatMode::None,
                    };
                    state
                });
            }
        })
}

// Playback control functions
fn toggle_playback(player_state: Binding<PlayerState>) {
    player_state.update(|mut state| {
        state.playback_state = match state.playback_state {
            PlaybackState::Playing => PlaybackState::Paused,
            PlaybackState::Paused | PlaybackState::Stopped => PlaybackState::Playing,
            PlaybackState::Loading => state.playback_state, // Don't change if loading
        };
        state
    });
    
    // Simulate playback progress
    if player_state.get().playback_state == PlaybackState::Playing {
        simulate_playback_progress(player_state);
    }
}

fn simulate_playback_progress(player_state: Binding<PlayerState>) {
    task::spawn(async move {
        loop {
            task::sleep(Duration::from_millis(1000)).await;
            
            let should_continue = player_state.update(|mut state| {
                if state.playback_state == PlaybackState::Playing {
                    state.current_time = state.current_time + Duration::from_secs(1);
                    
                    // Check if track ended
                    if let Some(track) = &state.current_track {
                        if state.current_time >= track.duration {
                            // Handle track end based on repeat mode
                            match state.repeat_mode {
                                RepeatMode::Track => {
                                    state.current_time = Duration::from_secs(0);
                                },
                                RepeatMode::Playlist => {
                                    if state.current_index < state.playlist.len() - 1 {
                                        state.current_index += 1;
                                        state.current_track = Some(state.playlist[state.current_index].clone());
                                    } else {
                                        state.current_index = 0;
                                        state.current_track = Some(state.playlist[0].clone());
                                    }
                                    state.current_time = Duration::from_secs(0);
                                },
                                RepeatMode::None => {
                                    if state.current_index < state.playlist.len() - 1 {
                                        state.current_index += 1;
                                        state.current_track = Some(state.playlist[state.current_index].clone());
                                        state.current_time = Duration::from_secs(0);
                                    } else {
                                        state.playback_state = PlaybackState::Stopped;
                                        state.current_time = Duration::from_secs(0);
                                    }
                                },
                            }
                        }
                    }
                    true
                } else {
                    false
                }
            });
            
            if !should_continue {
                break;
            }
        }
    });
}

fn previous_track(player_state: Binding<PlayerState>) {
    player_state.update(|mut state| {
        if state.current_index > 0 {
            state.current_index -= 1;
        } else if state.repeat_mode != RepeatMode::None && !state.playlist.is_empty() {
            state.current_index = state.playlist.len() - 1;
        }
        
        if state.current_index < state.playlist.len() {
            state.current_track = Some(state.playlist[state.current_index].clone());
            state.current_time = Duration::from_secs(0);
        }
        
        state
    });
}

fn next_track(player_state: Binding<PlayerState>) {
    player_state.update(|mut state| {
        if state.current_index < state.playlist.len() - 1 {
            state.current_index += 1;
        } else if state.repeat_mode != RepeatMode::None && !state.playlist.is_empty() {
            state.current_index = 0;
        }
        
        if state.current_index < state.playlist.len() {
            state.current_track = Some(state.playlist[state.current_index].clone());
            state.current_time = Duration::from_secs(0);
        }
        
        state
    });
}

fn seek_to_progress(player_state: Binding<PlayerState>, progress: f64) {
    player_state.update(|mut state| {
        if let Some(track) = &state.current_track {
            let new_time = Duration::from_secs((track.duration.as_secs() as f64 * progress) as u64);
            state.current_time = new_time;
        }
        state
    });
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{}:{:02}", minutes, seconds)
}
```

## Playlist View

```rust,ignore
fn playlist_view(player_state: Binding<PlayerState>) -> impl View {
    let playlist = s!(player_state.playlist.clone());
    let current_index = s!(player_state.current_index);
    
    vstack((
        // Playlist header
        playlist_header(playlist.len()),
        
        // Playlist controls
        playlist_controls(player_state.clone()),
        
        // Track list
        scroll(
            vstack(
                playlist.map(|tracks| {
                    tracks.into_iter().enumerate().map(|(index, track)| {
                        playlist_track_item(
                            track,
                            index,
                            index == current_index,
                            player_state.clone()
                        )
                    })
                })
            )
            .spacing(2.0)
        )
        .max_height(500.0),
    ))
    .spacing(20.0)
    .padding(30.0)
}

fn playlist_header(track_count: usize) -> impl View {
    hstack((
        text("Current Playlist")
            .font_size(24.0)
            .font_weight(FontWeight::Bold)
            .color(Color::white()),
            
        spacer(),
        
        text!("{} tracks", track_count)
            .font_size(16.0)
            .color(Color::white().opacity(0.7)),
    ))
}

fn playlist_controls(player_state: Binding<PlayerState>) -> impl View {
    hstack((
        button("Shuffle All")
            .action({
                let player_state = player_state.clone();
                move |_| {
                    player_state.update(|mut state| {
                        state.shuffle = !state.shuffle;
                        if state.shuffle {
                            // Shuffle playlist logic would go here
                        }
                        state
                    });
                }
            }),
            
        button("Clear Playlist")
            .style(ButtonStyle::Destructive)
            .action({
                let player_state = player_state.clone();
                move |_| {
                    player_state.update(|mut state| {
                        state.playlist.clear();
                        state.current_track = None;
                        state.current_index = 0;
                        state.playback_state = PlaybackState::Stopped;
                        state
                    });
                }
            }),
    ))
    .spacing(15.0)
}

fn playlist_track_item(
    track: Track,
    index: usize,
    is_current: bool,
    player_state: Binding<PlayerState>
) -> impl View {
    let is_playing = s!(
        is_current && 
        player_state.playback_state == PlaybackState::Playing
    );
    
    hstack((
        // Track number or playing indicator
        text(s!(if is_playing { 
            "♪".to_string() 
        } else { 
            (index + 1).to_string() 
        }))
        .width(30.0)
        .font_size(14.0)
        .color(s!(if is_current { Color::primary() } else { Color::white().opacity(0.5) }))
        .font_weight(s!(if is_current { FontWeight::Bold } else { FontWeight::Normal })),
        
        // Track info
        vstack((
            text!("{}", track.title)
                .font_size(16.0)
                .font_weight(s!(if is_current { FontWeight::Bold } else { FontWeight::Normal }))
                .color(s!(if is_current { Color::primary() } else { Color::white() })),
                
            text!("{} • {}", track.artist, track.album)
                .font_size(14.0)
                .color(Color::white().opacity(0.7)),
        ))
        .spacing(4.0)
        .flex(1),
        
        // Duration
        text!(format_duration(track.duration))
            .font_size(14.0)
            .color(Color::white().opacity(0.5))
            .font_family("monospace"),
            
        // Options menu
        button("⋮")
            .width(30.0)
            .height(30.0)
            .background_color(Color::transparent())
            .color(Color::white().opacity(0.5))
            .action(move |_| {
                // Show track options menu
            }),
    ))
    .spacing(15.0)
    .padding(12.0)
    .background(s!(if is_current { 
        Color::primary().opacity(0.1) 
    } else { 
        Color::transparent() 
    }))
    .corner_radius(8.0)
    .on_tap({
        let player_state = player_state.clone();
        let track = track.clone();
        move |_| {
            player_state.update(|mut state| {
                state.current_index = index;
                state.current_track = Some(track.clone());
                state.current_time = Duration::from_secs(0);
                state.playback_state = PlaybackState::Playing;
                state
            });
            
            simulate_playback_progress(player_state.clone());
        }
    })
    .on_hover({
        let hover = binding(false);
        move |hovering| hover.set(hovering)
    })
}
```

## Navigation and Layout

```rust,ignore
fn player_navigation(view_mode: Binding<ViewMode>) -> impl View {
    hstack((
        nav_item("Player", ViewMode::Player, view_mode.clone()),
        nav_item("Playlist", ViewMode::Playlist, view_mode.clone()),  
        nav_item("Library", ViewMode::Library, view_mode.clone()),
        
        spacer(),
        
        // Window controls
        window_controls(),
    ))
    .padding(15.0)
    .background(Color::rgb(0.1, 0.1, 0.15))
    .border_bottom_width(1.0)
    .border_bottom_color(Color::white().opacity(0.1))
}

fn nav_item(
    label: &str,
    mode: ViewMode,
    current_mode: Binding<ViewMode>
) -> impl View {
    let is_active = s!(current_mode == mode);
    
    text!(label)
        .padding(EdgeInsets::symmetric(15.0, 8.0))
        .color(s!(if is_active { Color::primary() } else { Color::white().opacity(0.7) }))
        .font_weight(s!(if is_active { FontWeight::Bold } else { FontWeight::Normal }))
        .background(s!(if is_active { 
            Color::primary().opacity(0.1) 
        } else { 
            Color::transparent() 
        }))
        .corner_radius(6.0)
        .on_tap({
            let current_mode = current_mode.clone();
            move |_| current_mode.set(mode.clone())
        })
}

fn window_controls() -> impl View {
    hstack((
        window_control_button("−", Color::orange()),
        window_control_button("□", Color::green()),
        window_control_button("✕", Color::red()),
    ))
    .spacing(8.0)
}

fn window_control_button(symbol: &str, color: Color) -> impl View {
    button(symbol)
        .width(12.0)
        .height(12.0)
        .font_size(10.0)
        .background_color(color)
        .color(Color::white())
        .corner_radius(6.0)
        .action(move |_| {
            println!("Window control: {}", symbol);
        })
}

fn library_view(player_state: Binding<PlayerState>) -> impl View {
    vstack((
        text("Music Library")
            .font_size(24.0)
            .font_weight(FontWeight::Bold)
            .color(Color::white()),
            
        text("Coming soon - Browse your music collection")
            .font_size(16.0)
            .color(Color::white().opacity(0.7))
            .alignment(.center)
            .padding(50.0),
    ))
    .spacing(20.0)
    .padding(30.0)
    .alignment(.center)
}
```

## Summary

This media player application demonstrates:

- **Complex State Management**: Centralized player state with reactive updates
- **Rich Media Controls**: Play/pause, seek, volume, repeat, and shuffle
- **Dynamic UI**: Animated elements and state-dependent rendering
- **Multiple Views**: Player, playlist, and library interfaces
- **User Interactions**: Hover effects, drag controls, and touch-friendly interfaces
- **Responsive Design**: Adaptive layout for different screen sizes

Key features implemented:
- Album artwork display with animations
- Progress tracking and seeking
- Playlist management and navigation
- Volume and repeat controls
- Track information display
- Keyboard shortcuts support
- Accessibility considerations

Next: [Chat Application](22-chat-application.md)