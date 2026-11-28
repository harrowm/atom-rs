//! The 6847 character generator: 64 characters, each 12 scanlines of 8 pixels.
//!
//! Character code `n` (0-63) starts at byte `n * 12`. In each byte bit 7 is the
//! leftmost pixel. Codes 0-31 are `@A-Z[\]` then an up arrow and a left arrow,
//! and 32-63 are space, the punctuation and the digits (the order of the
//! MC6847 data sheet's Figure 20).
//!
//! # Where this data comes from
//!
//! The table below is read **by a program from a photograph of the chip's ROM**,
//! and is not copied from another emulator.
//!
//! * **The glyph bits.** Wikipedia's article on the Motorola 6847 has a
//!   diagram, `File:MC6847_Character_ROM_memory_organization.png` by Robcfg
//!   (CC BY-SA 4.0), which is a die photograph of the character ROM array with a
//!   white dot on every set bit. The array has one strip per character, and
//!   five blocks of seven rows: one block for each of a glyph's five columns,
//!   one row for each of its seven bits. [`glyphs_from_die_diagram`] finds the
//!   dots and turns them into glyphs (character 0, the `@`, is shown with
//!   coloured dots and a key, and is read from those). Run by
//!   `cargo run --example make_font`; the test in `tests/font_source.rs`
//!   checks the table against the photograph (it is downloaded by
//!   `scripts/fetch_font_source.sh`; it is not part of this repository).
//! * **The size and placement** come from the Motorola MC6847/MC6847Y data
//!   sheet, page 18: the characters are "in a 5 x 7 format", each character
//!   element is "8 half periods by 12 scan lines", and "the 5 x 7 character
//!   font is positioned two columns to the right and three rows down within
//!   the 8 x 12 character element". So the glyph's rows go in scanlines 3-9
//!   and its columns in pixels 2-6.
//! * **The order of the characters** is the data sheet's Figure 20.
//!
//! The table is not edited by hand. It also agrees with a screenshot of a real
//! Acorn Atom (the shapes of `A C O R N T M E P H I S L 9 4 1 0 " >` were
//! compared, and the `E`'s four-dot middle bar measured), and with Atomulator's
//! table, which it matches exactly: they describe the same ROM.
//! An earlier version of this file was a copy of Atomulator's `video.c`.

pub const GLYPH_HEIGHT: usize = 12;

/// The scanline of the cell the 5 x 7 pattern starts on (the data sheet's
/// "three rows down"), its size, and the pixels in from the left of the
/// 8-wide cell ("two columns to the right").
const FIRST_SCANLINE: usize = 3;
const PATTERN_HEIGHT: usize = 7;
const PATTERN_WIDTH: usize = 5;
const LEFT_PIXELS: usize = 2;

// ---- Reading the die photograph -----------------------------------------------------

/// The diagram's size in pixels. The numbers below are for this picture.
const DIAGRAM_SIZE: (u32, u32) = (1500, 820);
/// The photograph has 35 rows (five blocks of seven bits: the top block is the
/// glyph's column 4, the bottom block its column 0) and, left to right, one
/// strip for each character from 1 to 63 except 32, the space, which has no
/// set bits and so no dots.
const ROWS: usize = PATTERN_WIDTH * PATTERN_HEIGHT;
const STRIPS: usize = 62;
const SPACE: usize = 32;
/// A pixel is part of a dot when its brightness is at least this (of 255).
const WHITE: f32 = 244.8;
/// What a dot looks like: its area in pixels and the size of its bounding box.
const DOT_AREA: std::ops::RangeInclusive<usize> = 30..=80;
const DOT_SIZE: std::ops::RangeInclusive<usize> = 7..=10;
/// The part of the picture holding the ROM array (the rest is the key and the
/// labels).
const ARRAY_X: std::ops::Range<f64> = 140.0..1345.0;
const ARRAY_Y: std::ops::Range<f64> = 20.0..780.0;
/// Two dots whose centres are closer than this are in the same row or strip.
const SAME_LINE: f64 = 4.0;
/// Character 0, the `@`, is shown with coloured dots at the left edge instead
/// of white ones, in the colours of the key beside the picture: one for each of
/// its five columns. This is where to look and how close a colour must be.
const KEY_COLOURS: [[u8; 3]; 5] = [
    [0, 15, 255],
    [14, 240, 0],
    [191, 191, 0],
    [255, 86, 0],
    [255, 0, 0],
];
const KEY_X: std::ops::Range<u32> = 153..176;
const KEY_TOLERANCE: f32 = 25.0;

