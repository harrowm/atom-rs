//! Mapping host (PC/Mac) keys onto the Atom keyboard.
//!
//! A [`KeyMap`] is a table from host keys to Atom keys. The defaults follow
//! Atomulator and map by key *position*, so they assume a US-style layout: the
//! key labelled `;` on your keyboard is the Atom's `;` key, and Shift works the
//! same way as on the Atom (Shift+`;` types `+`, not `:`). The cursor keys
//! need special handling because the Atom has one key for up/down and one for
//! left/right, selected with Shift.
//!
//! The table can be changed from the Keyboard window and is saved to
//! `~/.config/atom-rs/keymap.json`.
//!
//! The host's Shift, Ctrl and Alt keys are not in the table (they are
//! modifiers, not ordinary keys): they are always the Atom's SHIFT, CTRL and
//! REPT. Any *other* host key can be bound to those Atom keys too.

use std::collections::BTreeMap;
use std::path::PathBuf;

use atom_rs::machine::keyboard::{LAYOUT, find_key};
use eframe::egui::Key;
use serde::{Deserialize, Serialize};

/// What pressing a host key does on the Atom.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Target {
    /// A key of the Atom's matrix, by its name in
    /// [`atom_rs::machine::keyboard::LAYOUT`], with the Atom's Shift held too
    /// if `shift` is set (only the Down and Left arrows need that).
    Key {
        name: String,
        shift: bool,
    },
    /// The Atom's three keys that are wired separately from the matrix.
    Shift,
    Ctrl,
    Rept,
}

impl Target {
    pub fn key(name: &str) -> Target {
        Target::Key {
            name: name.to_string(),
            shift: false,
        }
    }

    fn shifted(name: &str) -> Target {
        Target::Key {
            name: name.to_string(),
            shift: true,
        }
    }

    /// How the Atom key is shown to a person.
    pub fn label(&self) -> String {
        match self {
            Target::Key { name, shift: false } => name.clone(),
            Target::Key { name, shift: true } => format!("Shift + {name}"),
            Target::Shift => "SHIFT".to_string(),
            Target::Ctrl => "CTRL".to_string(),
            Target::Rept => "REPT".to_string(),
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Target::Key { name, .. } => find_key(name).is_some(),
            _ => true,
        }
    }
}

