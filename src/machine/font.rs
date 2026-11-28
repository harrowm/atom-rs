//! The 6847 character generator: 64 characters, each 12 scanlines of 8 pixels.
//!
//! Character code `n` (0-63) starts at byte `n * 12`. In each byte bit 7 is the
//! leftmost pixel. Codes 0-31 are `@A-Z[\]` then an up arrow and a left arrow,
//! and 32-63 are space, the punctuation and the digits (the order of the
//! MC6847 data sheet's Figure 20).
//!
//! # Where this data comes from
//!
//! It was made from published material, not copied from another emulator:
//!
//! * **The dot patterns** are read, by [`glyphs_from_specimen`] (used by
//!   `cargo run --example make_font`), from `assets/mc6847-character-set.png`,
//!   a specimen of the MC6847's built-in 64-character generator at three times
//!   size, published by Brad Neil in the "Motorola MC6847" section of
//!   <https://friedorange.github.io/MatrixSans/research.html> and released
//!   under CC0 (public domain). See `assets/README.md`.
//! * **The size and placement** come from the Motorola MC6847/MC6847Y data
//!   sheet, page 18: the characters are "in a 5 x 7 format", each character
//!   element is "8 half periods by 12 scan lines", and "the 5 x 7 character
//!   font is positioned two columns to the right and three rows down within
//!   the 8 x 12 character element". So the glyph's rows go in scanlines 3-9
//!   and its columns in pixels 2-6.
//! * **The order of the characters** is the data sheet's Figure 20.
//!
//! * **Corrections** where a real Atom's screen contradicts the specimen are
//!   listed in [`CORRECTIONS`], each with the evidence. There is one so far.
//!
//! The table below was produced by that program (the image, then those
//! corrections) and not edited by hand, and a test checks that the table and
//! the image still agree.
//!
//! # What is not yet checked against a real Atom
//!
//! The specimen is not quite the Atom's ROM (the one letter that could be
//! measured, `E`, differs), so other letters may differ by a dot too. The
//! patterns that have been compared with real hardware are the ones in the
//! test `the_glyphs_match_what_a_real_atom_shows` (the letters `ACORNTMEPHISL`,
//! the digits `9410`, the quote and `>`). **`F`, `G` and `W` are known to
//! differ between published fonts and have not been verified**, and the rest of
//! the set (punctuation and the arrows) only matches the specimen. A sharp
//! screenshot of a real Atom showing them would settle it.
//!
//! Nothing here was copied from another emulator. An earlier version of this
//! file was a copy of Atomulator's table (GPL-licensed) and was replaced so that
//! this project does not have to be GPL-licensed on its account.

pub const GLYPH_HEIGHT: usize = 12;

/// The scanline of the cell the 5 x 7 pattern starts on (the data sheet's
/// "three rows down") and the pattern's size.
const FIRST_SCANLINE: usize = 3;
const PATTERN_HEIGHT: usize = 7;
const PATTERN_WIDTH: usize = 5;
/// Pixels from the left of the 8-wide cell ("two columns to the right").
const LEFT_PIXELS: usize = 2;

// ---- Reading the specimen image ----------------------------------------------------

/// The specimen is a 3x enlargement: each dot of a glyph is a 3 x 3 block.
const SCALE: usize = 3;
/// A cell in the specimen is 7 dots wide: the 5-dot pattern with a blank dot
/// on each side. 32 cells make a row.
const CELL_DOTS: usize = 7;
const CELLS_PER_ROW: usize = 32;
/// The specimen's size in dots, and the dot rows its two bands of pattern
/// start on: the top band holds codes 32-63 and the bottom one codes 0-31.
const SPECIMEN_DOTS: (usize, usize) = (CELL_DOTS * CELLS_PER_ROW, 20);
const TOP_BAND_FIRST_DOT_ROW: usize = 1;
const BOTTOM_BAND_FIRST_DOT_ROW: usize = 11;

/// Where the specimen disagrees with a real Acorn Atom: `(character code,
/// scanline, the byte that scanline should be)`.
///
/// * `E` (`$05`) scanline 6, the middle bar, is `$3C` (four dots, `..####..`).
///   The specimen has three dots. Measured on a screenshot of a real Atom
///   (the "ERROR 94" line of the video "Acorn Atom [BASIC test] (1979)" on
///   YouTube, each Atom pixel being about 4.8 screen pixels): the middle bar
///   is 20 screen pixels long where the top bar is 24 and a three-dot bar
///   would be about 14.
pub const CORRECTIONS: [(usize, usize, u8); 1] = [(0x05, 6, 0x3C)];

