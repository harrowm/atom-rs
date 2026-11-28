//! The character font against the photograph of the MC6847's ROM it was read
//! from. The photograph is not part of the repository (it is CC BY-SA); get it
//! with `./scripts/fetch_font_source.sh`. Without it this test says so and
//! passes, unless REQUIRE_TEST_DATA is set (as it is in CI).

mod common;

use atom_rs::machine::font::{FONT, glyphs_from_die_diagram};

const PHOTOGRAPH: &str = "testdata/font/MC6847_Character_ROM_memory_organization.png";

#[test]
fn the_font_is_what_the_roms_die_photograph_shows() {
    let Ok(file) = std::fs::read(PHOTOGRAPH) else {
        common::skipped(format!(
            "{PHOTOGRAPH} not found (run scripts/fetch_font_source.sh)"
        ));
        return;
    };
    let read = glyphs_from_die_diagram(&file).expect("the photograph reads");
    assert_eq!(read.len(), FONT.len());
    assert!(
        read == FONT,
        "font.rs differs from the photograph: run `cargo run --example make_font` and update FONT"
    );
}
