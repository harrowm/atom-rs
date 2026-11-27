//! Execution breakpoints and read/write watchpoints.

use atom_rs::machine::Atom;
use eframe::egui;

use super::parse_hex;

pub struct State {
    new_breakpoint: String,
    new_watch_addr: String,
    watch_read: bool,
    watch_write: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            new_breakpoint: String::new(),
            new_watch_addr: String::new(),
            watch_read: false,
            watch_write: true,
        }
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut State, atom: &mut Atom) {
    ui.heading("Breakpoints");
    ui.label("Stop before the instruction at an address runs.");
    let mut remove = None;
    for &addr in &atom.bus.debug.breakpoints {
        ui.horizontal(|ui| {
            ui.monospace(format!("${addr:04X}"));
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
        if ui.button("Add").clicked()
            && let Some(addr) = parse_hex(&state.new_breakpoint)
        {
            atom.bus.debug.add_breakpoint(addr);
            state.new_breakpoint.clear();
        }
    });

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
    fn the_new_watch_boxes_start_as_write_only() {
        let state = State::default();
        assert!(!state.watch_read && state.watch_write);
    }
}
