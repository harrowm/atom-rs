//! Helpers shared by the integration tests.
#![allow(dead_code)] // each test file uses a different subset

use std::path::Path;

use atom_rs::machine::Atom;
use atom_rs::machine::keyboard::{find_key, key_for_char};

pub const ROM_DIR: &str = "roms";

/// Say a test could not run because some data it needs is missing, and let it
/// pass, so the suite works on a machine without the (large, downloaded) test
/// data. On a continuous-integration server set `REQUIRE_TEST_DATA=1`: then a
/// missing file is a failure, so a green result always means the tests ran.
pub fn skipped(reason: impl std::fmt::Display) {
    assert!(
        std::env::var_os("REQUIRE_TEST_DATA").is_none(),
        "test data missing, and REQUIRE_TEST_DATA is set: {reason}"
    );
    eprintln!("SKIPPED: {reason}");
}

/// An Atom that has booted to the prompt, or `None` if there are no ROMs.
pub fn booted_atom(frames: u32) -> Option<Atom> {
    if !Path::new(ROM_DIR).join("akernel.rom").exists() {
        skipped(format!("no ROMs in {ROM_DIR}/"));
        return None;
    }
    let mut atom = Atom::new();
    atom.load_roms(Path::new(ROM_DIR)).expect("ROMs load");
    atom.reset();
    for _ in 0..frames {
        atom.run_frame();
    }
    Some(atom)
}

/// Hold a key down for a few frames, then release it, like a (fast) typist.
pub fn tap(atom: &mut Atom, key: &str, shift: bool) {
    let (row, column) = find_key(key).expect("key exists");
    atom.bus.ppi.keyboard.shift = shift;
    atom.bus.ppi.keyboard.set_key(row, column, true);
    for _ in 0..6 {
        atom.run_frame();
    }
    atom.bus.ppi.keyboard.set_key(row, column, false);
    atom.bus.ppi.keyboard.shift = false;
    for _ in 0..6 {
        atom.run_frame();
    }
}

/// Type a line of text (use `\n` for RETURN).
pub fn type_text(atom: &mut Atom, text: &str) {
    for c in text.chars() {
        let (key, shift) = key_for_char(c).unwrap_or_else(|| panic!("cannot type {c:?}"));
        tap(atom, key, shift);
    }
}

/// The whole text screen as one string (rows joined without separators, so
/// text that wrapped across rows reads continuously).
pub fn screen_string(atom: &Atom) -> String {
    atom.screen_text().concat()
}
