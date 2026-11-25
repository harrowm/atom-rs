//! Boots the Atom with its disk ROM and a disk image, and uses the real DOS
//! to list, load and save files. This exercises the memory map, the 8271
//! controller, its NMI-driven data transfers and the disk image layout together.

mod common;

use atom_rs::machine::Atom;
use atom_rs::machine::disk::DiskImage;
use common::{booted_atom, type_text};

const FILE_DATA: [u8; 12] = [
    0xA9, 0x42, 0x8D, 0x00, 0x30, 0x60, 1, 2, 3, 0xFF, 0x00, 0x55,
];
const LOAD_ADDRESS: u16 = 0x2900;

/// A blank disk with one file, `TEST`, stored at sector 2.
fn disk_with_one_file() -> DiskImage {
    let mut disk = DiskImage::blank_80_track(1);

    let mut names = disk.read_sector(0, 0, 0).unwrap().to_vec();
    // 7-character name, then the directory character (a space is Atom DOS's default).
    names[8..16].copy_from_slice(b"TEST    ");

    let mut info = disk.read_sector(0, 0, 1).unwrap().to_vec();
    info[5] = 8; // one catalogue entry (8 bytes each)
    let start_sector: u16 = 2;
    let length = FILE_DATA.len() as u16;
    info[8..16].copy_from_slice(&[
        LOAD_ADDRESS as u8,
        (LOAD_ADDRESS >> 8) as u8,
        LOAD_ADDRESS as u8, // execution address
        (LOAD_ADDRESS >> 8) as u8,
        length as u8,
        (length >> 8) as u8,
        (start_sector >> 8) as u8 & 3,
        start_sector as u8,
    ]);

    disk.write_sector(0, 0, 0, &names);
    disk.write_sector(0, 0, 1, &info);
    let mut file_sector = [0u8; 256];
    file_sector[..FILE_DATA.len()].copy_from_slice(&FILE_DATA);
    disk.write_sector(0, 0, 2, &file_sector);
    disk
}

/// An Atom that has booted, has `disk` in drive 0, and has run `*DOS` to
/// switch to the disk filing system. `None` if the ROMs (including the disk
/// ROM) are missing.
fn atom_with_disk(disk: DiskImage) -> Option<Atom> {
    let mut atom = booted_atom(120)?;
    if !atom.bus.memory.disk_interface_fitted {
        common::skipped("no dosrom.rom");
        return None;
    }
    atom.bus.fdc.drives[0] = Some(disk);
    type_text(&mut atom, "*DOS\n");
    run_seconds(&mut atom, 1);
    Some(atom)
}

fn run_seconds(atom: &mut Atom, seconds: u64) {
    atom.run_cycles(seconds * 1_000_000);
}

fn memory_at(atom: &Atom, addr: u16, len: usize) -> Vec<u8> {
    (0..len).map(|i| atom.bus.peek(addr + i as u16)).collect()
}

#[test]
fn dos_lists_the_catalogue_of_a_disk() {
    let Some(mut atom) = atom_with_disk(disk_with_one_file()) else {
        return;
    };
    type_text(&mut atom, "*CAT\n");
    run_seconds(&mut atom, 5);
    let screen = atom.screen_text().join("\n");
    eprintln!("{screen}");
    assert!(screen.contains("TEST"), "the catalogue should list TEST");
}

#[test]
fn dos_loads_a_file_from_disk() {
    let Some(mut atom) = atom_with_disk(disk_with_one_file()) else {
        return;
    };
    type_text(&mut atom, "*LOAD TEST\n");
    run_seconds(&mut atom, 5);
    eprintln!("{}", atom.screen_text().join("\n"));
    assert_eq!(memory_at(&atom, LOAD_ADDRESS, FILE_DATA.len()), FILE_DATA);
}

