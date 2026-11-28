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

// The border, from the MC6847 datasheet (Figure 13, and the text on page 14).
// Above the 192 picture lines there are 25 border lines and below it 26. The
// width from blanking to blanking is 193.1 colour clocks, of which the picture
// takes 128; the 65.1 left over is 65 pixels (a colour clock is 2 pixels),
// shared equally between left and right so the picture stays centred. (The
// datasheet's own figure for the left offset, "283 periods", is a misprint:
// it is longer than the whole line.)
pub const BORDER_TOP: usize = 25;
pub const BORDER_BOTTOM: usize = 26;
pub const BORDER_SIDE: usize = 65;
/// The picture plus its border.
pub const FULL_WIDTH: usize = WIDTH + 2 * BORDER_SIDE;
pub const FULL_HEIGHT: usize = BORDER_TOP + HEIGHT + BORDER_BOTTOM;

/// Which row of the picture-with-border a scanline of the frame is, if it is
/// shown. Scanlines count from the top of the picture: 0-191 are the picture,
/// the 26 after it are the bottom border, then come the vertical retrace and
/// blanking (not shown), and the last 25 lines of the frame are the top
/// border, just before the picture starts again.
pub fn border_row(line: u32, lines_per_frame: u32) -> Option<usize> {
    let (line, lines_per_frame) = (line as usize, lines_per_frame as usize);
    if line < HEIGHT + BORDER_BOTTOM {
        Some(BORDER_TOP + line)
    } else if line >= lines_per_frame - BORDER_TOP {
        Some(line - (lines_per_frame - BORDER_TOP))
    } else {
        None
    }
}

/// The colour of the border for a video mode: black with text and
/// semigraphics, and in the graphics modes green, or buff (off-white) when the
/// colour set pin selects the second set.
pub fn border_colour(mode: VideoMode, colour_set: bool) -> u8 {
    match mode {
        VideoMode::Text => BLACK,
        VideoMode::Graphics { .. } => COLOURS_BY_SET[colour_set as usize][0],
    }
}

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
    /// The 6847's text background (colour set 0 and 1).
    pub const DARK_GREEN: u8 = 9;
    pub const DARK_ORANGE: u8 = 10;
}
use colour::*;

/// Red, green, blue for each palette index.
pub const PALETTE: [[u8; 3]; 11] = [
    [0, 0, 0],       // black
    [0, 255, 0],     // green
    [255, 255, 0],   // yellow
    [0, 0, 255],     // blue
    [255, 0, 0],     // red
    [255, 255, 255], // buff
    [0, 255, 255],   // cyan
    [255, 0, 255],   // magenta
    [255, 129, 0],   // orange
    [0, 130, 0],     // dark green (approximate: the 6847 gives no exact value)
    [130, 55, 0],    // dark orange
];

/// What the picture looks like without the colour board: shades of grey.
/// (Atomulator's monochrome palette: black, then everything lit between mid
/// and full brightness.)
pub const MONOCHROME_PALETTE: [[u8; 3]; 11] = [
    [32, 32, 32],    // black
    [220, 220, 220], // green
    [252, 252, 252], // yellow
    [128, 128, 128], // blue
    [128, 128, 128], // red
    [252, 252, 252], // buff
    [220, 220, 220], // cyan
    [220, 220, 220], // magenta
    [220, 220, 220], // orange
    [72, 72, 72],    // dark green
    [72, 72, 72],    // dark orange
];

/// The four colours selectable by two bits, for each of the two colour sets.
const COLOURS_BY_SET: [[u8; 4]; 2] = [[GREEN, YELLOW, BLUE, RED], [BUFF, CYAN, MAGENTA, ORANGE]];

/// 256 x 192 palette indices.
#[derive(Clone)]
pub struct Framebuffer {
    pub pixels: Vec<u8>,
    /// Show shades of grey: an Atom without the colour board.
    pub monochrome: bool,
    /// Show the border around the picture (Hardware menu).
    pub show_border: bool,
    /// The border colour of each row of the picture-with-border, as it was
    /// when the beam drew that row.
    border: [u8; FULL_HEIGHT],
}

impl Framebuffer {
    pub fn new() -> Self {
        Framebuffer {
            pixels: vec![BLACK; WIDTH * HEIGHT],
            monochrome: false,
            show_border: true,
            border: [BLACK; FULL_HEIGHT],
        }
    }

    /// The beam is at the start of scanline `line` (of `lines_per_frame`) in
    /// this mode: remember the border colour for that row.
    pub fn note_border(
        &mut self,
        line: u32,
        lines_per_frame: u32,
        mode: VideoMode,
        colour_set: bool,
    ) {
        if let Some(row) = border_row(line, lines_per_frame) {
            self.border[row] = border_colour(mode, colour_set);
        }
    }

