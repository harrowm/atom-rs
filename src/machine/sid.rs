//! A SID sound chip card for the Atom.
//!
//! The SID (MOS 6581, from the Commodore 64) has three voices, each with a
//! choice of waveform and its own volume envelope, plus a filter. Several
//! add-on cards put one in the Atom, mapped at `BDC0-BDDF` (32 registers),
//! and tunes from the Atom Software Archive's `SID` folder use it. It is
//! clocked from the Atom's 1 MHz clock, as in Atomulator.
//!
//! The chip itself comes from the `sidera` crate (MIT licensed). This file is
//! the glue: it maps the card into the Atom's memory, remembers what was
//! written, and produces one audio sample for each sample the speaker
//! produces, so the two can be added together.
//!
//! The chip is stepped once per audio sample, running as many of its own
//! clock cycles as fit in that sample, rather than once per Atom cycle. A
//! register write therefore takes effect at the next sample boundary, about
//! 22 microseconds late at 44.1 kHz. Tunes, which change the chip about 50
//! times a second, cannot tell; sample playback through the volume register
//! would lose a little detail.

use serde::{Deserialize, Serialize};
use sidera::{ChipModel, Sid as Chip};

/// Which SID chip is fitted. The two sound different: the 8580 is the later,
/// cleaner chip, with a smoother filter, and tunes can sound quite different
/// on each. (Atomulator offers many models; sidera has these two.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidModel {
    #[default]
    Mos6581,
    Mos8580,
}

impl SidModel {
    pub const ALL: [SidModel; 2] = [SidModel::Mos6581, SidModel::Mos8580];

    pub fn label(self) -> &'static str {
        match self {
            SidModel::Mos6581 => "6581 (the original)",
            SidModel::Mos8580 => "8580 (the later chip)",
        }
    }
}

/// The card's registers sit here in the Atom's memory.
pub const FIRST_ADDRESS: u16 = 0xBDC0;
pub const LAST_ADDRESS: u16 = 0xBDDF;
const REGISTER_COUNT: usize = 32;

/// The SID is clocked at the Atom's 1 MHz.
const CHIP_CLOCK_HZ: u32 = 1_000_000;

/// How loud the chip is in the mix. The speaker swings between 0 and 1; the
/// chip's output spans the whole of an `i16`, so this brings it to a similar
/// level (and a little under, as the speaker is a harsh square wave).
const MIX_GAIN: f32 = 0.6 / 32768.0;

pub struct SidCard {
    chip: Chip,
    model: SidModel,
    sample_rate: u32,
    /// What has been written to each register. The chip's sample rate is fixed
    /// when it is made, so a change of rate builds a new chip and replays these.
    written: [u8; REGISTER_COUNT],
}

impl SidCard {
    pub fn new(sample_rate: u32) -> Self {
        SidCard {
            chip: new_chip(sample_rate, SidModel::default()),
            model: SidModel::default(),
            sample_rate,
            written: [0; REGISTER_COUNT],
        }
    }

    /// Use a different audio sample rate (when the sound card turns out to
    /// run at a rate other than the one first assumed).
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate == self.sample_rate {
            return;
        }
        self.sample_rate = sample_rate;
        self.chip = new_chip(sample_rate, self.model);
        for (register, &value) in self.written.iter().enumerate() {
            self.chip.write_register(register as u8, value);
        }
    }

    pub fn model(&self) -> SidModel {
        self.model
    }

    /// Fit a different SID chip. What has been written to it is kept, so a
    /// tune carries on playing, now on the new chip.
    pub fn set_model(&mut self, model: SidModel) {
        if model == self.model {
            return;
        }
        self.model = model;
        self.chip = new_chip(self.sample_rate, model);
        for (register, &value) in self.written.iter().enumerate() {
            self.chip.write_register(register as u8, value);
        }
    }

    /// The RESET line: back to silence.
    pub fn reset(&mut self) {
        self.chip.reset();
        self.written = [0; REGISTER_COUNT];
    }

    pub fn read(&self, addr: u16) -> u8 {
        self.chip.read_register(register_number(addr))
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        let register = register_number(addr);
        self.written[register as usize] = value;
        self.chip.write_register(register, value);
    }

    /// The chip's next audio sample, as a level to add to the speaker's.
    ///
    /// While the master volume is zero the chip is silent whatever else it is
    /// doing, so it is not stepped at all. That matters: almost no Atom
    /// software uses a SID, and stepping it costs about half the emulator's
    /// speed. (The price is that a program reading the voice 3 registers
    /// (`OSC3`, `ENV3`) with the volume at zero sees them stand still.)
    pub fn next_sample(&mut self) -> f32 {
        if self.written[usize::from(sidera::registers::MODE_VOL)] & 0x0F == 0 {
            return 0.0;
        }
        f32::from(self.chip.clock()) * MIX_GAIN
    }
}

