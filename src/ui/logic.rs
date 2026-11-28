//! The decisions the main window makes, as plain functions with no window in
//! sight: how many cycles to run, which Atom keys the host keys press, what to
//! type for `--run`, how the sound is scaled. `app.rs` calls these; the tests
//! here check them without opening anything.

use eframe::egui::{Key, Modifiers};

use super::keymap::{KeyMap, Target};
use atom_rs::machine::joystick::Joystick;
use atom_rs::machine::keyboard::{Keyboard, find_key};

/// The Atom's CPU clock.
pub const CLOCK_HZ: f64 = 1_000_000.0;
/// If the window stalls, catch up by at most this much real time.
pub const MAX_CATCH_UP_SECONDS: f64 = 0.1;
/// The most cycles run in one go between checks of the clock.
pub const SLICE_CYCLES: u64 = 20_000;
/// The fastest the emulation goes (fast tape).
pub const MAX_SPEED: f32 = 100.0;
/// Sound is only played at about normal speed or slower.
const MAX_AUDIBLE_SPEED: f32 = 1.05;

/// Emulated cycles we are owed but have not run yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CycleDebt(f64);

impl CycleDebt {
    /// Real time has passed: we now owe the cycles the Atom would have run.
    pub fn owe(&mut self, elapsed_seconds: f64, speed: f32) {
        self.0 += elapsed_seconds.clamp(0.0, MAX_CATCH_UP_SECONDS) * CLOCK_HZ * speed as f64;
    }

    /// The next batch of cycles to run, or `None` once caught up.
    pub fn next_slice(&self) -> Option<u64> {
        (self.0 >= 1.0).then(|| (self.0 as u64).min(SLICE_CYCLES))
    }

    /// `executed` cycles have been run (an instruction can overshoot a slice).
    pub fn paid(&mut self, executed: u64) {
        self.0 -= executed as f64;
    }

    /// Forget what is owed (the machine stopped, or we ran out of time).
    pub fn forget(&mut self) {
        self.0 = 0.0;
    }

    /// Out of time: drop a backlog rather than spiral; a fraction of a cycle stays.
    pub fn forget_backlog(&mut self) {
        if self.0 >= 1.0 {
            self.0 = 0.0;
        }
    }

    #[cfg(test)]
    pub fn cycles(&self) -> f64 {
        self.0
    }
}

/// The speed we really run at: the slider, unless fast-tape is on and the
/// tape is playing.
pub fn effective_speed(slider: f32, fast_tape: bool, tape_playing: bool) -> f32 {
    if fast_tape && tape_playing {
        MAX_SPEED
    } else {
        slider
    }
}

/// How fast we ran, as a multiple of a real Atom.
pub fn achieved_speed(cycles: u64, seconds: f64) -> f64 {
    if seconds <= 0.0 {
        return 0.0;
    }
    cycles as f64 / seconds / CLOCK_HZ
}

/// Should the speaker's samples be played at all?
pub fn sound_is_audible(muted: bool, speed: f32) -> bool {
    !muted && speed <= MAX_AUDIBLE_SPEED
}

/// Speaker samples are 0.0 to 1.0 (the fraction of time the speaker was high);
/// centre them on zero and apply the volume.
pub fn centred_samples(samples: &[f32], volume: f32) -> Vec<f32> {
    let gain = volume * 2.0;
    samples.iter().map(|s| (s - 0.5) * gain).collect()
}

/// What to type for `--run`. The disk system must be switched on before it can
/// run anything from a floppy; an SD card needs no such thing.
pub fn text_to_run(text: &str, floppy_in_use: bool, sd_card_in_use: bool) -> String {
    if floppy_in_use && !sd_card_in_use && !text.starts_with("*DOS") {
        format!("*DOS\n{text}")
    } else {
        text.to_string()
    }
}

/// With the joystick switched on, the arrow keys steer it and Space is fire.
/// Returns true if the key was used up (the arrows), so it does not also type
/// the Atom's cursor keys. Space is fire and still types a space, so menus
/// that wait for it keep working.
pub fn press_joystick_key(stick: &mut Joystick, key: Key) -> bool {
    match key {
        Key::ArrowLeft => stick.left = true,
        Key::ArrowRight => stick.right = true,
        Key::ArrowUp => stick.up = true,
        Key::ArrowDown => stick.down = true,
        Key::Space => {
            stick.fire = true;
            return false;
        }
        _ => return false,
    }
    true
}

