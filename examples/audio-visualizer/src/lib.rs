//! Audio Visualizer Example - Real-time waveform visualization
//!
//! This example demonstrates:
//! - `Waveform` component for real-time audio visualization
//! - GPU-accelerated rendering using microphone input
//! - Theme switching between cyber, recorder, and oscilloscope styles
//! - Interactive sensitivity control
//!
//! The visualizer captures audio from the device microphone and renders
//! a stylized waveform display with customizable visual effects.

use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui_visualizer::{AudioCapture, Waveform, WaveformTheme};

fn main() -> impl View {
    // State for theme (directly as Binding<WaveformTheme>)
    let theme = binding(WaveformTheme::cyber());

    // State for theme index (for the label display)
    let theme_index = Binding::usize(0);

    // State for sensitivity
    let sensitivity = Binding::f64(1.2);

    // Waveform visualizer with reactive bindings
    let capture = AudioCapture::new();
    let waveform = Waveform::new(capture)
        .theme(theme.clone())
        .sensitivity(sensitivity.clone());

    // Mode text from theme index
    let mode_text = theme_index.clone().map(|idx| match idx {
        0 => "Waveform Mode: Cyber",
        1 => "Waveform Mode: Recorder",
        2 => "Waveform Mode: Oscilloscope",
        _ => "Waveform Mode",
    });

    // Controls overlay at bottom
    let controls_overlay = vstack((
        spacer(),
        // Bottom control panel
        vstack((
            text("WaterUI Audio Visualizer").bold(),
            spacer_min(16.0),
            text("Theme"),
            hstack((
                button("Cyber")
                    .action(
                        |State(t): State<Binding<WaveformTheme>>,
                         State(i): State<Binding<usize>>| {
                            t.set(WaveformTheme::cyber());
                            i.set(0);
                        },
                    )
                    .state(&theme)
                    .state(&theme_index),
                button("Recorder")
                    .action(
                        |State(t): State<Binding<WaveformTheme>>,
                         State(i): State<Binding<usize>>| {
                            t.set(WaveformTheme::recorder());
                            i.set(1);
                        },
                    )
                    .state(&theme)
                    .state(&theme_index),
                button("Oscilloscope")
                    .action(
                        |State(t): State<Binding<WaveformTheme>>,
                         State(i): State<Binding<usize>>| {
                            t.set(WaveformTheme::oscilloscope());
                            i.set(2);
                        },
                    )
                    .state(&theme)
                    .state(&theme_index),
            ))
            .spacing(12.0),
            spacer_min(16.0),
            text("Sensitivity"),
            Slider::new("Sensitivity", &sensitivity)
                .range(0.5..=3.0)
                .hide_label(),
            spacer_min(8.0),
            text!("{mode_text}"),
        ))
        .padding_with(EdgeInsets::all(24.0))
        .background(Srgb::BLACK.with_opacity(0.7)),
    ));

    // Stack waveform with controls overlay
    zstack((waveform, controls_overlay)).ignore_safe_area(EdgeSet::ALL)
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
