//! How fast does the emulator run? `cargo run --release --example bench`
use std::path::Path;
use std::time::Instant;

fn main() {
    let mut atom = atom_rs::machine::Atom::new();
    atom.load_roms(Path::new("roms")).expect("ROMs in roms/");
    atom.reset();
    let cycles = 200_000_000;
    let start = Instant::now();
    atom.run_cycles(cycles);
    let seconds = start.elapsed().as_secs_f64();
    println!(
        "{cycles} cycles in {seconds:.2}s = {:.0} MHz, {:.0}x real Atom speed",
        cycles as f64 / seconds / 1e6,
        cycles as f64 / seconds / 1e6
    );
}
