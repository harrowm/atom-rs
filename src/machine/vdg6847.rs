//! The 6847 video display generator: timing and mode decoding.
//!
//! The chip fetches display bytes from video RAM and draws them, one
//! scanline at a time. This module tracks *where in the frame we are*, which
//! is what the CPU can observe (through the FS signal on 8255 port C). Turning
//! video RAM into pixels is done by the front end, from the same mode bits.
//!
//! Timing (Atomulator's NTSC model): 64 CPU cycles per scanline, 262 lines
//! per frame, so one frame is 16,768 cycles (about 59.6 Hz at 1 MHz).
//! Lines 0-191 show the picture. FS goes low for lines 192-223.

use serde::{Deserialize, Serialize};

pub const CYCLES_PER_LINE: u32 = 64;
pub const VISIBLE_LINES: u32 = 192;

/// The 6847 fetches the 32 bytes of a picture line during the display
/// window, which the datasheet puts at 128 of the 227.5 colour-clock periods
/// of a line (about 36 of the Atom's 64 CPU cycles), starting about 70
/// periods in (about cycle 20).
pub const FETCH_START_CYCLE: u32 = 20;
pub const FETCH_CYCLES: u32 = 36;
pub const FETCH_SLOTS: u32 = 32;

/// Which of the line's 32 fetches (counting from the left) is happening at
/// this cycle of the line, if the 6847 is fetching at all.
pub fn fetch_slot(cycle_in_line: u32) -> Option<usize> {
    let into_window = cycle_in_line.checked_sub(FETCH_START_CYCLE)?;
    (into_window < FETCH_CYCLES).then(|| (into_window * FETCH_SLOTS / FETCH_CYCLES) as usize)
}
const FS_LOW_RANGE: std::ops::Range<u32> = 192..224;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Standard {
    #[default]
    Ntsc,
    Pal,
}

impl Standard {
    pub fn lines_per_frame(self) -> u32 {
        match self {
            Standard::Ntsc => 262,
            Standard::Pal => 312,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Vdg {
    standard: Standard,
    line: u32,
    cycle_in_line: u32,
    /// Number of complete frames since power-on.
    pub frames: u64,
}

impl Vdg {
    pub fn new(standard: Standard) -> Self {
        Vdg {
            standard,
            ..Vdg::default()
        }
    }

    /// Advance one CPU cycle.
    pub fn tick(&mut self) {
        self.cycle_in_line += 1;
        if self.cycle_in_line == CYCLES_PER_LINE {
            self.cycle_in_line = 0;
            self.line += 1;
            if self.line == self.standard.lines_per_frame() {
                self.line = 0;
                self.frames += 1;
            }
        }
    }

    /// True on the first cycle of a scanline, when the picture line can be drawn.
    pub fn at_line_start(&self) -> bool {
        self.cycle_in_line == 0
    }

    /// How far through the current scanline we are, in CPU cycles.
    pub fn cycle_in_line(&self) -> u32 {
        self.cycle_in_line
    }

    /// The scanline currently being drawn (0 is the top of the picture).
    pub fn line(&self) -> u32 {
        self.line
    }

    /// Is the FS (field sync) output low?
    pub fn field_sync_low(&self) -> bool {
        FS_LOW_RANGE.contains(&self.line)
    }
}

/// The display mode, decoded from the high nibble of 8255 port A
/// (bit 0 = A/G, bits 1-3 = GM0-GM2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoMode {
    /// 32 x 16 characters (also used for the semigraphics blocks).
    Text,
    /// Pixel graphics: `width` x `height` pixels, `colours` colours.
    Graphics {
        width: u32,
        height: u32,
        colours: u32,
    },
}

impl VideoMode {
    pub fn from_port_a_high_nibble(bits: u8) -> Self {
        let (width, height, colours) = match bits & 0x0F {
            0x0 | 0x2 | 0x4 | 0x6 | 0x8 | 0xA | 0xC | 0xE => return VideoMode::Text,
            0x1 => (64, 64, 4),   // CLEAR 1a
            0x3 => (128, 64, 2),  // CLEAR 1
            0x5 => (128, 64, 4),  // CLEAR 2a
            0x7 => (128, 96, 2),  // CLEAR 2
            0x9 => (128, 96, 4),  // CLEAR 3a
            0xB => (128, 192, 2), // CLEAR 3
            0xD => (128, 192, 4), // CLEAR 4a
            _ => (256, 192, 2),   // 0xF: CLEAR 4
        };
        VideoMode::Graphics {
            width,
            height,
            colours,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_262_lines_of_64_cycles() {
        let mut vdg = Vdg::default();
        for _ in 0..(262 * 64 - 1) {
            vdg.tick();
        }
        assert_eq!(vdg.frames, 0);
        vdg.tick();
        assert_eq!(vdg.frames, 1);
        assert_eq!(vdg.line(), 0);
    }

    #[test]
    fn field_sync_is_low_on_lines_192_to_223() {
        let mut vdg = Vdg::default();
        let mut low_lines = vec![];
        for line in 0..262 {
            if vdg.field_sync_low() {
                low_lines.push(line);
            }
            for _ in 0..CYCLES_PER_LINE {
                vdg.tick();
            }
        }
        assert_eq!(low_lines, (192..224).collect::<Vec<_>>());
    }

    #[test]
    fn decodes_modes() {
        assert_eq!(VideoMode::from_port_a_high_nibble(0), VideoMode::Text);
        assert_eq!(
            VideoMode::from_port_a_high_nibble(0xF),
            VideoMode::Graphics {
                width: 256,
                height: 192,
                colours: 2
            }
        );
        assert_eq!(
            VideoMode::from_port_a_high_nibble(0x1),
            VideoMode::Graphics {
                width: 64,
                height: 64,
                colours: 4
            }
        );
    }

    #[test]
    fn the_fetches_happen_in_the_display_window_left_to_right() {
        assert_eq!(fetch_slot(0), None, "before the window");
        assert_eq!(fetch_slot(FETCH_START_CYCLE - 1), None);
        assert_eq!(fetch_slot(FETCH_START_CYCLE), Some(0), "the first byte");
        assert_eq!(
            fetch_slot(FETCH_START_CYCLE + FETCH_CYCLES - 1),
            Some(31),
            "the last"
        );
        assert_eq!(
            fetch_slot(FETCH_START_CYCLE + FETCH_CYCLES),
            None,
            "after the window"
        );
        assert_eq!(fetch_slot(CYCLES_PER_LINE - 1), None);
        // Left to right: the slot never goes backwards.
        let slots: Vec<usize> = (0..CYCLES_PER_LINE).filter_map(fetch_slot).collect();
        assert!(slots.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(slots.len() as u32, FETCH_CYCLES);
    }
}
