//! The settings that are remembered between runs.
//!
//! They live in `~/.config/atom-rs/settings.json` (next to the key map), as
//! plain JSON you can read or edit. A setting missing from the file takes its
//! default, so a file from an older version still loads. Command-line options
//! win over what is saved.

use std::ffi::OsString;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use atom_rs::machine::atom::Machine;
use atom_rs::machine::memory::{MAX_VIDEO_RAM_KB, RamPreset, SidAddress};
use atom_rs::machine::sid::SidModel;

/// The user's home folder: `HOME` (macOS and Linux), or `USERPROFILE`, which
/// is what Windows sets instead.
pub fn home_folder() -> Option<PathBuf> {
    home_from(std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
}

fn home_from(home: Option<OsString>, user_profile: Option<OsString>) -> Option<PathBuf> {
    home.or(user_profile)
        .filter(|folder| !folder.is_empty())
        .map(PathBuf::from)
}

/// `~/.config/atom-rs`
pub fn config_folder() -> Option<PathBuf> {
    Some(home_folder()?.join(".config").join("atom-rs"))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub machine: Machine,
    /// The folder used as the SD card (needed to come back to that machine).
    pub card_folder: Option<PathBuf>,
    pub ram_preset: RamPreset,
    pub video_ram_kb: u8,
    pub colour_board: bool,
    pub show_border: bool,
    pub snow: bool,
    pub sid_fitted: bool,
    pub sid_address: SidAddress,
    pub sid_model: SidModel,
    pub volume: f32,
    pub muted: bool,
    pub integer_scaling: bool,
    pub fast_tape: bool,
    pub joystick_keys: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            machine: Machine::FloppyAtom,
            card_folder: None,
            ram_preset: RamPreset::With22KExpansion,
            video_ram_kb: MAX_VIDEO_RAM_KB,
            colour_board: true,
            show_border: true,
            snow: false,
            sid_fitted: true,
            sid_address: SidAddress::default(),
            sid_model: SidModel::default(),
            volume: 0.2,
            muted: false,
            integer_scaling: true,
            fast_tape: false,
            joystick_keys: false,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        Some(config_folder()?.join("settings.json"))
    }

    /// Load the saved settings, or the defaults if there are none. The error
    /// describes a file that exists but could not be used.
    pub fn load() -> (Settings, Option<String>) {
        let Some(path) = Self::path() else {
            return (Settings::default(), None);
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(settings) => (settings, None),
                Err(e) => (
                    Settings::default(),
                    Some(format!(
                        "{} is not valid settings ({e}); using the defaults",
                        path.display()
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
            Err(e) => (
                Settings::default(),
                Some(format!("cannot read {}: {e}", path.display())),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no home folder to save the settings in")?;
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

    #[test]
    fn the_home_folder_is_home_or_else_the_windows_user_profile() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(home_from(os("/Users/m"), None), Some("/Users/m".into()));
        assert_eq!(
            home_from(None, os(r"C:\Users\Malcolm")),
            Some(r"C:\Users\Malcolm".into())
        );
        assert_eq!(
            home_from(os("/home/m"), os("ignored")),
            Some("/home/m".into())
        );
        assert_eq!(home_from(os(""), None), None, "an empty HOME is no home");
        assert_eq!(home_from(None, None), None);
    }

    #[test]
    fn settings_survive_a_round_trip_through_json() {
        let settings = Settings {
            machine: Machine::SdCard,
            card_folder: Some(PathBuf::from("/games/card")),
            ram_preset: RamPreset::Minimal,
            video_ram_kb: 3,
            colour_board: false,
            show_border: false,
            snow: true,
            sid_fitted: true,
            sid_address: SidAddress::NineFc0,
            sid_model: SidModel::Mos8580,
            volume: 0.7,
            muted: true,
            integer_scaling: false,
            fast_tape: true,
            joystick_keys: true,
        };
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), settings);
    }

    #[test]
    fn a_file_with_missing_or_unknown_settings_still_loads() {
        // From an older version (no snow), or edited by hand (an extra key).
        let loaded: Settings =
            serde_json::from_str(r#"{ "volume": 0.5, "from_the_future": 1 }"#).unwrap();
        assert_eq!(
            loaded,
            Settings {
                volume: 0.5,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn nonsense_is_refused() {
        assert!(serde_json::from_str::<Settings>(r#"{ "machine": "Cray" }"#).is_err());
    }

    #[test]
    fn the_defaults_match_a_freshly_built_machine() {
        use atom_rs::machine::Atom;
        let atom = Atom::new();
        let defaults = Settings::default();
        assert_eq!(atom.bus.memory.ram_preset(), defaults.ram_preset);
        assert_eq!(atom.bus.memory.video_ram_kb(), defaults.video_ram_kb);
        assert_eq!(atom.bus.memory.sid_fitted, defaults.sid_fitted);
        assert_eq!(atom.bus.memory.sid_address, defaults.sid_address);
        assert_eq!(atom.bus.sid.model(), defaults.sid_model);
        assert_eq!(atom.bus.snow, defaults.snow);
        assert_eq!(atom.bus.framebuffer.show_border, defaults.show_border);
    }
}
