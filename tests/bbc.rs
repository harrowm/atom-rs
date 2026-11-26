//! BBC BASIC on the Atom, through the Ramoth RAM/ROM board's BBC mode. Needs
//! `roms/ramrom.rom`, which holds BBC BASIC and the BBC OS; the tests skip
//! themselves without it.

mod common;

use std::path::Path;

use atom_rs::machine::Atom;
use atom_rs::machine::typist::Typist;

const ROM_DIR: &str = "roms";

fn bbc_atom() -> Option<Atom> {
    if !Path::new(ROM_DIR).join("ramrom.rom").exists() {
        common::skipped("no roms/ramrom.rom");
        return None;
    }
    let mut atom = Atom::new();
    atom.set_bbc_basic(true, Path::new(ROM_DIR))
        .expect("BBC mode starts");
    for _ in 0..300 {
        atom.run_frame();
    }
    Some(atom)
}

fn type_line(atom: &mut Atom, text: &str) {
    let mut typist = Typist::new();
    typist.type_text(text, atom.bus.vdg.frames);
    while typist.is_busy() {
        typist.drive(atom);
        atom.run_frame();
    }
    for _ in 0..120 {
        atom.run_frame();
    }
}

#[test]
fn bbc_basic_starts_and_does_arithmetic() {
    let Some(mut atom) = bbc_atom() else { return };
    // BBC BASIC's `?` writes a byte: it can only get there if BASIC is running.
    type_line(&mut atom, "?&2000=3+4*2\n");
    assert_eq!(atom.bus.peek(0x2000), 11, "3+4*2");
    type_line(&mut atom, "?&2001=INT(PI*10)\n");
    assert_eq!(atom.bus.peek(0x2001), 31, "floating point works too");
}

#[test]
fn bbc_basic_runs_a_program() {
    let Some(mut atom) = bbc_atom() else { return };
    type_line(&mut atom, "10 FOR I%=1 TO 5\n");
    type_line(&mut atom, "20 ?(&2100+I%)=I%*I%\n");
    type_line(&mut atom, "30 NEXT\n");
    type_line(&mut atom, "RUN\n");
    let squares: Vec<u8> = (1..=5).map(|i| atom.bus.peek(0x2100 + i)).collect();
    assert_eq!(squares, [1, 4, 9, 16, 25]);
}

#[test]
fn the_bbc_rom_images_are_copied_into_ram_at_6000_and_7000() {
    let Some(atom) = bbc_atom() else { return };
    let board = atom.bus.memory.board.as_ref().unwrap();
    let (at_6000, at_7000) = board.bbc_ram_images();
    for (i, &byte) in at_6000.iter().enumerate().step_by(97) {
        assert_eq!(atom.bus.peek(0x6000 + i as u16), byte, "6000+{i:X}");
    }
    for (i, &byte) in at_7000.iter().enumerate().step_by(97) {
        assert_eq!(atom.bus.peek(0x7000 + i as u16), byte, "7000+{i:X}");
    }
}

#[test]
fn switching_back_gives_the_ordinary_atom() {
    let Some(mut atom) = bbc_atom() else { return };
    assert!(atom.bbc_basic_active());
    atom.set_bbc_basic(false, Path::new(ROM_DIR)).unwrap();
    assert!(!atom.bbc_basic_active());
    // The stock ROMs are loaded again by this (when they exist).
    if !Path::new(ROM_DIR).join("akernel.rom").exists() {
        return;
    }
    for _ in 0..200 {
        atom.run_frame();
    }
    let screen = atom.screen_text().join("\n");
    assert!(screen.contains("ACORN ATOM"), "no Atom banner:\n{screen}");
}

#[test]
fn bbc_mode_can_be_switched_on_in_sd_card_mode_too() {
    let Some(_) = bbc_atom() else { return };
    let folder = std::env::temp_dir().join(format!("atom-rs-bbc-sd-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let mut atom = Atom::new();
    atom.use_sd_card(Path::new(ROM_DIR), folder.clone())
        .unwrap();
    assert!(!atom.bbc_basic_active());
    atom.set_bbc_basic(true, Path::new(ROM_DIR)).unwrap();
    assert!(atom.bbc_basic_active());
    for _ in 0..300 {
        atom.run_frame();
    }
    type_line(&mut atom, "?&2000=6*7\n");
    assert_eq!(atom.bus.peek(0x2000), 42);
    // Off again: still SD card mode, with the AtoMMC's sign-on.
    atom.set_bbc_basic(false, Path::new(ROM_DIR)).unwrap();
    assert!(atom.bus.atommc.card_folder().is_some());
    for _ in 0..200 {
        atom.run_frame();
    }
    assert!(atom.screen_text().join("\n").contains("ATOMMC"));
    let _ = std::fs::remove_dir_all(&folder);
}
