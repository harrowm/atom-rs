//! CSW tape files ("Compressed Square Wave").
//!
//! A CSW file records a tape as a list of pulse lengths: how long the signal
//! stayed high, then low, then high again, counted in samples. It is a faithful
//! copy of the sound on the tape, unlike UEF, which describes what the tape
//! says. To play one on the emulated Atom we turn the pulses back into the
//! tape chunks the player already understands (carrier tone, bytes, gaps).
//!
//! ## Reading the pulses
//!
//! The Atom's tape format has two tones: a `1` bit is 8 cycles of 2400 Hz and a
//! `0` bit is 4 cycles of 1200 Hz. Seen as pulses, one half-cycle of 2400 Hz
//! lasts about 208 microseconds (a *short* pulse) and one of 1200 Hz about 417
//! (a *long* one). A byte is a start bit (`0`: 8 long pulses), eight data bits
//! (least significant first) and a stop bit (`1`: 16 short pulses). The long
//! stretch of short pulses before a block is the leader (carrier).
//!
//! Real tapes drift in speed, so pulses are sorted by whether they are nearer
//! the short or the long length, not matched exactly.
//!
//! ## The file
//!
//! A 32-byte header (version 1) or 52-byte one plus an extension (version 2)
//! gives the sample rate and how the pulses are packed: run-length encoded
//! (a byte per pulse, or `0` followed by a 4-byte length), and in version 2
//! optionally zlib-compressed as well.

use std::io::Read;

use super::tape::{TapeChunk, TapeImage};

const SIGNATURE: &[u8] = b"Compressed Square Wave";
const TERMINATOR: u8 = 0x1A;

/// Player steps per second (see `tape.rs`): the player is stepped once per
/// half cycle of 2400 Hz.
const STEPS_PER_SECOND: f64 = 4800.0;
/// Half a cycle of 2400 Hz and of 1200 Hz, in microseconds.
const SHORT_PULSE_US: f64 = 1_000_000.0 / 4800.0;
const LONG_PULSE_US: f64 = 1_000_000.0 / 2400.0;
/// A pulse shorter than this counts as short, a longer one as long...
const SHORT_UP_TO_US: f64 = (SHORT_PULSE_US + LONG_PULSE_US) / 2.0;
/// ...up to this, beyond which the signal is silence (or noise).
const LONG_UP_TO_US: f64 = 1_000.0;

const SHORT_PULSES_PER_BIT: usize = 16;
const LONG_PULSES_PER_BIT: usize = 8;
/// How many pulses a bit may be missing and still count (jitter).
const SLACK: usize = 2;

/// Is this a CSW file?
pub fn is_csw(file: &[u8]) -> bool {
    file.starts_with(SIGNATURE)
}

/// Read a CSW file into a tape.
pub fn decode(file: &[u8]) -> Result<TapeImage, String> {
    let (sample_rate, pulses) = read_pulses(file)?;
    let microseconds = 1_000_000.0 / f64::from(sample_rate);
    let times: Vec<f64> = pulses
        .iter()
        .map(|&samples| f64::from(samples) * microseconds)
        .collect();
    Ok(TapeImage {
        chunks: chunks_from_pulses(&times),
    })
}

