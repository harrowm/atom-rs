//! Turning the Atom's 1-bit speaker into audio samples.
//!
//! The speaker is a single bit (8255 port C bit 2) that the program flips at
//! up to a few hundred kHz. We sample it once per CPU cycle (1 MHz) and
//! average it over each output sample period. Averaging is a simple low-pass
//! filter, which stops fast toggling from aliasing into audible noise.

/// Never keep more than this many unread samples, even if nobody listens.
const MAX_BUFFERED_SAMPLES: usize = 1 << 16;

#[derive(Clone, Debug)]
pub struct Sampler {
    /// Off by default so that headless runs and tests pay nothing.
    pub enabled: bool,
    cycles_per_sample: f64,
    elapsed_cycles: f64,
    high_cycles: f64,
    samples: Vec<f32>,
}

impl Sampler {
    /// `clock_hz` is the CPU clock, `sample_rate` the audio device's rate.
    pub fn new(clock_hz: f64, sample_rate: f64) -> Self {
        Sampler {
            enabled: false,
            cycles_per_sample: clock_hz / sample_rate,
            elapsed_cycles: 0.0,
            high_cycles: 0.0,
            samples: Vec::new(),
        }
    }

    pub fn set_sample_rate(&mut self, clock_hz: f64, sample_rate: f64) {
        self.cycles_per_sample = clock_hz / sample_rate;
    }

    /// Call once per CPU cycle with the current speaker level.
    pub fn tick(&mut self, speaker_high: bool) {
        if !self.enabled {
            return;
        }
        self.elapsed_cycles += 1.0;
        if speaker_high {
            self.high_cycles += 1.0;
        }
        if self.elapsed_cycles >= self.cycles_per_sample {
            // The fraction of this period the speaker was high: 0.0 to 1.0.
            self.samples
                .push((self.high_cycles / self.elapsed_cycles) as f32);
            self.elapsed_cycles -= self.cycles_per_sample;
            self.high_cycles = 0.0;
            if self.samples.len() > MAX_BUFFERED_SAMPLES {
                self.samples.drain(..MAX_BUFFERED_SAMPLES / 2);
            }
        }
    }

    /// Hand over the samples produced since the last call.
    pub fn take_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_one_sample_per_period() {
        let mut sampler = Sampler::new(1_000_000.0, 1000.0); // 1000 cycles per sample
        sampler.enabled = true;
        for _ in 0..5000 {
            sampler.tick(false);
        }
        assert_eq!(sampler.take_samples().len(), 5);
        assert!(sampler.take_samples().is_empty());
    }

    #[test]
    fn a_square_wave_averages_to_the_middle() {
        let mut sampler = Sampler::new(1_000_000.0, 1000.0);
        sampler.enabled = true;
        for cycle in 0..1000 {
            sampler.tick(cycle % 2 == 0); // toggling every cycle
        }
        let samples = sampler.take_samples();
        assert!((samples[0] - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_steady_level_gives_that_level() {
        let mut sampler = Sampler::new(1_000_000.0, 1000.0);
        sampler.enabled = true;
        for _ in 0..2000 {
            sampler.tick(true);
        }
        assert_eq!(sampler.take_samples(), vec![1.0, 1.0]);
    }

    #[test]
    fn disabled_sampler_collects_nothing() {
        let mut sampler = Sampler::new(1_000_000.0, 1000.0);
        for _ in 0..5000 {
            sampler.tick(true);
        }
        assert!(sampler.take_samples().is_empty());
    }
}
