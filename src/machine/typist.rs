//! An automatic typist: presses the Atom's keys one after another, as if
//! someone were typing a command. Used to start programs from the command
//! line and by tests.
//!
//! Timing is counted in video frames of the *emulated* machine, so typing
//! works the same at any emulation speed. Each key is held down for a few
//! frames, then released for a few more, because the Atom's OS scans the
//! keyboard only now and then and would miss a quicker tap.

use std::collections::VecDeque;

use super::keyboard::{Keyboard, find_key, key_for_char};

/// Frames a key stays down, and the gap before the next one.
const HOLD_FRAMES: u64 = 6;
const GAP_FRAMES: u64 = 6;

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Ready to press the next key.
    Idle,
    /// A key is down until this frame.
    Down {
        row: usize,
        column: usize,
        shift: bool,
        until: u64,
    },
    /// Between keys until this frame.
    Gap { until: u64 },
}

#[derive(Clone, Debug)]
pub struct Typist {
    pending: VecDeque<char>,
    start_at_frame: u64,
    phase: Phase,
}

impl Typist {
    pub fn new() -> Self {
        Typist {
            pending: VecDeque::new(),
            start_at_frame: 0,
            phase: Phase::Idle,
        }
    }

    /// Queue text to type (`\n` presses RETURN). Typing begins once the
    /// machine has reached `start_at_frame`, e.g. after it has booted.
    /// Characters the Atom keyboard cannot produce are skipped.
    pub fn type_text(&mut self, text: &str, start_at_frame: u64) {
        if self.pending.is_empty() {
            self.start_at_frame = start_at_frame;
        }
        self.pending.extend(text.chars());
    }

    /// How many characters are still waiting to be typed.
    pub fn remaining(&self) -> usize {
        self.pending.len()
    }

    /// The frame typing is due to begin at.
    pub fn start_frame(&self) -> u64 {
        self.start_at_frame
    }

    /// Is there still something to type, or a key still down?
    pub fn is_busy(&self) -> bool {
        !self.pending.is_empty() || !matches!(self.phase, Phase::Idle)
    }

    /// Call regularly (at least once per frame) with the machine's frame
    /// count. Presses or releases keys on `keyboard`; the key stays pressed in
    /// the keyboard until this releases it, but front ends that rebuild the
    /// keyboard each update should call this after doing so.
    pub fn update(&mut self, keyboard: &mut Keyboard, frame: u64) {
        if frame < self.start_at_frame {
            return;
        }
        loop {
            match self.phase {
                Phase::Idle => {
                    let Some(c) = self.pending.pop_front() else {
                        return;
                    };
                    let Some((name, shift)) = key_for_char(c) else {
                        continue;
                    };
                    let (row, column) = find_key(name).expect("key_for_char names real keys");
                    self.phase = Phase::Down {
                        row,
                        column,
                        shift,
                        until: frame + HOLD_FRAMES,
                    };
                }
                Phase::Down {
                    row,
                    column,
                    shift,
                    until,
                } => {
                    if frame >= until {
                        keyboard.set_key(row, column, false);
                        keyboard.shift = false;
                        self.phase = Phase::Gap {
                            until: frame + GAP_FRAMES,
                        };
                    } else {
                        keyboard.set_key(row, column, true);
                        keyboard.shift |= shift;
                        return;
                    }
                }
                Phase::Gap { until } => {
                    if frame >= until {
                        self.phase = Phase::Idle;
                    } else {
                        return;
                    }
                }
            }
        }
    }
}

impl Default for Typist {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the typist for `frames` frames, returning for each frame which
    /// (row, column) keys were down.
    fn run(typist: &mut Typist, frames: u64) -> Vec<Vec<(usize, usize)>> {
        let mut keyboard = Keyboard::default();
        (0..frames)
            .map(|frame| {
                typist.update(&mut keyboard, frame);
                let mut down = Vec::new();
                for row in 0..10 {
                    for column in 0..6 {
                        if keyboard.read_port_b(row as u8) & (1 << column) == 0 {
                            down.push((row, column));
                        }
                    }
                }
                down
            })
            .collect()
    }

    #[test]
    fn presses_each_key_in_turn_with_gaps() {
        let mut typist = Typist::new();
        typist.type_text("AB", 0);
        let frames = run(&mut typist, 40);
        let a = find_key("A").unwrap();
        let b = find_key("B").unwrap();
        assert_eq!(frames[0], vec![a]);
        assert_eq!(frames[HOLD_FRAMES as usize - 1], vec![a]);
        assert!(
            frames[HOLD_FRAMES as usize].is_empty(),
            "released between keys"
        );
        assert_eq!(frames[(HOLD_FRAMES + GAP_FRAMES) as usize], vec![b]);
        assert!(!typist.is_busy() || frames.len() < 40);
    }

    #[test]
    fn waits_until_the_start_frame() {
        let mut typist = Typist::new();
        typist.type_text("A", 20);
        let frames = run(&mut typist, 30);
        assert!(frames[..20].iter().all(Vec::is_empty));
        assert_eq!(frames[20], vec![find_key("A").unwrap()]);
    }

    #[test]
    fn holds_shift_for_shifted_characters_and_skips_untypeable_ones() {
        let mut typist = Typist::new();
        typist.type_text("~*", 0); // '~' cannot be typed; '*' is Shift+:
        let mut keyboard = Keyboard::default();
        typist.update(&mut keyboard, 0);
        assert_eq!(keyboard.read_port_b(0) & 0x80, 0, "shift is down");
        let (row, column) = find_key(":").unwrap();
        assert_eq!(keyboard.read_port_b(row as u8) & (1 << column), 0);
    }

    #[test]
    fn becomes_idle_after_the_last_key() {
        let mut typist = Typist::new();
        typist.type_text("A", 0);
        assert!(typist.is_busy());
        run(&mut typist, 40);
        assert!(!typist.is_busy());
    }
}
