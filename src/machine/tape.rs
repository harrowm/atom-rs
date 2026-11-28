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

/// A file found on a tape (see [`TapeImage::catalogue`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapeFile {
    pub name: String,
    pub load_address: u16,
    pub exec_address: u16,
    /// Bytes of data, over all its blocks.
    pub length: usize,
    pub blocks: usize,
    /// Was the last block (the one not marked "more to follow") there?
    pub complete: bool,
}

/// Every block starts with these four bytes.
const BLOCK_MARKER: [u8; 4] = [b'*'; 4];
/// A file name ends with a carriage return; names are at most 13 characters.
const MAX_NAME_LENGTH: usize = 13;
/// After the name: flags, block number (2), length minus 1, execution address
/// (2) and this block's load address (2).
const BLOCK_HEADER_LENGTH: usize = 8;
mod flag {
    /// Set unless this is the last block of the file.
    pub const MORE_BLOCKS_FOLLOW: u8 = 0x80;
    /// Set if the block has any data.
    pub const HAS_DATA: u8 = 0x40;
}

impl TapeImage {
    /// Read a tape file in any supported format: UEF (plain or gzipped) or CSW.
    pub fn from_file(file: &[u8]) -> Result<Self, String> {
        if super::csw::is_csw(file) {
            super::csw::decode(file)
        } else {
            Self::from_uef(file)
        }
    }

    /// The files on the tape, as the Atom's `*CAT` would show them.
    ///
    /// A block is `****`, the name and a carriage return, an 8-byte header
    /// (flags; block number, high byte first; length minus one; the file's
    /// execution address and this block's load address, high byte first), the
    /// data and a checksum byte. The OS writes them like that, which this was
    /// checked against by recording a real `*SAVE`.
    pub fn catalogue(&self) -> Vec<TapeFile> {
        let bytes: Vec<u8> = self
            .chunks
            .iter()
            .filter_map(|chunk| match chunk {
                TapeChunk::Byte(byte) => Some(*byte),
                _ => None,
            })
            .collect();

        let mut files: Vec<TapeFile> = Vec::new();
        let mut at = 0;
        while at + BLOCK_MARKER.len() <= bytes.len() {
            if bytes[at..at + BLOCK_MARKER.len()] != BLOCK_MARKER {
                at += 1;
                continue;
            }
            let name_start = at + BLOCK_MARKER.len();
            let Some(name_length) = bytes[name_start..]
                .iter()
                .take(MAX_NAME_LENGTH + 1)
                .position(|&b| b == b'\r')
            else {
                at += 1; // "****" that is not a block (it may be in the data)
                continue;
            };
            let header_start = name_start + name_length + 1;
            let Some(header) = bytes.get(header_start..header_start + BLOCK_HEADER_LENGTH) else {
                break; // the tape ends in the middle of a header
            };
            let flags = header[0];
            let number = u16::from_be_bytes([header[1], header[2]]);
            let data_length = if flags & flag::HAS_DATA != 0 {
                usize::from(header[3]) + 1
            } else {
                0
            };
            let exec_address = u16::from_be_bytes([header[4], header[5]]);
            let load_address = u16::from_be_bytes([header[6], header[7]]);
            let name: String = bytes[name_start..name_start + name_length]
                .iter()
                .map(|&b| b as char)
                .collect();
            let block_end = header_start + BLOCK_HEADER_LENGTH + data_length + 1; // + checksum
            let truncated = block_end > bytes.len();
            let last = flags & flag::MORE_BLOCKS_FOLLOW == 0;

            if number == 0 || files.last().is_none_or(|file| file.name != name) {
                files.push(TapeFile {
                    name,
                    load_address,
                    exec_address,
                    length: 0,
                    blocks: 0,
                    complete: false,
                });
            }
            let file = files.last_mut().expect("a file was just added");
            file.length += data_length;
            file.blocks += 1;
            file.complete = last && !truncated;
            if truncated {
                break;
            }
            at = block_end;
        }
        files
    }

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

    // ---- The catalogue ------------------------------------------------------------

