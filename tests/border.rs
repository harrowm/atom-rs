//! The border follows the video mode the program has chosen, line by line.

use atom_rs::cpu::Bus;
use atom_rs::machine::Atom;
use atom_rs::machine::video::{FULL_HEIGHT, FULL_WIDTH, PALETTE};

const PORT_A: u16 = 0xB000;
const PORT_C: u16 = 0xB002;
const TEXT: u8 = 0x00;
const GRAPHICS_RG6: u8 = 0xF0;
const GREEN: usize = 1;
const BUFF: usize = 5;
const BLACK: usize = 0;

/// The colour of a border pixel (left edge) on a row of the shown picture.
fn border_at(atom: &Atom, row: usize) -> [u8; 3] {
    let rgba = atom.bus.framebuffer.to_rgba_shown();
    assert_eq!(rgba.len(), FULL_WIDTH * FULL_HEIGHT * 4);
    let i = row * FULL_WIDTH * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

fn run(atom: &mut Atom, mode: u8, colour_set: bool) {
    atom.bus.write(PORT_A, mode);
    atom.bus.write(PORT_C, if colour_set { 0x08 } else { 0x00 });
    atom.run_frame();
    atom.run_frame();
}

#[test]
fn the_border_is_black_in_text_and_green_or_buff_in_graphics() {
    let mut atom = Atom::new();
    atom.bus.framebuffer.monochrome = false;
    run(&mut atom, TEXT, false);
    assert_eq!(border_at(&atom, 0), PALETTE[BLACK], "top, text");
    assert_eq!(border_at(&atom, 120), PALETTE[BLACK], "side, text");
    run(&mut atom, GRAPHICS_RG6, false);
    for row in [0, 24, 120, FULL_HEIGHT - 1] {
        assert_eq!(
            border_at(&atom, row),
            PALETTE[GREEN],
            "row {row}, graphics set 0"
        );
    }
    run(&mut atom, GRAPHICS_RG6, true);
    assert_eq!(border_at(&atom, 120), PALETTE[BUFF], "graphics set 1");
}

/// Change the mode in the middle of a frame (as split-screen programs do): the
/// border changes at the same scanline.
#[test]
fn the_border_changes_where_the_mode_changes() {
    let mut atom = Atom::new();
    atom.bus.framebuffer.monochrome = false;
    run(&mut atom, TEXT, false);
    // Run to the middle of the next frame's picture and switch to graphics.
    while atom.bus.vdg.line() != 100 {
        atom.run_cycles(1);
    }
    atom.bus.write(PORT_A, GRAPHICS_RG6);
    while atom.bus.vdg.line() != 150 {
        atom.run_cycles(1);
    }
    // Picture line L is row 25 + L: text above line 100, graphics below.
    assert_eq!(border_at(&atom, 25 + 50), PALETTE[BLACK]);
    assert_eq!(border_at(&atom, 25 + 120), PALETTE[GREEN]);
}
