//! Turning video RAM into pixels, the way the 6847 does it.
//!
//! The picture is 256 x 192. Each scanline is drawn at the moment the
//! [`Vdg`](super::vdg6847::Vdg) reaches it, from whatever video RAM and mode
//! bits hold at that time, so tricks that change mode part-way down the
//! screen work.
//!
//! Pixels are stored as palette indices; [`PALETTE`] turns them into colours.

use super::font::{FONT, GLYPH_HEIGHT};
use super::vdg6847::{VISIBLE_LINES, VideoMode};

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = VISIBLE_LINES as usize;

/// Start of video RAM; the display always begins here.
const VIDEO_RAM_START: usize = 0x8000;

/// Palette indices (the Atom's 6847 colours plus black).
pub mod colour {
    pub const BLACK: u8 = 0;
    pub const GREEN: u8 = 1;
    pub const YELLOW: u8 = 2;
    pub const BLUE: u8 = 3;
    pub const RED: u8 = 4;
    pub const BUFF: u8 = 5;
    pub const CYAN: u8 = 6;
    pub const MAGENTA: u8 = 7;
    pub const ORANGE: u8 = 8;
}
use colour::*;

/// Red, green, blue for each palette index.
pub const PALETTE: [[u8; 3]; 9] = [
    [0, 0, 0],       // black
    [0, 255, 0],     // green
    [255, 255, 0],   // yellow
    [0, 0, 255],     // blue
    [255, 0, 0],     // red
    [255, 255, 255], // buff
    [0, 255, 255],   // cyan
    [255, 0, 255],   // magenta
    [255, 129, 0],   // orange
];

/// The four colours selectable by two bits, for each of the two colour sets.
const COLOURS_BY_SET: [[u8; 4]; 2] = [[GREEN, YELLOW, BLUE, RED], [BUFF, CYAN, MAGENTA, ORANGE]];

/// 256 x 192 palette indices.
#[derive(Clone)]
pub struct Framebuffer {
    pub pixels: Vec<u8>,
}

impl Framebuffer {
    pub fn new() -> Self {
        Framebuffer {
            pixels: vec![BLACK; WIDTH * HEIGHT],
        }
    }

    /// Convert to RGBA bytes (4 per pixel), ready to upload as a texture.
    pub fn to_rgba(&self) -> Vec<u8> {
        self.pixels
            .iter()
            .flat_map(|&index| {
                let [r, g, b] = PALETTE[index as usize];
                [r, g, b, 255]
            })
            .collect()
    }

