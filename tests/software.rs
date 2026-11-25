//! Runs a real Atom game through the whole emulator: Chuckie Egg from the Atom
//! Software Archive, packed into a disk image with `examples/atm2ssd.rs`.
//!
//! The game is not part of this repository. If the disk image is not there the
//! test says so and passes; run `scripts/make_chuckie.sh` to make it.

mod common;

use std::path::Path;

use atom_rs::machine::Atom;
use atom_rs::machine::disk::DiskImage;
use atom_rs::machine::typist::Typist;
use atom_rs::machine::vdg6847::VideoMode;
use common::booted_atom;

const DISK: &str = "testdata/games/chuckie.ssd";

#[test]
fn chuckie_egg_menu_and_game_start() {
    if !Path::new(DISK).exists() {
        common::skipped(format!("{DISK} not found (run scripts/make_chuckie.sh)"));
        return;
    }
    let Some(mut atom) = booted_atom(0) else {
        return;
    };
    if !atom.bus.memory.disk_interface_fitted {
        common::skipped("no dosrom.rom");
        return;
    }
    // Log anything the game touches that the machine does not provide.
    atom.bus.debug.log_unmapped = true;
    let data = std::fs::read(DISK).unwrap();
    atom.bus.fdc.drives[0] = Some(DiskImage::from_bytes(data, 1).unwrap());

    // Exactly what `--disk chuckie.ssd --run '*RUN RUNME\n'` does.
    let mut typist = Typist::new();
    typist.type_text("*DOS\n*RUN RUNME\n", 100);

    // The loader draws the Retro Software menu: a full-resolution picture.
    let menu_mode = VideoMode::Graphics {
        width: 256,
        height: 192,
        colours: 2,
    };
    assert!(
        run_until(&mut atom, &mut typist, 3000, |a| a.video_mode()
            == menu_mode
            && lit_pixels(a) > 5000),
        "the Chuckie Egg menu never appeared:\n{}",
        atom.screen_text().join("\n")
    );

    // Choose "2. Colour". The game shows its instructions and high scores,
    // all in the 128 x 192, four colour mode.
    // (The picture appears before the loader prints its text and starts
    // reading keys, so give it a moment, as a person would.)
    run_until(&mut atom, &mut typist, 200, |_| false);
    typist.type_text("2", atom.bus.vdg.frames);
    let game_mode = VideoMode::Graphics {
        width: 128,
        height: 192,
        colours: 4,
    };
    assert!(
        run_until(&mut atom, &mut typist, 6000, |a| a.video_mode()
            == game_mode
            && lit_pixels(a) > 5000),
        "the game never started"
    );

    // Let it get to the high scores screen, which is built from tables the
    // game keeps in low memory (0400-1FFF). When that memory was missing the
    // table showed garbage and the score read 555555.
    run_until(&mut atom, &mut typist, 600, |_| false);
    let stray: Vec<String> = atom
        .bus
        .debug
        .unmapped
        .keys()
        .filter(|&&addr| addr < 0xB000)
        .map(|addr| format!("{addr:04X}"))
        .collect();
    assert!(
        stray.is_empty(),
        "the game touched memory that does not exist: {stray:?}"
    );
}

fn lit_pixels(atom: &Atom) -> usize {
    atom.bus
        .framebuffer
        .pixels
        .iter()
        .filter(|&&p| p != 0)
        .count()
}

/// Run frame by frame (typing as we go) until `done` holds or the frames run out.
fn run_until(
    atom: &mut Atom,
    typist: &mut Typist,
    max_frames: u32,
    done: impl Fn(&Atom) -> bool,
) -> bool {
    for _ in 0..max_frames {
        typist.drive(atom);
        atom.run_frame();
        if done(atom) {
            return true;
        }
    }
    false
}

const REPTON_DISK: &str = "testdata/games/repton.ssd";

fn hold_key(atom: &mut Atom, name: &str, frames: u32) {
    let (row, column) = atom_rs::machine::keyboard::find_key(name).expect("key exists");
    atom.bus.ppi.keyboard.set_key(row, column, true);
    for _ in 0..frames {
        atom.run_frame();
    }
    atom.bus.ppi.keyboard.release_all();
}

