//! A joystick: four directions and a fire button.
//!
//! The Atom has no joystick port of its own. Two add-ons are common, and both
//! are emulated here (as in Atomulator), so software written for either works:
//!
//! * **The AtoMMC's port.** The AtoMMC card has a spare input port that games
//!   such as Repton read with command `A2`. Each button pulls its bit low.
//! * **The keyboard-matrix interface.** A stick wired in parallel with some
//!   keys on keyboard row 0, so that pushing it looks like pressing them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Joystick {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub fire: bool,
}

impl Joystick {
    /// What the AtoMMC's port answers: 1 = not pressed, 0 = pressed.
    ///
    /// Bit 0 right, bit 1 left, bit 2 down, bit 3 up, bit 4 fire; the unused
    /// top three bits read 1. A stick at rest reads `FF`.
    pub fn port_bits(self) -> u8 {
        let mut bits = 0xFF;
        for (pressed, bit) in [
            (self.right, 0),
            (self.left, 1),
            (self.down, 2),
            (self.up, 3),
            (self.fire, 4),
        ] {
            if pressed {
                bits &= !(1 << bit);
            }
        }
        bits
    }

    /// Pull down the bits of keyboard row 0 that the matrix interface wires
    /// to the stick. `port_b` is what the keyboard alone would read.
    ///
    /// Row 0 holds the keys 3, minus, G and Q, and a column with no key
    /// fitted: left is `3`, down is `-`, right is `G`, up is `Q`, and fire
    /// uses the empty column.
    pub fn apply_to_row_zero(self, port_b: u8) -> u8 {
        let mut value = port_b;
        for (pressed, bit) in [
            (self.fire, 0),
            (self.left, 1),
            (self.down, 2),
            (self.right, 3),
            (self.up, 4),
        ] {
            if pressed {
                value &= !(1 << bit);
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stick_at_rest_reads_all_ones() {
        assert_eq!(Joystick::default().port_bits(), 0xFF);
        assert_eq!(Joystick::default().apply_to_row_zero(0xFF), 0xFF);
    }

    #[test]
    fn each_button_clears_its_own_bit_on_the_atommc_port() {
        let stick = |f: fn(&mut Joystick)| {
            let mut stick = Joystick::default();
            f(&mut stick);
            stick.port_bits()
        };
        assert_eq!(stick(|s| s.right = true), 0b1111_1110);
        assert_eq!(stick(|s| s.left = true), 0b1111_1101);
        assert_eq!(stick(|s| s.down = true), 0b1111_1011);
        assert_eq!(stick(|s| s.up = true), 0b1111_0111);
        assert_eq!(stick(|s| s.fire = true), 0b1110_1111);
    }

    #[test]
    fn diagonals_and_fire_combine() {
        let stick = Joystick {
            left: true,
            up: true,
            fire: true,
            ..Joystick::default()
        };
        assert_eq!(stick.port_bits(), 0b1110_0101);
    }

    #[test]
    fn the_matrix_interface_uses_the_keys_3_minus_g_and_q() {
        let stick = |f: fn(&mut Joystick)| {
            let mut stick = Joystick::default();
            f(&mut stick);
            stick.apply_to_row_zero(0xFF)
        };
        assert_eq!(stick(|s| s.left = true), 0b1111_1101, "3");
        assert_eq!(stick(|s| s.down = true), 0b1111_1011, "minus");
        assert_eq!(stick(|s| s.right = true), 0b1111_0111, "G");
        assert_eq!(stick(|s| s.up = true), 0b1110_1111, "Q");
        assert_eq!(stick(|s| s.fire = true), 0b1111_1110, "empty column");
    }

    #[test]
    fn the_matrix_interface_leaves_other_bits_alone() {
        let stick = Joystick {
            left: true,
            ..Joystick::default()
        };
        // Ctrl and Shift (bits 6 and 7) and ESC (bit 5) held down already.
        assert_eq!(stick.apply_to_row_zero(0b0001_1111), 0b0001_1101);
    }
}
