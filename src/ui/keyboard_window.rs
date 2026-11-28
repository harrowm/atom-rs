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

        let filter = self.filter.trim().to_string();
        let matches = |target: &Target| matches_filter(target, &filter);
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
        let (message, changed) = apply_capture(map, key, &target);
        self.message = message;
        changed
    }
}

/// A key was pressed while waiting to choose one for `target`: bind it (Esc
/// cancels). Returns what to tell the user and whether the map changed.
fn apply_capture(map: &mut KeyMap, key: Key, target: &Target) -> (String, bool) {
    if key == Key::Escape {
        return ("Cancelled".to_string(), false);
    }
    let message = match map.bind(key, target.clone()) {
        Some(old) => format!(
            "{} now presses {} (it used to press {})",
            host_label(key.name()),
            target.label(),
            old.label()
        ),
        None => format!("{} now presses {}", host_label(key.name()), target.label()),
    };
    (message, true)
}

/// Does `target` show up when the Filter box holds `filter`?
fn matches_filter(target: &Target, filter: &str) -> bool {
    let filter = filter.trim().to_ascii_uppercase();
    filter.is_empty() || target.label().to_ascii_uppercase().contains(&filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_target(name: &str) -> Target {
        Target::Key {
            name: name.to_string(),
            shift: false,
        }
    }

    #[test]
    fn host_keys_are_shown_as_their_symbols_when_those_are_plain_text() {
        assert_eq!(host_label("CloseBracket"), "]");
        assert_eq!(host_label("A"), "A");
        assert_eq!(
            host_label("ArrowLeft"),
            "ArrowLeft",
            "non-ASCII symbols keep their names"
        );
        assert_eq!(host_label("NoSuchKey"), "NoSuchKey");
    }

    #[test]
    fn a_pressed_key_is_bound_and_the_message_says_what_it_replaced() {
        let mut map = KeyMap::default();
        let (message, changed) = apply_capture(&mut map, Key::F1, &key_target("Z"));
        assert!(changed);
        assert!(message.contains("now presses Z"), "{message}");
        assert!(!message.contains("used to"), "{message}");
        assert_eq!(map.target_for(Key::F1), Some(&key_target("Z")));

        let (message, changed) = apply_capture(&mut map, Key::F1, &key_target("X"));
        assert!(changed);
        assert!(
            message.contains("now presses X (it used to press Z)"),
            "{message}"
        );
    }

    #[test]
    fn escape_cancels_and_changes_nothing() {
        let mut map = KeyMap::default();
        let (message, changed) = apply_capture(&mut map, Key::Escape, &key_target("Z"));
        assert!(!changed);
        assert_eq!(message, "Cancelled");
        assert_eq!(
            map.target_for(Key::Escape),
            KeyMap::default().target_for(Key::Escape)
        );
    }

    #[test]
    fn the_filter_ignores_case_and_surrounding_spaces_and_blank_matches_everything() {
        let target = key_target("LOCK");
        assert!(matches_filter(&target, ""));
        assert!(matches_filter(&target, "   "));
        assert!(matches_filter(&target, "loc"));
        assert!(matches_filter(&target, " OCK "));
        assert!(!matches_filter(&target, "zzz"));
    }

    /// Run one frame of the window's contents with the given events.
    fn frame(window: &mut KeyboardWindow, map: &mut KeyMap, events: Vec<Event>) -> bool {
        let ctx = egui::Context::default();
        let mut changed = false;
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            changed |= window.contents(ui, map); // egui may run a frame twice
        });
        output.textures_delta.clear();
        changed
    }

    fn key_event(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn the_window_draws_and_waiting_for_a_key_takes_the_next_one_pressed() {
        let mut window = KeyboardWindow::default();
        let mut map = KeyMap::default();
        assert!(
            !frame(&mut window, &mut map, vec![]),
            "drawing alone changes nothing"
        );

        window.capturing = Some(key_target("Z"));
        assert!(!frame(&mut window, &mut map, vec![]), "still waiting");
        assert!(window.capturing.is_some());
        assert!(frame(&mut window, &mut map, vec![key_event(Key::F2)]));
        assert!(window.capturing.is_none());
        assert_eq!(map.target_for(Key::F2), Some(&key_target("Z")));
        assert!(window.message.contains("now presses Z"));
    }

    #[test]
    fn escape_while_waiting_stops_waiting() {
        let mut window = KeyboardWindow::default();
        let mut map = KeyMap::default();
        window.capturing = Some(key_target("Z"));
        assert!(!frame(&mut window, &mut map, vec![key_event(Key::Escape)]));
        assert!(window.capturing.is_none());
        assert_eq!(window.message, "Cancelled");
    }

    #[test]
    fn the_filter_box_narrows_what_is_listed_and_still_draws() {
        let mut window = KeyboardWindow {
            filter: "lock".into(),
            ..Default::default()
        };
        let mut map = KeyMap::default();
        assert!(!frame(&mut window, &mut map, vec![]));
        window.filter = "nothing-matches-this".into();
        assert!(!frame(&mut window, &mut map, vec![]));
    }

    #[test]
    fn a_closed_window_does_nothing() {
        let mut window = KeyboardWindow::default();
        let mut map = KeyMap::default();
        let ctx = egui::Context::default();
        assert!(!window.show(&ctx, &mut map));
    }
}