fn register_number(addr: u16) -> u8 {
    (addr & 0x1F) as u8
}

fn new_chip(sample_rate: u32, model: SidModel) -> Chip {
    let mut chip = Chip::new(sample_rate);
    chip.set_model(match model {
        SidModel::Mos6581 => ChipModel::Mos6581,
        SidModel::Mos8580 => ChipModel::Mos8580,
    });
    chip.set_chip_clock(CHIP_CLOCK_HZ);
    chip
}

#[cfg(test)]
mod tests {
    use super::*;
    use sidera::registers::*;

    const RATE: u32 = 44_100;

    /// Set voice 1 playing a tone: the register value for `hz` is
    /// `hz * 2^24 / clock`.
    fn start_tone(card: &mut SidCard, hz: f64, control: u8) {
        let value = (hz * 16_777_216.0 / f64::from(CHIP_CLOCK_HZ)).round() as u16;
        card.write(FIRST_ADDRESS + u16::from(V1_FREQ_LO), value as u8);
        card.write(FIRST_ADDRESS + u16::from(V1_FREQ_HI), (value >> 8) as u8);
        card.write(FIRST_ADDRESS + u16::from(V1_ATTACK_DECAY), 0x00);
        card.write(FIRST_ADDRESS + u16::from(V1_SUSTAIN_RELEASE), 0xF0);
        card.write(FIRST_ADDRESS + u16::from(MODE_VOL), 0x0F);
        card.write(FIRST_ADDRESS + u16::from(V1_CONTROL), control | CTRL_GATE);
    }

    fn render(card: &mut SidCard, seconds: f64) -> Vec<f32> {
        (0..(seconds * f64::from(RATE)) as usize)
            .map(|_| card.next_sample())
            .collect()
    }

    /// The pitch of a signal, from how often it crosses zero going upwards
    /// (with the steady level removed first).
    fn measured_hz(samples: &[f32]) -> f64 {
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let crossings = samples
            .windows(2)
            .filter(|pair| pair[0] - mean < 0.0 && pair[1] - mean >= 0.0)
            .count();
        crossings as f64 / (samples.len() as f64 / f64::from(RATE))
    }

    #[test]
    fn a_fresh_card_is_silent() {
        let mut card = SidCard::new(RATE);
        let loudest = render(&mut card, 0.1)
            .iter()
            .fold(0.0_f32, |a, &s| a.max(s.abs()));
        assert!(
            loudest < 0.01,
            "a chip with no volume set should be silent, got {loudest}"
        );
    }

    #[test]
    fn the_pitch_is_right_at_the_atoms_clock() {
        // This is the reason the chip's clock is set to 1 MHz: at the C64's
        // 985 kHz the same register value would sound 1.5% flat.
        for hz in [220.0, 440.0, 880.0] {
            let mut card = SidCard::new(RATE);
            start_tone(&mut card, hz, CTRL_SAW);
            render(&mut card, 0.2); // let the envelope open
            let measured = measured_hz(&render(&mut card, 1.0));
            assert!(
                (measured - hz).abs() < 1.5,
                "asked for {hz} Hz, heard {measured} Hz"
            );
        }
    }

    #[test]
    fn a_tone_is_audible_and_not_louder_than_the_speaker() {
        let mut card = SidCard::new(RATE);
        start_tone(&mut card, 440.0, CTRL_PULSE);
        card.write(FIRST_ADDRESS + u16::from(V1_PULSE_HI), 0x08);
        render(&mut card, 0.2);
        let loudest = render(&mut card, 0.5)
            .iter()
            .fold(0.0_f32, |a, &s| a.max(s.abs()));
        assert!(loudest > 0.05, "too quiet: {loudest}");
        assert!(loudest < 1.0, "should not swamp the speaker: {loudest}");
    }