    /// The bytes of one tape block, laid out as the Atom's OS writes them.
    fn block(name: &str, number: u16, data: &[u8], load: u16, exec: u16, last: bool) -> Vec<u8> {
        let mut block = b"****".to_vec();
        block.extend(name.bytes());
        block.push(b'\r');
        let mut flags = flag::HAS_DATA;
        if !last {
            flags |= flag::MORE_BLOCKS_FOLLOW;
        }
        block.push(flags);
        block.extend(number.to_be_bytes());
        block.push((data.len() - 1) as u8);
        block.extend(exec.to_be_bytes());
        block.extend(load.to_be_bytes());
        block.extend(data);
        block.push(0x5A); // the checksum (not checked)
        block
    }

    fn tape_of(bytes: &[u8]) -> TapeImage {
        TapeImage {
            chunks: std::iter::once(TapeChunk::Carrier { cycles: 100 })
                .chain(bytes.iter().map(|&b| TapeChunk::Byte(b)))
                .collect(),
        }
    }

    #[test]
    fn lists_a_file_made_of_several_blocks() {
        let mut bytes = block("TEST", 0, &[1; 256], 0x2900, 0x2905, false);
        bytes.extend(block("TEST", 1, &[2; 256], 0x2A00, 0x2905, false));
        bytes.extend(block("TEST", 2, &[3; 88], 0x2B00, 0x2905, true));
        assert_eq!(
            tape_of(&bytes).catalogue(),
            vec![TapeFile {
                name: "TEST".into(),
                load_address: 0x2900,
                exec_address: 0x2905,
                length: 600,
                blocks: 3,
                complete: true,
            }]
        );
    }

