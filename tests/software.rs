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
        eprintln!("SKIPPED: {DISK} not found (run scripts/make_chuckie.sh)");
        return;
    }
    let Some(mut atom) = booted_atom(0) else {
        return;
    };
    if !atom.bus.memory.disk_interface_fitted {
        eprintln!("SKIPPED: no dosrom.rom");
        return;
    }
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
        typist.update(&mut atom.bus.ppi.keyboard, atom.bus.vdg.frames);
        atom.run_frame();
        if done(atom) {
            return true;
        }
    }
    false
}
