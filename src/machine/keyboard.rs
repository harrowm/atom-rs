//! The Atom keyboard: a matrix of 10 rows by 6 columns, plus three keys
//! (Shift, Ctrl, Rept) that have their own wires.
//!
//! The OS selects a row by writing its number to the low nibble of 8255 port
//! A, then reads port B: a pressed key in that row pulls its column bit low.

use serde::{Deserialize, Serialize};

/// Number of rows in the matrix.
pub const ROWS: usize = 10;
/// Number of columns in the matrix.
pub const COLUMNS: usize = 6;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Keyboard {
    /// One byte per row; bit `n` set means the key in column `n` is down.
    matrix: [u8; ROWS],
    pub shift: bool,
    pub ctrl: bool,
    pub rept: bool,
}

impl Keyboard {
    /// Release every key (the front end rebuilds the state from the host
    /// keyboard each frame).
    pub fn release_all(&mut self) {
        *self = Keyboard::default();
    }

    pub fn set_key(&mut self, row: usize, column: usize, down: bool) {
        assert!(
            row < ROWS && column < COLUMNS,
            "no such key: row {row}, column {column}"
        );
        if down {
            self.matrix[row] |= 1 << column;
        } else {
            self.matrix[row] &= !(1 << column);
        }
    }

    /// What port B reads while `row` is selected: 1 = not pressed, 0 = pressed.
    /// Bits 0-5 are the columns, bit 6 is Ctrl and bit 7 is Shift.
    pub fn read_port_b(&self, row: u8) -> u8 {
        let row_bits = self.matrix.get(row as usize).copied().unwrap_or(0);
        let mut value = !row_bits & 0x3F;
        value |= if self.ctrl { 0 } else { 0x40 };
        value |= if self.shift { 0 } else { 0x80 };
        value
    }
}

/// The key matrix as a table (`LAYOUT[row][column]`), from Atomulator and
/// checked against what the real ROMs print. Shift gives the other symbol on
/// each key; `""` means "no key".
pub const LAYOUT: [[&str; COLUMNS]; ROWS] = [
    ["", "3", "-", "G", "Q", "ESC"],
    ["", "2", ",", "F", "P", "Z"],
    ["UP", "1", ";", "E", "O", "Y"],
    ["RIGHT", "0", ":", "D", "N", "X"],
    ["LOCK", "DELETE", "9", "C", "M", "W"],
    ["^", "COPY", "8", "B", "L", "V"],
    ["]", "RETURN", "7", "A", "K", "U"],
    ["\\", "", "6", "@", "J", "T"],
    ["[", "", "5", "/", "I", "S"],
    ["SPACE", "", "4", ".", "H", "R"],
];

/// Find the (row, column) of a key by its name in [`LAYOUT`].
pub fn find_key(name: &str) -> Option<(usize, usize)> {
    LAYOUT.iter().enumerate().find_map(|(row, columns)| {
        columns
            .iter()
            .position(|&key| key == name && !key.is_empty())
            .map(|col| (row, col))
    })
}

/// How to type a character on the Atom: the key's name and whether Shift is
/// held. `None` for characters the Atom keyboard cannot produce (lower case
/// letters type as upper case).
pub fn key_for_char(c: char) -> Option<(&'static str, bool)> {
    let (name, shift) = match c {
        'a'..='z' | 'A'..='Z' | '0'..='9' => (c.to_ascii_uppercase().to_string(), false),
        ' ' => ("SPACE".to_string(), false),
        '\n' => ("RETURN".to_string(), false),
        ':' | '-' | ',' | ';' | '.' | '/' | '@' => (c.to_string(), false),
        // Shifted symbols: the key that makes them and Shift.
        '*' => (":".to_string(), true),
        '=' => ("-".to_string(), true),
        '<' => (",".to_string(), true),
        '+' => (";".to_string(), true),
        '>' => (".".to_string(), true),
        '?' => ("/".to_string(), true),
        '!' => ("1".to_string(), true),
        '"' => ("2".to_string(), true),
        '#' => ("3".to_string(), true),
        '$' => ("4".to_string(), true),
        '%' => ("5".to_string(), true),
        '&' => ("6".to_string(), true),
        '\'' => ("7".to_string(), true),
        '(' => ("8".to_string(), true),
        ')' => ("9".to_string(), true),
        _ => return None,
    };
    // Return the name as it is spelled in LAYOUT, so it can be 'static.
    LAYOUT
        .iter()
        .flatten()
        .find(|&&key| key == name)
        .map(|&key| (key, shift))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_key_pulls_its_column_low_only_in_its_row() {
        let mut keyboard = Keyboard::default();
        let (row, column) = find_key("A").unwrap();
        keyboard.set_key(row, column, true);
        assert_eq!(keyboard.read_port_b(row as u8), !(1u8 << column));
        assert_eq!(keyboard.read_port_b(0), 0xFF);
    }

    #[test]
    fn characters_map_to_keys_with_shift_where_needed() {
        assert_eq!(key_for_char('a'), Some(("A", false)));
        assert_eq!(key_for_char('*'), Some((":", true)));
        assert_eq!(key_for_char('\n'), Some(("RETURN", false)));
        assert_eq!(key_for_char('~'), None);
        for c in "AZ09 :-,;./@*=<+>?!\"#$%&'()\n".chars() {
            let (name, _) = key_for_char(c).unwrap();
            assert!(find_key(name).is_some(), "{c:?} -> {name}");
        }
    }

    #[test]
    fn shift_and_ctrl_use_the_top_bits() {
        let keyboard = Keyboard {
            shift: true,
            ctrl: true,
            ..Keyboard::default()
        };
        assert_eq!(keyboard.read_port_b(0), 0x3F);
    }
}
