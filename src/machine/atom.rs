//! The whole computer: a CPU wired to the Atom's memory and peripherals.
//!
//! [`AtomBus`] is the heart of the cycle accuracy. It implements the CPU's
//! [`Bus`] trait, and on *every* bus access it first advances all the other
//! chips by one clock cycle, then performs the access. The CPU therefore
//! drives the clock and everything stays in lock-step.

use std::path::{Path, PathBuf};

use super::atm::AtmFile;
use super::atommc::AtoMmc;
use super::debug::{Debugger, StopReason};
use super::fdc8271::Fdc8271;
use super::memory::{Memory, ROM_SIZE, RamLayout, Region, RomSlot, open_bus};
use super::ppi8255::Ppi;
use super::sound::Sampler;
use super::tape::Tape;
use super::vdg6847::{Standard, VISIBLE_LINES, Vdg, VideoMode};
use super::via6522::Via;
use super::video::Framebuffer;
use crate::cpu::{Bus, Cpu};

/// The Atom's CPU clock.
pub const CLOCK_HZ: f64 = 1_000_000.0;

/// The Ramoth RAM/ROM image holds 128K of ROMs. These are the parts of it that
/// make an AtoMMC machine: where in the file each 4K ROM starts.
const RAMROM_IMAGE_SIZE: usize = 0x20000;
const RAMROM_SLICES: [(RomSlot, usize); 5] = [
    (RomSlot::Utility, 0x00000),       // #A000 bank 0
    (RomSlot::Basic, 0x10000),         // Atom BASIC (disk ROM enabled)
    (RomSlot::FloatingPoint, 0x11000), // Atom floating point
    (RomSlot::Disk, 0x12000),          // the AtoMMC ROM, instead of Atom DOS
    (RomSlot::Kernel, 0x13000),        // Atom kernel
];

/// Everything the CPU can reach over its bus.
pub struct AtomBus {
    pub memory: Memory,
    pub ppi: Ppi,
    pub via: Via,
    pub vdg: Vdg,
    /// The picture, drawn one scanline at a time as the video beam reaches it.
    pub framebuffer: Framebuffer,
    /// Total CPU cycles since power-on.
    pub cycles: u64,
    /// Speaker output, turned into audio samples.
    pub sound: Sampler,
    /// The cassette player.
    pub tape: Tape,
    /// The disk controller (reachable once a disk ROM is loaded).
    pub fdc: Fdc8271,
    /// The AtoMMC's joystick port (the SD card itself is not emulated).
    pub atommc: AtoMmc,
    pub(super) fdc_nmi_was_asserted: bool,
    /// Breakpoints and watchpoints.
    pub debug: Debugger,
    /// Hold the IRQ line low from outside (the debugger's IRQ button).
    pub external_irq: bool,
    /// A falling edge on NMI is waiting to be seen by the CPU.
    pub nmi_edge: bool,
}

impl AtomBus {
    fn new(standard: Standard) -> Self {
        AtomBus {
            memory: Memory::new(RamLayout::expanded()),
            ppi: Ppi::default(),
            via: Via::new(),
            vdg: Vdg::new(standard),
            framebuffer: Framebuffer::new(),
            cycles: 0,
            sound: Sampler::new(CLOCK_HZ, 44_100.0),
            tape: Tape::new(),
            fdc: Fdc8271::new(),
            atommc: AtoMmc::default(),
            fdc_nmi_was_asserted: false,
            debug: Debugger::default(),
            external_irq: false,
            nmi_edge: false,
        }
    }

    /// One clock cycle passes for every chip.
    fn tick(&mut self) {
        self.cycles += 1;
        self.via.tick();
        if self.ppi.tick() {
            // Every 208 cycles the cassette signal can change.
            self.ppi.tape_input = self.tape.step();
            self.tape.record(self.ppi.tape_output());
        }
        self.sound.tick(self.ppi.speaker());
        self.fdc.tick();
        // The controller's NMI output is level-sensitive, the CPU's input is
        // edge-sensitive: remember each time it goes active.
        let fdc_nmi = self.fdc.nmi_asserted();
        if fdc_nmi && !self.fdc_nmi_was_asserted {
            self.nmi_edge = true;
        }
        self.fdc_nmi_was_asserted = fdc_nmi;
        self.vdg.tick();
        self.ppi.field_sync_low = self.vdg.field_sync_low();
        if self.vdg.at_line_start() && self.vdg.line() < VISIBLE_LINES {
            self.draw_scanline();
        }
    }

