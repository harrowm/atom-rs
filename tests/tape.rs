//! Saves a program to the emulated cassette with `*SAVE`, wipes memory, and
//! loads it back with `*LOAD`. This exercises tape output, the recorder and
//! decoder, UEF files, tape playback and tape input, all with the real ROMs.

mod common;

use common::{booted_atom, screen_string, type_text};

const START: u16 = 0x2900;
const DATA: [u8; 12] = [
    0xA9, 0x42, 0x8D, 0x00, 0x30, 0x60, 1, 2, 3, 0xFF, 0x00, 0x55,
];

/// Run the Atom for this many seconds of emulated time.
fn run_seconds(atom: &mut atom_rs::machine::Atom, seconds: u32) {
    atom.run_cycles(seconds as u64 * 1_000_000);
}

#[test]
fn save_to_tape_then_load_it_back() {
    let Some(mut atom) = booted_atom(120) else {
        return;
    };
    for (i, &byte) in DATA.iter().enumerate() {
        assert!(atom.bus.poke(START + i as u16, byte));
    }

    // ---- Save ----
    atom.bus.tape.recording = true;
    type_text(&mut atom, "*SAVE \"TEST\" 2900 290C\n");
    // Like the real machine, the Atom asks you to start the recorder first.
    assert!(screen_string(&atom).contains("RECORD TAPE"));
    type_text(&mut atom, "\n");
    run_seconds(&mut atom, 25);
    let tape = atom.bus.tape.finish_recording();
    assert!(!tape.chunks.is_empty(), "nothing was written to the tape");

    // The recording can be written as a UEF file and read back.
    let tape = atom_rs::machine::tape::TapeImage::from_uef(&tape.to_uef()).unwrap();
    let bytes: Vec<u8> = tape
        .chunks
        .iter()
        .filter_map(|c| match c {
            atom_rs::machine::tape::TapeChunk::Byte(b) => Some(*b),
            _ => None,
        })
        .collect();
    eprintln!("{} bytes on tape", bytes.len());
    assert!(
        bytes.windows(DATA.len()).any(|w| w == DATA),
        "the saved data is not on the tape: {bytes:02X?}"
    );

    // ---- Wipe and load ----
    for i in 0..DATA.len() {
        atom.bus.poke(START + i as u16, 0);
    }
    atom.bus.tape.insert(tape);
    type_text(&mut atom, "*LOAD \"TEST\"\n");
    assert!(screen_string(&atom).contains("PLAY TAPE"));
    // Press play on the recorder, then a key on the Atom.
    atom.bus.tape.playing = true;
    type_text(&mut atom, "\n");
    run_seconds(&mut atom, 25);
    eprintln!("{}", atom.screen_text().join("\n"));

    let loaded: Vec<u8> = (0..DATA.len())
        .map(|i| atom.bus.peek(START + i as u16))
        .collect();
    assert_eq!(loaded, DATA, "program was not loaded back from tape");
}

/// What `--tape FILE --run '*LOAD "TEST"\n'` does: the typist types the
/// command and auto-play presses play and a key when the Atom asks.
#[test]
fn tape_loads_hands_free_with_typist_and_autoplay() {
    use atom_rs::machine::autoplay::TapeAutoplay;
    use atom_rs::machine::typist::Typist;

    // First make a tape by saving, as above.
    let Some(mut atom) = booted_atom(120) else {
        return;
    };
    for (i, &byte) in DATA.iter().enumerate() {
        atom.bus.poke(START + i as u16, byte);
    }
    atom.bus.tape.recording = true;
    type_text(&mut atom, "*SAVE \"TEST\" 2900 290C\n");
    type_text(&mut atom, "\n");
    run_seconds(&mut atom, 25);
    let tape = atom.bus.tape.finish_recording();

    // A brand new Atom with the tape inserted, and nobody touching the keys.
    let Some(mut atom) = booted_atom(0) else {
        return;
    };
    atom.bus.tape.insert(tape);
    let mut typist = Typist::new();
    let mut autoplay = TapeAutoplay::default();
    typist.type_text("*LOAD \"TEST\"\n", 100);
    for _ in 0..4000 {
        autoplay.update(&mut atom, &mut typist);
        typist.drive(&mut atom);
        atom.run_frame();
        if (0..DATA.len()).all(|i| atom.bus.peek(START + i as u16) == DATA[i]) {
            return; // loaded
        }
    }
    panic!(
        "the program never loaded:\n{}",
        atom.screen_text().join("\n")
    );
}

/// Record a real `*SAVE` on the Atom and return the tape.
fn recorded_save() -> Option<atom_rs::machine::tape::TapeImage> {
    let mut atom = booted_atom(120)?;
    for (i, &byte) in DATA.iter().enumerate() {
        assert!(atom.bus.poke(START + i as u16, byte));
    }
    atom.bus.tape.recording = true;
    type_text(&mut atom, "*SAVE \"TEST\" 2900 290C\n");
    type_text(&mut atom, "\n");
    run_seconds(&mut atom, 25);
    Some(atom.bus.tape.finish_recording())
}

/// The catalogue reads what the real OS wrote: the name, where it loads and
/// runs, and how long it is.
#[test]
fn the_catalogue_lists_a_real_save() {
    let Some(tape) = recorded_save() else { return };
    let catalogue = tape.catalogue();
    assert_eq!(catalogue.len(), 1, "{catalogue:?}");
    let file = &catalogue[0];
    assert_eq!(file.name, "TEST");
    assert_eq!(file.length, DATA.len());
    assert_eq!(file.load_address, START);
    assert_eq!(file.exec_address, START);
    assert!(file.complete);
}

/// A CSW tape (a recording of the actual signal) loads on the real Atom, so
/// the pulse decoder produces what the OS reads, not just something that
/// looks right on paper.
#[test]
fn a_csw_tape_loads_on_the_real_atom() {
    use atom_rs::machine::csw;
    use atom_rs::machine::tape::TapeImage;

    let Some(recorded) = recorded_save() else {
        return;
    };
    let csw_file = csw::encode(&recorded, 48_000);
    let tape = TapeImage::from_file(&csw_file).expect("the CSW file reads");
    assert_eq!(
        tape.catalogue(),
        recorded.catalogue(),
        "same files either way"
    );

    let Some(mut atom) = booted_atom(120) else {
        return;
    };
    atom.bus.tape.insert(tape);
    type_text(&mut atom, "*LOAD \"TEST\"\n");
    assert!(screen_string(&atom).contains("PLAY TAPE"));
    atom.bus.tape.playing = true;
    type_text(&mut atom, "\n");
    run_seconds(&mut atom, 25);
    let loaded: Vec<u8> = (0..DATA.len())
        .map(|i| atom.bus.peek(START + i as u16))
        .collect();
    assert_eq!(loaded, DATA, "{}", atom.screen_text().join("\n"));
}