/// The sample rate and the pulse lengths (in samples) in a file.
fn read_pulses(file: &[u8]) -> Result<(u32, Vec<u32>), String> {
    if !is_csw(file) || file.get(SIGNATURE.len()) != Some(&TERMINATOR) {
        return Err("not a CSW file".to_string());
    }
    let major = *file.get(0x17).ok_or("the CSW header is cut short")?;
    let (sample_rate, compression, data_start) = match major {
        1 => {
            let header = file.get(..0x20).ok_or("the CSW header is cut short")?;
            let rate = u16::from_le_bytes([header[0x19], header[0x1A]]);
            (u32::from(rate), header[0x1B], 0x20)
        }
        2 => {
            let header = file.get(..0x34).ok_or("the CSW header is cut short")?;
            let rate = u32::from_le_bytes(header[0x19..0x1D].try_into().unwrap());
            let extension = usize::from(header[0x23]);
            (rate, header[0x21], 0x34 + extension)
        }
        other => return Err(format!("CSW version {other} is not supported")),
    };
    if sample_rate == 0 {
        return Err("the CSW file has no sample rate".to_string());
    }
    let packed = file
        .get(data_start..)
        .ok_or("the CSW file has no pulse data")?;
    let rle = match (major, compression) {
        (_, 1) => packed.to_vec(),
        (2, 2) => {
            let mut unpacked = Vec::new();
            flate2::read::ZlibDecoder::new(packed)
                .read_to_end(&mut unpacked)
                .map_err(|e| format!("cannot decompress the CSW pulses: {e}"))?;
            unpacked
        }
        (_, other) => return Err(format!("CSW compression type {other} is not supported")),
    };
    Ok((sample_rate, run_lengths(&rle)))
}

/// Unpack run-length encoded pulses: a byte is a length, except `0`, which is
/// followed by the length as a 4-byte number.
fn run_lengths(rle: &[u8]) -> Vec<u32> {
    let mut pulses = Vec::new();
    let mut at = 0;
    while at < rle.len() {
        if rle[at] != 0 {
            pulses.push(u32::from(rle[at]));
            at += 1;
        } else if let Some(long) = rle.get(at + 1..at + 5) {
            pulses.push(u32::from_le_bytes(long.try_into().unwrap()));
            at += 5;
        } else {
            break; // a cut-off last entry
        }
    }
    pulses
}

// ---- From pulses to tape chunks ---------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pulse {
    Short,
    Long,
    /// Silence: this many player steps.
    Gap(u32),
}

fn classify(microseconds: f64) -> Pulse {
    if microseconds < SHORT_UP_TO_US {
        Pulse::Short
    } else if microseconds < LONG_UP_TO_US {
        Pulse::Long
    } else {
        Pulse::Gap((microseconds * STEPS_PER_SECOND / 1_000_000.0).round() as u32)
    }
}

/// Turn pulse lengths (in microseconds) into carrier tones, bytes and gaps.
fn chunks_from_pulses(times: &[f64]) -> Vec<TapeChunk> {
    let pulses: Vec<Pulse> = times.iter().map(|&t| classify(t)).collect();
    let mut chunks = Vec::new();
    let mut shorts_in_a_row = 0usize;
    let mut at = 0;

    let flush_carrier = |chunks: &mut Vec<TapeChunk>, shorts: &mut usize| {
        if *shorts >= 2 {
            chunks.push(TapeChunk::Carrier {
                cycles: (*shorts / 2) as u32,
            });
        }
        *shorts = 0;
    };

    while at < pulses.len() {
        match pulses[at] {
            Pulse::Short => {
                shorts_in_a_row += 1;
                at += 1;
            }
            Pulse::Gap(steps) => {
                flush_carrier(&mut chunks, &mut shorts_in_a_row);
                chunks.push(TapeChunk::Gap { steps });
                at += 1;
            }
            Pulse::Long => {
                // A long pulse after the carrier begins a byte's start bit.
                match read_byte(&pulses[at..]) {
                    Some((byte, used)) => {
                        flush_carrier(&mut chunks, &mut shorts_in_a_row);
                        chunks.push(TapeChunk::Byte(byte));
                        at += used;
                    }
                    None => at += 1, // a stray long pulse: ignore it
                }
            }
        }
    }
    flush_carrier(&mut chunks, &mut shorts_in_a_row);
    chunks
}

/// Read one byte (start bit, 8 data bits, stop bit) from the start of
/// `pulses`. Returns it and how many pulses it used.
fn read_byte(pulses: &[Pulse]) -> Option<(u8, usize)> {
    let mut at = 0;
    // The start bit: a `0`.
    if take_bit(pulses, &mut at)? {
        return None;
    }
    let mut byte = 0u8;
    for bit in 0..8 {
        if take_bit(pulses, &mut at)? {
            byte |= 1 << bit;
        }
    }
    // The stop bit: a `1`.
    if !take_bit(pulses, &mut at)? {
        return None;
    }
    Some((byte, at))
}

