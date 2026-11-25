//! Execution breakpoints and read/write watchpoints.

use atom_rs::machine::Atom;
use atom_rs::machine::debug::Watchpoint;
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
        atom.bus.debug.breakpoints.remove(&addr);
    }
    ui.horizontal(|ui| {
        ui.label("$");
        ui.add(egui::TextEdit::singleline(&mut state.new_breakpoint).desired_width(50.0));
        if ui.button("Add").clicked()
            && let Some(addr) = parse_hex(&state.new_breakpoint)
        {
            atom.bus.debug.breakpoints.insert(addr);
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
            atom.bus.debug.watchpoints.push(Watchpoint {
                addr,
                on_read: state.watch_read,
                on_write: state.watch_write,
            });
            state.new_watch_addr.clear();
        }
    });
}