/// Read the 64 glyphs out of the die photograph of the ROM (the contents of the
/// PNG file) and lay them out as [`FONT`] is: 12 bytes per character, the 5 x 7
/// pattern in scanlines 3-9 and pixels 2-6 of the 8 x 12 cell.
///
/// It checks the picture is what it should be (the size, 35 rows and 62 strips
/// of dots) and says so if not, rather than guess.
pub fn glyphs_from_die_diagram(png_file: &[u8]) -> Result<Vec<u8>, String> {
    let picture = Picture::decode(png_file)?;
    let dots = picture.white_dots();
    let rows = centres(dots.iter().map(|d| d.1));
    if rows.len() != ROWS {
        return Err(format!("found {} rows of dots, not {ROWS}", rows.len()));
    }
    let strips = centres(dots.iter().map(|d| d.0));
    if strips.len() != STRIPS {
        return Err(format!(
            "found {} strips of dots, not {STRIPS}",
            strips.len()
        ));
    }

    // bit[character][glyph column][glyph row]
    let mut bit = vec![[[false; PATTERN_HEIGHT]; PATTERN_WIDTH]; 64];
    let nearest = |centres: &[f64], at: f64| {
        (0..centres.len())
            .min_by(|&a, &b| (centres[a] - at).abs().total_cmp(&(centres[b] - at).abs()))
            .expect("there is at least one")
    };
    for &(x, y) in &dots {
        // Strips are the characters in order, with the space missing.
        let strip = nearest(&strips, x);
        let character = if strip + 1 < SPACE {
            strip + 1
        } else {
            strip + 2
        };
        let (block, row) = (
            nearest(&rows, y) / PATTERN_HEIGHT,
            nearest(&rows, y) % PATTERN_HEIGHT,
        );
        bit[character][PATTERN_WIDTH - 1 - block][row] = true;
    }
    for (index, &y) in rows.iter().enumerate() {
        let (block, row) = (index / PATTERN_HEIGHT, index % PATTERN_HEIGHT);
        if picture.key_coloured_dot_at(y) {
            bit[0][PATTERN_WIDTH - 1 - block][row] = true;
        }
    }

    let mut font = vec![0u8; 64 * GLYPH_HEIGHT];
    for (character, glyph) in bit.iter().enumerate() {
        for row in 0..PATTERN_HEIGHT {
            for (column, bits) in glyph.iter().enumerate() {
                if bits[row] {
                    // Bit 7 is the cell's leftmost pixel.
                    font[character * GLYPH_HEIGHT + FIRST_SCANLINE + row] |=
                        0x80 >> (LEFT_PIXELS + column);
                }
            }
        }
    }
    Ok(font)
}

/// The diagram as red, green and blue pixels (laid over black if it has any
/// transparency).
struct Picture {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 3]>,
}

