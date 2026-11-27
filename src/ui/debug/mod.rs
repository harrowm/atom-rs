//! The debugger: each tool is its own OS window (an egui "viewport").
//!
//! The windows never run the emulator themselves. They show the machine's
//! state, let you edit it, and return [`Command`]s (run, step, ...) that the
//! main app carries out. That keeps all control of emulation in one place.

mod breakpoints_window;
mod console_window;
mod cpu_window;
mod disasm_window;
mod hardware_window;
mod history_window;
mod memory_window;

use atom_rs::machine::Atom;
use atom_rs::machine::debug_session::DebugSession;
use eframe::egui::{self, ViewportBuilder, ViewportId};

pub use atom_rs::machine::debug_console::parse_hex;
pub use atom_rs::machine::debug_session::Command;

/// Which windows are open, plus each window's own little bit of state.
#[derive(Default)]
pub struct DebugWindows {
    pub show_cpu: bool,
    pub show_disassembly: bool,
    pub show_memory: bool,
    pub show_breakpoints: bool,
    pub show_hardware: bool,
    pub show_console: bool,
    pub show_history: bool,

    disassembly: disasm_window::State,
    memory: memory_window::State,
    breakpoints: breakpoints_window::State,
    console: console_window::State,
    history: history_window::State,
}

impl DebugWindows {
    /// Show every open window and return the commands the user gave.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        atom: &mut Atom,
        session: &mut DebugSession,
    ) -> Vec<Command> {
        let mut commands = Vec::new();

        // Each window follows the same pattern; the macro just avoids repeating it.
        macro_rules! window {
            ($open:expr, $id:literal, $title:literal, $size:expr, |$ui:ident| $body:expr) => {
                if $open {
                    let mut close_requested = false;
                    ctx.show_viewport_immediate(
                        ViewportId::from_hash_of($id),
                        ViewportBuilder::default()
                            .with_title($title)
                            .with_inner_size($size),
                        |ui, _class| {
                            close_requested = ui.input(|i| i.viewport().close_requested());
                            egui::CentralPanel::default().show(ui, |$ui| $body);
                        },
                    );
                    if close_requested {
                        $open = false;
                    }
                }
            };
        }

        window!(self.show_cpu, "debug-cpu", "CPU", [360.0, 420.0], |ui| {
            cpu_window::show(ui, atom, session, &mut commands)
        });
        window!(
            self.show_disassembly,
            "debug-disassembly",
            "Disassembly",
            [420.0, 520.0],
            |ui| { disasm_window::show(ui, &mut self.disassembly, atom, &mut commands) }
        );
        window!(
            self.show_memory,
            "debug-memory",
            "Memory",
            [640.0, 520.0],
            |ui| { memory_window::show(ui, &mut self.memory, atom) }
        );
        window!(
            self.show_breakpoints,
            "debug-breakpoints",
            "Breakpoints",
            [360.0, 420.0],
            |ui| { breakpoints_window::show(ui, &mut self.breakpoints, atom) }
        );
        window!(
            self.show_console,
            "debug-console",
            "Command box",
            [620.0, 440.0],
            |ui| { console_window::show(ui, &mut self.console, atom, session) }
        );
        window!(
            self.show_history,
            "debug-history",
            "Instruction history",
            [720.0, 460.0],
            |ui| { history_window::show(ui, &mut self.history, atom) }
        );
        window!(
            self.show_hardware,
            "debug-hardware",
            "Chip registers",
            [380.0, 460.0],
            |ui| { hardware_window::show(ui, atom) }
        );

        commands
    }
}