/// Every Atom key a host key can be bound to, in the order the Keyboard
/// window lists them: letters, digits, then everything else.
pub fn all_targets() -> Vec<Target> {
    let mut names: Vec<&str> = LAYOUT
        .iter()
        .flatten()
        .copied()
        .filter(|k| !k.is_empty())
        .collect();
    let group = |name: &str| match name.chars().next() {
        Some(c) if name.len() == 1 && c.is_ascii_alphabetic() => 0,
        Some(c) if name.len() == 1 && c.is_ascii_digit() => 1,
        _ => 2,
    };
    names.sort_by_key(|name| (group(name), *name));
    let mut targets: Vec<Target> = names.into_iter().map(Target::key).collect();
    targets.extend([Target::Shift, Target::Ctrl, Target::Rept]);
    targets
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMap {
    /// Host key (by egui's name for it, e.g. `"ArrowUp"`, `"A"`) to Atom key.
    bindings: BTreeMap<String, Target>,
}

impl Default for KeyMap {
    fn default() -> Self {
        let mut map = KeyMap {
            bindings: BTreeMap::new(),
        };
        let mut set = |key: Key, target: Target| {
            map.bindings.insert(key.name().to_string(), target);
        };

        // Letters and digits: the Atom key of the same name.
        for &key in Key::ALL {
            let name = key.name();
            if name.len() == 1
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            {
                set(key, Target::key(name));
            }
        }
        set(Key::Space, Target::key("SPACE"));
        set(Key::Enter, Target::key("RETURN"));
        set(Key::Backspace, Target::key("DELETE"));
        set(Key::Delete, Target::key("DELETE"));
        set(Key::Escape, Target::key("ESC"));
        // Caps Lock behaves as a toggle on a Mac, which is awkward as a "left"
        // key in a game, so the Backtick key (above Tab) is an alternative.
        set(Key::CapsLock, Target::key("LOCK"));
        set(Key::Backtick, Target::key("LOCK"));
        set(Key::End, Target::key("COPY"));
        set(Key::Tab, Target::key("^"));
        set(Key::ArrowUp, Target::key("UP"));
        set(Key::ArrowDown, Target::shifted("UP"));
        set(Key::ArrowRight, Target::key("RIGHT"));
        set(Key::ArrowLeft, Target::shifted("RIGHT"));
        set(Key::Minus, Target::key("-"));
        set(Key::Comma, Target::key(","));
        set(Key::Period, Target::key("."));
        set(Key::Slash, Target::key("/"));
        set(Key::Semicolon, Target::key(";"));
        set(Key::Quote, Target::key(":"));
        set(Key::Colon, Target::key(":"));
        set(Key::Equals, Target::key("@"));
        set(Key::OpenBracket, Target::key("["));
        set(Key::CloseBracket, Target::key("]"));
        set(Key::Backslash, Target::key("\\"));
        map
    }
}

impl KeyMap {
    /// What a host key does, if anything.
    pub fn target_for(&self, key: Key) -> Option<&Target> {
        self.bindings.get(key.name())
    }

    /// Make `key` press `target`. Returns what the key did before, if it did
    /// something else (it can only do one thing).
    pub fn bind(&mut self, key: Key, target: Target) -> Option<Target> {
        let old = self
            .bindings
            .insert(key.name().to_string(), target.clone())?;
        (old != target).then_some(old)
    }

    /// Stop a host key doing anything.
    pub fn unbind(&mut self, host_key_name: &str) {
        self.bindings.remove(host_key_name);
    }

    /// The host keys that press `target`, by name.
    pub fn host_keys_for(&self, target: &Target) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|(_, bound)| *bound == target)
            .map(|(host, _)| host.clone())
            .collect()
    }

    /// Bindings to Atom keys other than the plain matrix keys and the three
    /// special ones, such as the Down arrow's `Shift + UP`.
    pub fn shifted_bindings(&self) -> Vec<(String, Target)> {
        self.bindings
            .iter()
            .filter(|(_, target)| matches!(target, Target::Key { shift: true, .. }))
            .map(|(host, target)| (host.clone(), target.clone()))
            .collect()
    }

    /// Drop anything that no longer makes sense (an unknown key name, for
    /// example from an edited or older file).
    fn sanitised(mut self) -> Self {
        self.bindings
            .retain(|host, target| Key::from_name(host).is_some() && target.is_valid());
        self
    }

    // ---- Saving and loading -------------------------------------------------

    /// `~/.config/atom-rs/keymap.json`
    pub fn path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join(".config")
                .join("atom-rs")
                .join("keymap.json"),
        )
    }

    /// Load the saved map, or the defaults if there is none. The error
    /// describes a file that exists but could not be used.
    pub fn load() -> (KeyMap, Option<String>) {
        let Some(path) = Self::path() else {
            return (KeyMap::default(), None);
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<KeyMap>(&text) {
                Ok(map) => (map.sanitised(), None),
                Err(e) => (
                    KeyMap::default(),
                    Some(format!(
                        "{} is not a valid key map ({e}); using the defaults",
                        path.display()
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (KeyMap::default(), None),
            Err(e) => (
                KeyMap::default(),
                Some(format!("cannot read {}: {e}", path.display())),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no home folder to save the key map in")?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(map: &KeyMap, key: Key) -> Option<Target> {
        map.target_for(key).cloned()
    }

    #[test]
    fn defaults_map_letters_digits_and_the_special_keys() {
        let map = KeyMap::default();
        assert_eq!(target(&map, Key::A), Some(Target::key("A")));
        assert_eq!(target(&map, Key::Num7), Some(Target::key("7")));
        assert_eq!(target(&map, Key::Enter), Some(Target::key("RETURN")));
        assert_eq!(target(&map, Key::CapsLock), Some(Target::key("LOCK")));
        assert_eq!(target(&map, Key::Backtick), Some(Target::key("LOCK")));
        assert_eq!(
            target(&map, Key::F12),
            None,
            "F12 is the emulator's own key"
        );
    }

    #[test]
    fn down_and_left_are_shifted_up_and_right() {
        let map = KeyMap::default();
        assert_eq!(target(&map, Key::ArrowDown), Some(Target::shifted("UP")));
        assert_eq!(target(&map, Key::ArrowLeft), Some(Target::shifted("RIGHT")));
        assert_eq!(map.shifted_bindings().len(), 2);
    }

    #[test]
    fn every_default_target_is_a_real_atom_key() {
        let map = KeyMap::default();
        for (host, target) in &map.bindings {
            assert!(target.is_valid(), "{host} -> {target:?}");
            assert!(
                Key::from_name(host).is_some(),
                "{host} is not an egui key name"
            );
        }
    }

    #[test]
    fn every_atom_key_can_be_bound_and_most_have_a_default() {
        let targets = all_targets();
        assert!(targets.contains(&Target::Rept) && targets.contains(&Target::key("LOCK")));
        assert_eq!(
            targets.first(),
            Some(&Target::key("A")),
            "letters come first"
        );
        let map = KeyMap::default();
        let unbound: Vec<String> = targets
            .iter()
            .filter(|t| map.host_keys_for(t).is_empty())
            .map(Target::label)
            .collect();
        // Only the three separately wired keys have no ordinary host key.
        assert_eq!(unbound, vec!["SHIFT", "CTRL", "REPT"]);
    }

    #[test]
    fn binding_a_key_replaces_what_it_did_before() {
        let mut map = KeyMap::default();
        let replaced = map.bind(Key::Z, Target::key("LOCK"));
        assert_eq!(
            replaced,
            Some(Target::key("Z")),
            "Z used to be the Atom's Z"
        );
        assert_eq!(target(&map, Key::Z), Some(Target::key("LOCK")));
        let mut lock_keys = map.host_keys_for(&Target::key("LOCK"));
        lock_keys.sort();
        assert_eq!(lock_keys, vec!["Backtick", "CapsLock", "Z"]);
        // Binding a key to what it already does replaces nothing.
        assert_eq!(map.bind(Key::A, Target::key("A")), None);
    }

    #[test]
    fn keys_can_be_bound_to_the_separately_wired_atom_keys() {
        let mut map = KeyMap::default();
        map.bind(Key::F1, Target::Rept);
        assert_eq!(map.host_keys_for(&Target::Rept), vec!["F1"]);
    }

    #[test]
    fn unbinding_removes_a_key() {
        let mut map = KeyMap::default();
        map.unbind("Backtick");
        assert_eq!(target(&map, Key::Backtick), None);
    }

    #[test]
    fn survives_a_round_trip_through_json() {
        let mut map = KeyMap::default();
        map.bind(Key::F2, Target::Ctrl);
        map.unbind("Tab");
        let text = serde_json::to_string(&map).unwrap();
        let again: KeyMap = serde_json::from_str(&text).unwrap();
        assert_eq!(again, map);
    }

    #[test]
    fn nonsense_in_a_saved_file_is_dropped() {
        let text = r#"{"bindings": {"A": {"Key": {"name": "A", "shift": false}},
                                      "NoSuchKey": {"Key": {"name": "A", "shift": false}},
                                      "B": {"Key": {"name": "NOPE", "shift": false}}}}"#;
        let map: KeyMap = serde_json::from_str::<KeyMap>(text).unwrap().sanitised();
        assert_eq!(map.bindings.len(), 1);
        assert_eq!(target(&map, Key::A), Some(Target::key("A")));
    }
}
