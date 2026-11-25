//! An automatic typist: presses the Atom's keys one after another, as if
//! someone were typing a command. Used to start programs from the command
//! line and by tests.
//!
//! Timing is counted in video frames of the *emulated* machine, so typing
//! works the same at any emulation speed. Each key is held down for a few
//! frames, then released for a few more, because the Atom's OS scans the
//! keyboard only now and then and would miss a quicker tap.
//!
//! Like a person, the typist waits for the Atom to finish a command before
//! typing the next line: after RETURN it waits for the `>` prompt (see
//! [`Atom::ready_for_input`]). Without this, keys typed while the Atom is busy
//! loading from disk are simply lost.

use std::collections::VecDeque;

use super::atom::Atom;
use super::keyboard::{Keyboard, find_key, key_for_char};

/// Frames a key stays down, and the gap before the next one.
const HOLD_FRAMES: u64 = 6;
const GAP_FRAMES: u64 = 6;

/// After RETURN the typist waits for the Atom to show its prompt again, but
/// never longer than this (about 10 seconds), in case a program that is not
/// at a prompt is waiting for the next key.
const PROMPT_TIMEOUT_FRAMES: u64 = 600;

#[derive(Clone, Copy, Debug)]
enum Phase {
    /// Ready to press the next key.
    Idle,
    /// A key is down until this frame.
    Down(KeyPress),
    /// Between keys until this frame.
    Gap { until: u64 },
    /// RETURN was pressed; wait for the Atom to finish the command before
    /// typing more. `since` is the frame we started waiting.
    WaitForPrompt { since: u64 },
}

#[derive(Clone, Copy, Debug)]
struct KeyPress {
    row: usize,
    column: usize,
    shift: bool,
    until: u64,
    /// Wait for the Atom's prompt after this key (an ordinary RETURN).
    waits_for_prompt: bool,
}

#[derive(Clone, Debug)]
pub struct Typist {
    pending: VecDeque<char>,
    /// How many of the characters at the front of `pending` were interjected
    /// (see [`Typist::interject`]) and so must not wait for a prompt.
    urgent: usize,
    start_at_frame: u64,
    phase: Phase,
}