/// The specimen's glyphs with [`CORRECTIONS`] applied: what [`FONT`] is.
pub fn glyphs_for_the_atom(png_file: &[u8]) -> Result<Vec<u8>, String> {
    let mut font = glyphs_from_specimen(png_file)?;
    for &(code, scanline, row) in &CORRECTIONS {
        font[code * GLYPH_HEIGHT + scanline] = row;
    }
    Ok(font)
}

/// Read the 64 glyphs out of the specimen image (the contents of the PNG
/// file) and lay them out as [`FONT`] is: 12 bytes per character, the 5 x 7
/// pattern in scanlines 3-9 and pixels 2-6 of the 8 x 12 cell.
///
/// It checks the image is what it should be (exactly 3 x 3 pixel dots, blank
/// space round every pattern) rather than guess.
pub fn glyphs_from_specimen(png_file: &[u8]) -> Result<Vec<u8>, String> {
    let (width, height) = (SPECIMEN_DOTS.0 * SCALE, SPECIMEN_DOTS.1 * SCALE);
    let lit = lit_pixels(png_file, width, height)?;

    // Collapse each 3 x 3 block to one dot, insisting that the block is uniform.
    let mut dots = vec![vec![false; SPECIMEN_DOTS.1]; SPECIMEN_DOTS.0];
    for (dx, column) in dots.iter_mut().enumerate() {
        for (dy, dot) in column.iter_mut().enumerate() {
            let block: Vec<bool> = (0..SCALE * SCALE)
                .map(|i| lit[(dy * SCALE + i / SCALE) * width + dx * SCALE + i % SCALE])
                .collect();
            if block.iter().any(|&p| p != block[0]) {
                return Err(format!(
                    "the dot at ({dx}, {dy}) is not a clean 3 x 3 block"
                ));
            }
            *dot = block[0];
        }
    }

    let mut font = vec![0u8; 64 * GLYPH_HEIGHT];
    let mut pattern_dots = 0;
    for code in 0..64 {
        let first_dot_row = if code >= 32 {
            TOP_BAND_FIRST_DOT_ROW
        } else {
            BOTTOM_BAND_FIRST_DOT_ROW
        };
        let cell = code % 32;
        for row in 0..PATTERN_HEIGHT {
            for column in 0..PATTERN_WIDTH {
                if dots[cell * CELL_DOTS + 1 + column][first_dot_row + row] {
                    // Bit 7 is the cell's leftmost pixel.
                    font[code * GLYPH_HEIGHT + FIRST_SCANLINE + row] |=
                        0x80 >> (LEFT_PIXELS + column);
                    pattern_dots += 1;
                }
            }
        }
    }
    // Every lit dot must have been read as part of some pattern, so nothing
    // in the margins was missed (or mistaken for part of a glyph).
    let lit_dots: usize = dots.iter().flatten().filter(|&&d| d).count();
    if lit_dots != pattern_dots {
        return Err(format!(
            "{} lit dots lie outside the glyph patterns",
            lit_dots - pattern_dots
        ));
    }
    Ok(font)
}

/// Decode the PNG to one bool per pixel (true = black), checking its size.
fn lit_pixels(png_file: &[u8], width: usize, height: usize) -> Result<Vec<bool>, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut data = vec![0; reader.output_buffer_size().ok_or("the image is too big")?];
    let info = reader.next_frame(&mut data).map_err(|e| e.to_string())?;
    if (info.width as usize, info.height as usize) != (width, height) {
        return Err(format!(
            "the specimen should be {width} x {height} pixels, not {} x {}",
            info.width, info.height
        ));
    }
    let channels = info.color_type.samples();
    Ok(data
        .chunks(channels)
        .take(width * height)
        .map(|pixel| pixel[0] < 128)
        .collect())
}

// ---- The table -----------------------------------------------------------------------

