//! Shared audio capture logic for visualizers.

use futures::StreamExt;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use waterkit_audio::AudioRecorderBuilder;

/// Number of audio samples in the buffer.
pub const SAMPLES_COUNT: usize = 1024;

/// Shared audio state between capture thread and renderer.
///
/// Creating an `AudioCapture` starts one recorder thread. Clone the handle to
/// share that same capture session across multiple visualizers.
#[derive(Clone, Debug)]
pub struct AudioCapture {
    /// Raw audio samples.
    pub samples: Arc<Mutex<Vec<f32>>>,
    /// Smoothed samples for display.
    pub smoothed: Arc<Mutex<Vec<f32>>>,
    /// Whether capture is active.
    running: Arc<AtomicBool>,
}

impl AudioCapture {
    /// Create a new audio capture and start the capture thread.
    #[must_use]
    pub fn new() -> Self {
        let samples = Arc::new(Mutex::new(vec![0.0; SAMPLES_COUNT]));
        let smoothed = Arc::new(Mutex::new(vec![0.0; SAMPLES_COUNT]));
        let running = Arc::new(AtomicBool::new(true));

        let capture = Self {
            samples: samples.clone(),
            smoothed,
            running: running.clone(),
        };

        // Spawn audio capture thread
        std::thread::spawn(move || {
            let mut recorder = match AudioRecorderBuilder::new().build() {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("Failed to create AudioRecorder: {:?}", e);
                    return;
                }
            };

            if let Err(e) = pollster::block_on(recorder.start()) {
                tracing::error!("Failed to start recording: {:?}", e);
                return;
            }

            tracing::info!("Audio capture started");

            let mut write_pos: usize = 0;

            let mut audio_stream = Box::pin(recorder.stream());

            while running.load(Ordering::Relaxed) {
                if let Some(buffer) = pollster::block_on(audio_stream.next()) {
                    let audio_samples = buffer.samples();
                    if let Ok(mut lock) = samples.lock() {
                        for &s in audio_samples {
                            lock[write_pos] = s;
                            write_pos = (write_pos + 1) % SAMPLES_COUNT;
                        }
                    }
                } else {
                    tracing::error!("Audio stream ended");
                    break;
                }
            }
        });

        capture
    }

    /// Get smoothed samples with temporal interpolation.
    ///
    /// # Panics
    ///
    /// Panics if the internal sample mutex is poisoned.
    #[must_use]
    pub fn get_smoothed_samples(&self, smoothing: f32) -> Vec<f32> {
        let raw = self.samples.lock().unwrap().clone();
        let mut smoothed = self.smoothed.lock().unwrap();

        for (i, &target) in raw.iter().enumerate() {
            smoothed[i] = smoothed[i].mul_add(1.0 - smoothing, target * smoothing);
        }

        smoothed.clone()
    }
}

impl Default for AudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
