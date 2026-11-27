//! The command box: type debugger commands (`break 2900`, `s 10`, `help`...)
//! and read what comes back. The commands themselves are carried out by
//! [`Console`], which knows nothing about windows.

use atom_rs::machine::Atom;
use atom_rs::machine::debug_console::{Console, describe_trace, state_report};
use atom_rs::machine::debug_session::DebugSession;
use eframe::egui::{self, Key, RichText};

/// How many lines of output are kept.
const MAX_LINES: usize = 500;

pub struct State {
    console: Console,
    lines: Vec<String>,
    input: String,
    /// Commands typed, oldest first, for the Up and Down keys.
    history: Vec<String>,
    /// Which history entry the Up/Down keys are showing, if any.
    history_position: Option<usize>,
    /// Was the machine running when we last looked? (To notice it stopping.)
    was_running: bool,
    focus_input: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            console: Console::default(),
            lines: vec!["Type help for the commands.".to_string()],
            input: String::new(),
            history: Vec::new(),
            history_position: None,
            was_running: false,
            focus_input: true,
        }
    }
}

impl State {
    fn print(&mut self, text: &str) {
        self.lines.extend(text.lines().map(str::to_string));
        let excess = self.lines.len().saturating_sub(MAX_LINES);
        self.lines.drain(..excess);
    }

    /// Show what has happened since the last frame: accesses logged by
    /// `watchr` and `watchw`, and the machine stopping at a breakpoint.
    fn report_events(&mut self, atom: &mut Atom, session: &DebugSession) {
        for hit in atom.bus.debug.take_trace_log() {
            self.print(&describe_trace(&hit));
        }
        if self.was_running && !session.running {
            if let Some(message) = &session.stop_message {
                self.print(message);
            }
            self.print(&state_report(atom));
        }
        self.was_running = session.running;
    }

    /// Run the line in the box, and remember it.
    fn submit(&mut self, atom: &mut Atom, session: &mut DebugSession) {
        let line = std::mem::take(&mut self.input);
        if !line.trim().is_empty() {
            self.print(&format!("> {}", line.trim()));
            if self.history.last() != Some(&line) {
                self.history.push(line.clone());
            }
        } else {
            self.print(">");
        }
        self.history_position = None;
        let reply = self.console.execute(&line, atom, session);
        self.print(&reply);
        // What the command itself started (running, say) is not a stop to report.
        self.was_running = session.running;
    }

    /// Up and Down walk through the commands typed before.
    fn browse_history(&mut self, up: bool) {
        if self.history.is_empty() {
            return;
        }
        let position = match (self.history_position, up) {
            (None, true) => self.history.len() - 1,
            (Some(0), true) => 0,
            (Some(p), true) => p - 1,
            (Some(p), false) if p + 1 < self.history.len() => p + 1,
            (None, false) => return,
            (Some(_), false) => {
                self.history_position = None;
                self.input.clear();
                return;
            }
        };
        self.history_position = Some(position);
        self.input = self.history[position].clone();
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut State, atom: &mut Atom, session: &mut DebugSession) {
    state.report_events(atom, session);

    egui::Panel::bottom("console-input").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.monospace(">");
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.input)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
            if response.has_focus() {
                if ui.input(|i| i.key_pressed(Key::ArrowUp)) {
                    state.browse_history(true);
                }
                if ui.input(|i| i.key_pressed(Key::ArrowDown)) {
                    state.browse_history(false);
                }
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                state.submit(atom, session);
                state.focus_input = true;
            }
            if std::mem::take(&mut state.focus_input) {
                response.request_focus();
            }
        });
    });

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for line in &state.lines {
                ui.label(RichText::new(line).monospace());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draws(state: &mut State, atom: &mut Atom, session: &mut DebugSession) {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, state, atom, session);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_window_draws_and_a_submitted_command_is_answered() {
        let mut state = State::default();
        let mut atom = Atom::new();
        let mut session = DebugSession::default();
        draws(&mut state, &mut atom, &mut session);
        state.input = "break 2900".to_string();
        state.submit(&mut atom, &mut session);
        assert!(atom.bus.debug.breakpoints.contains(&0x2900));
        let text = state.lines.join("\n");
        assert!(
            text.contains("> break 2900") && text.contains("Breakpoint at 2900"),
            "{text}"
        );
        assert!(state.input.is_empty());
        draws(&mut state, &mut atom, &mut session);
    }

    #[test]
    fn up_and_down_walk_through_what_was_typed() {
        let mut state = State::default();
        let mut atom = Atom::new();
        let mut session = DebugSession::default();
        for line in ["r", "blist", "help"] {
            state.input = line.to_string();
            state.submit(&mut atom, &mut session);
        }
        state.browse_history(true);
        assert_eq!(state.input, "help");
        state.browse_history(true);
        assert_eq!(state.input, "blist");
        state.browse_history(true);
        state.browse_history(true);
        assert_eq!(state.input, "r", "stops at the oldest");
        state.browse_history(false);
        assert_eq!(state.input, "blist");
        state.browse_history(false);
        state.browse_history(false);
        assert_eq!(state.input, "", "back to a blank line");
    }

    #[test]
    fn the_machine_stopping_is_reported_once() {
        let mut state = State::default();
        let mut atom = Atom::new();
        let mut session = DebugSession::default(); // running
        draws(&mut state, &mut atom, &mut session); // sees it running
        session.running = false;
        session.stop_message = Some("Breakpoint at $2900".to_string());
        draws(&mut state, &mut atom, &mut session);
        draws(&mut state, &mut atom, &mut session);
        let text = state.lines.join("\n");
        assert_eq!(text.matches("Breakpoint at $2900").count(), 1, "{text}");
        assert!(text.contains("PC="), "the registers come with it: {text}");
    }

    #[test]
    fn logged_accesses_appear_and_the_log_is_bounded() {
        let mut state = State::default();
        let mut atom = Atom::new();
        let mut session = DebugSession::default();
        atom.bus.debug.add_trace(0x0200, false, true);
        atom.bus.poke(0x2900, 0x8D); // STA $0200
        atom.bus.poke(0x2901, 0x00);
        atom.bus.poke(0x2902, 0x02);
        atom.cpu.regs.pc = 0x2900;
        atom.step();
        draws(&mut state, &mut atom, &mut session);
        assert!(
            state
                .lines
                .iter()
                .any(|l| l.starts_with("Write 00 to 0200"))
        );
        for n in 0..(MAX_LINES * 2) {
            state.print(&format!("line {n}"));
        }
        assert_eq!(state.lines.len(), MAX_LINES);
        assert!(
            state
                .lines
                .last()
                .unwrap()
                .ends_with(&format!("{}", MAX_LINES * 2 - 1))
        );
    }
}
