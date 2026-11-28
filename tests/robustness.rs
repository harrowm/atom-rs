//! No file or line of text a user can hand the emulator may make it panic.
//!
//! Each parser is fed thousands of damaged versions of a good input (bytes
//! flipped, cut short, padded, spliced) and of pure noise. The generator is
//! seeded, so a failure repeats; the tests only need to get through without a
//! panic, and anything returned is then used the way the program would use it.

use atom_rs::machine::Atom;
use atom_rs::machine::atm::AtmFile;
use atom_rs::machine::condition::Condition;
use atom_rs::machine::debug_console::{Console, parse_hex};
use atom_rs::machine::debug_session::DebugSession;
use atom_rs::machine::disk::DiskImage;
use atom_rs::machine::sddos::names_in_config;
use atom_rs::machine::tape::{TapeChunk, TapeImage};
use atom_rs::machine::typist::Typist;

/// A small, fast, seeded random number generator (xorshift64*).
struct Random(u64);

impl Random {
    fn new(seed: u64) -> Random {
        Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, limit: usize) -> usize {
        if limit == 0 {
            0
        } else {
            (self.next() % limit as u64) as usize
        }
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 24) as u8
    }
}

/// Damage `good` in a random way.
fn damaged(good: &[u8], random: &mut Random) -> Vec<u8> {
    let mut data = good.to_vec();
    for _ in 0..1 + random.below(4) {
        match random.below(7) {
            0 => {
                // Flip some bytes.
                for _ in 0..1 + random.below(8) {
                    if !data.is_empty() {
                        let at = random.below(data.len());
                        data[at] = random.byte();
                    }
                }
            }
            1 => data.truncate(random.below(data.len() + 1)),
            2 => {
                let at = random.below(data.len() + 1);
                for _ in 0..random.below(40) {
                    data.insert(at, random.byte());
                }
            }
            3 => {
                if !data.is_empty() {
                    let from = random.below(data.len());
                    let to = (from + random.below(64)).min(data.len());
                    data.drain(from..to);
                }
            }
            4 => {
                // Make a byte extreme (often a length field).
                if !data.is_empty() {
                    let at = random.below(data.len());
                    data[at] = [0x00, 0x01, 0x7F, 0x80, 0xFF][random.below(5)];
                }
            }
            5 => {
                // Repeat a chunk of the file.
                if !data.is_empty() {
                    let from = random.below(data.len());
                    let to = (from + random.below(64)).min(data.len());
                    let piece = data[from..to].to_vec();
                    let at = random.below(data.len() + 1);
                    for (offset, byte) in piece.into_iter().enumerate() {
                        data.insert((at + offset).min(data.len()), byte);
                    }
                }
            }
            _ => {
                // Replace the start with noise.
                for byte in data.iter_mut().take(random.below(32)) {
                    *byte = random.byte();
                }
            }
        }
    }
    data
}

fn noise(random: &mut Random, longest: usize) -> Vec<u8> {
    (0..random.below(longest + 1))
        .map(|_| random.byte())
        .collect()
}

/// How many times longer to fuzz than usual: `FUZZ_SCALE=100 cargo test --test robustness`.
fn scale() -> usize {
    std::env::var("FUZZ_SCALE")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(1)
}

/// Run `check` on many damaged copies of each seed file and on noise.
fn fuzz(seeds: &[Vec<u8>], rounds: usize, check: impl Fn(&[u8])) {
    let rounds = rounds * scale();
    let mut random = Random::new(0xA70_0001);
    for seed in seeds {
        check(seed);
        for _ in 0..rounds {
            check(&damaged(seed, &mut random));
        }
    }
    for _ in 0..rounds {
        check(&noise(&mut random, 600));
    }
}

// ---- Seed files ----------------------------------------------------------------------------

fn sample_tape() -> TapeImage {
    // A block the way the Atom writes it: ****NAME<CR>, 8 byte header, data, checksum.
    let mut bytes = b"****HELLO\r".to_vec();
    bytes.extend([0x40, 0x00, 0x00, 0x03, 0x29, 0x00, 0x29, 0x00]);
    bytes.extend([1, 2, 3, 4, 0x55]);
    let mut chunks = vec![TapeChunk::Carrier { cycles: 100 }];
    chunks.extend(bytes.into_iter().map(TapeChunk::Byte));
    chunks.push(TapeChunk::Gap { steps: 1200 });
    TapeImage { chunks }
}

fn uef_seed() -> Vec<u8> {
    sample_tape().to_uef()
}

fn gzipped_uef_seed() -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&uef_seed()).unwrap();
    encoder.finish().unwrap()
}

fn csw_v1_seed() -> Vec<u8> {
    let mut file = b"Compressed Square Wave\x1A".to_vec();
    file.extend([1, 1]); // version 1.1
    file.extend(44_100u16.to_le_bytes());
    file.extend([1, 0, 0]); // compression 1 (RLE), flags, header extension
    file.resize(0x20, 0);
    file.extend(std::iter::repeat_n(9u8, 400));
    for _ in 0..200 {
        file.extend([18, 18, 9, 9, 9, 9]);
    }
    file.extend([0, 0x10, 0x27, 0, 0]);
    file
}