    /// Draw the current scanline using the video mode as it is right now.
    fn draw_scanline(&mut self) {
        let mode = VideoMode::from_port_a_high_nibble(self.ppi.video_mode_bits());
        let line = self.vdg.line() as usize;
        self.framebuffer
            .render_line(line, &self.memory.ram[..], mode, self.ppi.colour_set());
    }

    /// Write to RAM without a clock tick, for the debugger. Returns false
    /// (and does nothing) if the address is not RAM.
    pub fn poke(&mut self, addr: u16, value: u8) -> bool {
        let is_ram = matches!(self.memory.region(addr), Region::Ram | Region::VideoRam);
        if is_ram {
            self.memory.ram[addr as usize] = value;
        }
        is_ram
    }

    /// The RESET line, as far as the chips are concerned: the 8255, the 6522
    /// and the disk controller all go back to their power-on state. (Without
    /// this a program's sound or timer settings would survive a reset.)
    fn reset_peripherals(&mut self) {
        self.ppi.reset();
        self.via = Via::new();
        self.fdc.reset();
        self.atommc.reset();
        self.nmi_edge = false;
        self.fdc_nmi_was_asserted = false;
    }

    /// Read without side effects or clock ticks, for the debugger.
    pub fn peek(&self, addr: u16) -> u8 {
        match self.memory.region(addr) {
            Region::Ram | Region::VideoRam => self.memory.ram[addr as usize],
            Region::Rom(slot) => self.memory.read_rom(slot, addr),
            Region::Ppi | Region::Via | Region::Fdc | Region::AtoMmc | Region::Unmapped => {
                open_bus(addr)
            }
        }
    }
}

impl Bus for AtomBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.tick();
        if !self.debug.watchpoints.is_empty() {
            self.debug.note_access(addr, false);
        }
        match self.memory.region(addr) {
            Region::Ram | Region::VideoRam => self.memory.ram[addr as usize],
            Region::Rom(slot) => self.memory.read_rom(slot, addr),
            Region::Ppi => self.ppi.read(addr),
            Region::Via => self.via.read(addr),
            Region::Fdc => self.fdc.read(addr),
            Region::AtoMmc => self.atommc.read(addr),
            Region::Unmapped => {
                if self.debug.log_unmapped {
                    self.debug.note_unmapped(addr, false);
                }
                open_bus(addr)
            }
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.tick();
        if !self.debug.watchpoints.is_empty() {
            self.debug.note_access(addr, true);
        }
        match self.memory.region(addr) {
            Region::Ram | Region::VideoRam => self.memory.ram[addr as usize] = value,
            Region::Ppi => self.ppi.write(addr, value),
            Region::Via => self.via.write(addr, value),
            Region::Fdc => self.fdc.write(addr, value),
            Region::AtoMmc => self.atommc.write(addr, value),
            Region::Rom(_) => {} // writes to ROM go nowhere
            Region::Unmapped => {
                if self.debug.log_unmapped {
                    self.debug.note_unmapped(addr, true);
                }
            }
        }
    }

    fn irq_asserted(&mut self) -> bool {
        self.via.irq() || self.external_irq
    }

    fn take_nmi_edge(&mut self) -> bool {
        std::mem::take(&mut self.nmi_edge)
    }
}

/// An Acorn Atom.
pub struct Atom {
    pub cpu: Cpu,
    pub bus: AtomBus,
}

impl Atom {
    pub fn new() -> Self {
        Atom {
            cpu: Cpu::new(),
            bus: AtomBus::new(Standard::Ntsc),
        }
    }

