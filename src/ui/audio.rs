//! Sound output through the computer's speakers, using cpal.
//!
//! The emulator produces samples in bursts (once per screen refresh) while the
//! sound card asks for them continuously from its own thread. A shared queue
//! sits between the two: [`AudioOutput::push`] adds samples and the sound
//! card's callback takes them off.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

/// Wait until this much sound is queued before starting to play, so small
/// delays in the emulator do not cause gaps.
const PREBUFFER_SECONDS: f32 = 0.04;
/// Never queue more than this much; the oldest sound is dropped instead of
/// letting the delay between action and sound grow.
const MAX_QUEUED_SECONDS: f32 = 0.2;

#[derive(Default)]
struct Queue {
    samples: VecDeque<f32>,
    /// Playing, or waiting for enough samples to build up?
    started: bool,
}

pub struct AudioOutput {
    /// Dropping the stream stops the sound, so it must be kept alive.
    _stream: cpal::Stream,
    queue: Arc<Mutex<Queue>>,
    sample_rate: u32,
}

impl AudioOutput {
    /// Open the default output device.
    pub fn new() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no audio output device found")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let queue = Arc::new(Mutex::new(Queue::default()));

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, &queue)?,
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, &queue)?,
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, &queue)?,
            other => return Err(format!("unsupported audio sample format {other}")),
        };
        stream.play().map_err(|e| e.to_string())?;
        Ok(AudioOutput {
            _stream: stream,
            queue,
            sample_rate: config.sample_rate(),
        })
    }

    /// The device's sample rate; the emulator must generate samples at this rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Queue samples (each between -1.0 and 1.0) for playing.
    pub fn push(&self, samples: &[f32]) {
        let mut queue = self.queue.lock().expect("audio queue lock");
        queue.samples.extend(samples);
        let max = (MAX_QUEUED_SECONDS * self.sample_rate as f32) as usize;
        let excess = queue.samples.len().saturating_sub(max);
        queue.samples.drain(..excess);
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    queue: &Arc<Mutex<Queue>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels() as usize;
    let prebuffer = (PREBUFFER_SECONDS * config.sample_rate() as f32) as usize;
    let queue = Arc::clone(queue);

    let fill = move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
        let mut queue = queue.lock().expect("audio queue lock");
        if !queue.started && queue.samples.len() >= prebuffer {
            queue.started = true;
        }
        for frame in output.chunks_mut(channels) {
            let value = if queue.started {
                queue.samples.pop_front().unwrap_or_else(|| {
                    queue.started = false; // ran dry: rebuild the buffer
                    0.0
                })
            } else {
                0.0
            };
            frame.fill(T::from_sample(value)); // the same sound on every channel
        }
    };
    let on_error = |error: cpal::Error| eprintln!("audio error: {error}");

    device
        .build_output_stream(config.config(), fill, on_error, None)
        .map_err(|e| e.to_string())
}