#[rustfmt::skip]
pub const FONT: [u8; 64 * GLYPH_HEIGHT] = [
    0x00, 0x00, 0x00, 0x1c, 0x22, 0x02, 0x1a, 0x2a, 0x2a, 0x1c, 0x00, 0x00, // $00 @
    0x00, 0x00, 0x00, 0x08, 0x14, 0x22, 0x22, 0x3e, 0x22, 0x22, 0x00, 0x00, // $01 A
    0x00, 0x00, 0x00, 0x3c, 0x12, 0x12, 0x1c, 0x12, 0x12, 0x3c, 0x00, 0x00, // $02 B
    0x00, 0x00, 0x00, 0x1c, 0x22, 0x20, 0x20, 0x20, 0x22, 0x1c, 0x00, 0x00, // $03 C
    0x00, 0x00, 0x00, 0x3c, 0x12, 0x12, 0x12, 0x12, 0x12, 0x3c, 0x00, 0x00, // $04 D
    0x00, 0x00, 0x00, 0x3e, 0x20, 0x20, 0x3c, 0x20, 0x20, 0x3e, 0x00, 0x00, // $05 E
    0x00, 0x00, 0x00, 0x3e, 0x20, 0x20, 0x38, 0x20, 0x20, 0x20, 0x00, 0x00, // $06 F
    0x00, 0x00, 0x00, 0x1c, 0x22, 0x20, 0x20, 0x26, 0x22, 0x1e, 0x00, 0x00, // $07 G
    0x00, 0x00, 0x00, 0x22, 0x22, 0x22, 0x3e, 0x22, 0x22, 0x22, 0x00, 0x00, // $08 H
    0x00, 0x00, 0x00, 0x1c, 0x08, 0x08, 0x08, 0x08, 0x08, 0x1c, 0x00, 0x00, // $09 I
    0x00, 0x00, 0x00, 0x02, 0x02, 0x02, 0x02, 0x22, 0x22, 0x1c, 0x00, 0x00, // $0A J
    0x00, 0x00, 0x00, 0x22, 0x24, 0x28, 0x30, 0x28, 0x24, 0x22, 0x00, 0x00, // $0B K
    0x00, 0x00, 0x00, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x3e, 0x00, 0x00, // $0C L
    0x00, 0x00, 0x00, 0x22, 0x36, 0x2a, 0x2a, 0x22, 0x22, 0x22, 0x00, 0x00, // $0D M
    0x00, 0x00, 0x00, 0x22, 0x32, 0x2a, 0x26, 0x22, 0x22, 0x22, 0x00, 0x00, // $0E N
    0x00, 0x00, 0x00, 0x3e, 0x22, 0x22, 0x22, 0x22, 0x22, 0x3e, 0x00, 0x00, // $0F O
    0x00, 0x00, 0x00, 0x3c, 0x22, 0x22, 0x3c, 0x20, 0x20, 0x20, 0x00, 0x00, // $10 P
    0x00, 0x00, 0x00, 0x1c, 0x22, 0x22, 0x22, 0x2a, 0x24, 0x1a, 0x00, 0x00, // $11 Q
    0x00, 0x00, 0x00, 0x3c, 0x22, 0x22, 0x3c, 0x28, 0x24, 0x22, 0x00, 0x00, // $12 R
    0x00, 0x00, 0x00, 0x1c, 0x22, 0x10, 0x08, 0x04, 0x22, 0x1c, 0x00, 0x00, // $13 S
    0x00, 0x00, 0x00, 0x3e, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00, 0x00, // $14 T
    0x00, 0x00, 0x00, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x1c, 0x00, 0x00, // $15 U
    0x00, 0x00, 0x00, 0x22, 0x22, 0x22, 0x14, 0x14, 0x08, 0x08, 0x00, 0x00, // $16 V
    0x00, 0x00, 0x00, 0x22, 0x22, 0x22, 0x22, 0x2a, 0x36, 0x22, 0x00, 0x00, // $17 W
    0x00, 0x00, 0x00, 0x22, 0x22, 0x14, 0x08, 0x14, 0x22, 0x22, 0x00, 0x00, // $18 X
    0x00, 0x00, 0x00, 0x22, 0x22, 0x14, 0x08, 0x08, 0x08, 0x08, 0x00, 0x00, // $19 Y
    0x00, 0x00, 0x00, 0x3e, 0x02, 0x04, 0x08, 0x10, 0x20, 0x3e, 0x00, 0x00, // $1A Z
    0x00, 0x00, 0x00, 0x38, 0x20, 0x20, 0x20, 0x20, 0x20, 0x38, 0x00, 0x00, // $1B [
    0x00, 0x00, 0x00, 0x20, 0x20, 0x10, 0x08, 0x04, 0x02, 0x02, 0x00, 0x00, // $1C \
    0x00, 0x00, 0x00, 0x0e, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0e, 0x00, 0x00, // $1D ]
    0x00, 0x00, 0x00, 0x08, 0x1c, 0x2a, 0x08, 0x08, 0x08, 0x08, 0x00, 0x00, // $1E up arrow
    0x00, 0x00, 0x00, 0x00, 0x08, 0x10, 0x3e, 0x10, 0x08, 0x00, 0x00, 0x00, // $1F left arrow
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // $20 space
    0x00, 0x00, 0x00, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00, 0x08, 0x00, 0x00, // $21 !
    0x00, 0x00, 0x00, 0x14, 0x14, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // $22 "
    0x00, 0x00, 0x00, 0x14, 0x14, 0x36, 0x00, 0x36, 0x14, 0x14, 0x00, 0x00, // $23 #
    0x00, 0x00, 0x00, 0x08, 0x1e, 0x20, 0x1c, 0x02, 0x3c, 0x08, 0x00, 0x00, // $24 $
    0x00, 0x00, 0x00, 0x32, 0x32, 0x04, 0x08, 0x10, 0x26, 0x26, 0x00, 0x00, // $25 %
    0x00, 0x00, 0x00, 0x10, 0x28, 0x28, 0x10, 0x2a, 0x24, 0x1a, 0x00, 0x00, // $26 &
    0x00, 0x00, 0x00, 0x18, 0x18, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // $27 '
    0x00, 0x00, 0x00, 0x08, 0x10, 0x20, 0x20, 0x20, 0x10, 0x08, 0x00, 0x00, // $28 (
    0x00, 0x00, 0x00, 0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08, 0x00, 0x00, // $29 )
    0x00, 0x00, 0x00, 0x00, 0x08, 0x1c, 0x3e, 0x1c, 0x08, 0x00, 0x00, 0x00, // $2A *
    0x00, 0x00, 0x00, 0x00, 0x08, 0x08, 0x3e, 0x08, 0x08, 0x00, 0x00, 0x00, // $2B +
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x30, 0x10, 0x20, 0x00, 0x00, // $2C ,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x00, 0x00, 0x00, 0x00, 0x00, // $2D -
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x30, 0x00, 0x00, // $2E .
    0x00, 0x00, 0x00, 0x02, 0x02, 0x04, 0x08, 0x10, 0x20, 0x20, 0x00, 0x00, // $2F /
    0x00, 0x00, 0x00, 0x18, 0x24, 0x24, 0x24, 0x24, 0x24, 0x18, 0x00, 0x00, // $30 0
    0x00, 0x00, 0x00, 0x08, 0x18, 0x08, 0x08, 0x08, 0x08, 0x1c, 0x00, 0x00, // $31 1
    0x00, 0x00, 0x00, 0x1c, 0x22, 0x02, 0x1c, 0x20, 0x20, 0x3e, 0x00, 0x00, // $32 2
    0x00, 0x00, 0x00, 0x1c, 0x22, 0x02, 0x0c, 0x02, 0x22, 0x1c, 0x00, 0x00, // $33 3
    0x00, 0x00, 0x00, 0x04, 0x0c, 0x14, 0x3e, 0x04, 0x04, 0x04, 0x00, 0x00, // $34 4
    0x00, 0x00, 0x00, 0x3e, 0x20, 0x3c, 0x02, 0x02, 0x22, 0x1c, 0x00, 0x00, // $35 5
    0x00, 0x00, 0x00, 0x1c, 0x20, 0x20, 0x3c, 0x22, 0x22, 0x1c, 0x00, 0x00, // $36 6
    0x00, 0x00, 0x00, 0x3e, 0x02, 0x04, 0x08, 0x10, 0x20, 0x20, 0x00, 0x00, // $37 7
    0x00, 0x00, 0x00, 0x1c, 0x22, 0x22, 0x1c, 0x22, 0x22, 0x1c, 0x00, 0x00, // $38 8
    0x00, 0x00, 0x00, 0x1c, 0x22, 0x22, 0x1e, 0x02, 0x02, 0x1c, 0x00, 0x00, // $39 9
    0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00, 0x00, // $3A :
    0x00, 0x00, 0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x08, 0x10, 0x00, 0x00, // $3B ;
    0x00, 0x00, 0x00, 0x04, 0x08, 0x10, 0x20, 0x10, 0x08, 0x04, 0x00, 0x00, // $3C <
    0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x00, 0x3e, 0x00, 0x00, 0x00, 0x00, // $3D =
    0x00, 0x00, 0x00, 0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10, 0x00, 0x00, // $3E >
    0x00, 0x00, 0x00, 0x18, 0x24, 0x04, 0x08, 0x08, 0x00, 0x08, 0x00, 0x00, // $3F ?
];

