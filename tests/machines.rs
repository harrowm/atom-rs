//! Switching between the four kinds of Atom (Hardware > Machine): from any
//! one to any other, each must come up as itself, with nothing left over from
//! the one before. Needs the Atom ROMs, `ramrom.rom` and `gdos2015.rom` in
//! `roms/`; the tests skip themselves without them.

mod common;

use std::path::Path;

use atom_rs::machine::Atom;
use atom_rs::machine::atom::Machine;
use atom_rs::machine::memory::RamPreset;
use atom_rs::machine::typist::Typist;
use common::booted_atom;

const ROM_DIR: &str = "roms";

fn roms_present() -> bool {
    ["akernel.rom", "ramrom.rom", "gdos2015.rom"]
        .iter()
        .all(|rom| Path::new(ROM_DIR).join(rom).exists())
}

fn type_line(atom: &mut Atom, text: &str) {
    let mut typist = Typist::new();
    typist.type_text(text, atom.bus.vdg.frames);
    while typist.is_busy() {
        typist.drive(atom);
        atom.run_frame();
    }
    for _ in 0..300 {
        atom.run_frame();
    }
}

fn settle(atom: &mut Atom) {
    for _ in 0..250 {
        atom.run_frame();
    }
}

/// Check the Atom really is `machine`, by what it does.
fn assert_is(atom: &mut Atom, machine: Machine, context: &str) {
    assert_eq!(atom.machine(), machine, "{context}");
    match machine {
        Machine::FloppyAtom => {
            let screen = atom.screen_text().join("\n");
            assert!(
                screen.contains("ACORN ATOM") && !screen.contains("ATOMMC"),
                "{context}: no plain Atom banner:\n{screen}"
            );
            assert_eq!(
                atom.bus.memory.ram_preset(),
                RamPreset::With22KExpansion,
                "{context}"
            );
            assert!(
                !atom.gdos_active() && atom.bus.memory.board.is_none(),
                "{context}"
            );
        }
        Machine::GdosAtom => {
            type_line(atom, "*DOS\n");
            let screen = atom.screen_text().join("\n");
            assert!(
                screen.contains("ADOS-1770"),
                "{context}: not GDOS:\n{screen}"
            );
        }
        Machine::SdCard => {
            let screen = atom.screen_text().join("\n");
            assert!(
                screen.contains("ATOMMC"),
                "{context}: no AtoMMC banner:\n{screen}"
            );
            assert!(atom.bus.atommc.card_folder().is_some(), "{context}");
        }
        Machine::BbcBasic => {
            type_line(atom, "?&2000=6*7\n");
            assert_eq!(
                atom.bus.peek(0x2000),
                42,
                "{context}: BBC BASIC is not running"
            );
        }
    }
}

#[test]
fn every_machine_can_be_switched_to_from_every_other() {
    if !roms_present() {
        common::skipped("needs roms/akernel.rom, ramrom.rom and gdos2015.rom");
        return;
    }
    let card = std::env::temp_dir().join(format!("atom-rs-machines-{}", std::process::id()));
    std::fs::create_dir_all(&card).unwrap();
    for from in Machine::ALL {
        for to in Machine::ALL {
            let mut atom = booted_atom(0).unwrap();
            let context = format!("{from:?} -> {to:?}");
            atom.set_machine(from, Path::new(ROM_DIR), Some(&card))
                .unwrap_or_else(|e| panic!("{context}: could not become {from:?}: {e}"));
            settle(&mut atom);
            atom.set_machine(to, Path::new(ROM_DIR), Some(&card))
                .unwrap_or_else(|e| panic!("{context}: {e}"));
            settle(&mut atom);
            assert_is(&mut atom, to, &context);
        }
    }
    let _ = std::fs::remove_dir_all(&card);
}

#[test]
fn the_sd_card_machine_needs_a_folder() {
    if !roms_present() {
        common::skipped("needs roms/akernel.rom, ramrom.rom and gdos2015.rom");
        return;
    }
    let mut atom = booted_atom(0).unwrap();
    let error = atom
        .set_machine(Machine::SdCard, Path::new(ROM_DIR), None)
        .unwrap_err();
    assert!(error.contains("folder"), "{error}");
    assert_eq!(atom.machine(), Machine::FloppyAtom, "nothing changed");
}

#[test]
fn only_the_floppy_machines_have_choosable_ram() {
    assert!(Machine::FloppyAtom.has_choosable_ram());
    assert!(Machine::GdosAtom.has_choosable_ram());
    assert!(!Machine::SdCard.has_choosable_ram());
    assert!(!Machine::BbcBasic.has_choosable_ram());
    // The ram choice really is refused where the menu says it is.
    if !roms_present() {
        return;
    }
    let mut atom = booted_atom(0).unwrap();
    atom.set_machine(Machine::BbcBasic, Path::new(ROM_DIR), None)
        .unwrap();
    assert!(atom.set_ram_preset(RamPreset::Minimal).is_err());
}

#[test]
fn every_machine_has_a_label_and_a_description() {
    let labels: std::collections::HashSet<_> = Machine::ALL.iter().map(|m| m.label()).collect();
    assert_eq!(labels.len(), Machine::ALL.len());
    assert!(Machine::ALL.iter().all(|m| !m.description().is_empty()));
}
