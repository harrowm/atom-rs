//! The Tape catalogue window: what is on the tape in the recorder.
//!
//! The real Atom cannot list a tape without playing it (you type `*CAT` and
//! wait). This reads the tape image instead, so you can see what to `LOAD`.

use eframe::egui::{self, RichText};

use atom_rs::machine::tape::{TapeFile, TapeImage};

#[derive(Default)]
pub struct TapeCatalogueWindow {
    pub open: bool,
}

impl TapeCatalogueWindow {
    pub fn show(&mut self, ctx: &egui::Context, tape: Option<&TapeImage>) {
        if !self.open {
            return;
        }
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("tape-catalogue"),
            egui::ViewportBuilder::default()
                .with_title("Tape catalogue")
                .with_inner_size([560.0, 360.0]),
            |ui, _class| {
                close = ui.input(|i| i.viewport().close_requested());
                egui::CentralPanel::default().show(ui, |ui| match tape {
                    Some(tape) => contents(ui, &tape.catalogue()),
                    None => {
                        ui.label("There is no tape in the recorder.");
                    }
                });
            },
        );
        self.open &= !close;
    }
}

fn contents(ui: &mut egui::Ui, files: &[TapeFile]) {
    if files.is_empty() {
        ui.label("No Atom files were found on this tape.");
        return;
    }
    ui.label(
        RichText::new("To load one, type  LOAD \"NAME\"  (or *RUN \"NAME\")  and press Play.")
            .weak(),
    );
    ui.add_space(6.0);
    egui::Grid::new("tape-catalogue")
        .striped(true)
        .num_columns(5)
        .show(ui, |ui| {
            for heading in ["Name", "Load", "Run", "Size", "Blocks"] {
                ui.label(RichText::new(heading).strong());
            }
            ui.end_row();
            for file in files {
                ui.label(RichText::new(&file.name).monospace());
                ui.label(RichText::new(format!("{:04X}", file.load_address)).monospace());
                ui.label(RichText::new(format!("{:04X}", file.exec_address)).monospace());
                ui.label(RichText::new(format!("{:04X}", file.length)).monospace());
                let blocks = if file.complete {
                    file.blocks.to_string()
                } else {
                    format!("{} (incomplete)", file.blocks)
                };
                ui.label(blocks);
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draws(contents: impl FnOnce(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        let mut contents = Some(contents);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                if let Some(contents) = contents.take() {
                    contents(ui);
                }
            });
        });
        output.textures_delta.clear();
    }

    #[test]
    fn draws_a_catalogue_an_empty_one_and_no_tape() {
        let file = TapeFile {
            name: "PINBALL".into(),
            load_address: 0x2900,
            exec_address: 0x2900,
            length: 0x0AFF,
            blocks: 11,
            complete: true,
        };
        draws(|ui| contents(ui, std::slice::from_ref(&file)));
        draws(|ui| contents(ui, &[]));
        draws(|ui| {
            contents(
                ui,
                &[TapeFile {
                    complete: false,
                    ..file
                }],
            );
        });
    }
}
