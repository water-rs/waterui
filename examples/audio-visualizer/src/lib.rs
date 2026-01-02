use waterui::prelude::*;
use waterui_visualizer::{Waveform, WaveformTheme};

#[view]
fn main_view() -> impl View {
    VStack {
        // Overlay Title and Controls
        ZStack {
            Waveform::new()
                .theme(WaveformTheme::cyber())
                .sensitivity(1.2)
                .glow(0.8)
                .expand()

            VStack {
                Text::new("WaterUI Visualizer")
                    .font_size(24.0)
                    .color(Color::WHITE)
                    .font_weight(FontWeight::Bold)
                    .padding(20.0)

                Spacer()
                
                Text::new("Waveform Mode")
                    .font_size(14.0)
                    .color(Color::WHITE.with_alpha(0.5))
                    .padding_bottom(20.0)
            }
        }
    }
    .background(Color::BLACK)
}

#[waterui::main]
fn main() {
    waterui::run(main_view)
}
