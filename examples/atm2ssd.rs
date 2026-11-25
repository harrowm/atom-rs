//! Pack Atom program files (`.atm` format: a 22-byte header then the data) into
//! a disk image that the emulated Atom DOS can read.
//!
//! ```text
//! cargo run --release --example atm2ssd -- chuckie.ssd path/to/FILE1 path/to/FILE2 ...
//! ```
//!
//! The Atom Software Archive stores each program as a file in this format, so
//! this turns a game's folder into a floppy disk. File names come from the
//! header inside each file (made upper case, at most 7 characters).

use std::process::ExitCode;

use atom_rs::machine::atm::AtmFile;
use atom_rs::machine::disk::DiskImage;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((output, inputs)) = args.split_first() else {
        eprintln!("usage: atm2ssd OUTPUT.ssd FILE...");
        return ExitCode::from(2);
    };
    match build(output, inputs) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("atm2ssd: {message}");
            ExitCode::FAILURE
        }
    }
}

fn build(output: &str, inputs: &[String]) -> Result<(), String> {
    let mut disk = DiskImage::blank_80_track(1);
    for path in inputs {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let file = AtmFile::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
        disk.add_file(
            &file.name,
            file.load_address as u32,
            file.exec_address as u32,
            &file.data,
        )
        .map_err(|e| format!("{path}: {e}"))?;
        println!(
            "{:<8} load {:04X} exec {:04X} length {:04X}",
            file.name.to_ascii_uppercase(),
            file.load_address,
            file.exec_address,
            file.data.len()
        );
    }
    std::fs::write(output, disk.to_bytes()).map_err(|e| format!("{output}: {e}"))?;
    println!("wrote {output} ({} files)", disk.catalogue().len());
    Ok(())
}
