//! Registers, flags and the run/step controls.

use atom_rs::cpu::flags;
use atom_rs::machine::Atom;
use atom_rs::machine::debug_session::DebugSession;
use eframe::egui::{self, DragValue};

use super::Command;

/// The flags in display order, with their one-letter names.
const FLAGS: [(char, u8); 8] = [
    ('N', flags::NEGATIVE),
    ('V', flags::OVERFLOW),
    ('-', flags::UNUSED),
    ('B', flags::BREAK),
    ('D', flags::DECIMAL),
    ('I', flags::IRQ_DISABLE),
    ('Z', flags::ZERO),
    ('C', flags::CARRY),
];

pub fn show(
    ui: &mut egui::Ui,
    atom: &mut Atom,
    session: &DebugSession,
    commands: &mut Vec<Command>,
) {
    controls(ui, atom, session, commands);
    ui.separator();

    // Registers can only be edited while stopped; otherwise the running
    // program would overwrite your edit a microsecond later.
    ui.add_enabled_ui(!session.running, |ui| {
        registers(ui, atom);
        ui.separator();
        flag_checkboxes(ui, atom);
    });
    ui.separator();

    ui.monospace(format!("Cycles: {}", atom.bus.cycles));
    ui.monospace(format!(
        "Frame {}  line {}",
        atom.bus.vdg.frames,
        atom.bus.vdg.line()
    ));
    if let Some(message) = &session.stop_message {
        ui.colored_label(egui::Color32::LIGHT_RED, message);
    }
    ui.separator();
    stack(ui, atom);
}

fn controls(
    ui: &mut egui::Ui,
    atom: &mut Atom,
    session: &DebugSession,
    commands: &mut Vec<Command>,
) {
    ui.horizontal_wrapped(|ui| {
        if session.running {
            if ui.button("Pause").clicked() {
                commands.push(Command::Pause);
            }
        } else if ui.button("Run").clicked() {
            commands.push(Command::Run);
        }
        if ui
            .button("Step")
            .on_hover_text("Execute one instruction")
            .clicked()
        {
            commands.push(Command::Step);
        }
        if ui
            .button("Step over")
            .on_hover_text("Run a JSR as one step")
            .clicked()
        {
            commands.push(Command::StepOver);
        }
        if ui
            .button("Frame")
            .on_hover_text("Run for one video frame")
            .clicked()
        {
            commands.push(Command::StepFrame);
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Reset").clicked() {
            commands.push(Command::Reset);
        }
        if ui
            .button("NMI")
            .on_hover_text("Trigger a non-maskable interrupt")
            .clicked()
        {
            commands.push(Command::Nmi);
        }
        let irq_label = if atom.bus.external_irq {
            "IRQ held"
        } else {
            "IRQ"
        };
        if ui
            .selectable_label(atom.bus.external_irq, irq_label)
            .on_hover_text("Hold the IRQ line asserted")
            .clicked()
        {
            commands.push(Command::ToggleIrq);
        }
    });
}

fn registers(ui: &mut egui::Ui, atom: &mut Atom) {
    let regs = &mut atom.cpu.regs;
    egui::Grid::new("registers").num_columns(4).show(ui, |ui| {
        ui.monospace("PC");
        ui.add(DragValue::new(&mut regs.pc).hexadecimal(4, false, true));
        ui.monospace("SP");
        ui.add(DragValue::new(&mut regs.sp).hexadecimal(2, false, true));
        ui.end_row();
        ui.monospace("A");
        ui.add(DragValue::new(&mut regs.a).hexadecimal(2, false, true));
        ui.monospace("P");
        ui.add(DragValue::new(&mut regs.p).hexadecimal(2, false, true));
        ui.end_row();
        ui.monospace("X");
        ui.add(DragValue::new(&mut regs.x).hexadecimal(2, false, true));
        ui.monospace("Y");
        ui.add(DragValue::new(&mut regs.y).hexadecimal(2, false, true));
        ui.end_row();
    });
}

fn flag_checkboxes(ui: &mut egui::Ui, atom: &mut Atom) {
    ui.horizontal(|ui| {
        for (name, mask) in FLAGS {
            let mut set = atom.cpu.regs.flag(mask);
            if ui.checkbox(&mut set, name.to_string()).changed() {
                atom.cpu.regs.set_flag(mask, set);
            }
        }
    });
}

/// The top few bytes of the stack, starting at the next byte to be pulled.
fn stack(ui: &mut egui::Ui, atom: &Atom) {
    let sp = atom.cpu.regs.sp;
    let bytes: Vec<String> = (1..=8u8)
        .map(|offset| {
            let addr = 0x0100 | sp.wrapping_add(offset) as u16;
            format!("{:02X}", atom.bus.peek(addr))
        })
        .collect();
    ui.monospace(format!("Stack: {}", bytes.join(" ")));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draws(atom: &mut Atom, session: &DebugSession) -> Vec<Command> {
        let ctx = egui::Context::default();
        let mut commands = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, atom, session, &mut commands);
        });
        output.textures_delta.clear();
        commands
    }

    #[test]
    fn the_window_draws_running_paused_and_stopped_at_a_breakpoint() {
        let mut atom = Atom::new();
        let mut session = DebugSession::default();
        assert!(draws(&mut atom, &session).is_empty(), "nothing is pressed");
        session.running = false;
        draws(&mut atom, &session);
        session.stop_message = Some("Breakpoint at $2900".to_string());
        draws(&mut atom, &session);
        atom.bus.external_irq = true;
        draws(&mut atom, &session);
    }

    #[test]
    fn every_flag_has_its_own_bit_and_letter() {
        let masks: Vec<u8> = FLAGS.iter().map(|&(_, mask)| mask).collect();
        assert_eq!(
            masks.iter().fold(0, |all, m| all | m),
            0xFF,
            "all eight bits"
        );
        let letters: String = FLAGS.iter().map(|&(letter, _)| letter).collect();
        assert_eq!(letters, "NV-BDIZC");
    }
}
