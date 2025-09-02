# Media Components

WaterUI provides powerful media components for displaying images, videos, and other rich media content with responsive layouts and performance optimizations.

## Images

### Basic Image Display

```rust,ignore
fn basic_image_demo() -> impl View {
    vstack((
        image("assets/photo.jpg"),
        image("https://example.com/remote-image.png"),
        image(ImageSource::Base64("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChAGA4sNvBQAAAABJRU5ErkJggg==")),
    ))
    .spacing(10.0)
}
```

### Responsive Images

```rust,ignore
fn responsive_image_demo() -> impl View {
    vstack((
        // Fit modes
        image("assets/landscape.jpg")
            .fit(ImageFit::Fill)
            .width(300.0)
            .height(200.0),
            
        image("assets/portrait.jpg")
            .fit(ImageFit::Contain)
            .width(300.0)
            .height(200.0),
            
        image("assets/square.jpg")
            .fit(ImageFit::Cover)
            .width(300.0)
            .height(200.0),
            
        // Aspect ratio preservation
        image("assets/photo.jpg")
            .aspect_ratio(16.0 / 9.0)
            .width(400.0),
    ))
    .spacing(15.0)
}
```

### Interactive Images

```rust,ignore
fn interactive_image_demo() -> impl View {
    let zoom_level = binding(1.0);
    let is_loading = binding(false);
    
    vstack((
        image("assets/zoomable.jpg")
            .scale(zoom_level.get())
            .on_tap({
                let zoom_level = zoom_level.clone();
                move |_| {
                    zoom_level.update(|z| if z >= 2.0 { 1.0 } else { z + 0.5 });
                }
            })
            .loading_state(is_loading.get()),
            
        hstack((
            button("Zoom In")
                .action({
                    let zoom_level = zoom_level.clone();
                    move |_| zoom_level.update(|z| (z * 1.2).min(3.0))
                }),
            button("Zoom Out")
                .action({
                    let zoom_level = zoom_level.clone();
                    move |_| zoom_level.update(|z| (z / 1.2).max(0.5))
                }),
            button("Reset")
                .action({
                    let zoom_level = zoom_level.clone();
                    move |_| zoom_level.set(1.0)
                }),
        ))
        .spacing(10.0),
        
        text!("Zoom: {:.1}x", zoom_level),
    ))
    .spacing(10.0)
}
```

### Image Gallery

```rust,ignore
fn image_gallery_demo() -> impl View {
    let current_index = binding(0);
    let images = vec![
        "assets/gallery1.jpg",
        "assets/gallery2.jpg", 
        "assets/gallery3.jpg",
        "assets/gallery4.jpg",
    ];
    let total_images = images.len();
    
    vstack((
        // Main image display
        image(s!(images.get(current_index.min(total_images - 1)).unwrap_or(&"")))
            .fit(ImageFit::Cover)
            .width(400.0)
            .height(300.0)
            .corner_radius(10.0),
            
        // Navigation
        hstack((
            button("← Previous")
                .disabled(s!(current_index == 0))
                .action({
                    let current_index = current_index.clone();
                    move |_| current_index.update(|i| i.saturating_sub(1))
                }),
                
            text!("{} / {}", s!(current_index + 1), total_images)
                .flex(1)
                .alignment(TextAlignment::Center),
                
            button("Next →")
                .disabled(s!(current_index >= total_images - 1))
                .action({
                    let current_index = current_index.clone();
                    move |_| current_index.update(|i| (i + 1).min(total_images - 1))
                }),
        ))
        .spacing(10.0),
        
        // Thumbnail strip
        scroll_horizontal(
            hstack(
                images.into_iter().enumerate().map(|(idx, img)| {
                    image(img)
                        .width(60.0)
                        .height(60.0)
                        .fit(ImageFit::Cover)
                        .corner_radius(5.0)
                        .border_width(s!(if current_index == idx { 3.0 } else { 0.0 }))
                        .border_color(Color::primary())
                        .on_tap({
                            let current_index = current_index.clone();
                            move |_| current_index.set(idx)
                        })
                })
            )
            .spacing(8.0)
        ),
    ))
    .spacing(15.0)
}
```

## Video Components

### Basic Video Player