#[cfg(test)]
mod tests {
    use super::*;

    const SPECIMEN: &[u8] = include_bytes!("../../assets/mc6847-character-set.png");

    fn glyph(code: usize) -> &'static [u8] {
        &FONT[code * GLYPH_HEIGHT..(code + 1) * GLYPH_HEIGHT]
    }

    /// Draw a glyph as text, to compare against what a person can see.
    fn picture(code: usize) -> Vec<String> {
        glyph(code)
            .iter()
            .map(|&row| {
                (0..8)
                    .map(|x| if row & (0x80 >> x) != 0 { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_table_is_what_the_specimen_image_and_the_corrections_give() {
        let made = glyphs_for_the_atom(SPECIMEN).expect("the specimen reads");
        assert_eq!(made.len(), FONT.len());
        assert!(
            made == FONT,
            "run `cargo run --example make_font` and update FONT"
        );
    }

    #[test]
    fn every_correction_really_differs_from_the_specimen_and_is_in_the_pattern_area() {
        let specimen = glyphs_from_specimen(SPECIMEN).unwrap();
        for &(code, scanline, row) in &CORRECTIONS {
            assert!((3..=9).contains(&scanline), "scanline {scanline}");
            assert_eq!(row & !0x3E, 0, "the byte strays outside columns 2-6");
            assert_ne!(
                specimen[code * GLYPH_HEIGHT + scanline],
                row,
                "code {code} scanline {scanline} already agrees with the specimen: drop the correction"
            );
            assert_eq!(FONT[code * GLYPH_HEIGHT + scanline], row);
        }
    }

    /// Everything else in the table is exactly what the specimen shows.
    #[test]
    fn only_the_corrected_bytes_differ_from_the_specimen() {
        let specimen = glyphs_from_specimen(SPECIMEN).unwrap();
        let differing: Vec<(usize, usize)> = (0..FONT.len())
            .filter(|&i| FONT[i] != specimen[i])
            .map(|i| (i / GLYPH_HEIGHT, i % GLYPH_HEIGHT))
            .collect();
        let corrected: Vec<(usize, usize)> = CORRECTIONS.iter().map(|&(c, s, _)| (c, s)).collect();
        assert_eq!(differing, corrected);
    }

    // ---- Checks that do not depend on the image ---------------------------------------

    #[test]
    fn every_pattern_sits_where_the_data_sheet_puts_it() {
        // Two columns in from the left of the 8-wide cell, five wide: bits 5-1.
        // Three rows down, seven high: scanlines 3-9.
        for code in 0..64 {
            for (scanline, &row) in glyph(code).iter().enumerate() {
                assert_eq!(
                    row & !0x3E,
                    0,
                    "code {code} scanline {scanline} strays outside columns 2-6"
                );
                if !(3..=9).contains(&scanline) {
                    assert_eq!(row, 0, "code {code} scanline {scanline} should be blank");
                }
            }
        }
    }

    #[test]
    fn only_the_space_is_blank_and_no_two_characters_look_alike() {
        for code in 0..64 {
            let blank = glyph(code).iter().all(|&row| row == 0);
            assert_eq!(blank, code == 32, "code {code}");
        }
        for a in 0..64 {
            for b in (a + 1)..64 {
                assert_ne!(glyph(a), glyph(b), "codes {a} and {b} are the same");
            }
        }
    }

    /// The code of a character, in the data sheet's order (Figure 20).
    fn code_of(c: char) -> usize {
        match c {
            '@'..='_' => c as usize - 0x40,
            ' '..='?' => c as usize,
            _ => panic!("{c} is not in the character set"),
        }
    }

    /// The glyphs that appear in a screenshot of a real Acorn Atom (its start-up
    /// banner `ACORN ATOM`, `ERROR 94`, and a typed line `>10 PRINT"THIS`,
    /// `>LIST`), compared with it by eye when the font was made. Each is the
    /// seven scanlines (3 to 9) of the pattern, in the 8-pixel cell.
    #[test]
    fn the_glyphs_match_what_a_real_atom_shows() {
        let seen: [(char, [&str; 7]); 20] = [
            (
                'A',
                [
                    "....#...", "...#.#..", "..#...#.", "..#...#.", "..#####.", "..#...#.",
                    "..#...#.",
                ],
            ),
            (
                'C',
                [
                    "...###..", "..#...#.", "..#.....", "..#.....", "..#.....", "..#...#.",
                    "...###..",
                ],
            ),
            (
                'O',
                [
                    "..#####.", "..#...#.", "..#...#.", "..#...#.", "..#...#.", "..#...#.",
                    "..#####.",
                ],
            ),
            (
                'R',
                [
                    "..####..", "..#...#.", "..#...#.", "..####..", "..#.#...", "..#..#..",
                    "..#...#.",
                ],
            ),
            (
                'N',
                [
                    "..#...#.", "..##..#.", "..#.#.#.", "..#..##.", "..#...#.", "..#...#.",
                    "..#...#.",
                ],
            ),
            (
                'T',
                [
                    "..#####.", "....#...", "....#...", "....#...", "....#...", "....#...",
                    "....#...",
                ],
            ),
            (
                'M',
                [
                    "..#...#.", "..##.##.", "..#.#.#.", "..#.#.#.", "..#...#.", "..#...#.",
                    "..#...#.",
                ],
            ),
            (
                'E', // (the middle bar is four dots: see CORRECTIONS)
                [
                    "..#####.", "..#.....", "..#.....", "..####..", "..#.....", "..#.....",
                    "..#####.",
                ],
            ),
            (
                'P',
                [
                    "..####..", "..#...#.", "..#...#.", "..####..", "..#.....", "..#.....",
                    "..#.....",
                ],
            ),
            (
                'H',
                [
                    "..#...#.", "..#...#.", "..#...#.", "..#####.", "..#...#.", "..#...#.",
                    "..#...#.",
                ],
            ),
            (
                'I',
                [
                    "...###..", "....#...", "....#...", "....#...", "....#...", "....#...",
                    "...###..",
                ],
            ),
            (
                'S',
                [
                    "...###..", "..#...#.", "...#....", "....#...", ".....#..", "..#...#.",
                    "...###..",
                ],
            ),
            (
                'L',
                [
                    "..#.....", "..#.....", "..#.....", "..#.....", "..#.....", "..#.....",
                    "..#####.",
                ],
            ),
            (
                '9',
                [
                    "...###..", "..#...#.", "..#...#.", "...####.", "......#.", "......#.",
                    "...###..",
                ],
            ),
            (
                '4',
                [
                    ".....#..", "....##..", "...#.#..", "..#####.", ".....#..", ".....#..",
                    ".....#..",
                ],
            ),
            (
                '1',
                [
                    "....#...", "...##...", "....#...", "....#...", "....#...", "....#...",
                    "...###..",
                ],
            ),
            (
                '0',
                [
                    "...##...", "..#..#..", "..#..#..", "..#..#..", "..#..#..", "..#..#..",
                    "...##...",
                ],
            ),
            (
                '"',
                [
                    "...#.#..", "...#.#..", "...#.#..", "........", "........", "........",
                    "........",
                ],
            ),
            (
                '>',
                [
                    "...#....", "....#...", ".....#..", "......#.", ".....#..", "....#...",
                    "...#....",
                ],
            ),
            (' ', ["........"; 7]),
        ];
        for (character, expected) in seen {
            assert_eq!(&picture(code_of(character))[3..10], expected, "{character}");
        }
    }

    #[test]
    fn letters_that_are_symmetrical_are_symmetrical() {
        // Reflect a row within the five columns (bits 5..1) and compare.
        let mirror = |row: u8| (0..5).fold(0, |acc, i| acc | (((row >> (1 + i)) & 1) << (5 - i)));
        // H, I, O, T, U, V, W, X, Y, A, M, !, +, 8, 0 are all left-right symmetric.
        for letter in "HIOTUVWXYAM".chars() {
            let code = letter as usize - 0x40;
            for &row in glyph(code) {
                assert_eq!(mirror(row), row, "{letter} is not symmetrical");
            }
        }
        // F, L, P and R are not.
        for letter in "FLPR".chars() {
            let code = letter as usize - 0x40;
            assert!(
                glyph(code).iter().any(|&row| mirror(row) != row),
                "{letter}"
            );
        }
    }

    #[test]
    fn the_character_order_is_the_data_sheets() {
        // Figure 20: row 0_ is @ A ... O, 1_ is P ... Z [ \ ] up left, 2_ is
        // space ! " # $ % & ' ( ) * + , - . / and 3_ is 0-9 : ; < = > ?.
        // The glyphs above were checked by shape, so check the places of a few
        // others by what must be true of them.
        let lit_rows = |c: char| {
            picture(code_of(c))
                .iter()
                .filter(|r| r.contains('#'))
                .count()
        };
        assert_eq!(
            lit_rows('!'),
            6,
            "a stem of five dots, a gap, then the point"
        );
        assert_eq!(lit_rows(' '), 0);
        assert!(
            picture(code_of('-'))
                .iter()
                .filter(|r| r.contains('#'))
                .count()
                == 1,
            "a dash is one row"
        );
        assert!(
            picture(code_of('.'))
                .iter()
                .filter(|r| r.contains('#'))
                .count()
                <= 2,
            "a point is small"
        );
        // Letters and digits: a letter's code is its place in the alphabet.
        assert_eq!(code_of('A'), 1);
        assert_eq!(code_of('Z'), 26);
        assert_eq!(code_of('0'), 0x30);
        assert_eq!(code_of('?'), 0x3F);
    }

    // ---- The image reader --------------------------------------------------------------

    fn png_of(width: u32, height: u32, grey: impl Fn(u32, u32) -> u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let data: Vec<u8> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| grey(x, y))
            .collect();
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();
        bytes
    }

    #[test]
    fn an_image_of_the_wrong_size_is_refused() {
        let image = png_of(10, 10, |_, _| 255);
        assert!(
            glyphs_from_specimen(&image)
                .unwrap_err()
                .contains("should be 672 x 60")
        );
    }

    #[test]
    fn what_is_not_a_png_is_refused() {
        assert!(glyphs_from_specimen(b"not an image").is_err());
        assert!(glyphs_from_specimen(&[]).is_err());
    }

    #[test]
    fn a_blank_specimen_gives_a_blank_font() {
        let image = png_of(672, 60, |_, _| 255);
        let font = glyphs_from_specimen(&image).unwrap();
        assert!(font.iter().all(|&b| b == 0));
    }

    #[test]
    fn a_dot_that_is_not_a_clean_three_by_three_block_is_refused() {
        // One stray black pixel in an otherwise white image.
        let image = png_of(672, 60, |x, y| if (x, y) == (4, 4) { 0 } else { 255 });
        assert!(
            glyphs_from_specimen(&image)
                .unwrap_err()
                .contains("not a clean 3 x 3 block")
        );
    }

    #[test]
    fn a_lit_dot_in_the_margin_is_refused() {
        // A whole 3 x 3 block, but in the blank column left of a pattern.
        let image = png_of(
            672,
            60,
            |x, y| if x < 3 && (3..6).contains(&y) { 0 } else { 255 },
        );
        assert!(
            glyphs_from_specimen(&image)
                .unwrap_err()
                .contains("outside the glyph patterns")
        );
    }

    #[test]
    fn one_lit_dot_lands_in_the_right_bit_and_row() {
        // The first pattern dot of the bottom band's first cell: code 0, the
        // top-left dot of the 5 x 7 pattern: scanline 3, bit 5 ($20).
        let (x0, y0) = (3, 33); // pixel of that dot's top-left corner
        let image = png_of(672, 60, |x, y| {
            if (x0..x0 + 3).contains(&x) && (y0..y0 + 3).contains(&y) {
                0
            } else {
                255
            }
        });
        let font = glyphs_from_specimen(&image).unwrap();
        assert_eq!(font[3], 0x20);
        assert_eq!(font.iter().filter(|&&b| b != 0).count(), 1);
    }
}
