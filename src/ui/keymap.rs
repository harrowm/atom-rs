//! Mapping host (PC/Mac) keys onto the Atom keyboard.
//!
//! The mapping is by key *position*, following Atomulator, so it assumes a
//! US-style layout: the key labelled `;` on your keyboard is the Atom's `;`
//! key, and Shift works the same way as on the Atom (so Shift+`;` types `+`,
//! not `:`). The cursor keys need special handling because the Atom has one
//! key for up/down and one for left/right, selected with Shift.

use eframe::egui::Key;

/// A press of one Atom matrix key, possibly with the Atom's Shift held too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtomKeyPress {
    /// The key's name in [`atom_rs::machine::keyboard::LAYOUT`].
    pub key: &'static str,
    /// Hold the Atom's Shift as well (only the Down and Left arrows need this).
    pub shift: bool,
}

const fn plain(key: &'static str) -> Option<AtomKeyPress> {
    Some(AtomKeyPress { key, shift: false })
}

const fn shifted(key: &'static str) -> Option<AtomKeyPress> {
    Some(AtomKeyPress { key, shift: true })
}

/// Which Atom key (if any) a host key presses.
pub fn atom_key_for(host: Key) -> Option<AtomKeyPress> {
    // Letters and digits map to the Atom key of the same name.
    let name = host.name();
    if let Some(&key) = LETTERS_AND_DIGITS.iter().find(|&&k| k == name) {
        return plain(key);
    }
    match host {
        Key::Space => plain("SPACE"),
        Key::Enter => plain("RETURN"),
        Key::Backspace | Key::Delete => plain("DELETE"),
        Key::Escape => plain("ESC"),
        Key::CapsLock => plain("LOCK"),
        Key::End => plain("COPY"),
        Key::Tab => plain("^"),
        Key::ArrowUp => plain("UP"),
        Key::ArrowDown => shifted("UP"),
        Key::ArrowRight => plain("RIGHT"),
        Key::ArrowLeft => shifted("RIGHT"),
        Key::Minus => plain("-"),
        Key::Comma => plain(","),
        Key::Period => plain("."),
        Key::Slash => plain("/"),
        Key::Semicolon => plain(";"),
        Key::Quote | Key::Colon => plain(":"),
        Key::Equals => plain("@"),
        Key::OpenBracket => plain("["),
        Key::CloseBracket => plain("]"),
        Key::Backslash => plain("\\"),
        _ => None,
    }
}

/// `'static` names for every letter and digit key, so the result can borrow them.
const LETTERS_AND_DIGITS: [&str; 36] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
];

#[cfg(test)]
mod tests {
    use super::*;
    use atom_rs::machine::keyboard::find_key;

    #[test]
    fn letters_and_digits_map_to_the_same_name() {
        assert_eq!(atom_key_for(Key::A), plain("A"));
        assert_eq!(atom_key_for(Key::Num7), plain("7"));
    }

    #[test]
    fn down_and_left_are_shifted_up_and_right() {
        assert_eq!(atom_key_for(Key::ArrowDown), shifted("UP"));
        assert_eq!(atom_key_for(Key::ArrowLeft), shifted("RIGHT"));
    }

    #[test]
    fn every_mapped_key_exists_in_the_matrix() {
        for host in Key::ALL {
            if let Some(press) = atom_key_for(*host) {
                assert!(
                    find_key(press.key).is_some(),
                    "{host:?} -> {press:?} not in matrix"
                );
            }
        }
    }
}
