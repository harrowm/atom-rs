//! The GDOS/ADOS 2015 disk system: the real ROM, talking to the emulated
//! WD1770 controller, on disk images. Needs `roms/gdos2015.rom` and the Atom's
//! own ROMs; the tests skip themselves without them.

mod common;

use std::path::Path;

use atom_rs::machine::Atom;
use atom_rs::machine::disk::DiskImage;
use atom_rs::machine::typist::Typist;
use atom_rs::machine::vdg6847::VideoMode;
use common::booted_atom;

const FILE: [u8; 12] = [0xA9, 0x41, 0x20, 0xF4, 0xFF, 0x60, 1, 2, 3, 4, 5, 6];

/// An Atom with the GDOS disk system and a disk holding one file, `HELLO`,
/// in drive 0 (typing `*DOS` starts the disk system).
fn gdos_atom() -> Option<Atom> {
    if !Path::new("roms/gdos2015.rom").exists() {
        common::skipped("no roms/gdos2015.rom");
        return None;
    }
    let mut atom = booted_atom(0)?;
    let mut disk = DiskImage::blank_80_track(1);
    disk.add_file("HELLO", 0x2900, 0x2900, &FILE).unwrap();
    atom.bus.fdc.drives[0] = Some(disk);
    atom.use_gdos(Path::new("roms")).expect("GDOS starts");
    for _ in 0..150 {
        atom.run_frame();
    }
    Some(atom)
}

fn run_command(atom: &mut Atom, text: &str) {
    let mut typist = Typist::new();
    typist.type_text(text, atom.bus.vdg.frames);
    while typist.is_busy() {
        typist.drive(atom);
        atom.run_frame();
    }
    for _ in 0..400 {
        atom.run_frame();
    }
}

#[test]
fn gdos_starts_and_lists_a_disk() {
    let Some(mut atom) = gdos_atom() else { return };
    run_command(&mut atom, "*DOS\n");
    let screen = atom.screen_text().join("\n");
    assert!(screen.contains("ADOS-1770"), "no GDOS sign-on:\n{screen}");
    run_command(&mut atom, "*.\n");
    let screen = atom.screen_text().join("\n");
    assert!(
        screen.contains("HELLO"),
        "the disk's file is not listed:\n{screen}"
    );
}

#[test]
fn gdos_loads_a_file_from_the_disk() {
    let Some(mut atom) = gdos_atom() else { return };
    run_command(&mut atom, "*DOS\n");
    run_command(&mut atom, "*LOAD HELLO\n");
    let loaded: Vec<u8> = (0..FILE.len())
        .map(|i| atom.bus.peek(0x2900 + i as u16))
        .collect();
    assert_eq!(loaded, FILE, "\n{}", atom.screen_text().join("\n"));
}

#[test]
fn gdos_saves_a_file_to_the_disk() {
    let Some(mut atom) = gdos_atom() else { return };
    let program: Vec<u8> = (0..20).map(|i| 0x50 + i).collect();
    for (i, &byte) in program.iter().enumerate() {
        atom.bus.poke(0x3000 + i as u16, byte);
    }
    run_command(&mut atom, "*DOS\n");
    run_command(&mut atom, "*SAVE WORLD 3000 3014\n");

    let disk = atom.bus.fdc.drives[0].as_ref().unwrap();
    let names: Vec<String> = disk.catalogue().iter().map(|f| f.name.clone()).collect();
    assert!(
        names.iter().any(|n| n == "WORLD") && names.iter().any(|n| n == "HELLO"),
        "catalogue is {names:?}\n{}",
        atom.screen_text().join("\n")
    );
    // And the data is on the disk where the catalogue says.
    let entry = disk
        .catalogue()
        .into_iter()
        .find(|f| f.name == "WORLD")
        .unwrap();
    let sector = disk
        .read_sector(
            (entry.start_sector / 10) as u8,
            0,
            (entry.start_sector % 10) as u8,
        )
        .unwrap();
    assert_eq!(&sector[..program.len()], &program[..]);
}

#[test]
fn gdos_runs_a_real_game_from_a_disk_image() {
    let disk = "testdata/games/chuckie.ssd";
    if !Path::new(disk).exists() || !Path::new("roms/gdos2015.rom").exists() {
        common::skipped(
            "needs testdata/games/chuckie.ssd (scripts/make_all_games.sh) and roms/gdos2015.rom",
        );
        return;
    }
    let Some(mut atom) = booted_atom(0) else {
        return;
    };
    atom.bus.fdc.drives[0] = Some(DiskImage::from_bytes(std::fs::read(disk).unwrap(), 1).unwrap());
    atom.use_gdos(Path::new("roms")).unwrap();
    for _ in 0..150 {
        atom.run_frame();
    }
    let mut typist = Typist::new();
    typist.type_text("*DOS\n*RUN RUNME\n", atom.bus.vdg.frames);
    let menu = VideoMode::Graphics {
        width: 256,
        height: 192,
        colours: 2,
    };
    let lit = |a: &Atom| a.bus.framebuffer.pixels.iter().filter(|&&p| p != 0).count();
    let mut shown = false;
    for _ in 0..4000 {
        typist.drive(&mut atom);
        atom.run_frame();
        if atom.video_mode() == menu && lit(&atom) > 5000 {
            shown = true;
            break;
        }
    }
    assert!(
        shown,
        "Chuckie Egg's menu never appeared:\n{}",
        atom.screen_text().join("\n")
    );
}

#[test]
fn going_back_to_the_standard_disk_system_removes_gdos() {
    let Some(mut atom) = gdos_atom() else { return };
    assert!(atom.gdos_active());
    atom.use_floppy_disks(Path::new("roms")).unwrap();
    assert!(!atom.gdos_active());
    run_command(&mut atom, "*DOS\n");
    let screen = atom.screen_text().join("\n");
    assert!(!screen.contains("ADOS-1770"), "still GDOS:\n{screen}");
}