/// Read one bit: a run of short pulses is a `1`, a run of long ones a `0`.
/// A few pulses fewer than the full run are allowed, for jitter.
fn take_bit(pulses: &[Pulse], at: &mut usize) -> Option<bool> {
    let first = *pulses.get(*at)?;
    let (kind, wanted, bit) = match first {
        Pulse::Short => (Pulse::Short, SHORT_PULSES_PER_BIT, true),
        Pulse::Long => (Pulse::Long, LONG_PULSES_PER_BIT, false),
        Pulse::Gap(_) => return None,
    };
    let mut taken = 0;
    while taken < wanted && pulses.get(*at + taken) == Some(&kind) {
        taken += 1;
    }
    if taken + SLACK < wanted {
        return None;
    }
    *at += taken;
    Some(bit)
}

// ---- From a tape to a CSW file -------------------------------------------------------

/// Write a tape as a version 1 CSW file at `sample_rate`. (The tone half
/// cycles are rounded to whole samples, so a rate that divides evenly, such
/// as 48000, is the most exact.)
pub fn encode(image: &TapeImage, sample_rate: u16) -> Vec<u8> {
    let samples = |microseconds: f64| {
        (microseconds * f64::from(sample_rate) / 1_000_000.0)
            .round()
            .max(1.0) as u32
    };
    let (short, long) = (samples(SHORT_PULSE_US), samples(LONG_PULSE_US));

    let mut pulses: Vec<u32> = Vec::new();
    let bit = |pulses: &mut Vec<u32>, value: bool| {
        let (length, count) = if value {
            (short, SHORT_PULSES_PER_BIT)
        } else {
            (long, LONG_PULSES_PER_BIT)
        };
        pulses.extend(std::iter::repeat_n(length, count));
    };
    for chunk in &image.chunks {
        match *chunk {
            TapeChunk::Carrier { cycles } => {
                pulses.extend(std::iter::repeat_n(short, 2 * cycles as usize));
            }
            TapeChunk::Byte(byte) => {
                bit(&mut pulses, false);
                for n in 0..8 {
                    bit(&mut pulses, byte >> n & 1 != 0);
                }
                bit(&mut pulses, true);
            }
            TapeChunk::Gap { steps } => {
                pulses.push(samples(f64::from(steps) * 1_000_000.0 / STEPS_PER_SECOND));
            }
        }
    }

    let mut file = SIGNATURE.to_vec();
    file.push(TERMINATOR);
    file.extend([1, 0]); // version 1.0
    file.extend(sample_rate.to_le_bytes());
    file.extend([1, 0, 0, 0, 0]); // RLE, starts high, three reserved bytes
    for pulse in pulses {
        match u8::try_from(pulse) {
            Ok(byte) if byte != 0 => file.push(byte),
            _ => {
                file.push(0);
                file.extend(pulse.to_le_bytes());
            }
        }
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const RATE: u16 = 48_000;

    fn bytes_of(image: &TapeImage) -> Vec<u8> {
        image
            .chunks
            .iter()
            .filter_map(|c| match c {
                TapeChunk::Byte(b) => Some(*b),
                _ => None,
            })
            .collect()
    }

    /// A tape like the Atom's: a leader, a block's bytes with small gaps
    /// between groups (as the real OS makes), and a silence.
    fn sample_tape() -> TapeImage {
        let mut chunks = vec![TapeChunk::Carrier { cycles: 2400 }];
        for group in [[0x2A, 0x2A, 0x2A, 0x2A], [0x54, 0x45, 0x53, 0x54]] {
            chunks.extend(group.map(TapeChunk::Byte));
            chunks.push(TapeChunk::Carrier { cycles: 4 });
        }
        chunks.extend([0x0D, 0xC5, 0x00, 0xFF].map(TapeChunk::Byte));
        chunks.push(TapeChunk::Gap { steps: 4800 });
        chunks.extend([0x00, 0x80].map(TapeChunk::Byte));
        chunks.push(TapeChunk::Carrier { cycles: 100 });
        TapeImage { chunks }
    }

    #[test]
    fn a_tape_survives_being_written_and_read_back() {
        let tape = sample_tape();
        let back = decode(&encode(&tape, RATE)).unwrap();
        assert_eq!(back.chunks, tape.chunks);
    }

    #[test]
    fn long_pulses_use_the_five_byte_form() {
        // A gap of a second is 48000 samples: too long for one byte.
        let tape = TapeImage {
            chunks: vec![TapeChunk::Gap { steps: 4800 }],
        };
        let file = encode(&tape, RATE);
        assert_eq!(
            &file[0x20..],
            [0, 0x80, 0xBB, 0, 0],
            "0 then 48000 little-endian"
        );
        assert_eq!(decode(&file).unwrap().chunks, tape.chunks);
    }

    #[test]
    fn a_version_2_file_with_zlib_compression_reads_the_same() {
        let tape = sample_tape();
        let v1 = encode(&tape, RATE);
        let rle = &v1[0x20..];

        let mut zipped = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        zipped.write_all(rle).unwrap();
        let zipped = zipped.finish().unwrap();

        let mut v2 = SIGNATURE.to_vec();
        v2.push(TERMINATOR);
        v2.extend([2, 0]); // version 2.0
        v2.extend(u32::from(RATE).to_le_bytes());
        v2.extend(0u32.to_le_bytes()); // pulse count (not needed)
        v2.extend([2, 0, 0]); // Z-RLE, flags, no header extension
        v2.extend([0; 16]); // application description
        assert_eq!(v2.len(), 0x34);
        v2.extend(zipped);
        assert_eq!(decode(&v2).unwrap().chunks, tape.chunks);
    }

    #[test]
    fn a_tape_running_fast_or_slow_still_reads() {
        // Real cassettes drift by 10-20%: stretch every pulse and read again.
        let tape = sample_tape();
        let file = encode(&tape, RATE);
        for factor in [0.85, 1.15] {
            let stretched: Vec<u8> = {
                let mut out = file[..0x20].to_vec();
                for pulse in run_lengths(&file[0x20..]) {
                    let scaled = (f64::from(pulse) * factor).round().max(1.0) as u32;
                    match u8::try_from(scaled) {
                        Ok(byte) if byte != 0 => out.push(byte),
                        _ => {
                            out.push(0);
                            out.extend(scaled.to_le_bytes());
                        }
                    }
                }
                out
            };
            assert_eq!(
                bytes_of(&decode(&stretched).unwrap()),
                bytes_of(&tape),
                "tape speed x{factor}"
            );
        }
    }

    #[test]
    fn a_stray_noise_pulse_in_the_carrier_does_not_upset_the_bytes() {
        let mut tape = sample_tape();
        // Noise: one long pulse in the middle of the leader.
        let mut file = encode(&tape, RATE);
        let long_pulse = 20u8;
        file.insert(0x20 + 1000, long_pulse);
        tape = decode(&file).unwrap();
        assert_eq!(bytes_of(&tape), bytes_of(&sample_tape()));
    }

    #[test]
    fn what_is_not_a_csw_file_is_refused() {
        assert!(decode(b"not a tape").is_err());
        assert!(!is_csw(b"UEF File!\0"));
        assert!(is_csw(&encode(&TapeImage::default(), RATE)));
        let mut unsupported = encode(&TapeImage::default(), RATE);
        unsupported[0x17] = 9;
        assert!(decode(&unsupported).unwrap_err().contains("version 9"));
        assert!(decode(&unsupported[..10]).is_err(), "a cut-off header");
    }

    #[test]
    fn an_empty_tape_is_an_empty_tape() {
        let file = encode(&TapeImage::default(), RATE);
        assert!(decode(&file).unwrap().chunks.is_empty());
    }
}
