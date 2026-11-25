//! Cassette tape: playback and recording.
//!
//! The Atom stores data on tape at 300 baud using the "Kansas City" style:
//!
//! * a `0` bit is 4 cycles of a 1200 Hz tone, a `1` bit is 8 cycles of 2400 Hz,
//!   so every bit lasts 3.33 ms;
//! * a byte is sent as a `0` start bit, 8 data bits (least significant first)
//!   and a `1` stop bit;
//! * blocks are preceded by a long leader of 2400 Hz.
//!
//! Tapes are loaded from UEF files (the standard emulator tape format), which
//! describe the tape as chunks of carrier tone, data bytes and silence. The
//! player turns those chunks into the signal on 8255 port C bit 5.
//!
//! Timing: the player is stepped once per *half cycle of 2400 Hz* (every 208
//! CPU cycles, the same rate the 8255's tone input toggles). So a leader of
//! `n` cycles takes `2n` steps, a bit takes 16 steps, and a second of tape is
//! 4800 steps.

use serde::{Deserialize, Serialize};

use std::io::Read;

/// Player steps per second of tape.
const STEPS_PER_SECOND: f64 = 4800.0;
const STEPS_PER_BIT: u8 = 16;
/// Recorded leader tones are cut down to this many cycles (5 seconds).
const MAX_RECORDED_LEADER_CYCLES: u32 = 2400 * 5;
const BITS_PER_BYTE_ON_TAPE: u8 = 10; // start + 8 data + stop

/// One piece of a tape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TapeChunk {
    /// A tone of 2400 Hz lasting this many cycles.
    Carrier { cycles: u32 },
    /// One data byte (start and stop bits are added when played).
    Byte(u8),
    /// Silence, measured in player steps.
    Gap { steps: u32 },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TapeImage {
    pub chunks: Vec<TapeChunk>,
}

impl TapeImage {
    /// Parse a UEF file (gzip-compressed or plain).
    pub fn from_uef(file: &[u8]) -> Result<Self, String> {
        let data = if file.starts_with(&[0x1F, 0x8B]) {
            let mut unzipped = Vec::new();
            flate2::read::GzDecoder::new(file)
                .read_to_end(&mut unzipped)
                .map_err(|e| format!("cannot decompress UEF: {e}"))?;
            unzipped
        } else {
            file.to_vec()
        };

        const MAGIC: &[u8] = b"UEF File!\0";
        const HEADER_LEN: usize = MAGIC.len() + 2; // plus minor and major version
        if !data.starts_with(MAGIC) || data.len() < HEADER_LEN {
            return Err("not a UEF file".to_string());
        }

        let mut chunks = Vec::new();
        let mut position = HEADER_LEN;
        while position + 6 <= data.len() {
            let id = u16::from_le_bytes([data[position], data[position + 1]]);
            let length = u32::from_le_bytes(data[position + 2..position + 6].try_into().unwrap());
            let start = position + 6;
            let end = start
                .checked_add(length as usize)
                .filter(|&end| end <= data.len())
                .ok_or("UEF chunk runs past the end of the file")?;
            parse_chunk(id, &data[start..end], &mut chunks);
            position = end;
        }
        Ok(TapeImage { chunks })
    }
}

