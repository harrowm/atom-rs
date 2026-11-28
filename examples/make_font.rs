//! Prints the character generator table for `src/machine/font.rs`, read from
//! the published specimen image in `assets/`.
//!
//!     cargo run --example make_font > /tmp/font_table.txt
//!
//! (See the top of `src/machine/font.rs` for where the data comes from.)

use atom_rs::machine::font::{GLYPH_HEIGHT, glyphs_for_the_atom};

/// What each of the 64 characters is, for the comments.
fn name(code: usize) -> String {
    match code {
        0x1E => "up arrow".to_string(),
        0x1F => "left arrow".to_string(),
        0x00..=0x1D => ((code as u8) + 0x40).to_string_char(),
        _ => (code as u8).to_string_char(),
    }
}

trait ToStringChar {
    fn to_string_char(self) -> String;
}
impl ToStringChar for u8 {
    fn to_string_char(self) -> String {
        match self {
            b' ' => "space".to_string(),
            c => (c as char).to_string(),
        }
    }
}

fn main() {
    let file =
        std::fs::read("assets/mc6847-character-set.png").expect("run this from the project folder");
    let font = glyphs_for_the_atom(&file).expect("the specimen image reads");
    println!("pub const FONT: [u8; 64 * GLYPH_HEIGHT] = [");
    for (code, glyph) in font.chunks(GLYPH_HEIGHT).enumerate() {
        let bytes: Vec<String> = glyph.iter().map(|b| format!("0x{b:02x}")).collect();
        println!("    {}, // ${code:02X} {}", bytes.join(", "), name(code));
    }
    println!("];");
}
