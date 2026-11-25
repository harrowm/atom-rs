//! Boots the emulated Atom from the real ROMs and checks what appears on the
//! text screen. Needs the ROM images in `roms/` (see `plan.md`).

mod common;

use common::{booted_atom, screen_string, type_text};

#[test]
fn boots_to_the_atom_prompt() {
    let Some(atom) = booted_atom(120) else { return };
    let screen = atom.screen_text();
    eprintln!("{}", screen.join("\n"));
    assert!(
        screen.iter().any(|row| row.contains("ACORN ATOM")),
        "no start-up banner on screen"
    );
    assert!(
        screen.iter().any(|row| row.starts_with('>')),
        "no '>' prompt on screen"
    );
}

#[test]
fn basic_runs_a_typed_command() {
    let Some(mut atom) = booted_atom(120) else {
        return;
    };
    type_text(&mut atom, "PRINT 3\n");
    for _ in 0..20 {
        atom.run_frame();
    }
    let screen = atom.screen_text();
    eprintln!("{}", screen.join("\n"));
    assert!(
        screen.iter().any(|row| row.starts_with(">PRINT 3")),
        "typed line not echoed"
    );
    // Atom BASIC right-aligns numbers in a field of 8 characters, and the
    // prompt follows on the same line.
    assert!(
        screen.iter().any(|row| row.starts_with("       3")),
        "BASIC did not print 3"
    );
}

/// The drawn picture should show the banner at the top of the screen.
/// Set `SCREENSHOT=/path/file.ppm` to save the picture for a look.
#[test]
fn framebuffer_shows_the_screen() {
    let Some(atom) = booted_atom(120) else { return };
    let pixels = &atom.bus.framebuffer.pixels;
    let width = atom_rs::machine::video::WIDTH;
    let lit_in = |lines: std::ops::Range<usize>| {
        pixels[lines.start * width..lines.end * width]
            .iter()
            .filter(|&&p| p != 0)
            .count()
    };
    assert!(lit_in(0..12) > 100, "banner row should be drawn");
    assert!(lit_in(100..192) == 0, "lower screen should be blank");

    if let Ok(path) = std::env::var("SCREENSHOT") {
        let mut ppm = format!("P6\n{} {}\n255\n", width, pixels.len() / width).into_bytes();
        for rgba in atom.bus.framebuffer.to_rgba().chunks(4) {
            ppm.extend_from_slice(&rgba[..3]);
        }
        std::fs::write(path, ppm).unwrap();
    }
}

/// Every character the keyboard mapping claims to produce should appear on
/// screen as itself, which checks the Shift pairings against the real ROMs.
#[test]
fn typed_characters_appear_as_themselves() {
    let Some(mut atom) = booted_atom(120) else {
        return;
    };
    let text = "!\"#$%&'()*+,-./:;<=>?@";
    type_text(&mut atom, text);
    let screen = screen_string(&atom);
    assert!(
        screen.contains(text),
        "typed {text:?} but screen shows {:?}",
        screen.trim_end()
    );
}

/// A state saved after booting and typing restores the whole screen.
#[test]
fn save_state_restores_a_running_basic_session() {
    let Some(mut atom) = booted_atom(120) else {
        return;
    };
    type_text(&mut atom, "PRINT 3\n");
    let state = atom.save_state().unwrap();
    let before = atom.screen_text();
    eprintln!("state is {} bytes compressed", state.len());

    // Wreck the machine, then bring it back.
    atom.reset();
    for _ in 0..60 {
        atom.run_frame();
    }
    assert_ne!(atom.screen_text(), before);
    atom.load_state(&state).unwrap();
    assert_eq!(atom.screen_text(), before);

    // And it carries on working: BASIC still answers.
    type_text(&mut atom, "PRINT 4\n");
    assert!(screen_string(&atom).contains("       4"));
}

/// The typist types a BASIC line into a running Atom.
#[test]
fn typist_types_a_basic_line() {
    use atom_rs::machine::typist::Typist;
    let Some(mut atom) = booted_atom(100) else {
        return;
    };
    let mut typist = Typist::new();
    typist.type_text("PRINT 5\n", 0);
    while typist.is_busy() {
        typist.drive(&mut atom);
        atom.run_frame();
    }
    for _ in 0..30 {
        atom.run_frame();
    }
    assert!(screen_string(&atom).contains("       5"));
}