#[test]
fn dos_saves_a_file_to_disk_and_loads_it_back() {
    let Some(mut atom) = atom_with_disk(DiskImage::blank_80_track(1)) else {
        return;
    };
    let program = [0xEA, 0xEA, 0x60, 0x11, 0x22, 0x33, 0x44, 0x55];
    for (i, &byte) in program.iter().enumerate() {
        assert!(atom.bus.poke(0x3000 + i as u16, byte));
    }

    type_text(&mut atom, "*SAVE SAVED 3000 3008\n");
    run_seconds(&mut atom, 5);
    eprintln!("{}", atom.screen_text().join("\n"));
    assert!(
        atom.bus.fdc.drives[0].as_ref().unwrap().modified,
        "disk was not written"
    );

    for i in 0..program.len() {
        atom.bus.poke(0x3000 + i as u16, 0);
    }
    type_text(&mut atom, "*LOAD SAVED\n");
    run_seconds(&mut atom, 5);
    assert_eq!(memory_at(&atom, 0x3000, program.len()), program);

    type_text(&mut atom, "*CAT\n");
    run_seconds(&mut atom, 5);
    assert!(atom.screen_text().join("\n").contains("SAVED"));
}

/// What `--disk FILE --run '*LOAD TEST\n'` does: *DOS is typed first, then
/// the command.
#[test]
fn disk_loads_hands_free_with_typist() {
    use atom_rs::machine::typist::Typist;
    let Some(mut atom) = booted_atom(0) else {
        return;
    };
    if !atom.bus.memory.disk_interface_fitted {
        return;
    }
    atom.bus.fdc.drives[0] = Some(disk_with_one_file());
    let mut typist = Typist::new();
    typist.type_text("*DOS\n*LOAD TEST\n", 100);
    for _ in 0..1500 {
        typist.drive(&mut atom);
        atom.run_frame();
        if memory_at(&atom, LOAD_ADDRESS, FILE_DATA.len()) == FILE_DATA {
            return;
        }
    }
    panic!("the file never loaded:\n{}", atom.screen_text().join("\n"));
}

/// Files put on a disk with `add_file` (as the atm2ssd tool does) can be
/// loaded by the real DOS, with the load address overridden or not.
#[test]
fn dos_loads_a_file_added_with_add_file() {
    let mut disk = DiskImage::blank_80_track(1);
    let data: Vec<u8> = (0..700u32).map(|i| (i * 7) as u8).collect(); // spans 3 sectors
    disk.add_file("BIGFILE", 0x3000, 0x3000, &data).unwrap();
    let Some(mut atom) = atom_with_disk(disk) else {
        return;
    };

    type_text(&mut atom, "*LOAD BIGFILE\n");
    run_seconds(&mut atom, 5);
    assert_eq!(memory_at(&atom, 0x3000, data.len()), data);

    // A load address typed after the name overrides the one on the disk.
    type_text(&mut atom, "*LOAD BIGFILE 4000\n");
    run_seconds(&mut atom, 5);
    assert_eq!(memory_at(&atom, 0x4000, data.len()), data);
}

/// A line typed straight after a disk command must not lose its first key:
/// the typist has to wait for the Atom to finish and show its prompt, as a
/// person would. (Keys typed while the Atom is busy loading are simply lost.)
#[test]
fn typist_waits_for_a_disk_command_to_finish() {
    use atom_rs::machine::typist::Typist;
    let Some(mut atom) = booted_atom(0) else {
        return;
    };
    if !atom.bus.memory.disk_interface_fitted {
        return;
    }
    atom.bus.fdc.drives[0] = Some(disk_with_one_file());
    let mut typist = Typist::new();
    typist.type_text("*DOS\n*LOAD TEST\nPRINT 5\n", 100);
    for _ in 0..1500 {
        typist.drive(&mut atom);
        atom.run_frame();
    }
    let screen = atom.screen_text().join("\n");
    assert!(
        screen.contains("       5"),
        "PRINT 5 was not understood:\n{screen}"
    );
    assert!(!screen.contains("ERROR"), "something went wrong:\n{screen}");
}