```rust,ignore
fn basic_video_demo() -> impl View {
    let is_playing = binding(false);
    let current_time = binding(0.0);
    let duration = binding(60.0);
    
    vstack((
        video("assets/sample-video.mp4")
            .width(500.0)
            .height(300.0)
            .controls(true)
            .autoplay(false)
            .on_play({
                let is_playing = is_playing.clone();
                move || is_playing.set(true)
            })
            .on_pause({
                let is_playing = is_playing.clone();
                move || is_playing.set(false)
            })
            .on_time_update({
                let current_time = current_time.clone();
                move |time| current_time.set(time)
            }),
            
        // Custom controls
        video_controls(is_playing.clone(), current_time.clone(), duration.clone()),
    ))
    .spacing(10.0)
}

fn video_controls(
    is_playing: Binding<bool>,
    current_time: Binding<f64>,
    duration: Binding<f64>,
) -> impl View {
    vstack((
        // Progress bar
        slider(s!(current_time / duration))
            .on_change({
                let current_time = current_time.clone();
                let duration = duration.clone();
                move |progress| {
                    current_time.set(progress * duration.get());
                }
            }),
            
        // Control buttons
        hstack((
            button(s!(if is_playing { "⏸" } else { "▶" }))
                .action({
                    let is_playing = is_playing.clone();
                    move |_| is_playing.update(|p| !p)
                }),
                
            text!("{:.0}:{:02.0} / {:.0}:{:02.0}", 
                  s!(current_time / 60.0),
                  s!(current_time % 60.0),
                  s!(duration / 60.0),
                  s!(duration % 60.0)),
        ))
        .spacing(10.0),
    ))
    .spacing(5.0)
}
```

### Video Playlist

```rust,ignore
fn video_playlist_demo() -> impl View {
    let current_video = binding(0);
    let videos = vec![
        VideoItem { title: "Introduction", url: "assets/intro.mp4", duration: 120.0 },
        VideoItem { title: "Getting Started", url: "assets/getting-started.mp4", duration: 180.0 },
        VideoItem { title: "Advanced Topics", url: "assets/advanced.mp4", duration: 300.0 },
    ];
    
    hstack((
        // Video player
        video(s!(videos.get(current_video).map(|v| v.url).unwrap_or("")))
            .width(600.0)
            .height(400.0)
            .controls(true)
            .on_ended({
                let current_video = current_video.clone();
                let video_count = videos.len();
                move || {
                    current_video.update(|idx| {
                        if idx + 1 < video_count {
                            idx + 1
                        } else {
                            idx
                        }
                    });
                }
            }),
            
        // Playlist sidebar
        vstack((
            text("Playlist").font_weight(FontWeight::Bold),
            
            scroll(
                vstack(
                    videos.into_iter().enumerate().map(|(idx, video)| {
                        playlist_item(video, idx == current_video.get(), {
                            let current_video = current_video.clone();
                            move |_| current_video.set(idx)
                        })
                    })
                )
            )
            .max_height(300.0),
        ))
        .width(200.0)
        .spacing(10.0),
    ))
    .spacing(20.0)
}

#[derive(Clone)]
struct VideoItem {
    title: String,
    url: String,
    duration: f64,
}

fn playlist_item<F>(video: VideoItem, is_active: bool, on_select: F) -> impl View
where
    F: Fn() + Clone + 'static,
{
    vstack((
        text(&video.title)
            .font_weight(if is_active { FontWeight::Bold } else { FontWeight::Normal })
            .color(if is_active { Color::primary() } else { Color::text() }),
            
        text!("{:.0}:{:02.0}", video.duration / 60.0, video.duration % 60.0)
            .font_size(12.0)
            .color(Color::secondary()),
    ))
    .padding(10.0)
    .background(if is_active { Color::primary_background() } else { Color::transparent() })
    .corner_radius(5.0)
    .on_tap(move |_| on_select())
}
```

## Audio Components

### Audio Player

```rust,ignore
fn audio_player_demo() -> impl View {
    let is_playing = binding(false);
    let current_time = binding(0.0);
    let duration = binding(180.0);
    let volume = binding(0.8);
    
    vstack((
        // Album art and info
        hstack((
            image("assets/album-cover.jpg")
                .width(120.0)
                .height(120.0)
                .fit(ImageFit::Cover)
                .corner_radius(10.0),
                
            vstack((
                text("Song Title")
                    .font_size(18.0)
                    .font_weight(FontWeight::Bold),
                text("Artist Name")
                    .font_size(14.0)
                    .color(Color::secondary()),
                text("Album Name")
                    .font_size(12.0)
                    .color(Color::secondary()),
            ))
            .alignment(.leading)
            .flex(1),
        ))
        .spacing(15.0),
        
        // Progress
        vstack((
            slider(s!(current_time / duration))
                .on_change({
                    let current_time = current_time.clone();
                    let duration = duration.clone();
                    move |progress| {
                        current_time.set(progress * duration.get());
                    }
                }),
                
            hstack((
                text!("{:.0}:{:02.0}", s!(current_time / 60.0), s!(current_time % 60.0)),
                spacer(),
                text!("{:.0}:{:02.0}", s!(duration / 60.0), s!(duration % 60.0)),
            )),
        ))
        .spacing(5.0),
        
        // Controls
        hstack((
            button("⏮")
                .action(|| println!("Previous track")),
                
            button(s!(if is_playing { "⏸" } else { "▶" }))
                .font_size(20.0)
                .action({
                    let is_playing = is_playing.clone();
                    move |_| is_playing.update(|p| !p)
                }),
                
            button("⏭")
                .action(|| println!("Next track")),
                
            spacer(),
            
            hstack((
                text("🔊"),
                slider(volume.clone())
                    .width(80.0)
                    .on_change({
                        let volume = volume.clone();
                        move |v| volume.set(v)
                    }),
            ))
            .spacing(5.0),
        ))
        .spacing(15.0),
    ))
    .spacing(20.0)
    .padding(20.0)
}
```

