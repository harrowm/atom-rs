//! Sound output through the computer's speakers, using cpal.
//!
//! The emulator produces samples in bursts (once per screen refresh) while the
//! sound card asks for them continuously from its own thread. A shared queue
//! sits between the two: [`AudioOutput::push`] adds samples and the sound
//! card's callback takes them off.
//!
//! Two things keep it from clicking:
//!
//! * A [`DcBlocker`] removes any steady level from the signal. The Atom's
//!   speaker can sit high or low for ever, and a sound card that jumps between
//!   such a level and silence thumps.
//! * Playback fades in over a few milliseconds when it starts and fades out
//!   when the emulator stops supplying sound (paused, say), instead of
//!   cutting off at whatever level the wave happened to be at.

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
/// Playback ramps up to full volume over this many samples (about 5 ms).
const FADE_IN_SAMPLES: f32 = 220.0;
/// When the queue runs dry the last sample decays by this factor per sample
/// (to nothing in about a millisecond).
const FADE_OUT_FACTOR: f32 = 0.98;

/// A one-pole high-pass filter: passes sound, removes a steady level.
///
/// `y[n] = x[n] - x[n-1] + R * y[n-1]` with R just under 1, which puts the
/// cut-off a few tens of hertz below anything audible.
#[derive(Clone, Copy, Debug, Default)]
pub struct DcBlocker {
    previous_input: f32,
    previous_output: f32,
}

impl DcBlocker {
    const R: f32 = 0.995;

    pub fn process(&mut self, input: f32) -> f32 {
        let output = input - self.previous_input + Self::R * self.previous_output;
        self.previous_input = input;
        self.previous_output = output;
        output
    }

    /// Start from a signal that is already at `level`, so that the first
    /// sample after a gap does not look like a sudden step.
    pub fn prime(&mut self, level: f32) {
        self.previous_input = level;
        self.previous_output = 0.0;
    }
}

struct Queue {
    samples: VecDeque<f32>,
    /// Playing, or waiting for enough samples to build up?
    started: bool,
    /// The last sample played, which fades out if the queue runs dry.
    last_output: f32,
    /// 0.0 to 1.0: how far the fade-in has got.
    fade_in: f32,
    blocker: DcBlocker,
    /// The next pushed sample follows a gap, so re-prime the blocker.
    needs_prime: bool,
}

impl Queue {
    fn new() -> Self {
        Queue {
            samples: VecDeque::new(),
            started: false,
            last_output: 0.0,
            fade_in: 0.0,
            blocker: DcBlocker::default(),
            needs_prime: true,
        }
    }
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
        let queue = Arc::new(Mutex::new(Queue::new()));

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

    /// Throw away what is queued. The sound card then runs dry, which fades the
    /// last sample out smoothly; the next samples fade in again. Used when the
    /// machine restarts, so the jump in the speaker's level is never heard as
    /// a click.
    pub fn clear(&self) {
        self.queue.lock().expect("audio queue lock").samples.clear();
    }

    /// Queue samples (each between -1.0 and 1.0) for playing.
    pub fn push(&self, samples: &[f32]) {
        let Some(&first) = samples.first() else {
            return;
        };
        let mut queue = self.queue.lock().expect("audio queue lock");
        if queue.needs_prime {
            queue.blocker.prime(first);
            queue.needs_prime = false;
        }
        for &sample in samples {
            let filtered = queue.blocker.process(sample);
            queue.samples.push_back(filtered);
        }
        let max = (MAX_QUEUED_SECONDS * self.sample_rate as f32) as usize;
        let excess = queue.samples.len().saturating_sub(max);
        queue.samples.drain(..excess);
    }
}

/// The next sample to play, updating the fade state.
fn next_sample(queue: &mut Queue, prebuffer: usize) -> f32 {
    if !queue.started && queue.samples.len() >= prebuffer {
        queue.started = true;
        queue.fade_in = 0.0;
    }
    if queue.started {
        if let Some(sample) = queue.samples.pop_front() {
            queue.fade_in = (queue.fade_in + 1.0 / FADE_IN_SAMPLES).min(1.0);
            queue.last_output = sample * queue.fade_in;
            return queue.last_output;
        }
        // Ran dry: stop, and re-prime the filter for when sound resumes.
        queue.started = false;
        queue.needs_prime = true;
    }
    // Silent (or fading out from the last sample played).
    queue.last_output *= FADE_OUT_FACTOR;
    queue.last_output
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
        for frame in output.chunks_mut(channels) {
            let value = next_sample(&mut queue, prebuffer);
            frame.fill(T::from_sample(value)); // the same sound on every channel
        }
    };
    let on_error = |error: cpal::Error| eprintln!("audio error: {error}");

    device
        .build_output_stream(config.config(), fill, on_error, None)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_blocker_removes_a_steady_level() {
        let mut blocker = DcBlocker::default();
        let mut last = 1.0;
        for _ in 0..5000 {
            last = blocker.process(0.5);
        }
        assert!(
            last.abs() < 0.001,
            "a constant input should settle to zero, got {last}"
        );
    }

    #[test]
    fn dc_blocker_passes_audible_tones() {
        // A 1 kHz square wave at 44.1 kHz keeps nearly all of its swing.
        let mut blocker = DcBlocker::default();
        let mut peak = 0.0f32;
        for n in 0..4000 {
            let input = if (n / 22) % 2 == 0 { 0.5 } else { -0.5 };
            let output = blocker.process(input);
            if n > 2000 {
                peak = peak.max(output.abs());
            }
        }
        assert!(peak > 0.45, "tone was attenuated to {peak}");
    }

    #[test]
    fn a_primed_blocker_does_not_jump_at_the_start() {
        let mut blocker = DcBlocker::default();
        blocker.prime(0.5);
        assert!(
            blocker.process(0.5).abs() < 1e-6,
            "no step when sound starts at a steady level"
        );
    }

    #[test]
    fn playback_fades_in_instead_of_starting_with_a_click() {
        let mut queue = Queue::new();
        queue.samples.extend(std::iter::repeat_n(0.4, 1000));
        let first = next_sample(&mut queue, 100);
        let later: f32 = (0..400)
            .map(|_| next_sample(&mut queue, 100))
            .last()
            .unwrap();
        assert!(first < 0.01, "first sample {first} should be nearly silent");
        assert!(
            (later - 0.4).abs() < 0.01,
            "should reach full volume, got {later}"
        );
    }

    #[test]
    fn running_dry_fades_out_smoothly() {
        let mut queue = Queue::new();
        queue.samples.extend(std::iter::repeat_n(0.4, 600));
        for _ in 0..600 {
            next_sample(&mut queue, 100);
        }
        let mut previous = queue.last_output;
        for _ in 0..400 {
            let sample = next_sample(&mut queue, 100);
            assert!(
                sample.abs() <= previous.abs() + 1e-6,
                "fade-out must not grow"
            );
            previous = sample;
        }
        assert!(previous.abs() < 0.001, "should have faded to silence");
        assert!(
            queue.needs_prime,
            "the filter must be re-primed when sound resumes"
        );
    }
}
