//! The AtoMMC SD card interface, with the real AtoMMC ROM and a folder on the
//! host standing in for the card. Needs `roms/ramrom.rom` (the 128K image that
//! Atomulator ships, which holds the AtoMMC ROM); the tests skip themselves
//! without it.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use atom_rs::machine::Atom;
use atom_rs::machine::atm::AtmFile;
use atom_rs::machine::typist::Typist;

const ROM_DIR: &str = "roms";

/// A scratch folder that is removed afterwards.
struct Card(PathBuf);

impl Card {
    fn new(label: &str) -> Card {
        let path = std::env::temp_dir().join(format!("atom-rs-it-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Card(path)
    }

    /// Put a program on the card in `.atm` format (22 byte header, then data).
    fn add_program(&self, name: &str, load: u16, exec: u16, data: &[u8]) {
        let mut file = name.as_bytes().to_vec();
        file.resize(16, 0);
        file.extend(load.to_le_bytes());
        file.extend(exec.to_le_bytes());
        file.extend((data.len() as u16).to_le_bytes());
        file.extend(data);
        fs::write(self.0.join(name), file).unwrap();
    }
}

impl Drop for Card {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// An Atom with an AtoMMC whose card is `folder`, or `None` without the ROMs.
fn atom_with_card(folder: &Path) -> Option<Atom> {
    if !Path::new(ROM_DIR).join("ramrom.rom").exists() {
        eprintln!("SKIPPED: no roms/ramrom.rom");
        return None;
    }
    let mut atom = Atom::new();
    atom.use_sd_card(Path::new(ROM_DIR), folder.to_path_buf())
        .expect("AtoMMC ROMs load");
    for _ in 0..120 {
        atom.run_frame();
    }
    Some(atom)
}

/// Type a command and let the Atom work on it for a few seconds.
fn run_command(atom: &mut Atom, text: &str) {
    let mut typist = Typist::new();
    typist.type_text(text, 0);
    while typist.is_busy() {
        typist.drive(atom);
        atom.run_frame();
    }
    for _ in 0..300 {
        atom.run_frame();
    }
}

#[test]
fn boots_with_the_atommc_signon_and_lists_the_card() {
    let card = Card::new("cat");
    card.add_program("HELLO", 0x2900, 0x2900, &[1, 2, 3]);
    fs::create_dir(card.0.join("GAMES")).unwrap();
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };

    let boot = atom.screen_text().join("\n");
    assert!(boot.contains("ATOMMC"), "no AtoMMC sign-on:\n{boot}");

    run_command(&mut atom, "*CAT\n");
    let screen = atom.screen_text().join("\n");
    eprintln!("{screen}");
    assert!(
        screen.contains("<GAMES>"),
        "directories are shown in angle brackets"
    );
    assert!(screen.contains("HELLO"));
}

#[test]
fn loads_a_program_from_the_card() {
    let card = Card::new("load");
    let data: Vec<u8> = (0..600u32).map(|i| (i * 3) as u8).collect(); // more than one block
    card.add_program("TEST", 0x2900, 0x2900, &data);
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };

    run_command(&mut atom, "*LOAD TEST\n");
    let loaded: Vec<u8> = (0..data.len())
        .map(|i| atom.bus.peek(0x2900 + i as u16))
        .collect();
    assert_eq!(loaded, data, "\n{}", atom.screen_text().join("\n"));
}

#[test]
fn saves_a_program_to_the_card() {
    let card = Card::new("save");
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };
    let program = [0xA9, 0x42, 0x60, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];
    for (i, &byte) in program.iter().enumerate() {
        assert!(atom.bus.poke(0x3000 + i as u16, byte));
    }

    run_command(&mut atom, "*SAVE SAVED 3000 3010\n");
    let saved = card.0.join("SAVED");
    assert!(
        saved.exists(),
        "no file on the card:\n{}",
        atom.screen_text().join("\n")
    );
    let file = AtmFile::parse(&fs::read(saved).unwrap()).expect("saved in .atm format");
    assert_eq!(file.load_address, 0x3000);
    assert_eq!(file.data, program);

    // And it loads back.
    for i in 0..program.len() {
        atom.bus.poke(0x3000 + i as u16, 0);
    }
    run_command(&mut atom, "*LOAD SAVED\n");
    let again: Vec<u8> = (0..program.len())
        .map(|i| atom.bus.peek(0x3000 + i as u16))
        .collect();
    assert_eq!(again, program);
}

#[test]
fn going_back_to_floppy_disks_restores_atom_dos() {
    let card = Card::new("back");
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };
    atom.use_floppy_disks(Path::new(ROM_DIR)).unwrap();
    for _ in 0..120 {
        atom.run_frame();
    }
    assert!(atom.bus.memory.disk_interface_fitted);
    assert!(atom.bus.atommc.card_folder().is_none());
    let screen = atom.screen_text().join("\n");
    assert!(
        screen.contains("ACORN ATOM") && !screen.contains("ATOMMC"),
        "{screen}"
    );
}