## Live Photos and Animated Media

### Live Photo Component

```rust,ignore
fn live_photo_demo() -> impl View {
    let is_playing = binding(false);
    
    vstack((
        live_photo("assets/live-photo.mov", "assets/live-photo-preview.jpg")
            .width(300.0)
            .height(400.0)
            .is_playing(is_playing.get())
            .on_tap({
                let is_playing = is_playing.clone();
                move |_| {
                    is_playing.set(true);
                    
                    // Auto-stop after duration
                    let is_playing = is_playing.clone();
                    task::spawn(async move {
                        task::sleep(Duration::from_secs(3)).await;
                        is_playing.set(false);
                    });
                }
            }),
            
        text!("Status: {}", s!(if is_playing { "Playing" } else { "Tap to play" }))
            .alignment(TextAlignment::Center),
    ))
    .spacing(10.0)
}
```

### GIF and Animation Support

```rust,ignore
fn animated_media_demo() -> impl View {
    let gif_playing = binding(true);
    
    vstack((
        // Animated GIF
        image("assets/animation.gif")
            .width(200.0)
            .height(200.0)
            .animated(gif_playing.get())
            .on_tap({
                let gif_playing = gif_playing.clone();
                move |_| gif_playing.update(|p| !p)
            }),
            
        button(s!(if gif_playing { "Pause Animation" } else { "Play Animation" }))
            .action({
                let gif_playing = gif_playing.clone();
                move |_| gif_playing.update(|p| !p)
            }),
            
        // CSS-style animations
        animating_box(),
    ))
    .spacing(20.0)
}

fn animating_box() -> impl View {
    let animation_state = binding(0.0);
    
    // Start animation loop
    {
        let animation_state = animation_state.clone();
        task::spawn(async move {
            loop {
                for i in 0..=100 {
                    animation_state.set(i as f64 / 100.0);
                    task::sleep(Duration::from_millis(16)).await;
                }
                for i in (0..100).rev() {
                    animation_state.set(i as f64 / 100.0);
                    task::sleep(Duration::from_millis(16)).await;
                }
            }
        });
    }
    
    rectangle()
        .width(50.0)
        .height(50.0)
        .color(Color::primary())
        .corner_radius(s!(animation_state * 25.0)) // Animate corner radius
        .scale(s!(0.5 + animation_state * 0.5)) // Animate scale
        .rotation(s!(animation_state * 360.0)) // Animate rotation
}
```

## Media Performance and Optimization

### Lazy Loading Images

```rust,ignore
fn lazy_loading_demo() -> impl View {
    let images = (0..100).map(|i| format!("https://picsum.photos/200/200?random={}", i)).collect::<Vec<_>>();
    
    scroll(
        vstack(
            images.into_iter().map(|url| {
                lazy_image(url)
                    .width(200.0)
                    .height(200.0)
                    .placeholder(loading_placeholder())
                    .fade_in_duration(Duration::from_millis(300))
            })
        )
        .spacing(10.0)
    )
}

fn loading_placeholder() -> impl View {
    rectangle()
        .width(200.0)
        .height(200.0)
        .color(Color::light_gray())
        .overlay(
            text("Loading...")
                .color(Color::secondary())
        )
}

fn lazy_image(url: String) -> LazyImage {
    LazyImage::new(url)
        .threshold(100.0) // Start loading when 100px from viewport
        .retry_attempts(3)
        .cache_policy(CachePolicy::Aggressive)
}
```

### Image Caching and Preloading

