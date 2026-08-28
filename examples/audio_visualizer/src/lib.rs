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

use waterkit_permission::{Permission, PermissionStatus, check, request};
use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::slider::slider;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::shape::{Rectangle, ShapeExt};
use waterui_visualizer::{AudioCapture, Waveform, WaveformTheme};

#[preview]
pub fn demo() -> impl View {
    let permission_granted = Binding::container(false);
    let permission_status = Binding::container(Str::from_static(
        "Microphone access is required to start the live waveform.",
    ));
    let prompt_granted = permission_granted.clone();
    let prompt_status = permission_status.clone();

    Dynamic::watch(permission_granted, move |granted| {
        if granted {
            AnyView::new(visualizer())
        } else {
            AnyView::new(microphone_permission_prompt(
                prompt_granted.clone(),
                prompt_status.clone(),
            ))
        }
    })
}

fn microphone_permission_prompt(
    permission_granted: Binding<bool>,
    permission_status: Binding<Str>,
) -> impl View {
    zstack((
        Rectangle.fill(Srgb::BLACK),
        vstack((
            text("WaterUI Audio Visualizer")
                .title()
                .bold()
                .foreground(Srgb::WHITE),
            text!("{permission_status}").foreground(Srgb::WHITE),
            button("Enable Microphone")
                .action_async(
                    |State(granted): State<Binding<bool>>,
                     State(status): State<Binding<Str>>| async move {
                        if check(Permission::Microphone).await == PermissionStatus::Granted {
                            granted.set(true);
                            return;
                        }
                        match request(Permission::Microphone).await {
                            Ok(PermissionStatus::Granted) => granted.set(true),
                            Ok(_) => status.set(Str::from_static(
                                "Approve the system permission dialog, then tap Enable Microphone again.",
                            )),
                            Err(error) => {
                                status.set(
                                    format!("Microphone permission request failed: {error}").into(),
                                );
                            }
                        }
                    },
                )
                .state(&permission_granted)
                .state(&permission_status),
        ))
        .spacing(16.0)
        .padding_with(24.0),
    ))
    .ignore_safe_area(EdgeSet::ALL)
}

fn visualizer() -> impl View {
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
            text("WaterUI Audio Visualizer")
                .bold()
                .foreground(Srgb::WHITE),
            spacer_min(16.0),
            text("Theme").foreground(Srgb::WHITE),
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
            text("Sensitivity").foreground(Srgb::WHITE),
            slider("Sensitivity", &sensitivity)
                .range(0.5..=3.0)
                .hide_label(),
            spacer_min(8.0),
            text!("{mode_text}").foreground(Srgb::WHITE),
        ))
        .padding_with(EdgeInsets::all(24.0))
        .background(Srgb::BLACK.with_opacity(0.7)),
    ));

    // Stack waveform with controls overlay
    zstack((waveform, controls_overlay)).ignore_safe_area(EdgeSet::ALL)
}

pub fn app(mut env: Environment) -> App {
    env.install(Theme::new().color_scheme(ColorScheme::Dark));
    App::new(demo, env)
}