    /// Load the ROM images from a directory using Atomulator's file names.
    /// The kernel, BASIC and floating point ROMs are required; the utility and
    /// disk ROMs are optional.
    pub fn load_roms(&mut self, dir: &Path) -> Result<(), String> {
        let files = [
            (RomSlot::Kernel, "akernel.rom", true),
            (RomSlot::Basic, "abasic.rom", true),
            (RomSlot::FloatingPoint, "afloat.rom", true),
            (RomSlot::Utility, "axr1.rom", false),
            (RomSlot::Disk, "dosrom.rom", false),
        ];
        for (slot, name, required) in files {
            let path = dir.join(name);
            match std::fs::read(&path) {
                Ok(data) => {
                    self.bus.memory.load_rom(slot, &data)?;
                    if slot == RomSlot::Disk {
                        self.bus.memory.disk_interface_fitted = true;
                    }
                }
                Err(e) if required => return Err(format!("{}: {e}", path.display())),
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// Go back to the standard Atom: the ROMs in `rom_dir` (Atom DOS and the
    /// 8271 floppy controller) and no SD card. Resets the machine.
    pub fn use_floppy_disks(&mut self, rom_dir: &Path) -> Result<(), String> {
        self.load_roms(rom_dir)?;
        self.bus.memory.set_layout(RamLayout::expanded());
        self.bus.atommc.remove_card();
        self.reset();
        Ok(())
    }

    /// Turn this into an Atom with an AtoMMC SD card interface, using the
    /// folder `card_folder` as the card.
    ///
    /// The AtoMMC ROM replaces the Atom DOS ROM, and the machine uses the
    /// matching kernel and BASIC. All of these come from the 128K
    /// `ramrom.rom` image (the Ramoth RAM/ROM board's ROMs, as shipped with
    /// Atomulator), which must be in `rom_dir`. The disk controller's page at
    /// `0A00` becomes RAM. Resets the machine.
    pub fn use_sd_card(&mut self, rom_dir: &Path, card_folder: PathBuf) -> Result<(), String> {
        let path = rom_dir.join("ramrom.rom");
        let image = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if image.len() != RAMROM_IMAGE_SIZE {
            return Err(format!(
                "{} should be {RAMROM_IMAGE_SIZE} bytes",
                path.display()
            ));
        }
        let memory = &mut self.bus.memory;
        for (slot, offset) in RAMROM_SLICES {
            memory.load_rom(slot, &image[offset..offset + ROM_SIZE])?;
        }
        memory.disk_interface_fitted = false;
        memory.set_layout(RamLayout::full());

        let mut atommc = AtoMmc::with_card(card_folder);
        if let Ok(eeprom) = std::fs::read(rom_dir.join("mmc_eeprom.rom")) {
            atommc.load_eeprom(&eeprom);
        }
        atommc.joystick = self.bus.atommc.joystick;
        self.bus.atommc = atommc;
        self.reset();
        Ok(())
    }

    /// Copy an `.atm` program into RAM. It is not started; the caller can jump
    /// to `file.exec_address` (or you can type `LINK #xxxx` on the Atom).
    pub fn load_atm(&mut self, file: &AtmFile) -> Result<(), String> {
        for (offset, &byte) in file.data.iter().enumerate() {
            let addr = file.load_address.wrapping_add(offset as u16);
            if !self.bus.poke(addr, byte) {
                return Err(format!("no RAM at ${addr:04X}; cannot load {}", file.name));
            }
        }
        Ok(())
    }

    /// Press the reset button: the CPU loads PC from the reset vector.
    pub fn reset(&mut self) {
        self.bus.reset_peripherals();
        self.cpu.reset(&mut self.bus);
    }

    /// Execute one instruction; returns the cycles it took.
    pub fn step(&mut self) -> u64 {
        self.cpu.step(&mut self.bus)
    }

    /// Run until at least `cycles` clock cycles have elapsed, or until a
    /// breakpoint or watchpoint stops us (the reason is returned).
    pub fn run_cycles(&mut self, cycles: u64) -> Option<StopReason> {
        let end = self.bus.cycles + cycles;
        while self.bus.cycles < end {
            if let Some(reason) = self.bus.debug.check_before_instruction(self.cpu.regs.pc) {
                return Some(reason);
            }
            self.step();
            if let Some(reason) = self.bus.debug.stop.take() {
                return Some(reason);
            }
        }
        None
    }

    /// For "step over": if the next instruction is a `JSR`, the address just
    /// after it (where the subroutine will return to).
    pub fn step_over_target(&self) -> Option<u16> {
        const JSR: u8 = 0x20;
        let pc = self.cpu.regs.pc;
        (self.bus.peek(pc) == JSR).then_some(pc.wrapping_add(3))
    }

    /// Run until the video hardware finishes the current frame.
    pub fn run_frame(&mut self) {
        let frame = self.bus.vdg.frames;
        while self.bus.vdg.frames == frame {
            self.step();
        }
    }

    /// The video mode currently selected through the 8255.
    pub fn video_mode(&self) -> VideoMode {
        VideoMode::from_port_a_high_nibble(self.bus.ppi.video_mode_bits())
    }

    /// Is the Atom waiting for a command? True when BASIC/the OS is showing its
    /// `>` prompt on the last used line of the text screen, or when a program
    /// has taken over the display with a graphics mode (so keys go to it).
    /// False while a command is still running (loading from disk, say), when
    /// the Atom is not reading the keyboard and would miss key presses.
    pub fn ready_for_input(&self) -> bool {
        if self.video_mode() != VideoMode::Text {
            return true;
        }
        let screen = self.screen_text();
        let Some(last_line) = screen.iter().rev().find(|row| !row.trim().is_empty()) else {
            return false;
        };
        // The prompt is `>`, perhaps followed by the cursor (drawn as `#`).
        last_line
            .trim_end()
            .strip_prefix('>')
            .is_some_and(|rest| rest.chars().all(|c| c == ' ' || c == '#'))
    }

    /// The text screen (`8000-81FF`, 32 x 16) as ASCII, one `String` per row.
    /// Handy for tests and for the debugger. Semigraphics become `#`.
    pub fn screen_text(&self) -> Vec<String> {
        const COLUMNS: usize = 32;
        const ROWS: usize = 16;
        (0..ROWS)
            .map(|row| {
                (0..COLUMNS)
                    .map(|col| {
                        screen_code_to_char(self.bus.memory.ram[0x8000 + row * COLUMNS + col])
                    })
                    .collect()
            })
            .collect()
    }
}

impl Default for Atom {
    fn default() -> Self {
        Self::new()
    }
}

/// Video RAM holds 6847 codes: bits 0-5 pick the character, bit 6 selects
/// semigraphics, bit 7 inverts the colours.
fn screen_code_to_char(code: u8) -> char {
    if code & 0x40 != 0 {
        return '#';
    }
    let low = code & 0x3F;
    // Codes 0-31 are the letters and symbols that ASCII keeps at 64-95.
    let ascii = if low < 0x20 { low + 0x40 } else { low };
    ascii as char
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_codes_decode_to_ascii() {
        assert_eq!(screen_code_to_char(0x01), 'A');
        assert_eq!(screen_code_to_char(0x20), ' ');
        assert_eq!(screen_code_to_char(0x30), '0');
        assert_eq!(screen_code_to_char(0x81), 'A'); // inverse video
    }

    #[test]
    fn every_access_advances_the_clock_one_cycle() {
        let mut atom = Atom::new();
        atom.bus.write(0x0000, 1);
        atom.bus.read(0x0000);
        assert_eq!(atom.bus.cycles, 2);
        assert_eq!(atom.bus.vdg.line(), 0);
    }

    #[test]
    fn reset_returns_the_chips_to_their_power_on_state() {
        let mut atom = Atom::new();
        atom.bus.write(0xB002, 0x0F); // speaker and colour set on
        atom.bus.write(0xB000, 0xF5); // graphics mode bits and a keyboard row
        atom.bus.write(0xB80E, 0xC0); // enable the VIA's timer 1 interrupt
        atom.bus.write(0xB80B, 0x40); // free-running timer
        atom.bus.write(0xB804, 10);
        atom.bus.write(0xB805, 0);
        atom.reset();
        assert!(
            !atom.bus.ppi.speaker(),
            "the speaker must be off after a reset"
        );
        assert_eq!(atom.bus.ppi.port_a(), 0);
        assert!(!atom.bus.ppi.colour_set());
        let via = atom.bus.via.snapshot();
        assert_eq!((via.enables, via.flags, via.aux_control), (0, 0, 0));
        assert!(!atom.bus.via.irq());
    }

    #[test]
    fn atm_files_load_into_ram_at_their_load_address() {
        let mut atom = Atom::new();
        let file = AtmFile {
            name: "T".into(),
            load_address: 0x2900,
            exec_address: 0x2900,
            data: vec![1, 2, 3],
        };
        atom.load_atm(&file).unwrap();
        assert_eq!(&atom.bus.memory.ram[0x2900..0x2903], &[1, 2, 3]);

        let rom_file = AtmFile {
            load_address: 0xC000,
            ..file
        };
        assert!(atom.load_atm(&rom_file).is_err());
    }

    #[test]
    fn tape_signal_reaches_port_c_bit_5() {
        use crate::machine::tape::{TapeChunk, TapeImage};
        let mut atom = Atom::new();
        atom.bus.tape.insert(TapeImage {
            chunks: vec![TapeChunk::Carrier { cycles: 50 }],
        });
        atom.bus.tape.playing = true;
        // Sample port C once per tone step: a carrier alternates.
        let mut levels = Vec::new();
        for _ in 0..4 {
            for _ in 0..208 {
                atom.bus.write(0x0000, 0); // any bus access advances the clock
            }
            levels.push(atom.bus.ppi.read(2) & 0x20 != 0);
        }
        assert_eq!(levels, vec![true, false, true, false]);
    }

    #[test]
    fn run_stops_at_a_breakpoint_and_can_resume_from_it() {
        let mut atom = Atom::new();
        atom.bus.memory.ram[0x0100..0x0104].fill(0xEA); // NOPs
        atom.cpu.regs.pc = 0x0100;
        atom.bus.debug.toggle_breakpoint(0x0102);

        let stop = atom.run_cycles(100);
        assert_eq!(stop, Some(StopReason::Breakpoint(0x0102)));
        assert_eq!(atom.cpu.regs.pc, 0x0102);

        atom.bus.debug.skip_breakpoint_once = true;
        assert_eq!(atom.run_cycles(2), None, "runs the NOP at the breakpoint");
        assert_eq!(atom.cpu.regs.pc, 0x0103);
    }

    #[test]
    fn watchpoint_stops_after_the_writing_instruction() {
        let mut atom = Atom::new();
        // STA $0010 ; NOP
        atom.bus.memory.ram[0x0100..0x0104].copy_from_slice(&[0x85, 0x10, 0xEA, 0xEA]);
        atom.cpu.regs.pc = 0x0100;
        atom.bus
            .debug
            .watchpoints
            .push(crate::machine::debug::Watchpoint {
                addr: 0x0010,
                on_read: false,
                on_write: true,
            });
        let stop = atom.run_cycles(100);
        assert_eq!(
            stop,
            Some(StopReason::Watchpoint {
                addr: 0x0010,
                write: true
            })
        );
        assert_eq!(atom.cpu.regs.pc, 0x0102);
    }

    #[test]
    fn nmi_button_makes_the_cpu_enter_the_nmi_handler() {
        let mut atom = Atom::new();
        atom.cpu.regs.pc = 0x0100;
        let sp = atom.cpu.regs.sp;
        atom.bus.nmi_edge = true;
        assert_eq!(atom.step(), 7);
        assert_eq!(atom.cpu.regs.sp, sp.wrapping_sub(3));
        assert!(!atom.bus.nmi_edge, "the edge is consumed");
    }

    #[test]
    fn step_over_targets_the_instruction_after_a_jsr() {
        let mut atom = Atom::new();
        atom.bus.memory.ram[0x0100..0x0103].copy_from_slice(&[0x20, 0x00, 0xC0]);
        atom.cpu.regs.pc = 0x0100;
        assert_eq!(atom.step_over_target(), Some(0x0103));
        atom.bus.memory.ram[0x0100] = 0xEA;
        assert_eq!(atom.step_over_target(), None);
    }

    #[test]
    fn cpu_runs_code_from_ram() {
        let mut atom = Atom::new();
        // LDA #$42 ; STA $0010, placed in the 1K of RAM at the bottom of memory.
        atom.bus.memory.ram[0x0100..0x0105].copy_from_slice(&[0xA9, 0x42, 0x8D, 0x10, 0x00]);
        atom.cpu.regs.pc = 0x0100;
        assert_eq!(atom.step(), 2);
        assert_eq!(atom.step(), 4);
        assert_eq!(atom.bus.memory.ram[0x0010], 0x42);
    }
}