impl Typist {
    pub fn new() -> Self {
        Typist {
            pending: VecDeque::new(),
            urgent: 0,
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

    /// Type these characters next, straight away, without waiting for the
    /// Atom's prompt. For answering something the Atom is waiting for *now*,
    /// such as the key it wants after printing `PLAY TAPE`.
    pub fn interject(&mut self, text: &str) {
        for c in text.chars().rev() {
            self.pending.push_front(c);
        }
        self.urgent += text.chars().count();
        self.start_at_frame = 0;
        if matches!(self.phase, Phase::WaitForPrompt { .. }) {
            self.phase = Phase::Idle;
        }
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

    /// Type one step using `atom`'s keyboard, frame count and readiness.
    /// Call once per emulated frame.
    pub fn drive(&mut self, atom: &mut Atom) {
        let ready = atom.ready_for_input();
        self.update(&mut atom.bus.ppi.keyboard, atom.bus.vdg.frames, ready);
    }

    /// Call regularly (at least once per frame) with the machine's frame
    /// count and whether the Atom is ready for input (see
    /// [`Atom::ready_for_input`]). Presses or releases keys on `keyboard`; the
    /// key stays pressed in the keyboard until this releases it, but front
    /// ends that rebuild the keyboard each update should call this after
    /// doing so.
    pub fn update(&mut self, keyboard: &mut Keyboard, frame: u64, ready: bool) {
        if frame < self.start_at_frame {
            return;
        }
        loop {
            match self.phase {
                Phase::Idle => {
                    let Some(c) = self.pending.pop_front() else {
                        return;
                    };
                    // (Interjected characters are at the front, so they go first.)
                    let urgent = self.urgent > 0;
                    self.urgent = self.urgent.saturating_sub(1);
                    let Some((name, shift)) = key_for_char(c) else {
                        continue;
                    };
                    let (row, column) = find_key(name).expect("key_for_char names real keys");
                    self.phase = Phase::Down(KeyPress {
                        row,
                        column,
                        shift,
                        until: frame + HOLD_FRAMES,
                        // After an urgent RETURN there is no prompt to wait for.
                        waits_for_prompt: c == '\n' && !urgent,
                    });
                }
                Phase::Down(key) => {
                    if frame < key.until {
                        keyboard.set_key(key.row, key.column, true);
                        keyboard.shift |= key.shift;
                        return;
                    }
                    keyboard.set_key(key.row, key.column, false);
                    keyboard.shift = false;
                    self.phase =
                        if key.waits_for_prompt && self.urgent == 0 && !self.pending.is_empty() {
                            Phase::WaitForPrompt { since: frame }
                        } else {
                            Phase::Gap {
                                until: frame + GAP_FRAMES,
                            }
                        };
                }
                Phase::WaitForPrompt { since } => {
                    // Give the command a moment to start before judging whether
                    // it has finished, then wait for the prompt.
                    let waited = frame.saturating_sub(since);
                    if (waited >= GAP_FRAMES && ready) || waited >= PROMPT_TIMEOUT_FRAMES {
                        self.phase = Phase::Idle;
                    } else {
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
                typist.update(&mut keyboard, frame, true);
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
        typist.update(&mut keyboard, 0, true);
        assert_eq!(keyboard.read_port_b(0) & 0x80, 0, "shift is down");
        let (row, column) = find_key(":").unwrap();
        assert_eq!(keyboard.read_port_b(row as u8) & (1 << column), 0);
    }

    #[test]
    fn waits_for_the_prompt_after_return_before_typing_more() {
        let mut typist = Typist::new();
        typist.type_text("A\nB", 0);
        let mut keyboard = Keyboard::default();
        let b = find_key("B").unwrap();
        let b_down = |keyboard: &Keyboard| keyboard.read_port_b(b.0 as u8) & (1 << b.1) == 0;

        // The Atom is busy (not ready) for 100 frames after RETURN: no B yet.
        for frame in 0..100 {
            typist.update(&mut keyboard, frame, false);
            assert!(
                !b_down(&keyboard),
                "typed B at frame {frame} while the Atom was busy"
            );
        }
        // Once the prompt is back, B gets typed.
        let mut typed = false;
        for frame in 100..140 {
            typist.update(&mut keyboard, frame, true);
            typed |= b_down(&keyboard);
        }
        assert!(typed);
    }

    #[test]
    fn interjected_keys_do_not_wait_for_the_prompt() {
        let mut typist = Typist::new();
        typist.type_text("A\nB", 0);
        let mut keyboard = Keyboard::default();
        // Type the A and RETURN; the Atom stays busy (never ready).
        for frame in 0..40 {
            typist.update(&mut keyboard, frame, false);
        }
        // Something needs an immediate key press.
        typist.interject("C");
        let c = find_key("C").unwrap();
        let mut typed = false;
        for frame in 40..60 {
            typist.update(&mut keyboard, frame, false);
            typed |= keyboard.read_port_b(c.0 as u8) & (1 << c.1) == 0;
        }
        assert!(
            typed,
            "the interjected key was held up waiting for a prompt"
        );
    }

    #[test]
    fn gives_up_waiting_for_a_prompt_that_never_comes() {
        let mut typist = Typist::new();
        typist.type_text("\nB", 0);
        let mut keyboard = Keyboard::default();
        let b = find_key("B").unwrap();
        let mut typed = false;
        for frame in 0..(PROMPT_TIMEOUT_FRAMES + 40) {
            typist.update(&mut keyboard, frame, false);
            typed |= keyboard.read_port_b(b.0 as u8) & (1 << b.1) == 0;
        }
        assert!(
            typed,
            "a program that never shows a prompt must still get its keys"
        );
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