    #[test]
    fn lists_several_files_in_the_order_on_the_tape() {
        let mut bytes = block("INSTRUCTIONS", 0, &[7; 100], 0x2900, 0xC2B2, true);
        bytes.extend(block("PINBALL", 0, &[8; 200], 0x2900, 0x2900, true));
        let catalogue = tape_of(&bytes).catalogue();
        let names: Vec<_> = catalogue.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["INSTRUCTIONS", "PINBALL"]);
        assert_eq!(
            (catalogue[0].length, catalogue[0].exec_address),
            (100, 0xC2B2)
        );
        assert_eq!(
            (catalogue[1].length, catalogue[1].load_address),
            (200, 0x2900)
        );
    }

    #[test]
    fn a_file_with_its_last_block_missing_is_marked_incomplete() {
        let bytes = block("PART", 0, &[1; 256], 0x2900, 0x2900, false);
        let catalogue = tape_of(&bytes).catalogue();
        assert_eq!(catalogue.len(), 1);
        assert!(!catalogue[0].complete);
    }

    #[test]
    fn a_tape_cut_off_in_a_block_still_lists_what_there_was() {
        let mut bytes = block("CUT", 0, &[1; 256], 0x2900, 0x2900, true);
        bytes.truncate(bytes.len() - 100);
        let catalogue = tape_of(&bytes).catalogue();
        assert_eq!(catalogue.len(), 1);
        assert!(!catalogue[0].complete);
    }

    #[test]
    fn stars_inside_data_are_not_mistaken_for_a_block() {
        let mut data = vec![0; 40];
        data[10..14].copy_from_slice(b"****");
        let bytes = block("STARS", 0, &data, 0x2900, 0x2900, true);
        assert_eq!(tape_of(&bytes).catalogue().len(), 1);
    }

    #[test]
    fn an_empty_or_unreadable_tape_has_no_files() {
        assert!(TapeImage::default().catalogue().is_empty());
        assert!(tape_of(b"this is not an atom tape").catalogue().is_empty());
    }

    #[test]
    fn from_file_recognises_both_formats() {
        let tape = tape_of(&block("BOTH", 0, &[9; 20], 0x2900, 0x2900, true));
        let from_uef = TapeImage::from_file(&tape.to_uef()).unwrap();
        let from_csw = TapeImage::from_file(&super::super::csw::encode(&tape, 48_000)).unwrap();
        assert_eq!(from_uef.catalogue(), tape.catalogue());
        assert_eq!(from_csw.catalogue(), tape.catalogue());
        assert!(TapeImage::from_file(b"neither").is_err());
    }

    // ---- UEF chunks, one by one, and damaged files -----------------------------------------------

    fn chunks_of(file: &[u8]) -> Vec<TapeChunk> {
        TapeImage::from_uef(file).unwrap().chunks
    }

    #[test]
    fn a_carrier_with_a_dummy_byte_is_the_carrier_the_byte_and_the_carrier() {
        let body = [100u16.to_le_bytes(), 40u16.to_le_bytes()].concat();
        assert_eq!(
            chunks_of(&uef(&[(0x0111, body)])),
            vec![
                TapeChunk::Carrier { cycles: 100 },
                TapeChunk::Byte(0xAA),
                TapeChunk::Carrier { cycles: 40 },
            ]
        );
    }

    #[test]
    fn gaps_are_converted_from_the_uefs_units_to_player_steps() {
        // Integer gap: units of 1/600 second; the player runs at 4800 steps a second.
        let integer = chunks_of(&uef(&[(0x0112, 600u16.to_le_bytes().to_vec())]));
        assert_eq!(
            integer,
            vec![TapeChunk::Gap { steps: 4800 }],
            "600 units is one second"
        );
        // Floating point gap in seconds.
        let float = chunks_of(&uef(&[(0x0116, 0.5f32.to_le_bytes().to_vec())]));
        assert_eq!(float, vec![TapeChunk::Gap { steps: 2400 }]);
    }

    #[test]
    fn chunks_with_bodies_too_short_for_what_they_hold_are_skipped_not_fatal() {
        for (id, body) in [
            (0x0110u16, vec![1u8]),
            (0x0111, vec![1, 0, 2]),
            (0x0112, vec![9]),
            (0x0116, vec![0, 0, 0]),
            (0x0110, vec![]),
        ] {
            assert!(chunks_of(&uef(&[(id, body)])).is_empty(), "chunk {id:04X}");
        }
    }

    #[test]
    fn chunks_of_kinds_the_atom_does_not_use_are_ignored() {
        let file = uef(&[
            (0x0000, b"an origin message".to_vec()),
            (0x0100, vec![1, 2]),
            (0x0104, vec![9; 5]),
            (0x0117, vec![3, 3]),
            (0x0100, vec![3]),
        ]);
        let bytes: Vec<u8> = chunks_of(&file)
            .into_iter()
            .filter_map(|c| {
                if let TapeChunk::Byte(b) = c {
                    Some(b)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(bytes, [1, 2, 3], "only the data chunks count, in order");
    }

    #[test]
    fn a_chunk_that_claims_more_bytes_than_the_file_has_is_an_error() {
        let mut file = uef(&[(0x0100, vec![1, 2, 3])]);
        file.truncate(file.len() - 2);
        assert!(
            TapeImage::from_uef(&file)
                .unwrap_err()
                .contains("past the end")
        );
        // A length that would overflow a 32-bit address on a small machine.
        let mut huge = b"UEF File!\0\x0A\x00".to_vec();
        huge.extend(0x0100u16.to_le_bytes());
        huge.extend(u32::MAX.to_le_bytes());
        assert!(TapeImage::from_uef(&huge).is_err());
    }

    #[test]
    fn bytes_left_over_after_the_last_chunk_header_are_ignored() {
        let mut file = uef(&[(0x0100, vec![7])]);
        file.extend([0x00, 0x01, 0x05]); // the start of another chunk header, cut off
        assert_eq!(chunks_of(&file), vec![TapeChunk::Byte(7)]);
    }

    #[test]
    fn a_header_alone_is_an_empty_tape_and_a_short_or_wrong_one_is_refused() {
        assert!(chunks_of(b"UEF File!\0\x0A\x00").is_empty());
        assert!(
            TapeImage::from_uef(b"UEF File!\0").is_err(),
            "no version bytes"
        );
        assert!(TapeImage::from_uef(b"").is_err());
        assert!(TapeImage::from_uef(b"UEF File?\0\x0A\x00").is_err());
    }

    #[test]
    fn a_damaged_gzip_file_is_refused_with_a_message_about_decompressing() {
        let good = {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(&uef(&[(0x0100, vec![1, 2, 3])])).unwrap();
            encoder.finish().unwrap()
        };
        assert_eq!(
            chunks_of(&good),
            vec![TapeChunk::Byte(1), TapeChunk::Byte(2), TapeChunk::Byte(3)]
        );
        let mut cut = good.clone();
        cut.truncate(cut.len() / 2);
        assert!(
            TapeImage::from_uef(&cut)
                .unwrap_err()
                .contains("cannot decompress")
        );
        assert!(TapeImage::from_uef(&[0x1F, 0x8B, 0, 0, 0]).is_err());
    }

    #[test]
    fn a_long_carrier_is_cut_to_what_a_uef_chunk_can_hold_when_written() {
        let tape = TapeImage {
            chunks: vec![TapeChunk::Carrier { cycles: 100_000 }],
        };
        let back = TapeImage::from_uef(&tape.to_uef()).unwrap();
        assert_eq!(back.chunks, vec![TapeChunk::Carrier { cycles: 0xFFFF }]);
    }

    #[test]
    fn data_bytes_split_by_other_chunks_are_written_in_separate_runs_and_read_back_in_order() {
        let tape = TapeImage {
            chunks: vec![
                TapeChunk::Byte(1),
                TapeChunk::Byte(2),
                TapeChunk::Gap { steps: 4800 },
                TapeChunk::Byte(3),
                TapeChunk::Carrier { cycles: 50 },
            ],
        };
        assert_eq!(
            TapeImage::from_uef(&tape.to_uef()).unwrap().chunks,
            tape.chunks
        );
    }

    // ---- The player's state ------------------------------------------------------------------------

    fn two_bytes() -> TapeImage {
        TapeImage {
            chunks: vec![TapeChunk::Byte(0x41), TapeChunk::Byte(0x42)],
        }
    }

    #[test]
    fn insert_eject_and_rewind_manage_the_tape_and_the_play_button() {
        let mut player = Tape::new();
        assert!(!player.has_tape());
        assert!(player.image().is_none());
        assert_eq!(player.progress(), 0.0, "no tape: no progress");
        player.insert(two_bytes());
        assert!(player.has_tape());
        assert_eq!(player.image().unwrap().chunks.len(), 2);
        player.playing = true;
        for _ in 0..20 {
            player.step();
        }
        assert_eq!(player.progress(), 0.5, "the first byte has been started");
        player.rewind();
        assert_eq!(player.progress(), 0.0);
        assert!(!player.playing, "rewinding lets the button up");
        player.eject();
        assert!(!player.has_tape());
        assert_eq!(player.progress(), 0.0);
    }

    #[test]
    fn a_tape_of_nothing_has_no_progress_and_the_end_of_the_tape_lifts_the_play_button() {
        let mut player = Tape::new();
        player.insert(TapeImage::default());
        assert_eq!(player.progress(), 0.0);
        player.playing = true;
        assert!(!player.step(), "the level is low at the end");
        assert!(!player.playing, "the recorder stops itself");
        let mut player = Tape::new();
        player.insert(two_bytes());
        player.playing = true;
        for _ in 0..(2 * 10 * 16 + 5) {
            player.step();
        }
        assert!(!player.playing, "end of the tape");
        assert_eq!(player.progress(), 1.0);
    }

    #[test]
    fn zero_length_carriers_and_gaps_are_skipped_straight_over() {
        let mut player = Tape::new();
        player.insert(TapeImage {
            chunks: vec![
                TapeChunk::Carrier { cycles: 0 },
                TapeChunk::Gap { steps: 0 },
                TapeChunk::Carrier { cycles: 1 },
            ],
        });
        player.playing = true;
        let levels: Vec<bool> = (0..2).map(|_| player.step()).collect();
        assert_eq!(levels, [true, false], "one cycle of carrier: two steps");
    }

    #[test]
    fn recording_only_keeps_samples_while_the_record_button_is_down_and_counts_the_time() {
        let mut player = Tape::new();
        player.record(true);
        assert_eq!(
            player.recorded_seconds(),
            0.0,
            "not recording: nothing kept"
        );
        player.recording = true;
        for n in 0..4800 {
            player.record(n % 2 == 0);
        }
        assert!(
            (player.recorded_seconds() - 1.0).abs() < 1e-9,
            "4800 steps is a second"
        );
        let image = player.finish_recording();
        assert!(!player.recording, "finishing lets the button up");
        assert_eq!(player.recorded_seconds(), 0.0, "and forgets the samples");
        // A long square wave is a leader tone: cut to a realistic length.
        assert!(matches!(
            image.chunks.first(),
            Some(TapeChunk::Carrier { .. })
        ));
    }

    #[test]
    fn decoding_a_recording_of_silence_or_nonsense_never_panics() {
        for levels in [
            vec![],
            vec![true],
            vec![false; 1000],
            vec![true, false],
            (0..1000).map(|n| n % 7 < 3).collect::<Vec<_>>(),
            (0..1000).map(|n| n % 2 == 0).collect::<Vec<_>>(),
            (0..777).map(|n| (n * n) % 5 < 2).collect::<Vec<_>>(),
        ] {
            let _ = decode_recording(&levels);
        }
    }
}