/// Set the Atom's keyboard from the host keys held down and return the
/// joystick position the arrow keys make (if they are acting as one).
pub fn press_host_keys(
    keyboard: &mut Keyboard,
    keys_down: impl IntoIterator<Item = Key>,
    modifiers: Modifiers,
    keymap: &KeyMap,
    joystick_keys: bool,
) -> Joystick {
    keyboard.release_all();
    let mut shift = modifiers.shift;
    let (mut ctrl, mut rept) = (modifiers.ctrl, modifiers.alt);
    let mut stick = Joystick::default();
    for host_key in keys_down {
        if joystick_keys && press_joystick_key(&mut stick, host_key) {
            continue;
        }
        match keymap.target_for(host_key) {
            Some(Target::Key { name, shift: wants }) => {
                let (row, column) = find_key(name).expect("key maps only name real keys");
                keyboard.set_key(row, column, true);
                shift |= *wants;
            }
            Some(Target::Shift) => shift = true,
            Some(Target::Ctrl) => ctrl = true,
            Some(Target::Rept) => rept = true,
            None => {}
        }
    }
    keyboard.shift = shift;
    keyboard.ctrl = ctrl;
    keyboard.rept = rept;
    stick
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_time_turns_into_cycles_at_the_chosen_speed() {
        let mut debt = CycleDebt::default();
        debt.owe(0.01, 1.0);
        assert_eq!(debt.cycles(), 10_000.0);
        debt.owe(0.01, 2.0);
        assert_eq!(debt.cycles(), 30_000.0);
        debt.owe(0.01, 0.5);
        assert_eq!(debt.cycles(), 35_000.0);
    }

    #[test]
    fn a_stall_is_only_caught_up_by_a_tenth_of_a_second() {
        let mut debt = CycleDebt::default();
        debt.owe(5.0, 1.0);
        assert_eq!(debt.cycles(), 100_000.0);
        debt.owe(-1.0, 1.0);
        assert_eq!(debt.cycles(), 100_000.0, "time does not run backwards");
    }

    #[test]
    fn slices_are_capped_and_stop_when_caught_up() {
        let mut debt = CycleDebt::default();
        assert_eq!(debt.next_slice(), None);
        debt.owe(0.0000005, 1.0);
        assert_eq!(debt.next_slice(), None, "half a cycle is not worth running");
        debt = CycleDebt::default();
        debt.owe(0.05, 1.0); // 50,000
        assert_eq!(debt.next_slice(), Some(20_000));
        debt.paid(20_003); // an instruction overshot
        assert_eq!(debt.cycles(), 29_997.0);
        debt.paid(20_000);
        assert_eq!(debt.next_slice(), Some(9_997));
        debt.paid(9_997);
        assert_eq!(debt.next_slice(), None);
    }

    #[test]
    fn forgetting_clears_a_backlog_but_keeps_less_than_a_cycle() {
        let mut debt = CycleDebt::default();
        debt.owe(0.05, 1.0);
        debt.forget_backlog();
        assert_eq!(debt.cycles(), 0.0);
        let mut small = CycleDebt(0.5);
        small.forget_backlog();
        assert_eq!(small.cycles(), 0.5);
        small.forget();
        assert_eq!(small.cycles(), 0.0);
    }

    #[test]
    fn fast_tape_only_speeds_things_up_while_the_tape_plays() {
        assert_eq!(effective_speed(1.0, true, true), MAX_SPEED);
        assert_eq!(effective_speed(1.0, true, false), 1.0);
        assert_eq!(effective_speed(2.0, false, true), 2.0);
    }

    #[test]
    fn achieved_speed_is_cycles_over_time_relative_to_the_real_clock() {
        assert_eq!(achieved_speed(1_000_000, 1.0), 1.0);
        assert_eq!(achieved_speed(3_000_000, 2.0), 1.5);
        assert_eq!(achieved_speed(5, 0.0), 0.0);
    }

    #[test]
    fn sound_is_for_normal_speed_or_slower_and_not_when_muted() {
        assert!(sound_is_audible(false, 1.0));
        assert!(sound_is_audible(false, 0.25));
        assert!(sound_is_audible(false, 1.05));
        assert!(!sound_is_audible(false, 1.5));
        assert!(!sound_is_audible(true, 1.0));
    }

    #[test]
    fn samples_are_centred_on_zero_and_scaled_by_the_volume() {
        assert_eq!(centred_samples(&[0.0, 0.5, 1.0], 0.5), [-0.5, 0.0, 0.5]);
        assert_eq!(centred_samples(&[0.0, 1.0], 0.0), [0.0, 0.0]);
        assert!(centred_samples(&[], 1.0).is_empty());
    }

    #[test]
    fn run_text_gets_the_disk_system_switched_on_only_when_a_floppy_needs_it() {
        assert_eq!(text_to_run("CHAIN\"X\"", true, false), "*DOS\nCHAIN\"X\"");
        assert_eq!(
            text_to_run("*DOS\nHI", true, false),
            "*DOS\nHI",
            "already there"
        );
        assert_eq!(text_to_run("HI", false, false), "HI", "no floppy");
        assert_eq!(
            text_to_run("HI", true, true),
            "HI",
            "the SD card has no *DOS"
        );
    }

    #[test]
    fn arrows_steer_and_are_used_up() {
        let mut stick = Joystick::default();
        for key in [Key::ArrowLeft, Key::ArrowUp] {
            assert!(press_joystick_key(&mut stick, key));
        }
        assert!(stick.left && stick.up && !stick.right && !stick.down && !stick.fire);
        assert!(press_joystick_key(&mut stick, Key::ArrowRight));
        assert!(press_joystick_key(&mut stick, Key::ArrowDown));
        assert!(stick.right && stick.down);
    }

    #[test]
    fn space_fires_but_still_types() {
        let mut stick = Joystick::default();
        assert!(!press_joystick_key(&mut stick, Key::Space));
        assert!(stick.fire);
    }

    #[test]
    fn other_keys_are_left_alone() {
        let mut stick = Joystick::default();
        assert!(!press_joystick_key(&mut stick, Key::A));
        assert_eq!(stick, Joystick::default());
    }

    fn pressed(keyboard: &Keyboard, name: &str) -> bool {
        let (row, column) = find_key(name).unwrap();
        // Port B reads low for a pressed key on the selected row.
        keyboard.read_port_b(row as u8) & (1 << column) == 0
    }

    #[test]
    fn host_keys_press_the_atom_keys_they_are_mapped_to_and_release_the_rest() {
        let keymap = KeyMap::default();
        let mut keyboard = Keyboard::default();
        press_host_keys(&mut keyboard, [Key::A], Modifiers::NONE, &keymap, false);
        assert!(pressed(&keyboard, "A"));
        press_host_keys(&mut keyboard, [Key::B], Modifiers::NONE, &keymap, false);
        assert!(pressed(&keyboard, "B"));
        assert!(!pressed(&keyboard, "A"), "A was let go");
        press_host_keys(&mut keyboard, [], Modifiers::NONE, &keymap, false);
        assert!(!pressed(&keyboard, "B"));
    }

    #[test]
    fn host_modifiers_become_the_atoms_shift_ctrl_and_rept() {
        let keymap = KeyMap::default();
        let mut keyboard = Keyboard::default();
        let modifiers = Modifiers {
            shift: true,
            ctrl: true,
            alt: true,
            ..Modifiers::NONE
        };
        press_host_keys(&mut keyboard, [], modifiers, &keymap, false);
        assert!(keyboard.shift && keyboard.ctrl && keyboard.rept);
        press_host_keys(&mut keyboard, [], Modifiers::NONE, &keymap, false);
        assert!(!keyboard.shift && !keyboard.ctrl && !keyboard.rept);
    }

    #[test]
    fn arrow_keys_steer_the_joystick_instead_of_typing_when_it_is_on() {
        let keymap = KeyMap::default();
        let mut keyboard = Keyboard::default();
        let stick = press_host_keys(
            &mut keyboard,
            [Key::ArrowLeft],
            Modifiers::NONE,
            &keymap,
            true,
        );
        assert!(stick.left);
        assert!(
            !keyboard.shift,
            "the arrow was used up, so it typed nothing"
        );
        assert!(!pressed(&keyboard, "RIGHT"));
        let stick = press_host_keys(
            &mut keyboard,
            [Key::ArrowLeft],
            Modifiers::NONE,
            &keymap,
            false,
        );
        assert!(!stick.left, "joystick off: arrows are just keys");
        assert!(
            keyboard.shift || pressed(&keyboard, "RIGHT"),
            "and the Atom's cursor key is typed"
        );
    }

    #[test]
    fn a_host_key_with_no_mapping_does_nothing() {
        let keymap = KeyMap::default();
        let mut keyboard = Keyboard::default();
        press_host_keys(&mut keyboard, [Key::F9], Modifiers::NONE, &keymap, false);
        assert!(!keyboard.shift && !keyboard.ctrl && !keyboard.rept);
    }
}
