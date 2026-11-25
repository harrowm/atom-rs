//! The Keyboard window: shows which of your keys press which Atom keys, and
//! lets you change them.

use eframe::egui::{self, Color32, Event, Key, RichText};

use super::keymap::{Group, KeyMap, Target, all_targets};

/// How a host key is shown: `]` rather than `CloseBracket`. (egui's symbols
/// for keys such as the arrows are not ASCII and may not be in the font, so
/// those keep their names.)
fn host_label(name: &str) -> String {
    match Key::from_name(name) {
        Some(key) if key.symbol_or_name().is_ascii() => key.symbol_or_name().to_string(),
        _ => name.to_string(),
    }
}

#[derive(Default)]
pub struct KeyboardWindow {
    pub open: bool,
    /// The Atom key we are waiting for a host key to be chosen for.
    capturing: Option<Target>,
    filter: String,
    /// What the last change did, e.g. "Z is now LOCK (it was Z)".
    message: String,
}

impl KeyboardWindow {
    /// Show the window if it is open. Returns true if the map was changed
    /// (so the caller can save it).
    pub fn show(&mut self, ctx: &egui::Context, map: &mut KeyMap) -> bool {
        if !self.open {
            return false;
        }
        let mut changed = false;
        let mut close_requested = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("keyboard-mapping"),
            egui::ViewportBuilder::default()
                .with_title("Keyboard mapping")
                .with_inner_size([640.0, 680.0]),
            |ui, _class| {
                close_requested = ui.input(|i| i.viewport().close_requested());
                egui::CentralPanel::default().show(ui, |ui| {
                    changed = self.contents(ui, map);
                });
            },
        );
        if close_requested {
            self.open = false;
            self.capturing = None;
        }
        changed
    }

    fn contents(&mut self, ui: &mut egui::Ui, map: &mut KeyMap) -> bool {
        let mut changed = self.capture_key(ui, map);

        ui.label(
            "Click Set next to an Atom key, then press the key on your keyboard that \
             should act as it. Esc cancels. Click a key's name to remove it.",
        );
        ui.label(
            RichText::new(
                "Your Shift, Ctrl and Alt keys are always the Atom's SHIFT, CTRL and REPT. \
                 F6, F7 and F12 are the emulator's own keys.",
            )
            .weak(),
        );
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(100.0));
            if ui.button("Reset all to defaults").clicked() {
                *map = KeyMap::default();
                self.capturing = None;
                self.message = "Back to the default keys".to_string();
                changed = true;
            }
        });
        if !self.message.is_empty() {
            ui.colored_label(Color32::LIGHT_GREEN, &self.message);
        }
        ui.separator();

        let filter = self.filter.trim().to_ascii_uppercase();
        let matches = |target: &Target| {
            filter.is_empty() || target.label().to_ascii_uppercase().contains(&filter)
        };
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for group in Group::ALL {
                    let targets: Vec<Target> = all_targets()
                        .into_iter()
                        .filter(|t| t.group() == group)
                        .collect();
                    // While searching, open every section that has a match.
                    let has_match = targets.iter().any(&matches);
                    if !filter.is_empty() && !has_match && group != Group::Symbols {
                        continue;
                    }
                    let mut section = egui::CollapsingHeader::new(group.title())
                        .id_salt(group.title())
                        .default_open(group == Group::Symbols);
                    if !filter.is_empty() {
                        section = section.open(Some(true));
                    }
                    section.show(ui, |ui| {
                        egui::Grid::new(("keyboard-map", group.title()))
                            .num_columns(3)
                            .striped(true)
                            .show(ui, |ui| {
                                for target in targets.iter().filter(|t| matches(t)) {
                                    changed |= self.row(ui, map, target);
                                }
                                if group == Group::Symbols {
                                    changed |= self.shifted_rows(ui, map, &matches);
                                }
                            });
                    });
                }
            });
        changed
    }

    /// Keys such as the Down arrow press an Atom key with Shift held, so they
    /// have no row of their own: list them after the symbols.
    fn shifted_rows(
        &mut self,
        ui: &mut egui::Ui,
        map: &mut KeyMap,
        matches: &dyn Fn(&Target) -> bool,
    ) -> bool {
        let mut changed = false;
        for (host, target) in map.shifted_bindings() {
            if !matches(&target) {
                continue;
            }
            ui.monospace(target.label());
            if ui
                .small_button(format!("{}  x", host_label(&host)))
                .clicked()
            {
                map.unbind(&host);
                changed = true;
            }
            ui.label("");
            ui.end_row();
        }
        changed
    }

    /// One row: an Atom key, the host keys bound to it, and its Set button.
    fn row(&mut self, ui: &mut egui::Ui, map: &mut KeyMap, target: &Target) -> bool {
        let mut changed = false;
        ui.monospace(target.label());
        // Not wrapped: a key's name should never be split over two lines.
        ui.horizontal(|ui| {
            for host in map.host_keys_for(target) {
                if ui
                    .small_button(format!("{}  x", host_label(&host)))
                    .on_hover_text("Remove")
                    .clicked()
                {
                    map.unbind(&host);
                    self.message =
                        format!("{} no longer presses {}", host_label(&host), target.label());
                    changed = true;
                }
            }
        });
        let waiting = self.capturing.as_ref() == Some(target);
        let label = if waiting { "Press a key..." } else { "Set" };
        if ui.selectable_label(waiting, label).clicked() {
            self.capturing = if waiting { None } else { Some(target.clone()) };
        }
        ui.end_row();
        changed
    }

    /// If we are waiting for a key, take the next one pressed.
    fn capture_key(&mut self, ui: &mut egui::Ui, map: &mut KeyMap) -> bool {
        let Some(target) = self.capturing.clone() else {
            return false;
        };
        let pressed: Option<Key> = ui.input(|i| {
            i.events.iter().find_map(|event| match event {
                Event::Key {
                    key, pressed: true, ..
                } => Some(*key),
                _ => None,
            })
        });
        let Some(key) = pressed else { return false };
        self.capturing = None;
        if key == Key::Escape {
            self.message = "Cancelled".to_string();
            return false;
        }
        self.message = match map.bind(key, target.clone()) {
            Some(old) => format!(
                "{} now presses {} (it used to press {})",
                host_label(key.name()),
                target.label(),
                old.label()
            ),
            None => format!("{} now presses {}", host_label(key.name()), target.label()),
        };
        true
    }
}
