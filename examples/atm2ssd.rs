//! Pack Atom program files (`.atm` format: a 22-byte header then the data) into
//! a disk image that the emulated Atom DOS can read.
//!
//! ```text
//! cargo run --release --example atm2ssd -- chuckie.ssd path/to/FILE1 path/to/FILE2 ...
//! ```
//!
//! The Atom Software Archive stores each program as a file in this format, so
//! this turns a game's folder into a floppy disk. File names come from the
//! header inside each file (made upper case; a disk catalogue allows only 7
//! characters, so longer names are cut short, with a note).

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

/// BASIC's two "start this program" addresses: `CE86` runs the program that was
/// just loaded; `C2B2` goes to the BASIC prompt instead.
const BASIC_RUN_ADDRESS: u16 = 0xCE86;
const BASIC_PROMPT_ADDRESS: u16 = 0xC2B2;
/// Where BASIC programs are loaded.
const BASIC_PROGRAM_ADDRESS: u16 = 0x2900;

/// How to start a program, judging by its header, as text for `--run`.
fn how_to_start(name: &str, file: &AtmFile) -> String {
    let basic_program = file.exec_address == BASIC_PROMPT_ADDRESS
        || (file.load_address == BASIC_PROGRAM_ADDRESS && file.exec_address == file.load_address);
    if file.exec_address == BASIC_RUN_ADDRESS {
        format!("'*RUN {name}\\n'          (a BASIC program that starts itself)")
    } else if basic_program {
        format!("'LOAD \"{name}\"\\nRUN\\n'   (a BASIC program: load it, then RUN it)")
    } else {
        format!("'*RUN {name}\\n'          (a machine code program)")
    }
}

fn build(output: &str, inputs: &[String]) -> Result<(), String> {
    let mut disk = DiskImage::blank_80_track(1);
    for path in inputs {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let file = AtmFile::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
        let name: String = file.name.chars().take(7).collect();
        if name != file.name {
            println!(
                "note: name {:?} cut to {name:?} to fit the catalogue",
                file.name
            );
        }
        disk.add_file(
            &name,
            file.load_address as u32,
            file.exec_address as u32,
            &file.data,
        )
        .map_err(|e| format!("{path}: {e}"))?;
        println!(
            "{:<8} load {:04X} exec {:04X} length {:04X}",
            name.to_ascii_uppercase(),
            file.load_address,
            file.exec_address,
            file.data.len()
        );
    }
    std::fs::write(output, disk.to_bytes()).map_err(|e| format!("{output}: {e}"))?;
    println!("wrote {output} ({} files)", disk.catalogue().len());
    // A single program: say how to start it.
    if let [path] = inputs {
        let file = AtmFile::parse(&std::fs::read(path).map_err(|e| e.to_string())?)?;
        let name: String = file
            .name
            .chars()
            .take(7)
            .collect::<String>()
            .to_ascii_uppercase();
        println!("start it with --run {}", how_to_start(&name, &file));
    }
    Ok(())
}