    /// Draw one scanline (0-191) from video RAM.
    ///
    /// `colour_set` is the 8255 port C "CSS" bit, which picks green/yellow/blue/red
    /// (false) or buff/cyan/magenta/orange (true) for coloured pixels.
    pub fn render_line(&mut self, line: usize, ram: &[u8], mode: VideoMode, colour_set: bool) {
        let row = &mut self.pixels[line * WIDTH..(line + 1) * WIDTH];
        let set = colour_set as usize;
        match mode {
            VideoMode::Text => render_text_line(row, line, ram, set),
            VideoMode::Graphics {
                width,
                height,
                colours,
            } => render_graphics_line(row, line, ram, width, height, colours, set),
        }
    }
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Text mode: 32 characters of 8 x 12 pixels per row, 16 rows.
fn render_text_line(row: &mut [u8], line: usize, ram: &[u8], set: usize) {
    const COLUMNS: usize = 32;
    let char_row = line / GLYPH_HEIGHT;
    let scanline_in_char = line % GLYPH_HEIGHT;
    let row_start = VIDEO_RAM_START + char_row * COLUMNS;

    // In text mode the colour set chooses green or orange characters.
    let ink = if set == 0 { GREEN } else { ORANGE };

    for column in 0..COLUMNS {
        let code = ram[row_start + column];
        let cell = &mut row[column * 8..column * 8 + 8];
        if code & 0x40 != 0 {
            render_semigraphics_cell(cell, code, scanline_in_char, set);
        } else {
            let glyph_row = FONT[(code & 0x3F) as usize * GLYPH_HEIGHT + scanline_in_char];
            let inverse = code & 0x80 != 0;
            for (x, pixel) in cell.iter_mut().enumerate() {
                let lit = (glyph_row >> (7 - x)) & 1 != 0;
                *pixel = if lit != inverse { ink } else { BLACK };
            }
        }
    }
}

/// Semigraphics 6: the 8 x 12 cell is a 2 x 3 grid of blocks, one bit each
/// (bits 5-0, top-left first). Bits 7-6 pick the colour.
fn render_semigraphics_cell(cell: &mut [u8], code: u8, scanline_in_char: usize, set: usize) {
    let colour = COLOURS_BY_SET[set][(code >> 6) as usize];
    let block_row = scanline_in_char / 4; // 0, 1 or 2
    let bits = (code >> (4 - block_row * 2)) & 3; // this block row's two bits
    let left = if bits & 2 != 0 { colour } else { BLACK };
    let right = if bits & 1 != 0 { colour } else { BLACK };
    cell[..4].fill(left);
    cell[4..].fill(right);
}

/// Pixel graphics. Each byte holds 8 two-colour pixels or 4 four-colour
/// pixels, most significant bits first; pixels and lines are repeated to fill
/// the 256 x 192 picture.
fn render_graphics_line(
    row: &mut [u8],
    line: usize,
    ram: &[u8],
    width: u32,
    height: u32,
    colours: u32,
    set: usize,
) {
    let bits_per_pixel = if colours == 2 { 1 } else { 2 };
    let pixels_per_byte = 8 / bits_per_pixel;
    let bytes_per_row = width as usize / pixels_per_byte;
    let pixel_width = WIDTH / width as usize;
    let lines_per_row = HEIGHT / height as usize;

    let row_start = VIDEO_RAM_START + (line / lines_per_row) * bytes_per_row;
    let mut x = 0;
    for byte_index in 0..bytes_per_row {
        let mut byte = ram[row_start + byte_index];
        for _ in 0..pixels_per_byte {
            let value = byte >> (8 - bits_per_pixel);
            byte <<= bits_per_pixel;
            let colour = if colours == 2 {
                // Two colours: black and green (set 0) or buff (set 1).
                if value == 1 {
                    COLOURS_BY_SET[set][0]
                } else {
                    BLACK
                }
            } else {
                COLOURS_BY_SET[set][value as usize]
            };
            row[x..x + pixel_width].fill(colour);
            x += pixel_width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ram_with(bytes: &[(usize, u8)]) -> Vec<u8> {
        let mut ram = vec![0; 0x10000];
        for &(addr, value) in bytes {
            ram[addr] = value;
        }
        ram
    }

    #[test]
    fn text_character_a_is_drawn_from_the_font() {
        let ram = ram_with(&[(0x8000, 0x01)]); // 'A'
        let mut frame = Framebuffer::new();
        // The 'A' glyph's scanline 4 is 0x14: pixels 3 and 5 of the cell.
        frame.render_line(4, &ram, VideoMode::Text, false);
        let row = &frame.pixels[4 * WIDTH..4 * WIDTH + 8];
        assert_eq!(
            row,
            &[BLACK, BLACK, BLACK, GREEN, BLACK, GREEN, BLACK, BLACK]
        );
    }

    #[test]
    fn inverse_video_swaps_ink_and_background() {
        let ram = ram_with(&[(0x8000, 0x81)]);
        let mut frame = Framebuffer::new();
        frame.render_line(4, &ram, VideoMode::Text, false);
        let row = &frame.pixels[4 * WIDTH..4 * WIDTH + 8];
        assert_eq!(
            row,
            &[GREEN, GREEN, GREEN, BLACK, GREEN, BLACK, GREEN, GREEN]
        );
    }

    #[test]
    fn semigraphics_blocks_fill_half_cells() {
        // Bit 6 set = semigraphics; colour bits 7-6 = 01 (yellow in set 0).
        // Top row of blocks: left on (bit 5), right off.
        let ram = ram_with(&[(0x8000, 0x40 | 0b10_0000)]);
        let mut frame = Framebuffer::new();
        frame.render_line(0, &ram, VideoMode::Text, false);
        assert_eq!(&frame.pixels[0..4], &[YELLOW; 4]);
        assert_eq!(&frame.pixels[4..8], &[BLACK; 4]);
    }

    #[test]
    fn high_resolution_graphics_use_one_bit_per_pixel() {
        let ram = ram_with(&[(0x8000, 0b1010_0000), (0x8020, 0xFF)]);
        let mode = VideoMode::Graphics {
            width: 256,
            height: 192,
            colours: 2,
        };
        let mut frame = Framebuffer::new();
        frame.render_line(0, &ram, mode, false);
        assert_eq!(&frame.pixels[0..4], &[GREEN, BLACK, GREEN, BLACK]);
        frame.render_line(1, &ram, mode, false);
        assert_eq!(&frame.pixels[WIDTH..WIDTH + 2], &[GREEN, GREEN]);
    }

    #[test]
    fn low_resolution_graphics_repeat_pixels_and_lines() {
        // 64 x 64, 4 colours: each pixel is 4 wide and 3 lines tall.
        let ram = ram_with(&[(0x8000, 0b01_00_00_00), (0x8010, 0b11_00_00_00)]);
        let mode = VideoMode::Graphics {
            width: 64,
            height: 64,
            colours: 4,
        };
        let mut frame = Framebuffer::new();
        for line in 0..6 {
            frame.render_line(line, &ram, mode, false);
        }
        assert_eq!(&frame.pixels[0..4], &[YELLOW; 4]);
        assert_eq!(&frame.pixels[4..8], &[GREEN; 4]);
        // Line 2 repeats the first row; line 3 starts the next row (RAM + 16).
        assert_eq!(frame.pixels[2 * WIDTH], YELLOW);
        assert_eq!(frame.pixels[3 * WIDTH], RED);
    }
}
