//! Acorn Atom emulator: the desktop application.
//!
//! Run `atom-rs --help` for the options.

mod archive;
mod cli;
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

/// `--archive`: get the latest archive and make it the SD card, starting its
/// menu unless the user asked for something else to be typed.
fn use_latest_archive(options: &mut cli::Options) -> Result<(), String> {
    let cache = archive::cache_folder()?;
    let card = archive::prepare(&cache, &mut |message| eprintln!("{message}"))?;
    options.sdcard = Some(card);
    options.run.get_or_insert_with(|| "*MENU\n".to_string());
    Ok(())
}

fn main() -> eframe::Result {
    let mut options = match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Parsed::Run(options)) => *options,
        Ok(cli::Parsed::Help) => {
            print!("{}", cli::USAGE);
            return Ok(());
        }
        Err(message) => {
            eprintln!("atom-rs: {message}");
            std::process::exit(2);
        }
    };
    if options.archive
        && let Err(message) = use_latest_archive(&mut options)
    {
        eprintln!("atom-rs: {message}");
        std::process::exit(1);
    }
    let rom_dir = options.rom_dir.clone().unwrap_or_else(default_rom_dir);

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Acorn Atom")
            .with_inner_size([800.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Acorn Atom",
        native_options,
        Box::new(move |_creation_context| Ok(Box::new(ui::app::AtomApp::new(rom_dir, options)))),
    )
}