/// Repton reads its joystick through the AtoMMC interface (command A2 at
/// B400, answer at B401) and ignores the keyboard for any direction the
/// joystick seems to be pressing. With no AtoMMC the answer was open bus,
/// which read as right, left and up all held, so Z, X, ; and . did nothing.
#[test]
fn repton_responds_to_the_direction_keys() {
    if !Path::new(REPTON_DISK).exists() {
        common::skipped(format!(
            "{REPTON_DISK} not found (./scripts/make_game.sh RS/REPTON repton)"
        ));
        return;
    }
    let Some(mut atom) = booted_atom(0) else {
        return;
    };
    if !atom.bus.memory.disk_interface_fitted {
        return;
    }
    atom.bus.fdc.drives[0] =
        Some(DiskImage::from_bytes(std::fs::read(REPTON_DISK).unwrap(), 1).unwrap());

    let mut typist = Typist::new();
    typist.type_text("*DOS\n*RUN LOADER\n", 100);
    for _ in 0..1600 {
        typist.drive(&mut atom);
        atom.run_frame();
    }
    // Space: menu -> title -> high scores -> the game.
    for wait in [400, 360, 400] {
        hold_key(&mut atom, "SPACE", 20);
        for _ in 0..wait {
            atom.run_frame();
        }
    }

    let state = atom.save_state().unwrap();
    let picture_after = |key: Option<&str>| -> Vec<u8> {
        let mut copy = booted_atom(0).unwrap();
        copy.load_state(&state).unwrap();
        if let Some(key) = key {
            hold_key(&mut copy, key, 30);
        }
        for _ in 0..30 {
            copy.run_frame();
        }
        copy.bus.framebuffer.pixels.clone()
    };
    let idle = picture_after(None);
    for key in ["Z", "X", ";", "."] {
        let moved = picture_after(Some(key));
        let changed = moved.iter().zip(&idle).filter(|(a, b)| a != b).count();
        assert!(
            changed > 500,
            "holding {key} should move Repton, but only {changed} pixels changed"
        );
    }
}

const ARCHIVE: &str = "testdata/asa_v13_01/files";

/// The Atom Software Archive is laid out as an SD card. With the AtoMMC
/// emulated and the archive folder as the card, its own menu program runs: it
/// lists the titles and loads the one you pick.
#[test]
fn archive_menu_browses_the_sd_card_and_launches_a_game() {
    if !Path::new(ARCHIVE).join("MENU").exists() || !Path::new("roms/ramrom.rom").exists() {
        common::skipped("needs the archive (./scripts/get_archive.sh) and roms/ramrom.rom");
        return;
    }
    let mut atom = Atom::new();
    atom.use_sd_card(Path::new("roms"), ARCHIVE.into())
        .expect("AtoMMC ROMs load");
    for _ in 0..120 {
        atom.run_frame();
    }

    let mut typist = Typist::new();
    typist.type_text("*MENU\n", 0);
    let menu_mode = VideoMode::Graphics {
        width: 256,
        height: 192,
        colours: 2,
    };
    assert!(
        run_until(&mut atom, &mut typist, 3000, |a| a.video_mode()
            == menu_mode
            && lit_pixels(a) > 5000),
        "the archive's front menu never appeared:\n{}",
        atom.screen_text().join("\n")
    );

    // Give the menu a moment to start reading keys, then choose (A) Commercial.
    run_until(&mut atom, &mut typist, 200, |_| false);
    hold_key(&mut atom, "A", 15);
    assert!(
        run_until(&mut atom, &mut typist, 1500, |a| screen_has(a, "BY TITLE")),
        "the list of titles never appeared:\n{}",
        atom.screen_text().join("\n")
    );
    assert!(
        run_until(&mut atom, &mut typist, 600, |a| screen_has(
            a,
            "3D ASTEROIDS"
        )),
        "the first title should be 3D Asteroids:\n{}",
        atom.screen_text().join("\n")
    );

    // RETURN loads the highlighted title from the card and runs it. (As with
    // the first menu, wait a moment for it to start reading keys.)
    run_until(&mut atom, &mut typist, 300, |_| false);
    hold_key(&mut atom, "RETURN", 15);
    assert!(
        run_until(&mut atom, &mut typist, 4000, |a| screen_has(
            a,
            "ASTEROID STORM"
        )),
        "the game never loaded:\n{}",
        atom.screen_text().join("\n")
    );
}

