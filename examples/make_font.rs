//! Prints the character generator table for `src/machine/font.rs`, read from the
//! die photograph of the MC6847's ROM.
//!
//!     ./scripts/fetch_font_source.sh
//!     cargo run --example make_font > /tmp/font_table.txt
//!
//! (See the top of `src/machine/font.rs` for where the data comes from.)

use atom_rs::machine::font::{GLYPH_HEIGHT, glyphs_from_die_diagram};

const SOURCE: &str = "testdata/font/MC6847_Character_ROM_memory_organization.png";

/// What each of the 64 characters is, for the comments.
fn name(code: usize) -> String {
    match code {
        0x1E => "up arrow".to_string(),
        0x1F => "left arrow".to_string(),
        0x20 => "space".to_string(),
        0x00..=0x1D => char::from(code as u8 + 0x40).to_string(),
        _ => char::from(code as u8).to_string(),
    }
}

fn main() {
    let file = std::fs::read(SOURCE).expect("run ./scripts/fetch_font_source.sh first");
    let font = glyphs_from_die_diagram(&file).expect("the die photograph reads");
    println!("pub const FONT: [u8; 64 * GLYPH_HEIGHT] = [");
    for (code, glyph) in font.chunks(GLYPH_HEIGHT).enumerate() {
        let bytes: Vec<String> = glyph.iter().map(|b| format!("0x{b:02x}")).collect();
        println!("    {}, // ${code:02X} {}", bytes.join(", "), name(code));
    }
    println!("];");
}