```rust,ignore
fn image_cache_demo() -> impl View {
    let cache_stats = binding(CacheStats::default());
    
    vstack((
        text("Image Cache Demo").font_size(18.0),
        
        hstack((
            text!("Cached: {}", s!(cache_stats.cached_count)),
            text!("Memory: {:.1}MB", s!(cache_stats.memory_usage / 1024.0 / 1024.0)),
        ))
        .spacing(20.0),
        
        hstack((
            button("Preload Images")
                .action({
                    let cache_stats = cache_stats.clone();
                    move |_| {
                        let images = vec![
                            "assets/image1.jpg",
                            "assets/image2.jpg", 
                            "assets/image3.jpg",
                        ];
                        
                        for img in images {
                            ImageCache::preload(img);
                        }
                        
                        cache_stats.set(ImageCache::stats());
                    }
                }),
                
            button("Clear Cache")
                .action({
                    let cache_stats = cache_stats.clone();
                    move |_| {
                        ImageCache::clear();
                        cache_stats.set(ImageCache::stats());
                    }
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(15.0)
}

#[derive(Clone, Default)]
struct CacheStats {
    cached_count: usize,
    memory_usage: f64,
}
```

## Media Accessibility

### Accessible Media Components

```rust,ignore
fn accessible_media_demo() -> impl View {
    vstack((
        // Accessible image
        image("assets/chart.png")
            .accessibility_label("Sales chart showing 25% increase over last quarter")
            .accessibility_hint("Double-tap to view full-size chart"),
            
        // Accessible video
        video("assets/tutorial.mp4")
            .accessibility_label("WaterUI tutorial video")
            .accessibility_hint("Video controls available")
            .captions_enabled(true)
            .audio_description_enabled(true),
            
        // Alternative text for complex images
        vstack((
            image("assets/complex-diagram.svg")
                .accessibility_hidden(true), // Hide from screen readers
                
            text("Architecture diagram showing data flow from user interface through business logic to database layer")
                .accessibility_role(AccessibilityRole::Image)
                .font_size(14.0)
                .color(Color::secondary()),
        )),
        
        // Media with transcripts
        audio_with_transcript(),
    ))
    .spacing(20.0)
}

fn audio_with_transcript() -> impl View {
    let show_transcript = binding(false);
    
    vstack((
        audio("assets/podcast-episode.mp3")
            .controls(true)
            .accessibility_label("Podcast episode: Introduction to WaterUI"),
            
        toggle(show_transcript.clone())
            .label("Show transcript"),
            
        s!(if show_transcript {
            Some(
                scroll(
                    text("Transcript: Welcome to the WaterUI podcast. Today we'll be discussing...")
                        .accessibility_role(AccessibilityRole::Document)
                )
                .max_height(200.0)
                .padding(10.0)
                .background(Color::light_gray())
            )
        } else {
            None
        }),
    ))
    .spacing(10.0)
}
```

## Testing Media Components

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_image_gallery_navigation() {
        let current_index = binding(0);
        let images = vec!["img1.jpg", "img2.jpg", "img3.jpg"];
        
        // Test forward navigation
        assert_eq!(current_index.get(), 0);
        
        current_index.update(|i| (i + 1).min(images.len() - 1));
        assert_eq!(current_index.get(), 1);
        
        current_index.update(|i| (i + 1).min(images.len() - 1));
        assert_eq!(current_index.get(), 2);
        
        // Test boundary condition
        current_index.update(|i| (i + 1).min(images.len() - 1));
        assert_eq!(current_index.get(), 2); // Should not exceed bounds
        
        // Test backward navigation
        current_index.update(|i| i.saturating_sub(1));
        assert_eq!(current_index.get(), 1);
    }
    
    #[test]
    fn test_video_time_formatting() {
        let current_time = 125.5; // 2 minutes, 5.5 seconds
        let minutes = (current_time / 60.0) as i32;
        let seconds = (current_time % 60.0) as i32;
        
        assert_eq!(minutes, 2);
        assert_eq!(seconds, 5);
    }
}
```

## Summary

WaterUI's media system provides:

- **Image Support**: Static, responsive, and interactive images with multiple fit modes
- **Video Components**: Full-featured video players with custom controls
- **Audio Players**: Rich audio interfaces with playlist support  
- **Live Photos**: Support for animated media and Live Photos
- **Performance**: Lazy loading, caching, and memory optimization
- **Accessibility**: Screen reader support, captions, and transcripts

Key best practices:
- Use responsive image sizing and appropriate fit modes
- Implement lazy loading for large image collections
- Provide accessibility labels and alternative content
- Cache frequently accessed media assets
- Test media components across different screen sizes
- Consider bandwidth and performance implications

Next: [Form Components](11-forms.md)