//! Acorn Atom emulator: the desktop application.
//!
//! Usage: `atom-rs [--debug] [ROM_FOLDER]`

mod ui;

use std::path::PathBuf;

/// Where to look for the ROMs when no folder is given on the command line:
/// `./roms` first, then next to the executable, then the project folder the
/// program was built from. So it works wherever you start it from.
fn default_rom_dir() -> PathBuf {
    let mut candidates = vec![PathBuf::from("roms")];
    if let Ok(exe) = std::env::current_exe() {
        // target/release/atom-rs -> the project folder is three levels up.
        candidates.extend(exe.ancestors().skip(1).take(3).map(|dir| dir.join("roms")));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("roms"));

    candidates
        .iter()
        .find(|dir| dir.join("akernel.rom").exists())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("roms"))
}

fn main() -> eframe::Result {
    let mut open_debug_windows = false;
    let mut rom_dir = None;
    for arg in std::env::args_os().skip(1) {
        if arg == "--debug" {
            open_debug_windows = true;
        } else {
            rom_dir = Some(PathBuf::from(arg));
        }
    }
    let rom_dir = rom_dir.unwrap_or_else(default_rom_dir);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Acorn Atom")
            .with_inner_size([800.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Acorn Atom",
        options,
        Box::new(move |_creation_context| {
            Ok(Box::new(ui::app::AtomApp::new(rom_dir, open_debug_windows)))
        }),
    )
}
