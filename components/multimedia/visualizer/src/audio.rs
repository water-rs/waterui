//! Shared audio capture logic for visualizers.

use futures::StreamExt;
use std::{
    cell::{Cell, Ref, RefCell},
    fmt,
    rc::Rc,
};
use waterkit_audio::{AudioBuffer, AudioRecorderBuilder};

/// Number of audio samples in the buffer.
pub const SAMPLES_COUNT: usize = 1024;

/// Shared audio state between the local capture task and renderers.
///
/// Construction is side-effect free. The first GPU renderer using the capture
/// starts one recorder task during its setup; clones share that task and sample
/// buffer.
#[derive(Clone)]
pub struct AudioCapture {
    state: Rc<AudioCaptureState>,
}

struct AudioCaptureState {
    samples: RefCell<Box<[f32; SAMPLES_COUNT]>>,
    active: Cell<bool>,
}

impl fmt::Debug for AudioCapture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioCapture")
            .field("active", &self.state.active.get())
            .finish_non_exhaustive()
    }
}

impl AudioCapture {
    /// Create a shared audio capture session.
    ///
    /// Recording starts when the first visualizer using this value completes
    /// GPU setup.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Rc::new(AudioCaptureState {
                samples: RefCell::new(Box::new([0.0; SAMPLES_COUNT])),
                active: Cell::new(false),
            }),
        }
    }

    pub(crate) fn activate(&self) {
        if self.state.active.replace(true) {
            return;
        }

        let (sender, receiver) = async_channel::bounded(1);
        let state = Rc::downgrade(&self.state);

        executor_core::spawn_local(async move {
            let capture = blocking::unblock(move || capture_audio(sender));
            let consume = async move {
                let mut write_pos = 0;
                while let Ok(buffer) = receiver.recv().await {
                    let Some(state) = state.upgrade() else {
                        break;
                    };
                    let mut samples = state.samples.borrow_mut();
                    for &sample in buffer.samples() {
                        samples[write_pos] = sample;
                        write_pos = (write_pos + 1) % SAMPLES_COUNT;
                    }
                }
            };

            futures::future::join(capture, consume).await;
        })
        .detach();
    }

    pub(crate) fn samples(&self) -> Ref<'_, [f32; SAMPLES_COUNT]> {
        Ref::map(self.state.samples.borrow(), |samples| samples.as_ref())
    }
}

fn capture_audio(sender: async_channel::Sender<AudioBuffer>) {
    futures::executor::block_on(async move {
        let mut recorder = AudioRecorderBuilder::new()
            .build()
            .expect("AudioCapture failed to create the platform recorder");
        recorder
            .start()
            .await
            .expect("AudioCapture failed to start the platform recorder");
        tracing::info!("Audio capture started");

        let mut audio_stream = Box::pin(recorder.stream());
        loop {
            let buffer = audio_stream
                .next()
                .await
                .expect("AudioCapture stream ended while the capture session was active");
            if sender.force_send(buffer).is_err() {
                break;
            }
        }
        drop(audio_stream);

        recorder
            .stop()
            .await
            .expect("AudioCapture failed to stop the platform recorder");
    });
}

impl Default for AudioCapture {
    fn default() -> Self {
        Self::new()
    }
}