fn csw_v2_seed() -> Vec<u8> {
    let mut file = b"Compressed Square Wave\x1A".to_vec();
    file.extend([2, 0]);
    file.extend(44_100u32.to_le_bytes());
    file.extend(1000u32.to_le_bytes()); // number of pulses
    file.extend([1, 0, 0, 0]); // compression, flags, header extension, ...
    file.resize(0x34, 0);
    file.extend(std::iter::repeat_n(9u8, 600));
    file
}

fn atm_seed() -> Vec<u8> {
    let mut file = b"HELLO".to_vec();
    file.resize(16, 0);
    file.extend(0x2900u16.to_le_bytes());
    file.extend(0x2900u16.to_le_bytes());
    file.extend(8u16.to_le_bytes());
    file.extend([1, 2, 3, 4, 5, 6, 7, 8]);
    file
}

/// The fuzzing is only worth anything if the starting files are good ones.
#[test]
fn the_seed_files_are_valid_so_the_damage_reaches_deep_into_the_parsers() {
    assert!(TapeImage::from_uef(&uef_seed()).is_ok());
    assert!(TapeImage::from_uef(&gzipped_uef_seed()).is_ok());
    assert!(atom_rs::machine::csw::decode(&csw_v1_seed()).is_ok());
    assert!(atom_rs::machine::csw::decode(&csw_v2_seed()).is_ok());
    assert!(AtmFile::parse(&atm_seed()).is_ok());
    assert_eq!(
        sample_tape().catalogue().len(),
        1,
        "the tape holds one file"
    );
    for sides in [1, 2] {
        let disk = DiskImage::from_bytes(disk_seed(sides), sides).unwrap();
        assert!(
            !disk.catalogue().is_empty(),
            "{sides} sided disk lists its file"
        );
    }
    assert!(
        Atom::new()
            .load_state(&Atom::new().save_state().unwrap())
            .is_ok()
    );
}

// ---- Tape files --------------------------------------------------------------------------------

fn use_tape(tape: &TapeImage) {
    // What the program does with a tape it has just read.
    let _ = tape.catalogue();
    let _ = tape.to_uef();
    let mut player = atom_rs::machine::tape::Tape::new();
    player.insert(tape.clone());
    player.playing = true;
    for _ in 0..3000 {
        player.step();
    }
}

#[test]
fn damaged_uef_files_never_panic() {
    fuzz(&[uef_seed(), gzipped_uef_seed()], 2500, |data| {
        if let Ok(tape) = TapeImage::from_uef(data) {
            use_tape(&tape);
        }
    });
}

#[test]
fn damaged_csw_files_never_panic() {
    fuzz(&[csw_v1_seed(), csw_v2_seed()], 2500, |data| {
        if let Ok(tape) = atom_rs::machine::csw::decode(data) {
            use_tape(&tape);
        }
    });
}

#[test]
fn damaged_tape_files_of_any_format_never_panic_when_opened_by_content() {
    fuzz(&[uef_seed(), csw_v1_seed(), csw_v2_seed()], 1500, |data| {
        if let Ok(tape) = TapeImage::from_file(data) {
            use_tape(&tape);
        }
    });
}

#[test]
fn the_tape_catalogue_survives_any_bytes_on_the_tape() {
    let mut random = Random::new(7);
    let blocks = sample_tape();
    for _ in 0..3000 {
        let mut chunks = blocks.chunks.clone();
        // Damage the data bytes themselves (not the file container).
        for _ in 0..1 + random.below(6) {
            let at = random.below(chunks.len());
            chunks[at] = match random.below(3) {
                0 => TapeChunk::Byte(random.byte()),
                1 => TapeChunk::Carrier {
                    cycles: random.next() as u32,
                },
                _ => TapeChunk::Gap {
                    steps: random.next() as u32,
                },
            };
        }
        chunks.truncate(random.below(chunks.len() + 1));
        let _ = TapeImage { chunks }.catalogue();
    }
    for _ in 0..500 {
        let chunks = noise(&mut random, 400)
            .into_iter()
            .map(TapeChunk::Byte)
            .collect();
        let _ = TapeImage { chunks }.catalogue();
    }
}

// ---- Disks, programs, saved states ---------------------------------------------------------------

fn disk_seed(sides: u8) -> Vec<u8> {
    let mut disk = DiskImage::blank_80_track(sides);
    // A catalogue the way Acorn DFS writes it: names in sector 0, details in sector 1.
    let mut names = vec![0u8; 256];
    names[..8].copy_from_slice(b"TESTDISK");
    names[8..15].copy_from_slice(b"HELLO  ");
    names[15] = b'$';
    disk.write_sector(0, 0, 0, &names);
    let mut details = vec![0u8; 256];
    details[5] = 8; // eight bytes: one file
    details[8..16].copy_from_slice(&[0, 0x29, 0, 0x29, 100, 0, 0, 2]);
    disk.write_sector(0, 0, 1, &details);
    disk.to_bytes().to_vec()
}

