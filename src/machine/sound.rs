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
    /// How much of the current sample period has passed, in cycles (a
    /// fraction, because a sample is rarely a whole number of cycles).
    elapsed: f64,
    /// Cycles the speaker was high during that time (also fractional).
    high: f64,
    samples: Vec<f32>,
}

impl Sampler {
    /// `clock_hz` is the CPU clock, `sample_rate` the audio device's rate.
    pub fn new(clock_hz: f64, sample_rate: f64) -> Self {
        Sampler {
            enabled: false,
            cycles_per_sample: clock_hz / sample_rate,
            elapsed: 0.0,
            high: 0.0,
            samples: Vec::new(),
        }
    }

    pub fn set_sample_rate(&mut self, clock_hz: f64, sample_rate: f64) {
        self.cycles_per_sample = clock_hz / sample_rate;
    }

    /// Call once per CPU cycle with the current speaker level. Returns true
    /// when this cycle completed an output sample (so that something else can
    /// be mixed into it with [`Sampler::add_to_last_sample`]).
    ///
    /// Each output sample is the exact average of the speaker level over its
    /// period. A period is rarely a whole number of cycles (1 MHz / 44.1 kHz
    /// is 22.68), so the cycle that crosses a boundary is split between the
    /// two samples it touches. Getting this wrong makes a speaker that is
    /// perfectly still sound like a faint tone.
    pub fn tick(&mut self, speaker_high: bool) -> bool {
        if !self.enabled {
            return false;
        }
        let level = if speaker_high { 1.0 } else { 0.0 };
        let time_left = self.cycles_per_sample - self.elapsed;
        if time_left > 1.0 {
            self.elapsed += 1.0;
            self.high += level;
            return false;
        }
        // This cycle finishes the sample: the first `time_left` of it belongs
        // to this sample, the rest to the next.
        self.high += level * time_left;
        self.samples
            .push((self.high / self.cycles_per_sample) as f32);
        let carried = 1.0 - time_left;
        self.elapsed = carried;
        self.high = level * carried;
        if self.samples.len() > MAX_BUFFERED_SAMPLES {
            self.samples.drain(..MAX_BUFFERED_SAMPLES / 2);
        }
        true
    }

    /// Add another sound source (the SID) to the sample just finished.
    pub fn add_to_last_sample(&mut self, level: f32) {
        if let Some(last) = self.samples.last_mut() {
            *last += level;
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
    fn tick_says_when_a_sample_is_finished_and_more_can_be_added_to_it() {
        let mut sampler = Sampler::new(1_000_000.0, 100_000.0); // 10 cycles per sample
        sampler.enabled = true;
        let finished: Vec<bool> = (0..20).map(|_| sampler.tick(false)).collect();
        assert_eq!(finished.iter().filter(|&&done| done).count(), 2);
        sampler.add_to_last_sample(0.25);
        assert_eq!(sampler.take_samples(), vec![0.0, 0.25]);
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
    fn a_still_speaker_gives_a_perfectly_flat_signal_even_when_a_sample_is_not_a_whole_number_of_cycles()
     {
        // 1 MHz at 44.1 kHz: 22.68 cycles per sample. A speaker stuck high must
        // give exactly 1.0 every time, or it sounds like a faint whine.
        for level in [true, false] {
            let mut sampler = Sampler::new(1_000_000.0, 44_100.0);
            sampler.enabled = true;
            for _ in 0..1_000_000 {
                sampler.tick(level);
            }
            let samples = sampler.take_samples();
            assert!(samples.len() > 1000);
            let expected = if level { 1.0 } else { 0.0 };
            let worst = samples
                .iter()
                .map(|s| (s - expected).abs())
                .fold(0.0, f32::max);
            assert!(worst < 1e-5, "level {level}: samples vary by {worst}");
        }
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