impl Picture {
    fn decode(png_file: &[u8]) -> Result<Picture, String> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(png_file));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let mut data = vec![0; reader.output_buffer_size().ok_or("the image is too big")?];
        let info = reader.next_frame(&mut data).map_err(|e| e.to_string())?;
        if (info.width, info.height) != DIAGRAM_SIZE {
            return Err(format!(
                "the diagram should be {} x {} pixels, not {} x {}",
                DIAGRAM_SIZE.0, DIAGRAM_SIZE.1, info.width, info.height
            ));
        }
        let channels = info.color_type.samples();
        let pixels = data
            .chunks(channels)
            .take(info.width as usize * info.height as usize)
            .map(|p| {
                let (colour, alpha) = match channels {
                    1 => ([p[0]; 3], 255),
                    2 => ([p[0]; 3], p[1]),
                    3 => ([p[0], p[1], p[2]], 255),
                    _ => ([p[0], p[1], p[2]], p[3]),
                };
                colour.map(|c| (u16::from(c) * u16::from(alpha) / 255) as u8)
            })
            .collect();
        Ok(Picture {
            width: info.width as usize,
            height: info.height as usize,
            pixels,
        })
    }

    fn is_white(&self, x: usize, y: usize) -> bool {
        let [r, g, b] = self.pixels[y * self.width + x];
        0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b) >= WHITE
    }

    /// The centres of the white dots in the ROM array: groups of white pixels
    /// of about the right size and shape.
    fn white_dots(&self) -> Vec<(f64, f64)> {
        let mut seen = vec![false; self.width * self.height];
        let mut dots = Vec::new();
        for start in 0..seen.len() {
            if seen[start] || !self.is_white(start % self.width, start / self.width) {
                continue;
            }
            // Flood fill the group of white pixels touching this one.
            let (mut stack, mut count) = (vec![start], 0usize);
            let (mut sum_x, mut sum_y) = (0.0, 0.0);
            let (mut min, mut max) = ((usize::MAX, usize::MAX), (0, 0));
            seen[start] = true;
            while let Some(at) = stack.pop() {
                let (x, y) = (at % self.width, at / self.width);
                count += 1;
                sum_x += x as f64;
                sum_y += y as f64;
                min = (min.0.min(x), min.1.min(y));
                max = (max.0.max(x), max.1.max(y));
                let neighbours = [
                    (x + 1 < self.width).then(|| (x + 1, y)),
                    x.checked_sub(1).map(|x| (x, y)),
                    (y + 1 < self.height).then(|| (x, y + 1)),
                    y.checked_sub(1).map(|y| (x, y)),
                ];
                for (nx, ny) in neighbours.into_iter().flatten() {
                    let next = ny * self.width + nx;
                    if !seen[next] && self.is_white(nx, ny) {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
            let (w, h) = (max.0 - min.0 + 1, max.1 - min.1 + 1);
            let centre = (sum_x / count as f64, sum_y / count as f64);
            if DOT_AREA.contains(&count)
                && DOT_SIZE.contains(&w)
                && DOT_SIZE.contains(&h)
                && ARRAY_X.contains(&centre.0)
                && ARRAY_Y.contains(&centre.1)
            {
                dots.push(centre);
            }
        }
        dots
    }

    /// Is there a dot in one of the key's colours at the left edge, on the row
    /// at height `y`? (How the `@`'s set bits are shown.)
    fn key_coloured_dot_at(&self, y: f64) -> bool {
        let y = y.round() as i64;
        KEY_X.clone().any(|x| {
            (y - 2..=y + 2).any(|y| {
                let [r, g, b] = self.pixels[y as usize * self.width + x as usize];
                KEY_COLOURS.iter().any(|key| {
                    let distance = [r, g, b]
                        .iter()
                        .zip(key)
                        .map(|(&a, &b)| (f32::from(a) - f32::from(b)).powi(2))
                        .sum::<f32>()
                        .sqrt();
                    distance <= KEY_TOLERANCE
                })
            })
        })
    }
}

/// The centres of the rows (or strips) the values fall into, in order.
fn centres(values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut sorted: Vec<f64> = values.collect();
    sorted.sort_by(f64::total_cmp);
    let mut groups: Vec<Vec<f64>> = Vec::new();
    for value in sorted {
        match groups.last_mut() {
            Some(group) if value - group[group.len() - 1] <= SAME_LINE => group.push(value),
            _ => groups.push(vec![value]),
        }
    }
    groups
        .iter()
        .map(|g| g.iter().sum::<f64>() / g.len() as f64)
        .collect()
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
    0x00, 0x00, 0x00, 0x3e, 0x20, 0x20, 0x3c, 0x20, 0x20, 0x20, 0x00, 0x00, // $06 F
    0x00, 0x00, 0x00, 0x1e, 0x20, 0x20, 0x26, 0x22, 0x22, 0x1e, 0x00, 0x00, // $07 G
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
    0x00, 0x00, 0x00, 0x22, 0x22, 0x22, 0x2a, 0x2a, 0x36, 0x22, 0x00, 0x00, // $17 W
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

    /// What a phone video of a real Atom showed of `G` and `W` (see
    /// the video), as features that a blurry picture can still tell apart.
    #[test]
    fn g_and_w_match_the_video_of_a_real_atom() {
        let g = picture(code_of('G'));
        // A flat top: the top bar is four dots, starting one dot in from the
        // left stem, and the row under it is the stem alone, open at the right.
        assert_eq!(g[3], "...####.");
        assert_eq!(g[4], "..#.....");
        // The specimen's round-topped G would be "...###.." and "..#...#.".
        let w = picture(code_of('W'));
        // The centre stroke of the W is already there on the fourth row.
        assert_eq!(w[3 + 3], "..#.#.#.");
        // ...and not on the third.
        assert_eq!(w[3 + 2], "..#...#.");
        // F is E without its foot: the same top and middle bars.
        let f = picture(code_of('F'));
        assert_eq!(&f[3..9], &picture(code_of('E'))[3..9]);
        assert_eq!(f[9], "..#.....");
        // The E that the same video showed: the middle bar one dot shorter than the top.
        let e = picture(code_of('E'));
        assert_eq!(e[3], "..#####.");
        assert_eq!(e[6], "..####..");
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
                'E', // (the middle bar is four dots, as measured on a real Atom)
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

    // ---- The die photograph reader ----------------------------------------------------

    /// Draw a picture shaped like the die diagram: a black background, a white
    /// round dot (9 pixels across) for each set bit of characters 1-63, and the
    /// `@`'s bits as coloured squares at the left edge, as in the real one.
    /// `bits` lists `(character, glyph column, glyph row)`.
    fn diagram_with(bits: &[(usize, usize, usize)]) -> Vec<u8> {
        let (width, height) = (DIAGRAM_SIZE.0 as usize, DIAGRAM_SIZE.1 as usize);
        let mut rgba = vec![0u8; width * height * 4];
        for pixel in rgba.chunks_mut(4) {
            pixel[3] = 255;
        }
        let mut paint = |cx: f64, cy: f64, colour: [u8; 3], round: bool| {
            for y in (cy as i64 - 4)..=(cy as i64 + 4) {
                for x in (cx as i64 - 4)..=(cx as i64 + 4) {
                    let inside = !round || (x as f64 - cx).hypot(y as f64 - cy) <= 4.2;
                    if inside {
                        let at = (y as usize * width + x as usize) * 4;
                        rgba[at..at + 3].copy_from_slice(&colour);
                    }
                }
            }
        };
        // Rows: five blocks of seven, the top block being glyph column 4.
        let row_y = |column: usize, row: usize| {
            32.0 + (PATTERN_WIDTH - 1 - column) as f64 * 151.0 + row as f64 * 19.0
        };
        // Strips: characters 1-63 in order, with room for no strip for the space.
        let strip_x = |character: usize| {
            let strip = if character < SPACE {
                character - 1
            } else {
                character - 2
            };
            190.0 + strip as f64 * 18.5
        };
        for &(character, column, row) in bits {
            if character == 0 {
                paint(163.0, row_y(column, row), KEY_COLOURS[column], false);
            } else {
                paint(
                    strip_x(character),
                    row_y(column, row),
                    [255, 255, 255],
                    true,
                );
            }
        }
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rgba).unwrap();
        writer.finish().unwrap();
        bytes
    }

    /// The set bits of a font table, as `diagram_with` takes them.
    fn bits_of(font: &[u8]) -> Vec<(usize, usize, usize)> {
        let mut bits = Vec::new();
        for character in 0..64 {
            for row in 0..PATTERN_HEIGHT {
                for column in 0..PATTERN_WIDTH {
                    if font[character * GLYPH_HEIGHT + FIRST_SCANLINE + row]
                        & (0x80 >> (LEFT_PIXELS + column))
                        != 0
                    {
                        bits.push((character, column, row));
                    }
                }
            }
        }
        bits
    }

    #[test]
    fn a_font_drawn_as_a_die_diagram_is_read_back_exactly() {
        let picture = diagram_with(&bits_of(&FONT));
        assert_eq!(glyphs_from_die_diagram(&picture).unwrap(), FONT.to_vec());
    }

    /// Removing one dot from the full diagram must clear exactly that bit of
    /// exactly that character, and nothing else.
    #[test]
    fn each_dot_lands_in_the_right_character_column_row_and_bit() {
        let all = bits_of(&FONT);
        let base = glyphs_from_die_diagram(&diagram_with(&all)).unwrap();
        // The first and last set bit of characters at the edges of the strips
        // (the first, the ones either side of the missing space, the last).
        for character in [1usize, 31, 33, 63] {
            let own: Vec<(usize, usize, usize)> = all
                .iter()
                .copied()
                .filter(|&(c, _, _)| c == character)
                .collect();
            for &(_, column, row) in [own.first().unwrap(), own.last().unwrap()] {
                let mut fewer = all.clone();
                fewer.retain(|&b| b != (character, column, row));
                let font = glyphs_from_die_diagram(&diagram_with(&fewer)).unwrap();
                let changed: Vec<usize> = (0..font.len()).filter(|&i| font[i] != base[i]).collect();
                assert_eq!(
                    changed,
                    vec![character * GLYPH_HEIGHT + FIRST_SCANLINE + row],
                    "character {character} column {column} row {row}"
                );
                assert_eq!(
                    base[changed[0]] & !font[changed[0]],
                    0x80 >> (LEFT_PIXELS + column),
                    "the bit"
                );
            }
        }
    }

    #[test]
    fn the_at_sign_is_read_from_its_coloured_key_dots_and_not_confused_with_its_neighbour() {
        let mut bits = bits_of(&FONT);
        // The picture must still contain character 0's coloured dots.
        assert!(bits.iter().any(|&(c, _, _)| c == 0));
        let font = glyphs_from_die_diagram(&diagram_with(&bits)).unwrap();
        assert_eq!(&font[..GLYPH_HEIGHT], &FONT[..GLYPH_HEIGHT]);
        // Without them, the @ comes out blank and nothing else changes.
        bits.retain(|&(c, _, _)| c != 0);
        let font = glyphs_from_die_diagram(&diagram_with(&bits)).unwrap();
        assert!(font[..GLYPH_HEIGHT].iter().all(|&b| b == 0));
        assert_eq!(&font[GLYPH_HEIGHT..], &FONT[GLYPH_HEIGHT..]);
    }

    #[test]
    fn a_picture_of_the_wrong_size_or_kind_is_refused() {
        assert!(glyphs_from_die_diagram(b"not an image").is_err());
        assert!(glyphs_from_die_diagram(&[]).is_err());
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 10, 10);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255; 400]).unwrap();
        writer.finish().unwrap();
        assert!(
            glyphs_from_die_diagram(&bytes)
                .unwrap_err()
                .contains("should be 1500 x 820")
        );
    }

    #[test]
    fn a_picture_without_the_expected_rows_and_strips_is_refused_with_the_count() {
        let blank = diagram_with(&[]);
        assert!(
            glyphs_from_die_diagram(&blank)
                .unwrap_err()
                .contains("0 rows")
        );
        // Only one dot: one row and one strip, not 35 and 62.
        let one = diagram_with(&[(5, 1, 1)]);
        assert!(
            glyphs_from_die_diagram(&one)
                .unwrap_err()
                .contains("1 rows")
        );
    }

    #[test]
    fn rows_and_strips_are_found_by_grouping_nearby_positions() {
        assert_eq!(
            centres([1.0, 2.0, 3.0, 30.0, 31.0].into_iter()),
            vec![2.0, 30.5]
        );
        assert!(centres(std::iter::empty()).is_empty());
        assert_eq!(
            centres([5.0, 9.5].into_iter()),
            vec![5.0, 9.5],
            "more than 4 apart"
        );
        assert_eq!(
            centres([5.0, 9.0].into_iter()),
            vec![7.0],
            "4 apart is the same line"
        );
    }
}
