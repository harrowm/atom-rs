//! Execution breakpoints and read/write watchpoints.

use atom_rs::machine::Atom;
use eframe::egui;

use atom_rs::machine::condition::Condition;

use super::parse_hex;

pub struct State {
    new_breakpoint: String,
    /// Optional: only stop if this holds (see the Condition type).
    new_condition: String,
    /// What went wrong with the last Add, if anything.
    message: String,
    new_watch_addr: String,
    watch_read: bool,
    watch_write: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            new_breakpoint: String::new(),
            new_condition: String::new(),
            message: String::new(),
            new_watch_addr: String::new(),
            watch_read: false,
            watch_write: true,
        }
    }
}

/// What the Add button does for a breakpoint: returns what went wrong, or
/// nothing if the breakpoint was added.
fn add_breakpoint(atom: &mut Atom, addr_text: &str, condition_text: &str) -> String {
    let Some(addr) = parse_hex(addr_text) else {
        return "Enter the address in hex".to_string();
    };
    if condition_text.trim().is_empty() {
        atom.bus.debug.add_breakpoint(addr);
        return String::new();
    }
    match Condition::parse(condition_text) {
        Ok(condition) => {
            atom.bus.debug.add_conditional_breakpoint(addr, condition);
            String::new()
        }
        Err(message) => message,
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut State, atom: &mut Atom) {
    ui.heading("Breakpoints");
    ui.label("Stop before the instruction at an address runs, if the condition (when there is one) holds.");
    let mut remove = None;
    for &addr in &atom.bus.debug.breakpoints {
        ui.horizontal(|ui| {
            ui.monospace(format!("${addr:04X}"));
            if let Some(condition) = atom.bus.debug.conditions.get(&addr) {
                ui.label(format!("if {condition}"));
            }
            if ui.small_button("remove").clicked() {
                remove = Some(addr);
            }
        });
    }
    if let Some(addr) = remove {
        atom.bus.debug.remove_breakpoint(addr);
    }
    ui.horizontal(|ui| {
        ui.label("$");
        ui.add(egui::TextEdit::singleline(&mut state.new_breakpoint).desired_width(50.0));
        ui.label("if");
        ui.add(
            egui::TextEdit::singleline(&mut state.new_condition)
                .hint_text("a==5 && [0200]>2")
                .desired_width(140.0),
        );
        if ui.button("Add").clicked() {
            state.message = add_breakpoint(atom, &state.new_breakpoint, &state.new_condition);
            if state.message.is_empty() {
                state.new_breakpoint.clear();
                state.new_condition.clear();
            }
        }
    });
    if !state.message.is_empty() {
        ui.colored_label(egui::Color32::LIGHT_RED, &state.message);
    }

    ui.separator();
    ui.heading("Watchpoints");
    ui.label("Stop after an instruction reads or writes an address.");
    let mut remove = None;
    for (index, watch) in atom.bus.debug.watchpoints.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.monospace(format!("${:04X}", watch.addr));
            ui.checkbox(&mut watch.on_read, "read");
            ui.checkbox(&mut watch.on_write, "write");
            if ui.small_button("remove").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        atom.bus.debug.watchpoints.remove(index);
    }
    ui.horizontal(|ui| {
        ui.label("$");
        ui.add(egui::TextEdit::singleline(&mut state.new_watch_addr).desired_width(50.0));
        ui.checkbox(&mut state.watch_read, "read");
        ui.checkbox(&mut state.watch_write, "write");
        if ui.button("Add").clicked()
            && let Some(addr) = parse_hex(&state.new_watch_addr)
        {
            atom.bus
                .debug
                .add_watchpoint(addr, state.watch_read, state.watch_write);
            state.new_watch_addr.clear();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_draws_with_breakpoints_and_watchpoints_listed() {
        let ctx = egui::Context::default();
        let mut atom = Atom::new();
        let mut state = State::default();
        for listed in [false, true] {
            if listed {
                atom.bus.debug.add_breakpoint(0x2900);
                atom.bus.debug.add_watchpoint(0x0200, true, true);
            }
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, &mut state, &mut atom);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn adding_a_breakpoint_with_and_without_a_condition() {
        let mut atom = Atom::new();
        assert_eq!(add_breakpoint(&mut atom, "2900", ""), "");
        assert_eq!(
            add_breakpoint(&mut atom, "2910", "  "),
            "",
            "blank means no condition"
        );
        assert_eq!(add_breakpoint(&mut atom, "$2920", "a==5"), "");
        assert_eq!(atom.bus.debug.breakpoints.len(), 3);
        assert_eq!(atom.bus.debug.conditions.len(), 1);
        assert_eq!(atom.bus.debug.conditions[&0x2920].to_string(), "a==5");
    }

    #[test]
    fn a_bad_address_or_condition_adds_nothing_and_says_why() {
        let mut atom = Atom::new();
        assert!(add_breakpoint(&mut atom, "zz", "").contains("hex"));
        assert!(add_breakpoint(&mut atom, "2900", "q==1").contains("not a register"));
        assert!(add_breakpoint(&mut atom, "2900", "a").contains("needs a comparison"));
        assert!(atom.bus.debug.breakpoints.is_empty());
    }

    #[test]
    fn the_new_watch_boxes_start_as_write_only() {
        let state = State::default();
        assert!(!state.watch_read && state.watch_write);
    }
}
