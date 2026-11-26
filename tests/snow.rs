//! Video "snow": the CPU and the 6847 share video RAM, and when the CPU gets in
//! while the 6847 is fetching, the picture shows the CPU's byte.

use atom_rs::machine::Atom;

/// An Atom with no ROMs, running a tight loop that writes the same byte to
/// video RAM forever, with the text screen filled with spaces.
fn atom_writing_to_video_ram(snow: bool) -> Atom {
    let mut atom = Atom::new();
    for addr in 0x8000..0x8200u16 {
        atom.bus.poke(addr, 0x20); // inverse-less space
    }
    // loop: LDA #$41 ; STA $8100 ; JMP loop        (writes an "A")
    let program = [0xA9, 0x41, 0x8D, 0x00, 0x81, 0x4C, 0x00, 0x29];
    for (i, byte) in program.iter().enumerate() {
        atom.bus.poke(0x2900 + i as u16, *byte);
    }
    atom.cpu.regs.pc = 0x2900;
    atom.bus.snow = snow;
    atom
}

fn picture(atom: &mut Atom) -> Vec<u8> {
    // Two frames: the first may be partly drawn.
    atom.run_cycles(2 * 64 * 262);
    atom.bus.framebuffer.pixels.to_vec()
}

#[test]
fn with_snow_off_the_picture_ignores_what_the_cpu_does() {
    let mut quiet = atom_writing_to_video_ram(false);
    let off = picture(&mut quiet);
    // The CPU writes an "A" at 8100 (text row 8), so there is an A there...
    // but nothing but that one place differs from a blank screen.
    let mut blank = Atom::new();
    for addr in 0x8000..0x8200u16 {
        blank.bus.poke(addr, 0x20);
    }
    blank.bus.poke(0x8100, 0x41);
    assert_eq!(off, picture(&mut blank));
}

#[test]
fn with_snow_on_the_cpus_bytes_leak_into_the_picture() {
    let mut quiet = atom_writing_to_video_ram(false);
    let mut snowy = atom_writing_to_video_ram(true);
    let off = picture(&mut quiet);
    let on = picture(&mut snowy);
    let differing = off.iter().zip(&on).filter(|(a, b)| a != b).count();
    assert!(differing > 0, "no snow appeared");
    // Snow is small: some characters, not the whole screen.
    assert!(differing < off.len() / 2, "{differing} pixels differ");
}

#[test]
fn only_video_ram_accesses_cause_snow() {
    // The same loop, but storing to ordinary RAM (2800) instead.
    let mut quiet = atom_writing_to_video_ram(false);
    let mut snowy = atom_writing_to_video_ram(true);
    for atom in [&mut quiet, &mut snowy] {
        atom.bus.poke(0x2903, 0x00);
        atom.bus.poke(0x2904, 0x28);
    }
    assert_eq!(picture(&mut quiet), picture(&mut snowy));
}
