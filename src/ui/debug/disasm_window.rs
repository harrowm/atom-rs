//! Disassembly around the program counter, with click-to-set breakpoints.

use atom_rs::cpu::disasm::disassemble;
use atom_rs::machine::Atom;
use eframe::egui::{self, Color32, RichText};

use super::{Command, parse_hex};

const LINES: usize = 32;

pub struct State {
    follow_pc: bool,
    /// Where to start when not following the PC.
    start: u16,
    goto_text: String,
}

impl Default for State {
    fn default() -> Self {
        State {
            follow_pc: true,
            start: 0xC000,
            goto_text: String::new(),
        }
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut State, atom: &mut Atom, commands: &mut Vec<Command>) {
    let pc = atom.cpu.regs.pc;

    ui.horizontal(|ui| {
        ui.checkbox(&mut state.follow_pc, "Follow PC");
        ui.add_enabled_ui(!state.follow_pc, |ui| {
            ui.label("Go to $");
            let response =
                ui.add(egui::TextEdit::singleline(&mut state.goto_text).desired_width(50.0));
            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(addr) = parse_hex(&state.goto_text)
            {
                state.start = addr;
            }
        });
    });
    ui.label(RichText::new("Click ● to toggle a breakpoint. Right-click a line for more.").weak());
    ui.separator();

    let start = if state.follow_pc {
        start_before(pc, atom)
    } else {
        state.start
    };

    let mut addr = start;
    for _ in 0..LINES {
        let instruction = disassemble(addr, |a| atom.bus.peek(a));
        let bytes: String = (0..instruction.length)
            .map(|i| format!("{:02X} ", atom.bus.peek(addr.wrapping_add(i))))
            .collect();
        let has_breakpoint = atom.bus.debug.breakpoints.contains(&addr);

        ui.horizontal(|ui| {
            let dot = RichText::new("●").color(if has_breakpoint {
                Color32::RED
            } else {
                Color32::DARK_GRAY
            });
            if ui
                .add(egui::Label::new(dot).sense(egui::Sense::click()))
                .clicked()
            {
                atom.bus.debug.toggle_breakpoint(addr);
            }
            let mut text =
                RichText::new(format!("{addr:04X}  {bytes:<9} {}", instruction.text)).monospace();
            if addr == pc {
                text = text
                    .background_color(Color32::from_rgb(40, 80, 140))
                    .color(Color32::WHITE);
            }
            let line = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
            line.context_menu(|ui| {
                if ui.button("Run to here").clicked() {
                    commands.push(Command::RunTo(addr));
                    ui.close();
                }
                if ui.button("Set PC here").clicked() {
                    atom.cpu.regs.pc = addr;
                    ui.close();
                }
            });
        });
        addr = addr.wrapping_add(instruction.length);
    }
}

/// Find a start address a few instructions before `pc` that disassembles
/// cleanly onto `pc`, so you can see what led up to it. Machine code cannot be
/// disassembled backwards reliably, so we try a few starting points.
fn start_before(pc: u16, atom: &Atom) -> u16 {
    for back in (3..=12u16).rev() {
        let start = pc.wrapping_sub(back);
        let mut addr = start;
        while addr.wrapping_sub(start) < back {
            addr = addr.wrapping_add(disassemble(addr, |a| atom.bus.peek(a)).length);
        }
        if addr == pc {
            return start;
        }
    }
    pc
}