    /// The size of the picture as shown: with the border if it is on.
    pub fn shown_size(&self) -> (usize, usize) {
        if self.show_border {
            (FULL_WIDTH, FULL_HEIGHT)
        } else {
            (WIDTH, HEIGHT)
        }
    }

    /// The picture as shown (see [`Framebuffer::shown_size`]), as RGBA bytes.
    pub fn to_rgba_shown(&self) -> Vec<u8> {
        if !self.show_border {
            return self.to_rgba();
        }
        let palette = self.palette();
        let mut rgba = Vec::with_capacity(FULL_WIDTH * FULL_HEIGHT * 4);
        let mut push = |index: u8| {
            let [r, g, b] = palette[index as usize];
            rgba.extend([r, g, b, 255]);
        };
        for (row, &border) in self.border.iter().enumerate() {
            let picture_row = row.checked_sub(BORDER_TOP).filter(|&r| r < HEIGHT);
            for x in 0..FULL_WIDTH {
                let picture_x = x.checked_sub(BORDER_SIDE).filter(|&c| c < WIDTH);
                match (picture_row, picture_x) {
                    (Some(r), Some(c)) => push(self.pixels[r * WIDTH + c]),
                    _ => push(border),
                }
            }
        }
        rgba
    }

    fn palette(&self) -> &'static [[u8; 3]; 11] {
        if self.monochrome {
            &MONOCHROME_PALETTE
        } else {
            &PALETTE
        }
    }

    /// Convert to RGBA bytes (4 per pixel), ready to upload as a texture.
    pub fn to_rgba(&self) -> Vec<u8> {
        let palette = self.palette();
        self.pixels
            .iter()
            .flat_map(|&index| {
                let [r, g, b] = palette[index as usize];
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
/// The RAM address the 6847 reads for fetch `slot` (0-31, left to right) of
/// picture line `line` in `mode`. A graphics mode that needs only 16 bytes a
/// line reads each byte twice over the 32 fetches.
pub fn fetch_address(mode: VideoMode, line: usize, slot: usize) -> usize {
    match mode {
        VideoMode::Text => VIDEO_RAM_START + (line / GLYPH_HEIGHT) * 32 + slot,
        VideoMode::Graphics {
            width,
            height,
            colours,
        } => {
            let pixels_per_byte = if colours == 2 { 8 } else { 4 };
            let bytes_per_row = width as usize / pixels_per_byte;
            let lines_per_row = HEIGHT / height as usize;
            VIDEO_RAM_START + (line / lines_per_row) * bytes_per_row + slot * bytes_per_row / 32
        }
    }
}

fn render_text_line(row: &mut [u8], line: usize, ram: &[u8], set: usize) {
    const COLUMNS: usize = 32;
    let char_row = line / GLYPH_HEIGHT;
    let scanline_in_char = line % GLYPH_HEIGHT;
    let row_start = VIDEO_RAM_START + char_row * COLUMNS;

    // In text mode the colour set chooses green or orange characters, on a
    // dark green or dark orange background (as on a real 6847; semigraphics
    // blocks have a black one).
    let (ink, background) = if set == 0 {
        (GREEN, DARK_GREEN)
    } else {
        (ORANGE, DARK_ORANGE)
    };

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
                *pixel = if lit != inverse { ink } else { background };
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
        // Green on the 6847's dark green background.
        let d = DARK_GREEN;
        assert_eq!(row, &[d, d, d, GREEN, d, GREEN, d, d]);
    }

    #[test]
    fn the_second_colour_set_is_orange_on_dark_orange() {
        let ram = ram_with(&[(0x8000, 0x01)]);
        let mut frame = Framebuffer::new();
        frame.render_line(4, &ram, VideoMode::Text, true);
        let row = &frame.pixels[4 * WIDTH..4 * WIDTH + 8];
        let d = DARK_ORANGE;
        assert_eq!(row, &[d, d, d, ORANGE, d, ORANGE, d, d]);
    }

    #[test]
    fn inverse_video_swaps_ink_and_background() {
        let ram = ram_with(&[(0x8000, 0x81)]);
        let mut frame = Framebuffer::new();
        frame.render_line(4, &ram, VideoMode::Text, false);
        let row = &frame.pixels[4 * WIDTH..4 * WIDTH + 8];
        let d = DARK_GREEN;
        assert_eq!(row, &[GREEN, GREEN, GREEN, d, GREEN, d, GREEN, GREEN]);
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

    #[test]
    fn without_the_colour_board_every_pixel_is_grey() {
        let mut frame = Framebuffer::new();
        for (i, pixel) in frame.pixels.iter_mut().enumerate() {
            *pixel = (i % 9) as u8; // every palette index
        }
        let coloured = frame.to_rgba();
        assert!(
            coloured.chunks(4).any(|p| p[0] != p[1] || p[1] != p[2]),
            "with the board there is colour"
        );
        frame.monochrome = true;
        assert!(
            frame
                .to_rgba()
                .chunks(4)
                .all(|p| p[0] == p[1] && p[1] == p[2]),
            "without it there are only greys"
        );
    }

    #[test]
    fn monochrome_still_tells_lit_pixels_from_dark_ones() {
        let mut frame = Framebuffer::new();
        frame.monochrome = true;
        frame.pixels[0] = BLACK;
        frame.pixels[1] = GREEN;
        let rgba = frame.to_rgba();
        assert!(rgba[4] > rgba[0] + 100, "green is much brighter than black");
    }

    // ---- The border -----------------------------------------------------------------

    #[test]
    fn the_datasheet_sizes_add_up() {
        assert_eq!((FULL_WIDTH, FULL_HEIGHT), (386, 243));
        // 193.1 colour clocks across, 2 pixels each: about 386.
        assert!((FULL_WIDTH as f64 - 193.1 * 2.0).abs() < 1.0);
    }

    #[test]
    fn scanlines_map_to_border_rows() {
        // NTSC, 262 lines: picture, 26 bottom border, 6 + 13 not shown, 25 top.
        let ntsc = |line| border_row(line, 262);
        assert_eq!(ntsc(0), Some(25), "first picture line");
        assert_eq!(ntsc(191), Some(216), "last picture line");
        assert_eq!(ntsc(192), Some(217), "first bottom border line");
        assert_eq!(ntsc(217), Some(242), "last bottom border line");
        assert_eq!(ntsc(218), None, "vertical retrace");
        assert_eq!(ntsc(236), None, "vertical blanking");
        assert_eq!(ntsc(237), Some(0), "first top border line");
        assert_eq!(ntsc(261), Some(24), "last top border line");
        // PAL has more lines, all of them blank, but the border is the same.
        let pal = |line| border_row(line, 312);
        assert_eq!(pal(217), Some(242));
        assert_eq!(pal(218), None);
        assert_eq!(pal(286), None);
        assert_eq!(pal(287), Some(0));
        assert_eq!(pal(311), Some(24));
        // Every shown row is reached exactly once.
        let mut seen = [0; FULL_HEIGHT];
        for line in 0..262 {
            if let Some(row) = ntsc(line) {
                seen[row] += 1;
            }
        }
        assert!(seen.iter().all(|&n| n == 1));
    }

    #[test]
    fn the_border_is_black_with_text_and_lit_with_graphics() {
        let graphics = VideoMode::Graphics {
            width: 256,
            height: 192,
            colours: 2,
        };
        assert_eq!(border_colour(VideoMode::Text, false), BLACK);
        assert_eq!(border_colour(VideoMode::Text, true), BLACK);
        assert_eq!(border_colour(graphics, false), GREEN);
        assert_eq!(border_colour(graphics, true), BUFF);
    }

    #[test]
    fn the_picture_sits_in_the_middle_of_its_border() {
        let mut fb = Framebuffer::new();
        fb.pixels.fill(RED);
        for line in 0..262 {
            fb.note_border(line, 262, VideoMode::Text, false);
        }
        fb.border[0] = GREEN; // the top row
        let rgba = fb.to_rgba_shown();
        assert_eq!(rgba.len(), FULL_WIDTH * FULL_HEIGHT * 4);
        let at = |x: usize, y: usize| {
            let i = (y * FULL_WIDTH + x) * 4;
            [rgba[i], rgba[i + 1], rgba[i + 2]]
        };
        assert_eq!(at(10, 0), PALETTE[GREEN as usize], "the top border row");
        assert_eq!(at(10, 100), PALETTE[BLACK as usize], "left border");
        assert_eq!(at(BORDER_SIDE - 1, 100), PALETTE[BLACK as usize]);
        assert_eq!(
            at(BORDER_SIDE, BORDER_TOP),
            PALETTE[RED as usize],
            "picture corner"
        );
        assert_eq!(
            at(BORDER_SIDE + WIDTH - 1, BORDER_TOP + HEIGHT - 1),
            PALETTE[RED as usize]
        );
        assert_eq!(
            at(BORDER_SIDE + WIDTH, 100),
            PALETTE[BLACK as usize],
            "right border"
        );
        assert_eq!(at(10, FULL_HEIGHT - 1), PALETTE[BLACK as usize], "bottom");
    }

    #[test]
    fn with_the_border_off_only_the_picture_is_shown() {
        let mut fb = Framebuffer::new();
        fb.show_border = false;
        assert_eq!(fb.shown_size(), (WIDTH, HEIGHT));
        assert_eq!(fb.to_rgba_shown(), fb.to_rgba());
        fb.show_border = true;
        assert_eq!(fb.shown_size(), (FULL_WIDTH, FULL_HEIGHT));
    }

    // ---- Every graphics mode, every pixel ---------------------------------------------------

    /// What pixel (x, y) of a graphics screen should be, worked out directly from
    /// the data sheet: the picture is `width` x `height` dots, each stretched to
    /// fill the 256 x 192 screen; in two-colour modes a byte is eight dots, one
    /// bit each, in four-colour modes four dots, two bits each, leftmost dot in
    /// the highest bits; and rows follow each other in memory from $8000.
    fn expected_pixel(
        ram: &[u8],
        (width, height, colours): (u32, u32, u32),
        set: usize,
        x: usize,
        y: usize,
    ) -> u8 {
        let (width, height) = (width as usize, height as usize);
        let bits_per_dot = if colours == 2 { 1 } else { 2 };
        let dots_per_byte = 8 / bits_per_dot;
        let (dot_x, dot_y) = (x / (256 / width), y / (192 / height));
        let byte = ram[0x8000 + dot_y * (width / dots_per_byte) + dot_x / dots_per_byte];
        let shift = 8 - bits_per_dot * (dot_x % dots_per_byte + 1);
        let value = (byte >> shift) & ((1 << bits_per_dot) - 1);
        match (colours, value) {
            (2, 0) => BLACK,
            (2, _) => COLOURS_BY_SET[set][0],
            _ => COLOURS_BY_SET[set][value as usize],
        }
    }

    /// 64K of RAM with an unpredictable (but repeatable) pattern in the screen.
    fn noisy_ram() -> Vec<u8> {
        let mut ram = vec![0u8; 0x10000];
        let mut seed = 0x1234_5678u32;
        for byte in &mut ram[0x8000..0x9800] {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = (seed >> 24) as u8;
        }
        ram
    }

    #[test]
    fn every_graphics_mode_draws_every_pixel_as_the_data_sheet_says() {
        let ram = noisy_ram();
        let modes = [
            (64, 64, 4),
            (128, 64, 2),
            (128, 64, 4),
            (128, 96, 2),
            (128, 96, 4),
            (128, 192, 2),
            (128, 192, 4),
            (256, 192, 2),
        ];
        for mode in modes {
            for colour_set in [false, true] {
                let mut frame = Framebuffer::new();
                let (width, height, colours) = mode;
                for line in 0..HEIGHT {
                    frame.render_line(
                        line,
                        &ram,
                        VideoMode::Graphics {
                            width,
                            height,
                            colours,
                        },
                        colour_set,
                    );
                }
                for y in 0..HEIGHT {
                    for x in 0..WIDTH {
                        assert_eq!(
                            frame.pixels[y * WIDTH + x],
                            expected_pixel(&ram, mode, colour_set as usize, x, y),
                            "{width}x{height}x{colours} set {colour_set} at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_64_by_64_four_colour_mode_clear_1a_stretches_each_dot_to_four_by_three_pixels() {
        // The mode no test drew before: the lowest resolution.
        let mut ram = vec![0u8; 0x10000];
        ram[0x8000] = 0b00_01_10_11; // four dots: green, yellow, blue, red (set 0)
        let mut frame = Framebuffer::new();
        let mode = VideoMode::Graphics {
            width: 64,
            height: 64,
            colours: 4,
        };
        for line in 0..HEIGHT {
            frame.render_line(line, &ram, mode, false);
        }
        let row = |y: usize| &frame.pixels[y * WIDTH..y * WIDTH + 16];
        let expected: Vec<u8> = [GREEN, YELLOW, BLUE, RED]
            .into_iter()
            .flat_map(|c| [c; 4])
            .collect();
        for y in 0..3 {
            assert_eq!(row(y), expected, "scanline {y}: a dot is 3 scanlines tall");
        }
        assert_eq!(
            row(3)[0],
            GREEN,
            "the next dot row starts at scanline 3, and the byte there is 0 = green"
        );
    }

    #[test]
    fn the_video_memory_of_each_graphics_mode_starts_at_8000() {
        for (width, height, colours) in [(64, 64, 4), (128, 192, 2), (256, 192, 2)] {
            let mut ram = vec![0u8; 0x10000];
            ram[0x8000] = 0xFF;
            ram[0x7FFF] = 0xFF; // just before the screen: must not appear
            let mut frame = Framebuffer::new();
            frame.render_line(
                0,
                &ram,
                VideoMode::Graphics {
                    width,
                    height,
                    colours,
                },
                false,
            );
            assert_ne!(
                frame.pixels[0], BLACK,
                "{width}x{height}x{colours}: the first byte is drawn"
            );
        }
    }
}
