//! The 8255 programmable peripheral interface at `B000`.
//!
//! The Atom wires it up like this (the chip is fixed in one mode):
//!
//! | Port | Direction | Use |
//! |------|-----------|-----|
//! | A (`B000`) | output | bits 0-3: keyboard row. Bits 4-7: 6847 mode (A/G, GM0-2) |
//! | B (`B001`) | input  | bits 0-5: keyboard columns, bit 6: Ctrl, bit 7: Shift |
//! | C (`B002`) | mixed  | out: bit 0 tape, bit 2 speaker, bit 3 colour set. In: bit 4 2.4 kHz tone, bit 5 tape in, bit 6 Rept (0 = down), bit 7 FS (0 = vertical sync) |
//! | Control (`B003`) | | bit 7 set: mode word (ignored). Bit 7 clear: set or reset one bit of port C |

use serde::{Deserialize, Serialize};

use super::keyboard::Keyboard;

/// The tone input on port C bit 4 toggles every 208 CPU cycles, giving a
/// 2.4 kHz square wave. The OS uses it to time cassette bits.
const TONE_HALF_PERIOD_CYCLES: u32 = 208;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ppi {
    port_a: u8,
    port_c_out: u8,
    tone: bool,
    tone_counter: u32,
    pub keyboard: Keyboard,
    /// Level of the 6847's FS (field sync) output; low during vertical sync.
    pub field_sync_low: bool,
    /// Level of the cassette input.
    pub tape_input: bool,
}

impl Ppi {
    /// Advance one CPU cycle. Returns true on the cycle the 2.4 kHz tone
    /// flips, which is also when the cassette signal may change.
    pub fn tick(&mut self) -> bool {
        self.tone_counter += 1;
        if self.tone_counter == TONE_HALF_PERIOD_CYCLES {
            self.tone_counter = 0;
            self.tone = !self.tone;
            return true;
        }
        false
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr & 3 {
            0 => self.port_a,
            1 => self.keyboard.read_port_b(self.port_a & 0x0F),
            2 => self.read_port_c(),
            // The control register cannot be read.
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr & 3 {
            0 => self.port_a = value,
            2 => self.port_c_out = value & 0x0F,
            3 => self.write_control(value),
            _ => {} // port B is input only
        }
    }

    /// Control register: bit 7 = 1 is a mode-set word (the Atom never changes
    /// the mode, so it is ignored); bit 7 = 0 sets or resets a single bit of
    /// port C, selected by bits 1-3, to the value of bit 0.
    fn write_control(&mut self, value: u8) {
        if value & 0x80 == 0 {
            let bit = (value >> 1) & 7;
            if value & 1 != 0 {
                self.port_c_out |= 1 << bit;
            } else {
                self.port_c_out &= !(1 << bit);
            }
            self.port_c_out &= 0x0F;
        }
    }

    fn read_port_c(&self) -> u8 {
        let mut value = self.port_c_out & 0x0F;
        if self.tone {
            value |= 0x10;
        }
        if self.tape_input {
            value |= 0x20;
        }
        if !self.keyboard.rept {
            value |= 0x40;
        }
        if !self.field_sync_low {
            value |= 0x80;
        }
        value
    }

    /// The row being scanned and the video mode bits (port A).
    pub fn port_a(&self) -> u8 {
        self.port_a
    }

    /// The high nibble of port A: A/G and GM0-GM2 for the 6847.
    pub fn video_mode_bits(&self) -> u8 {
        self.port_a >> 4
    }

    /// Colour set select (port C bit 3).
    pub fn colour_set(&self) -> bool {
        self.port_c_out & 0x08 != 0
    }

    /// The signal sent to the cassette recorder.
    ///
    /// Port C bit 0 is the signal itself, which the OS toggles in software to
    /// make 1200 Hz tones (0 bits). Bit 1 mixes in the hardware 2400 Hz tone,
    /// which is how 1 bits and the leader are made. (Observed from what the
    /// real OS writes; the level may be inverted compared to the real circuit,
    /// which does not matter because only the edges carry information.)
    pub fn tape_output(&self) -> bool {
        let signal = self.port_c_out & 0x01 != 0;
        let tone_enabled = self.port_c_out & 0x02 != 0;
        signal ^ (tone_enabled && self.tone)
    }

    /// The speaker output level (port C bit 2).
    pub fn speaker(&self) -> bool {
        self.port_c_out & 0x04 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_row_comes_from_port_a_low_nibble() {
        let mut ppi = Ppi::default();
        let (row, column) = super::super::keyboard::find_key("RETURN").unwrap();
        ppi.keyboard.set_key(row, column, true);
        ppi.write(0, row as u8);
        assert_eq!(ppi.read(1) & (1 << column), 0);
        ppi.write(0, 0);
        assert_eq!(ppi.read(1) & (1 << column), 1 << column);
    }

    #[test]
    fn control_register_sets_and_resets_port_c_bits() {
        let mut ppi = Ppi::default();
        ppi.write(3, 0b0000_0101); // set bit 2 (speaker)
        assert!(ppi.speaker());
        ppi.write(3, 0b0000_0100); // reset bit 2
        assert!(!ppi.speaker());
        ppi.write(3, 0b0000_0111); // set bit 3 (colour set)
        assert!(ppi.colour_set());
    }

    #[test]
    fn field_sync_is_active_low_on_port_c_bit_7() {
        let mut ppi = Ppi::default();
        assert_eq!(ppi.read(2) & 0x80, 0x80);
        ppi.field_sync_low = true;
        assert_eq!(ppi.read(2) & 0x80, 0);
    }

    #[test]
    fn tone_toggles_every_208_cycles() {
        let mut ppi = Ppi::default();
        for _ in 0..TONE_HALF_PERIOD_CYCLES {
            ppi.tick();
        }
        assert_eq!(ppi.read(2) & 0x10, 0x10);
        for _ in 0..TONE_HALF_PERIOD_CYCLES {
            ppi.tick();
        }
        assert_eq!(ppi.read(2) & 0x10, 0);
    }
}