/// Add the tape chunks described by one UEF chunk. Chunks we do not need
/// (text, 1200 baud, ...) are ignored.
fn parse_chunk(id: u16, body: &[u8], chunks: &mut Vec<TapeChunk>) {
    let word = |at: usize| {
        body.get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    match id {
        // Implicit start/stop bit data: the bytes of a normal block.
        0x0100 => chunks.extend(body.iter().map(|&b| TapeChunk::Byte(b))),
        // Carrier tone.
        0x0110 => {
            if let Some(cycles) = word(0) {
                chunks.push(TapeChunk::Carrier {
                    cycles: cycles as u32,
                });
            }
        }
        // Carrier tone with a dummy byte (0xAA) in the middle.
        0x0111 => {
            if let (Some(before), Some(after)) = (word(0), word(2)) {
                chunks.push(TapeChunk::Carrier {
                    cycles: before as u32,
                });
                chunks.push(TapeChunk::Byte(0xAA));
                chunks.push(TapeChunk::Carrier {
                    cycles: after as u32,
                });
            }
        }
        // Integer gap, in units of 1/(2 x baud) seconds; at 300 baud 1/600 s.
        0x0112 => {
            if let Some(units) = word(0) {
                let seconds = units as f64 / 600.0;
                chunks.push(TapeChunk::Gap {
                    steps: (seconds * STEPS_PER_SECOND) as u32,
                });
            }
        }
        // Floating point gap in seconds.
        0x0116 => {
            if let Some(bytes) = body.get(0..4) {
                let seconds = f32::from_le_bytes(bytes.try_into().unwrap()) as f64;
                chunks.push(TapeChunk::Gap {
                    steps: (seconds * STEPS_PER_SECOND) as u32,
                });
            }
        }
        _ => {}
    }
}

impl TapeImage {
    /// Write the tape as an (uncompressed) UEF file.
    pub fn to_uef(&self) -> Vec<u8> {
        let mut file = b"UEF File!\0".to_vec();
        file.extend([10, 0]); // version 0.10
        let mut add_chunk = |id: u16, body: &[u8]| {
            file.extend(id.to_le_bytes());
            file.extend((body.len() as u32).to_le_bytes());
            file.extend(body);
        };
        // Consecutive data bytes go together in one chunk.
        let mut data = Vec::new();
        for chunk in &self.chunks {
            if let TapeChunk::Byte(byte) = chunk {
                data.push(*byte);
                continue;
            }
            if !data.is_empty() {
                add_chunk(0x0100, &std::mem::take(&mut data));
            }
            match chunk {
                TapeChunk::Carrier { cycles } => {
                    add_chunk(0x0110, &(*cycles).min(0xFFFF).to_le_bytes()[..2])
                }
                TapeChunk::Gap { steps } => {
                    let seconds = *steps as f64 / STEPS_PER_SECOND;
                    add_chunk(0x0116, &(seconds as f32).to_le_bytes());
                }
                TapeChunk::Byte(_) => unreachable!("handled above"),
            }
        }
        if !data.is_empty() {
            add_chunk(0x0100, &data);
        }
        file
    }
}

/// Recover the tape contents from the cassette output level, sampled once per
/// step. This is the reverse of playback: a leader tone alternates every step;
/// each byte is 10 windows of 16 steps, where a 2400 Hz window (a 1 bit) flips
/// nearly every step and a 1200 Hz window (a 0 bit) about half as often.
fn decode_recording(levels: &[bool]) -> TapeImage {
    let window = STEPS_PER_BIT as usize;
    let flips = |from: usize, to: usize| {
        (from + 1..to)
            .filter(|&t| levels[t] != levels[t - 1])
            .count()
    };
    // A window with this many flips or more counts as a 1 bit.
    let is_one = |start: usize| flips(start, start + window) >= 12;
    let is_zero = |start: usize| flips(start, start + window) <= 10;

    // Try to read a framed byte (0 start bit, 8 data bits, 1 stop bit) at `start`.
    let byte_at = |start: usize| -> Option<u8> {
        if start + 10 * window > levels.len() || !is_zero(start) || !is_one(start + 9 * window) {
            return None;
        }
        let mut byte = 0u8;
        for bit in 0..8 {
            if is_one(start + (1 + bit) * window) {
                byte |= 1 << bit;
            }
        }
        Some(byte)
    };

    let mut chunks = Vec::new();
    let mut pos = 0;
    // Skip any silence before the first sound.
    while pos + 1 < levels.len() && levels[pos] == levels[pos + 1] {
        pos += 1;
    }
    while pos + 1 < levels.len() {
        if let Some(byte) = byte_at(pos) {
            chunks.push(TapeChunk::Byte(byte));
            pos += 10 * window;
            continue;
        }
        // Not a byte: measure the leader (alternating levels) that follows.
        let mut end = pos + 1;
        while end < levels.len() && levels[end] != levels[end - 1] {
            end += 1;
        }
        let leader_steps = end - 1 - pos;
        if leader_steps >= 2 {
            chunks.push(TapeChunk::Carrier {
                cycles: (leader_steps / 2) as u32,
            });
        }
        // Constant level for a while is silence.
        let mut flat_end = end;
        while flat_end < levels.len() && levels[flat_end] == levels[end - 1] {
            flat_end += 1;
        }
        if flat_end - (end - 1) > 4 {
            if flat_end < levels.len() {
                chunks.push(TapeChunk::Gap {
                    steps: (flat_end - (end - 1)) as u32,
                });
            }
            pos = flat_end;
        } else {
            // The leader ends one step before the first repeated level, which
            // is where the start bit begins.
            pos = if leader_steps >= 2 { end - 1 } else { pos + 1 };
            if byte_at(pos).is_none() && end > pos + 1 {
                pos = end; // cannot make sense of it; skip ahead
            }
        }
    }
    TapeImage { chunks }
}

/// What the player is in the middle of.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
enum Playing {
    Nothing,
    Carrier { steps_left: u32 },
    Byte { frame: u16, bit: u8, step: u8 },
    Gap { steps_left: u32 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tape {
    image: Option<TapeImage>,
    next_chunk: usize,
    now: Playing,
    /// Is the "play" button down?
    pub playing: bool,
    level: bool,
    /// Is the "record" button down?
    pub recording: bool,
    /// The cassette output level at every step while recording.
    recorded: Vec<bool>,
}

impl Tape {
    pub fn new() -> Self {
        Tape {
            image: None,
            next_chunk: 0,
            now: Playing::Nothing,
            playing: false,
            level: false,
            recording: false,
            recorded: Vec::new(),
        }
    }

    pub fn insert(&mut self, image: TapeImage) {
        self.image = Some(image);
        self.rewind();
    }

    pub fn eject(&mut self) {
        self.image = None;
        self.rewind();
    }

    pub fn rewind(&mut self) {
        self.next_chunk = 0;
        self.now = Playing::Nothing;
        self.playing = false;
        self.level = false;
    }

    /// The tape in the player, if any.
    pub fn image(&self) -> Option<&TapeImage> {
        self.image.as_ref()
    }

    pub fn has_tape(&self) -> bool {
        self.image.is_some()
    }

    /// How far through the tape we are, 0.0 to 1.0.
    pub fn progress(&self) -> f32 {
        match &self.image {
            Some(image) if !image.chunks.is_empty() => {
                self.next_chunk as f32 / image.chunks.len() as f32
            }
            _ => 0.0,
        }
    }

    /// Called every step while recording, with the level the Atom is writing
    /// to the cassette output.
    pub fn record(&mut self, output_level: bool) {
        if self.recording {
            self.recorded.push(output_level);
        }
    }

    /// Stop recording and turn what was recorded into a tape image.
    pub fn finish_recording(&mut self) -> TapeImage {
        self.recording = false;
        let mut image = decode_recording(&std::mem::take(&mut self.recorded));
        // The Atom outputs a leader tone whenever it is idle, so a recording
        // can start with a very long one; keep it to a realistic length.
        for chunk in &mut image.chunks {
            if let TapeChunk::Carrier { cycles } = chunk {
                *cycles = (*cycles).min(MAX_RECORDED_LEADER_CYCLES);
            }
        }
        image
    }

    /// How many seconds of tape have been recorded so far.
    pub fn recorded_seconds(&self) -> f64 {
        self.recorded.len() as f64 / STEPS_PER_SECOND
    }

    /// Advance half a cycle of 2400 Hz and return the new tape signal level.
    pub fn step(&mut self) -> bool {
        if !self.playing {
            return self.level;
        }
        if matches!(self.now, Playing::Nothing) && !self.start_next_chunk() {
            self.playing = false; // end of tape
            self.level = false;
            return self.level;
        }

        match &mut self.now {
            Playing::Nothing => {}
            Playing::Carrier { steps_left } => {
                self.level = !self.level;
                *steps_left -= 1;
                if *steps_left == 0 {
                    self.now = Playing::Nothing;
                }
            }
            Playing::Byte { frame, bit, step } => {
                let bit_value = (*frame >> *bit) & 1 == 1;
                // A 1 is a 2400 Hz tone (level flips every step); a 0 is
                // 1200 Hz (flips every two steps).
                self.level = if bit_value {
                    *step & 1 == 1
                } else {
                    ((*step + 1) >> 1) & 1 == 1
                };
                *step += 1;
                if *step == STEPS_PER_BIT {
                    *step = 0;
                    *bit += 1;
                    if *bit == BITS_PER_BYTE_ON_TAPE {
                        self.now = Playing::Nothing;
                    }
                }
            }
            Playing::Gap { steps_left } => {
                self.level = false;
                *steps_left -= 1;
                if *steps_left == 0 {
                    self.now = Playing::Nothing;
                }
            }
        }
        self.level
    }

    /// Begin the next chunk that actually lasts a while. Returns false at the
    /// end of the tape.
    fn start_next_chunk(&mut self) -> bool {
        let Some(image) = &self.image else {
            return false;
        };
        while let Some(&chunk) = image.chunks.get(self.next_chunk) {
            self.next_chunk += 1;
            self.now = match chunk {
                TapeChunk::Carrier { cycles } if cycles > 0 => Playing::Carrier {
                    steps_left: cycles * 2,
                },
                // Bits go out least significant first, after a 0 start bit,
                // before a 1 stop bit; build that 10-bit frame.
                TapeChunk::Byte(byte) => Playing::Byte {
                    frame: (byte as u16) << 1 | 0x200,
                    bit: 0,
                    step: 0,
                },
                TapeChunk::Gap { steps } if steps > 0 => Playing::Gap { steps_left: steps },
                _ => continue,
            };
            return true;
        }
        false
    }
}

impl Default for Tape {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn uef(chunks: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut file = b"UEF File!\0\x0A\x00".to_vec();
        for (id, body) in chunks {
            file.extend(id.to_le_bytes());
            file.extend((body.len() as u32).to_le_bytes());
            file.extend(body);
        }
        file
    }

    fn play(image: TapeImage, steps: usize) -> Vec<bool> {
        let mut player = Tape::new();
        player.insert(image);
        player.playing = true;
        (0..steps).map(|_| player.step()).collect()
    }

    #[test]
    fn parses_carrier_data_and_gap_chunks() {
        let file = uef(&[
            (0x0110, 100u16.to_le_bytes().to_vec()),
            (0x0100, vec![0x2A, 0x55]),
            (0x0112, 600u16.to_le_bytes().to_vec()), // 1 second
        ]);
        let image = TapeImage::from_uef(&file).unwrap();
        assert_eq!(
            image.chunks,
            vec![
                TapeChunk::Carrier { cycles: 100 },
                TapeChunk::Byte(0x2A),
                TapeChunk::Byte(0x55),
                TapeChunk::Gap { steps: 4800 },
            ]
        );
    }

    #[test]
    fn reads_gzip_compressed_uef() {
        let plain = uef(&[(0x0100, vec![1, 2, 3])]);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&plain).unwrap();
        let image = TapeImage::from_uef(&encoder.finish().unwrap()).unwrap();
        assert_eq!(image.chunks.len(), 3);
    }

    #[test]
    fn rejects_files_that_are_not_uef() {
        assert!(TapeImage::from_uef(b"hello").is_err());
    }

    #[test]
    fn carrier_alternates_every_step_for_two_steps_per_cycle() {
        let image = TapeImage {
            chunks: vec![TapeChunk::Carrier { cycles: 2 }],
        };
        let levels = play(image, 6);
        assert_eq!(levels, vec![true, false, true, false, false, false]);
        // After the four carrier steps the tape ends and the level drops.
    }

    #[test]
    fn a_zero_bit_is_1200hz_and_a_one_bit_is_2400hz() {
        // Byte 0x00: start bit 0, eight 0 data bits, then a 1 stop bit.
        let levels = play(
            TapeImage {
                chunks: vec![TapeChunk::Byte(0x00)],
            },
            160,
        );
        let transitions = |bit: usize| {
            let bit_levels = &levels[bit * 16..(bit + 1) * 16];
            bit_levels.windows(2).filter(|w| w[0] != w[1]).count()
        };
        assert_eq!(transitions(0), 8, "start bit: 4 cycles of 1200 Hz");
        assert_eq!(transitions(9), 15, "stop bit: 8 cycles of 2400 Hz");
    }

    #[test]
    fn data_bits_are_sent_least_significant_first() {
        // 0x01: start 0, then data bit 0 = 1, then seven 0s, then stop 1.
        let levels = play(
            TapeImage {
                chunks: vec![TapeChunk::Byte(0x01)],
            },
            160,
        );
        let busy = |bit: usize| {
            levels[bit * 16..(bit + 1) * 16]
                .windows(2)
                .filter(|w| w[0] != w[1])
                .count()
        };
        assert_eq!(busy(1), 15, "first data bit is the 1");
        assert_eq!(busy(2), 8, "second data bit is a 0");
    }

    /// Play a tape and record what comes out, as if the Atom wrote it back.
    fn play_and_record(image: TapeImage, extra_steps: usize) -> TapeImage {
        let mut tape = Tape::new();
        tape.insert(image);
        tape.playing = true;
        tape.recording = true;
        let total: usize = tape
            .image
            .as_ref()
            .unwrap()
            .chunks
            .iter()
            .map(|c| match c {
                TapeChunk::Carrier { cycles } => *cycles as usize * 2,
                TapeChunk::Byte(_) => 160,
                TapeChunk::Gap { steps } => *steps as usize,
            })
            .sum();
        for _ in 0..total + extra_steps {
            let level = tape.step();
            tape.record(level);
        }
        tape.finish_recording()
    }

    #[test]
    fn what_is_played_can_be_recorded_and_decoded_again() {
        let original = TapeImage {
            chunks: vec![
                TapeChunk::Carrier { cycles: 40 },
                TapeChunk::Byte(0x2A),
                TapeChunk::Byte(0x00),
                TapeChunk::Byte(0xFF),
                TapeChunk::Byte(0xA5),
                TapeChunk::Carrier { cycles: 10 },
                TapeChunk::Byte(0x01),
            ],
        };
        let decoded = play_and_record(original.clone(), 20);
        let bytes = |image: &TapeImage| -> Vec<u8> {
            image
                .chunks
                .iter()
                .filter_map(|c| match c {
                    TapeChunk::Byte(b) => Some(*b),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(bytes(&decoded), bytes(&original));
        assert!(
            decoded
                .chunks
                .iter()
                .any(|c| matches!(c, TapeChunk::Carrier { cycles } if *cycles >= 30))
        );
    }

    #[test]
    fn uef_written_by_to_uef_reads_back_the_same() {
        let image = TapeImage {
            chunks: vec![
                TapeChunk::Carrier { cycles: 300 },
                TapeChunk::Byte(1),
                TapeChunk::Byte(2),
                TapeChunk::Gap { steps: 4800 },
                TapeChunk::Byte(3),
            ],
        };
        let again = TapeImage::from_uef(&image.to_uef()).unwrap();
        assert_eq!(again.chunks, image.chunks);
    }

    #[test]
    fn stopped_player_holds_its_level_and_stays_put() {
        let mut player = Tape::new();
        player.insert(TapeImage {
            chunks: vec![TapeChunk::Carrier { cycles: 5 }],
        });
        assert!(!player.step());
        assert_eq!(player.progress(), 0.0);
        player.playing = true;
        player.step();
        assert_eq!(player.progress(), 1.0);
    }
}