#[test]
fn damaged_disk_images_never_panic() {
    for sides in [1u8, 2] {
        fuzz(&[disk_seed(sides)], 800, |data| {
            if let Ok(disk) = DiskImage::from_bytes(data.to_vec(), sides) {
                let _ = disk.catalogue();
                let _ = disk.read_sector(0, 0, 0);
                let _ = disk.read_sector(79, 1, 9);
                let _ = disk.to_bytes();
            }
        });
    }
}

#[test]
fn damaged_atm_files_never_panic() {
    fuzz(&[atm_seed()], 4000, |data| {
        let _ = AtmFile::parse(data);
    });
}

#[test]
fn damaged_saved_states_never_panic_and_never_disturb_the_machine() {
    let state = Atom::new().save_state().unwrap();
    let mut atom = Atom::new();
    atom.cpu.regs.a = 0x5A;
    let mut random = Random::new(99);
    for round in 0..300 {
        let bytes = if round % 3 == 0 {
            noise(&mut random, 200)
        } else {
            damaged(&state, &mut random)
        };
        if atom.load_state(&bytes).is_err() {
            assert_eq!(
                atom.cpu.regs.a, 0x5A,
                "a refused state leaves the machine alone"
            );
        }
        atom.cpu.regs.a = 0x5A;
    }
}

#[test]
fn damaged_boot_configs_for_the_sd_card_never_panic() {
    fuzz(
        &[vec![b'A'; 64], vec![0; 64], vec![0xFF; 64]],
        3000,
        |data| {
            let _ = names_in_config(data);
        },
    );
}

// ---- Text people type -------------------------------------------------------------------------------

const WORDS: &[&str] = &[
    "a",
    "x",
    "y",
    "s",
    "p",
    "pc",
    "sp",
    "f",
    "[",
    "]",
    "[0200]",
    "[ffff]",
    "==",
    "!=",
    "<",
    ">",
    "<=",
    ">=",
    "&&",
    "||",
    "&",
    "(",
    ")",
    "5",
    "$ff",
    "0x10",
    "ffff",
    "12345678901234567890",
    "-1",
    "",
    " ",
    "z",
    "!",
    "~",
    "+",
    "*",
    "pc==2900",
    "a==5",
    "[0200]>2",
    "é",
    "\u{0}",
    "99999",
];

fn random_text(random: &mut Random, words: &[&str], longest: usize) -> String {
    let mut text = String::new();
    for _ in 0..random.below(longest + 1) {
        match random.below(10) {
            0 => text.push(char::from_u32(random.below(0x300) as u32).unwrap_or('?')),
            _ => {
                text.push_str(words[random.below(words.len())]);
                if random.below(3) > 0 {
                    text.push(' ');
                }
            }
        }
    }
    text
}

#[test]
fn any_breakpoint_condition_text_is_parsed_or_refused_without_panicking() {
    let mut random = Random::new(1234);
    let regs = atom_rs::cpu::Registers::default();
    for _ in 0..20_000 {
        let text = random_text(&mut random, WORDS, 12);
        if let Ok(condition) = Condition::parse(&text) {
            let _ = condition.holds(&regs, |address| address as u8);
            // And what it prints parses back to the same condition.
            let shown = condition.to_string();
            let again = Condition::parse(&shown).unwrap_or_else(|e| panic!("{shown:?}: {e}"));
            assert_eq!(again.to_string(), shown, "from {text:?}");
        }
    }
}

#[test]
fn any_hex_text_is_read_or_refused_without_panicking() {
    let mut random = Random::new(55);
    for _ in 0..20_000 {
        let text = random_text(&mut random, WORDS, 3);
        let _ = parse_hex(&text);
    }
}

const COMMANDS: &[&str] = &[
    "help", "r", "set", "writem", "w", "m", "d", "s", "over", "out", "trace", "history", "c", "f",
    "p", "g", "break", "bclear", "breakr", "breakw", "bclearr", "bclearw", "watchr", "watchw",
    "wclearr", "wclearw", "blist", "wlist", "nonsense", "", "?",
];

#[test]
fn any_line_typed_into_the_command_box_is_answered_without_panicking() {
    let mut random = Random::new(2026);
    let mut atom = Atom::new();
    let mut session = DebugSession::default();
    let mut console = Console::default();
    for _ in 0..4000 {
        let mut line = COMMANDS[random.below(COMMANDS.len())].to_string();
        line.push(' ');
        line.push_str(&random_text(&mut random, WORDS, 5));
        // Keep the run-away commands short: running is covered elsewhere.
        if matches!(
            line.split_whitespace().next(),
            Some("c" | "g" | "f" | "over" | "out")
        ) {
            line = format!("p {line}");
        }
        let _ = console.execute(&line, &mut atom, &mut session);
        session.running = false;
    }
}

#[test]
fn any_text_given_to_the_typist_is_accepted() {
    let mut random = Random::new(8);
    let mut typist = Typist::new();
    for _ in 0..500 {
        let text = random_text(&mut random, WORDS, 20);
        typist.type_text(&text, 0);
    }
    let mut atom = Atom::new();
    for frame in 0..2000 {
        let ready = frame % 7 == 0;
        typist.update(&mut atom.bus.ppi.keyboard, frame, ready);
    }
}
