//! The AtoMMC SD card interface, with the real AtoMMC ROM and a folder on the
//! host standing in for the card. Needs `roms/ramrom.rom` (the 128K image that
//! Atomulator ships, which holds the AtoMMC ROM); the tests skip themselves
//! without it.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use atom_rs::machine::Atom;
use atom_rs::machine::atm::AtmFile;
use atom_rs::machine::disk::DiskImage;
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

impl Card {
    /// Put a disk image on the card holding one file, `HELLO`, which loads
    /// at 2900.
    fn add_disk_with_a_file(&self, name: &str, data: &[u8]) {
        let mut disk = DiskImage::blank_80_track(1);
        disk.add_file("HELLO", 0x2900, 0x2900, data).unwrap();
        fs::write(self.0.join(name), disk.to_bytes()).unwrap();
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
        common::skipped("no roms/ramrom.rom");
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

// ---- SDDOS: disk images on the card, used as floppy disks ---------------------------

const HELLO_DATA: [u8; 8] = [0xA9, 0x41, 0x20, 0xF4, 0xFF, 0x60, 0x11, 0x22];

#[test]
fn sddos_mounts_a_disk_image_from_the_card_and_lists_it() {
    let card = Card::new("sddos-cat");
    card.add_disk_with_a_file("TEST.SSD", &HELLO_DATA);
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };
    run_command(&mut atom, "SDDOS\n"); // switches SDDOS on (no star)
    run_command(&mut atom, "*DIN 0 TEST.SSD\n");
    run_command(&mut atom, "*.\n");
    let screen = atom.screen_text().join("\n");
    assert!(
        screen.contains("HELLO"),
        "the image's file is not listed:\n{screen}"
    );
}

#[test]
fn sddos_loads_a_file_from_the_image() {
    let card = Card::new("sddos-load");
    card.add_disk_with_a_file("TEST.SSD", &HELLO_DATA);
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };
    run_command(&mut atom, "SDDOS\n");
    run_command(&mut atom, "*DIN 0 TEST.SSD\n");
    run_command(&mut atom, "*LOAD HELLO\n");
    let loaded: Vec<u8> = (0..HELLO_DATA.len())
        .map(|i| atom.bus.peek(0x2900 + i as u16))
        .collect();
    assert_eq!(loaded, HELLO_DATA, "\n{}", atom.screen_text().join("\n"));
}

#[test]
fn sddos_saves_a_file_into_the_image_on_the_card() {
    let card = Card::new("sddos-save");
    card.add_disk_with_a_file("TEST.SSD", &HELLO_DATA);
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };
    let program: Vec<u8> = (0..16).map(|i| 0x30 + i).collect();
    for (i, &byte) in program.iter().enumerate() {
        atom.bus.poke(0x3000 + i as u16, byte);
    }
    run_command(&mut atom, "SDDOS\n");
    run_command(&mut atom, "*DIN 0 TEST.SSD\n");
    run_command(&mut atom, "*SAVE WORLD 3000 3010\n");

    // The image file on the host now has the new file in its catalogue.
    let image = DiskImage::from_bytes(fs::read(card.0.join("TEST.SSD")).unwrap(), 1).unwrap();
    let names: Vec<String> = image.catalogue().iter().map(|f| f.name.clone()).collect();
    assert!(
        names.iter().any(|n| n == "WORLD") && names.iter().any(|n| n == "HELLO"),
        "catalogue is {names:?}\n{}",
        atom.screen_text().join("\n")
    );
}

#[test]
fn sddos_remembers_the_mounted_images_next_time() {
    let card = Card::new("sddos-remember");
    card.add_disk_with_a_file("TEST.SSD", &HELLO_DATA);
    {
        let Some(mut atom) = atom_with_card(&card.0) else {
            return;
        };
        run_command(&mut atom, "SDDOS\n");
        run_command(&mut atom, "*DIN 0 TEST.SSD\n");
    }
    assert!(
        card.0.join("BOOTDRV.CFG").exists(),
        "the list is saved on the card"
    );

    // A new Atom (a power-on) on the same card: SDDOS brings the drive back.
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };
    run_command(&mut atom, "SDDOS\n");
    run_command(&mut atom, "*.\n");
    let screen = atom.screen_text().join("\n");
    assert!(
        screen.contains("HELLO"),
        "drive 0 was not remounted:\n{screen}"
    );
}

/// A BASIC statement that is just a space makes BASIC offer it to the utility
/// ROMs, which runs the AtoMMC ROM's scanner over the ROM banks and wraps the
/// OS vectors that point into the banked `A000` area. That wrapping saves the
/// ROM's current bank from `$FD`, which must start at 0. When RAM powered up
/// full of junk it did not, and SDDOS's `*` commands then ran the wrong bank
/// and failed with an error, as Chuckie Egg's loader (which has such lines)
/// did. This is the smallest program that shows it.
#[test]
fn sddos_star_commands_work_after_a_blank_basic_statement() {
    let card = Card::new("sddos-blank");
    card.add_disk_with_a_file("TEST.SSD", &HELLO_DATA);
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };
    run_command(&mut atom, "SDDOS\n");
    run_command(&mut atom, "*DIN 0 TEST.SSD\n");
    // Line 20 is a single space.
    for line in [
        "10 ?#2000=7\n",
        "20 \n",
        "30 *NOMON\n",
        "40 ?#2001=9\n",
        "50 END\n",
        "RUN\n",
    ] {
        run_command(&mut atom, line);
    }
    let screen = atom.screen_text().join("\n");
    assert!(!screen.contains("ERROR"), "the program failed:\n{screen}");
    assert_eq!(atom.bus.peek(0x2000), 7, "line 10 ran");
    assert_eq!(atom.bus.peek(0x2001), 9, "line 40 ran, after the *NOMON");
}

/// The real loader that first showed it: Chuckie Egg's `RUNME`, from a disk
/// image on the card, through SDDOS.
#[test]
fn chuckie_egg_loads_from_a_disk_image_through_sddos() {
    let game = "testdata/games/chuckie.ssd";
    if !Path::new(game).exists() {
        common::skipped("needs testdata/games/chuckie.ssd (scripts/make_all_games.sh)");
        return;
    }
    let card = Card::new("sddos-chuckie");
    fs::copy(game, card.0.join("CHUCKIE.SSD")).unwrap();
    let Some(mut atom) = atom_with_card(&card.0) else {
        return;
    };
    let mut typist = Typist::new();
    typist.type_text(
        "SDDOS\n*DIN 0 CHUCKIE.SSD\nLOAD \"RUNME\"\nRUN\n",
        atom.bus.vdg.frames,
    );
    // The loader draws the Retro Software menu: a 256 x 192 picture.
    let menu = atom_rs::machine::vdg6847::VideoMode::Graphics {
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