    #[test]
    fn releasing_the_gate_fades_the_note_out() {
        let mut card = SidCard::new(RATE);
        start_tone(&mut card, 440.0, CTRL_SAW);
        card.write(FIRST_ADDRESS + u16::from(V1_SUSTAIN_RELEASE), 0xF1); // quick release
        render(&mut card, 0.2);
        card.write(FIRST_ADDRESS + u16::from(V1_CONTROL), CTRL_SAW); // gate off
        render(&mut card, 0.5);
        let loudest = render(&mut card, 0.1)
            .iter()
            .fold(0.0_f32, |a, &s| a.max(s.abs()));
        assert!(loudest < 0.01, "still sounding after release: {loudest}");
    }

    #[test]
    fn the_registers_repeat_every_32_bytes() {
        // Only the low five address bits are decoded: BDE0 would not be this
        // card, but the card sees BDC0+n as register n.
        assert_eq!(register_number(0xBDC0), 0);
        assert_eq!(register_number(0xBDD8), 0x18);
        assert_eq!(register_number(0xBDDF), 0x1F);
    }

    #[test]
    fn voice_3_can_be_read_back() {
        // Programs read OSC3 (register 1B) as a random number source, with
        // voice 3 set to noise.
        let mut card = SidCard::new(RATE);
        card.write(FIRST_ADDRESS + u16::from(MODE_VOL), 0x0F);
        card.write(FIRST_ADDRESS + u16::from(V3_FREQ_HI), 0xFF);
        card.write(
            FIRST_ADDRESS + u16::from(V3_CONTROL),
            CTRL_NOISE | CTRL_GATE,
        );
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            render(&mut card, 0.001);
            seen.insert(card.read(FIRST_ADDRESS + u16::from(OSC3)));
        }
        assert!(
            seen.len() > 10,
            "OSC3 should vary with noise on, saw {} values",
            seen.len()
        );
    }

    #[test]
    fn changing_the_sample_rate_keeps_what_was_written() {
        let mut card = SidCard::new(RATE);
        start_tone(&mut card, 440.0, CTRL_SAW);
        card.set_sample_rate(48_000);
        let mut render_at_48k = |seconds: f64| -> Vec<f32> {
            (0..(seconds * 48_000.0) as usize)
                .map(|_| card.next_sample())
                .collect()
        };
        render_at_48k(0.2); // let the envelope open, as in the other tests
        let samples = render_at_48k(1.0);
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let crossings = samples
            .windows(2)
            .filter(|pair| pair[0] - mean < 0.0 && pair[1] - mean >= 0.0)
            .count();
        assert!(
            (crossings as f64 - 440.0).abs() < 2.0,
            "heard {crossings} Hz"
        );
    }

    #[test]
    fn reset_silences_the_card() {
        let mut card = SidCard::new(RATE);
        start_tone(&mut card, 440.0, CTRL_SAW);
        render(&mut card, 0.1);
        card.reset();
        let loudest = render(&mut card, 0.2)
            .iter()
            .fold(0.0_f32, |a, &s| a.max(s.abs()));
        assert!(
            loudest < 0.01,
            "reset should silence the chip, got {loudest}"
        );
    }

    #[test]
    fn the_two_chips_give_different_combined_waveforms() {
        // A plain wave is the same on both; the models differ in combined
        // waveforms (and the filter). Sawtooth + triangle is such a wave.
        let play = |model: SidModel| {
            let mut card = SidCard::new(RATE);
            card.set_model(model);
            start_tone(&mut card, 440.0, CTRL_SAW | CTRL_TRI);
            render(&mut card, 0.2);
            render(&mut card, 0.5)
        };
        let (original, later) = (play(SidModel::Mos6581), play(SidModel::Mos8580));
        assert!(
            original != later,
            "the models should give different waveforms"
        );
    }

    #[test]
    fn changing_the_model_keeps_the_note_going_and_reset_keeps_the_model() {
        let mut card = SidCard::new(RATE);
        start_tone(&mut card, 440.0, CTRL_SAW);
        card.set_model(SidModel::Mos8580);
        render(&mut card, 0.2);
        let heard = measured_hz(&render(&mut card, 1.0));
        assert!(
            (heard - 440.0).abs() < 2.0,
            "still playing after the swap: {heard} Hz"
        );
        card.reset();
        assert_eq!(card.model(), SidModel::Mos8580);
        card.set_sample_rate(48_000);
        assert_eq!(card.model(), SidModel::Mos8580, "and through a rate change");
    }
}
