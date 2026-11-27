//! A hex dump of all 64 KiB, with a poke box for editing RAM.

use atom_rs::machine::Atom;
use atom_rs::machine::memory::Region;
use eframe::egui::{self, Color32, RichText};

use super::parse_hex;

const BYTES_PER_ROW: usize = 16;
const ROWS: usize = 0x10000 / BYTES_PER_ROW;

#[derive(Default)]
pub struct State {
    goto_text: String,
    scroll_to_row: Option<usize>,
    poke_addr: String,
    poke_value: String,
    message: String,
}

pub fn show(ui: &mut egui::Ui, state: &mut State, atom: &mut Atom) {
    ui.horizontal(|ui| {
        ui.label("Go to $");
        let response = ui.add(egui::TextEdit::singleline(&mut state.goto_text).desired_width(50.0));
        if response.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Some(addr) = parse_hex(&state.goto_text)
        {
            state.scroll_to_row = Some(addr as usize / BYTES_PER_ROW);
        }
        for (label, addr) in [
            ("Zero page", 0x0000),
            ("Video", 0x8000),
            ("I/O", 0xB000),
            ("BASIC", 0xC000),
            ("Kernel", 0xF000),
        ] {
            if ui.small_button(label).clicked() {
                state.scroll_to_row = Some(addr / BYTES_PER_ROW);
            }
        }
    });
    ui.separator();

    poke_box(ui, state, atom);
    ui.separator();

    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let mut scroll = egui::ScrollArea::vertical().auto_shrink(false);
    if let Some(row) = state.scroll_to_row.take() {
        scroll =
            scroll.vertical_scroll_offset(row as f32 * (row_height + ui.spacing().item_spacing.y));
    }
    scroll.show_rows(ui, row_height, ROWS, |ui, rows| {
        for row in rows {
            hex_row(ui, state, atom, row);
        }
    });
}

fn hex_row(ui: &mut egui::Ui, state: &mut State, atom: &Atom, row: usize) {
    let base = (row * BYTES_PER_ROW) as u16;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.monospace(format!("{base:04X}"));
        let mut ascii = String::new();
        for offset in 0..BYTES_PER_ROW as u16 {
            let addr = base + offset;
            let value = atom.bus.peek(addr);
            let colour = region_colour(atom.bus.memory.region(addr));
            let mut text = RichText::new(format!("{value:02X}"))
                .monospace()
                .color(colour);
            if addr == atom.cpu.regs.pc {
                text = text.background_color(Color32::from_rgb(40, 80, 140));
            }
            if ui
                .add(egui::Label::new(text).sense(egui::Sense::click()))
                .clicked()
            {
                state.poke_addr = format!("{addr:04X}");
                state.poke_value = format!("{value:02X}");
            }
            ascii.push(if (0x20..0x7F).contains(&value) {
                value as char
            } else {
                '.'
            });
        }
        ui.monospace(ascii);
    });
}

/// Colour-code by what is at the address, so ROM, RAM and I/O stand out.
fn region_colour(region: Region) -> Color32 {
    match region {
        Region::Ram => Color32::LIGHT_GRAY,
        Region::VideoRam => Color32::LIGHT_GREEN,
        Region::Rom(_) => Color32::from_rgb(140, 170, 220),
        Region::Ppi
        | Region::Via
        | Region::Fdc
        | Region::AtoMmc
        | Region::Sid
        | Region::RamRom
        | Region::Gdos1770 => Color32::from_rgb(240, 200, 90),
        Region::Unmapped => Color32::DARK_GRAY,
    }
}

/// What the Write button does: put the byte typed in the value box at the
/// address typed in the address box, and say what happened.
fn write_byte(atom: &mut Atom, addr_text: &str, value_text: &str) -> String {
    match (parse_hex(addr_text), parse_hex(value_text)) {
        (Some(addr), Some(value)) if value <= 0xFF => {
            if atom.bus.poke(addr, value as u8) {
                format!("Wrote ${value:02X} to ${addr:04X}")
            } else {
                format!("${addr:04X} is not RAM")
            }
        }
        _ => "Enter hex values".to_string(),
    }
}

fn poke_box(ui: &mut egui::Ui, state: &mut State, atom: &mut Atom) {
    ui.horizontal(|ui| {
        ui.label("Write $");
        ui.add(egui::TextEdit::singleline(&mut state.poke_value).desired_width(30.0));
        ui.label("to $");
        ui.add(egui::TextEdit::singleline(&mut state.poke_addr).desired_width(50.0));
        if ui.button("Write").clicked() {
            state.message = write_byte(atom, &state.poke_addr, &state.poke_value);
        }
        ui.label(RichText::new(&state.message).weak());
    });
    ui.label(
        RichText::new(
            "Click a byte to fill the boxes. Blue = ROM, green = video RAM, yellow = I/O.",
        )
        .weak(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_a_byte_changes_ram_and_says_so() {
        let mut atom = Atom::new();
        assert_eq!(write_byte(&mut atom, "2900", "A9"), "Wrote $A9 to $2900");
        assert_eq!(atom.bus.peek(0x2900), 0xA9);
        assert_eq!(write_byte(&mut atom, "$0300", "0xFF"), "Wrote $FF to $0300");
        assert_eq!(atom.bus.peek(0x0300), 0xFF);
        assert_eq!(
            write_byte(&mut atom, "8000", "41"),
            "Wrote $41 to $8000",
            "video RAM too"
        );
    }

    #[test]
    fn rom_and_chips_cannot_be_written() {
        let mut atom = Atom::new();
        assert_eq!(write_byte(&mut atom, "C000", "12"), "$C000 is not RAM");
        assert_eq!(
            write_byte(&mut atom, "B000", "12"),
            "$B000 is not RAM",
            "an I/O address"
        );
    }

    #[test]
    fn bad_input_is_refused_and_changes_nothing() {
        let mut atom = Atom::new();
        atom.bus.poke(0x2900, 7);
        for (addr, value) in [
            ("zz", "1"),
            ("2900", "xx"),
            ("2900", "100"),
            ("", ""),
            ("12345", "1"),
        ] {
            assert_eq!(
                write_byte(&mut atom, addr, value),
                "Enter hex values",
                "{addr:?} {value:?}"
            );
        }
        assert_eq!(atom.bus.peek(0x2900), 7);
    }

    fn draws(state: &mut State, atom: &mut Atom) {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, state, atom);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_window_draws() {
        let mut state = State::default();
        let mut atom = Atom::new();
        draws(&mut state, &mut atom);
        state.scroll_to_row = Some(0x2900 / BYTES_PER_ROW);
        draws(&mut state, &mut atom);
    }
}
