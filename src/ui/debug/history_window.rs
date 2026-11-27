//! The instruction history: the last instructions that ran, with the
//! registers before each. Turn recording on, run (or step), and look back at
//! how the machine got where it stopped.

use atom_rs::machine::Atom;
use atom_rs::machine::debug::HISTORY_LENGTH;
use atom_rs::machine::debug_console::{format_history_entry, history_report};
use eframe::egui::{self, RichText};

/// How many of the newest instructions are shown (all are kept, and saved).
const SHOWN: usize = 400;

#[derive(Default)]
pub struct State {
    message: String,
}

pub fn show(ui: &mut egui::Ui, state: &mut State, atom: &mut Atom) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut atom.bus.debug.recording, "Record every instruction")
            .on_hover_text("Slows the emulator down a little while it is on");
        if ui.button("Clear").clicked() {
            atom.bus.debug.clear_history();
        }
        if ui.button("Save as text...").clicked() {
            state.message = save(atom);
        }
        ui.label(
            RichText::new(format!(
                "{} of the last {HISTORY_LENGTH} kept",
                atom.bus.debug.history_len()
            ))
            .weak(),
        );
    });
    if !state.message.is_empty() {
        ui.label(RichText::new(&state.message).weak());
    }
    ui.separator();

    if atom.bus.debug.history_len() == 0 {
        ui.label("Nothing recorded yet. Tick the box, then run or step the machine.");
        return;
    }
    egui::ScrollArea::both()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for entry in atom.bus.debug.recent_history(SHOWN) {
                ui.label(RichText::new(format_history_entry(entry)).monospace());
            }
        });
}

/// Ask where to save the whole history as a text file, and write it.
fn save(atom: &Atom) -> String {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Save the instruction history")
        .set_file_name("history.txt")
        .save_file()
    else {
        return String::new();
    };
    let text = history_report(atom, HISTORY_LENGTH);
    match std::fs::write(&path, text) {
        Ok(()) => format!("Saved to {}", path.display()),
        Err(e) => format!("Cannot save {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draws(state: &mut State, atom: &mut Atom) {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, state, atom);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_window_draws_empty_and_with_a_history() {
        let mut state = State::default();
        let mut atom = Atom::new();
        draws(&mut state, &mut atom);
        atom.bus.debug.recording = true;
        atom.bus.poke(0x2900, 0xEA);
        atom.cpu.regs.pc = 0x2900;
        for _ in 0..3 {
            atom.step();
        }
        assert_eq!(atom.bus.debug.history_len(), 3);
        draws(&mut state, &mut atom);
    }
}