fn screen_has(atom: &Atom, text: &str) -> bool {
    atom.screen_text().iter().any(|row| row.contains(text))
}

const ASTEROIDS_DISK: &str = "testdata/games/asteroid.ssd";

/// Acornsoft's Asteroids places its rocks with BASIC's `RND`, whose seed lives
/// in the zero page and is never set. When the RAM started out all zeros the
/// seed was zero, `RND` gave zero and no asteroids ever appeared.
#[test]
fn asteroids_has_asteroids() {
    if !Path::new(ASTEROIDS_DISK).exists() {
        common::skipped(format!(
            "{ASTEROIDS_DISK} not found (./scripts/make_game.sh)"
        ));
        return;
    }
    let Some(mut atom) = booted_atom(0) else {
        return;
    };
    if !atom.bus.memory.disk_interface_fitted {
        return;
    }
    atom.bus.fdc.drives[0] =
        Some(DiskImage::from_bytes(std::fs::read(ASTEROIDS_DISK).unwrap(), 1).unwrap());
    let mut typist = Typist::new();
    typist.type_text("*DOS\n*RUN ASTEROI\n", 100);

    // Once playing, the picture is the ship and score (about 50 lit pixels)
    // plus the rocks. Look for the rocks over the first few seconds of play.
    let playing = VideoMode::Graphics {
        width: 256,
        height: 192,
        colours: 2,
    };
    assert!(
        run_until(&mut atom, &mut typist, 3000, |a| a.video_mode() == playing),
        "the game never started"
    );
    assert!(
        run_until(&mut atom, &mut typist, 300, |a| lit_pixels(a) > 100),
        "no asteroids: only {} lit pixels",
        lit_pixels(&atom)
    );
}

/// The archive's `SID` folder holds tunes for SID sound cards, with a player
/// menu. Play one: the menu is silent, and once SPACE starts a tune the SID
/// (at `BDC0`) makes sound.
#[test]
fn a_sid_tune_from_the_archive_plays() {
    if !Path::new(ARCHIVE).join("ASA/SID/ATMSID1/SIDMENU").exists()
        || !Path::new("roms/ramrom.rom").exists()
    {
        common::skipped("needs the archive (./scripts/get_archive.sh) and roms/ramrom.rom");
        return;
    }
    let mut atom = Atom::new();
    atom.use_sd_card(Path::new("roms"), ARCHIVE.into())
        .expect("AtoMMC ROMs load");
    atom.bus.set_audio_sample_rate(44_100);
    atom.bus.sound.enabled = true;
    for _ in 0..120 {
        atom.run_frame();
    }

    let mut typist = Typist::new();
    typist.type_text("*CWD ASA/SID/ATMSID1\n*RUN SIDMENU\n", 0);
    assert!(
        run_until(&mut atom, &mut typist, 3000, |a| screen_has(a, "SPC=PLAY")),
        "the SID player menu never appeared:\n{}",
        atom.screen_text().join("\n")
    );
    // The menu itself makes no sound.
    run_until(&mut atom, &mut typist, 100, |_| false);
    let quiet = ac_level(&atom.bus.sound.take_samples());
    assert!(quiet < 0.001, "the menu should be silent, level {quiet}");

    // Play the first tune for a few seconds.
    hold_key(&mut atom, "SPACE", 10);
    run_until(&mut atom, &mut typist, 150, |_| false);
    let playing = ac_level(&atom.bus.sound.take_samples());
    assert!(playing > 0.01, "the tune made no sound, level {playing}");
}

/// How loud a stretch of audio is: the RMS once its steady level is removed.
fn ac_level(samples: &[f32]) -> f32 {
    let mean = samples.iter().sum::<f32>() / samples.len().max(1) as f32;
    let power = samples.iter().map(|s| (s - mean) * (s - mean)).sum::<f32>();
    (power / samples.len().max(1) as f32).sqrt()
}
