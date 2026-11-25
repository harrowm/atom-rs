//! Pressing play on the tape recorder at the right moment.
//!
//! When a program asks the Atom to load from tape, the Atom prints `PLAY TAPE`
//! and waits for a key. A person would press play on the cassette recorder and
//! then a key. [`TapeAutoplay`] does both when it sees that message.

use super::atom::Atom;
use super::typist::Typist;

const MESSAGE: &str = "PLAY TAPE";

#[derive(Clone, Debug, Default)]
pub struct TapeAutoplay {
    /// How many lines on the screen held the message last time we looked.
    rows_seen: usize,
}

impl TapeAutoplay {
    /// Call regularly (once per frame is plenty). Starts the tape and queues
    /// a RETURN key press when a *new* `PLAY TAPE` line appears.
    pub fn update(&mut self, atom: &mut Atom, typist: &mut Typist) {
        if !atom.bus.tape.has_tape() || atom.bus.tape.playing {
            return;
        }
        let rows = atom
            .screen_text()
            .iter()
            .filter(|row| row.contains(MESSAGE))
            .count();
        if rows > self.rows_seen {
            atom.bus.tape.playing = true;
            typist.type_text("\n", atom.bus.vdg.frames);
        }
        self.rows_seen = rows;
    }
}
