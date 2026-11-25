//! The debugger: each tool is its own OS window (an egui "viewport").
//!
//! The windows never run the emulator themselves. They show the machine's
//! state, let you edit it, and return [`Command`]s (run, step, ...) that the
//! main app carries out. That keeps all control of emulation in one place.

mod breakpoints_window;
mod cpu_window;
mod disasm_window;
mod hardware_window;
mod memory_window;

use atom_rs::machine::Atom;
use eframe::egui::{self, ViewportBuilder, ViewportId};

/// Something the user asked the debugger to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Run,
    Pause,
    /// Execute one instruction.
    Step,
    /// Like `Step`, but run a whole `JSR` subroutine as one step.
    StepOver,
    /// Run for one video frame.
    StepFrame,
    /// Run until the PC reaches this address.
    RunTo(u16),
    Reset,
    Nmi,
    ToggleIrq,
}

/// What the windows need to know about the app, beyond the machine itself.
pub struct AppState<'a> {
    pub running: bool,
    pub stop_message: Option<&'a str>,
}

/// Which windows are open, plus each window's own little bit of state.
#[derive(Default)]
pub struct DebugWindows {
    pub show_cpu: bool,
    pub show_disassembly: bool,
    pub show_memory: bool,
    pub show_breakpoints: bool,
    pub show_hardware: bool,

    disassembly: disasm_window::State,
    memory: memory_window::State,
    breakpoints: breakpoints_window::State,
}

impl DebugWindows {
    /// Show every open window and return the commands the user gave.
    pub fn show(&mut self, ctx: &egui::Context, atom: &mut Atom, app: &AppState) -> Vec<Command> {
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
            cpu_window::show(ui, atom, app, &mut commands)
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
            self.show_hardware,
            "debug-hardware",
            "Hardware",
            [380.0, 460.0],
            |ui| { hardware_window::show(ui, atom) }
        );

        commands
    }
}

/// Parse a hex number as typed by a person: `C000`, `$C000` or `0xC000`.
pub fn parse_hex(text: &str) -> Option<u16> {
    let text = text.trim();
    let text = text
        .strip_prefix('$')
        .or_else(|| text.strip_prefix("0x"))
        .unwrap_or(text);
    u16::from_str_radix(text, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::parse_hex;

    #[test]
    fn parses_hex_in_common_notations() {
        assert_eq!(parse_hex("C000"), Some(0xC000));
        assert_eq!(parse_hex(" $b800 "), Some(0xB800));
        assert_eq!(parse_hex("0xFF"), Some(0xFF));
        assert_eq!(parse_hex("zz"), None);
        assert_eq!(parse_hex("12345"), None, "too big for 16 bits");
    }
}
