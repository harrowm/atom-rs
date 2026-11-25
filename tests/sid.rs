//! The SID sound card, reached through the Atom's memory map by a real 6502
//! program, with the sound it makes measured.

mod common;

use atom_rs::cpu::Bus;
use atom_rs::machine::Atom;
use common::booted_atom;

const SAMPLE_RATE: u32 = 44_100;

/// A program that plays a sawtooth at 440 Hz on voice 1, then loops for ever.
/// (The frequency register for 440 Hz at 1 MHz is 440 * 2^24 / 1e6 = 0x1CD6.)
const PLAY_A440: &[u8] = &[
    0xA9, 0x0F, 0x8D, 0xD8, 0xBD, // LDA #$0F : STA $BDD8   volume 15
    0xA9, 0x00, 0x8D, 0xC5, 0xBD, // LDA #$00 : STA $BDC5   attack/decay
    0xA9, 0xF0, 0x8D, 0xC6, 0xBD, // LDA #$F0 : STA $BDC6   sustain/release
    0xA9, 0xD6, 0x8D, 0xC0, 0xBD, // LDA #$D6 : STA $BDC0   frequency low
    0xA9, 0x1C, 0x8D, 0xC1, 0xBD, // LDA #$1C : STA $BDC1   frequency high
    0xA9, 0x21, 0x8D, 0xC4, 0xBD, // LDA #$21 : STA $BDC4   sawtooth, gate on
    0x4C, 0x1E, 0x20, //            JMP $201E              wait here
];
const PROGRAM_START: u16 = 0x2000;

fn atom_running(program: &[u8]) -> Option<Atom> {
    let mut atom = booted_atom(0)?;
    atom.bus.set_audio_sample_rate(SAMPLE_RATE);
    atom.bus.sound.enabled = true;
    for (offset, &byte) in program.iter().enumerate() {
        assert!(atom.bus.poke(PROGRAM_START + offset as u16, byte));
    }
    atom.cpu.regs.pc = PROGRAM_START;
    Some(atom)
}

fn samples_after(atom: &mut Atom, seconds: f64) -> Vec<f32> {
    let cycles = (seconds * 1_000_000.0) as u64;
    atom.run_cycles(cycles);
    atom.bus.sound.take_samples()
}

fn pitch(samples: &[f32]) -> f64 {
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    let crossings = samples
        .windows(2)
        .filter(|pair| pair[0] - mean < 0.0 && pair[1] - mean >= 0.0)
        .count();
    crossings as f64 / (samples.len() as f64 / f64::from(SAMPLE_RATE))
}

#[test]
fn a_program_plays_a_note_on_the_sid() {
    let Some(mut atom) = atom_running(PLAY_A440) else {
        return;
    };
    samples_after(&mut atom, 0.3); // the note starts and its envelope opens
    let heard = pitch(&samples_after(&mut atom, 1.0));
    assert!(
        (heard - 440.0).abs() < 2.0,
        "the program asked for 440 Hz but the audio is {heard} Hz"
    );
}

#[test]
fn without_a_sid_card_the_program_is_silent() {
    let Some(mut atom) = atom_running(PLAY_A440) else {
        return;
    };
    atom.bus.memory.sid_fitted = false;
    let samples = samples_after(&mut atom, 0.5);
    let loudest = samples.iter().fold(0.0_f32, |a, &s| a.max(s.abs()));
    assert!(loudest < 0.001, "no card, but heard {loudest}");
}

#[test]
fn the_sid_sits_on_top_of_the_speaker_not_instead_of_it() {
    // The speaker toggles (port C bit 2); with the SID also playing, both are
    // in the audio. A steady speaker level plus the tone: the tone still
    // measures 440 Hz, and the level shifts everything up.
    let Some(mut atom) = atom_running(PLAY_A440) else {
        return;
    };
    atom.bus.write(0xB002, 0x04); // speaker held high
    samples_after(&mut atom, 0.3);
    let samples = samples_after(&mut atom, 0.5);
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    assert!(
        mean > 0.8,
        "the speaker's level should still be there, mean {mean}"
    );
    assert!((pitch(&samples) - 440.0).abs() < 3.0);
}

#[test]
fn reading_the_card_through_the_bus_does_not_disturb_it() {
    let Some(mut atom) = atom_running(PLAY_A440) else {
        return;
    };
    samples_after(&mut atom, 0.3);
    // Registers other than OSC3/ENV3 are write-only: they read back as 0.
    assert_eq!(atom.bus.peek(0xBDC0), 0);
    let heard = pitch(&samples_after(&mut atom, 1.0));
    assert!((heard - 440.0).abs() < 2.0);
}
