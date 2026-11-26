//! The whole computer: a CPU wired to the Atom's memory and peripherals.
//!
//! [`AtomBus`] is the heart of the cycle accuracy. It implements the CPU's
//! [`Bus`] trait, and on *every* bus access it first advances all the other
//! chips by one clock cycle, then performs the access. The CPU therefore
//! drives the clock and everything stays in lock-step.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::atm::AtmFile;
use super::atommc::AtoMmc;
use super::debug::{Debugger, StopReason};
use super::fdc1770::Fdc1770;
use super::fdc8271::Fdc8271;
use super::joystick::Joystick;
use super::memory::{Memory, RamPreset, Region, RomSlot, open_bus};
use super::ppi8255::Ppi;
use super::ramrom::{self, RamRomBoard};
use super::sid::SidCard;
use super::sound::Sampler;
use super::tape::Tape;
use super::vdg6847::{FETCH_SLOTS, Standard, VISIBLE_LINES, Vdg, VideoMode, fetch_slot};
use super::via6522::Via;
use super::video::{Framebuffer, fetch_address};
use crate::cpu::{Bus, Cpu};

/// The Atom's CPU clock.
pub const CLOCK_HZ: f64 = 1_000_000.0;

/// What kind of Atom this is: the four ways the emulator can be set up. The
/// Hardware menu chooses between them, and the settings that apply (RAM size,
/// the card folder, the GDOS ROM bank) depend on which.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Machine {
    /// The standard disk Atom: an 8271 floppy controller and Atom DOS.
    FloppyAtom,
    /// An Atom with the GDOS/ADOS 2015 disk system: a WD1770 controller.
    GdosAtom,
    /// An Atom with an AtoMMC SD card and the Ramoth RAM/ROM board.
    SdCard,
    /// An Atom running BBC BASIC, through the RAM/ROM board's BBC mode.
    BbcBasic,
}

impl Machine {
    pub const ALL: [Machine; 4] = [
        Machine::FloppyAtom,
        Machine::GdosAtom,
        Machine::SdCard,
        Machine::BbcBasic,
    ];

    /// How it is named in the menu.
    pub fn label(self) -> &'static str {
        match self {
            Machine::FloppyAtom => "Floppy disk Atom",
            Machine::GdosAtom => "Floppy disk Atom with GDOS/ADOS",
            Machine::SdCard => "SD card (AtoMMC)",
            Machine::BbcBasic => "BBC BASIC",
        }
    }

    /// One line saying what it is, for the menu.
    pub fn description(self) -> &'static str {
        match self {
            Machine::FloppyAtom => {
                "Intel 8271 disk controller and Atom DOS. Disk images load into its drives."
            }
            Machine::GdosAtom => {
                "WD1770 disk controller and the GDOS/ADOS ROM (type *DOS to start it)."
            }
            Machine::SdCard => {
                "A folder is the SD card. Includes the RAM/ROM board (utility ROMs, SDDOS images)."
            }
            Machine::BbcBasic => {
                "BBC BASIC 2 and a small BBC operating system, from the RAM/ROM board."
            }
        }
    }

    /// Can the amount of main RAM be chosen? (The SD card and BBC machines
    /// have the RAM/ROM board, which provides RAM everywhere.)
    pub fn has_choosable_ram(self) -> bool {
        matches!(self, Machine::FloppyAtom | Machine::GdosAtom)
    }
}

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
    /// The SID sound card, mixed in with the speaker.
    pub sid: SidCard,
    /// The cassette player.
    pub tape: Tape,
    /// The disk controller (reachable once a disk ROM is loaded).
    pub fdc: Fdc8271,
    /// The GDOS disk system's controller (reachable once GDOS is fitted).
    pub fdc1770: Fdc1770,
    /// The AtoMMC's joystick port (the SD card itself is not emulated).
    pub atommc: AtoMmc,
    pub(super) fdc_nmi_was_asserted: bool,
    /// Breakpoints and watchpoints.
    pub debug: Debugger,
    /// Hold the IRQ line low from outside (the debugger's IRQ button).
    pub external_irq: bool,
    /// A falling edge on NMI is waiting to be seen by the CPU.
    pub nmi_edge: bool,
    /// Model the picture "snow" from the CPU and the 6847 sharing video RAM
    /// (off by default; see [`AtomBus::note_snow`]).
    pub snow: bool,
    /// With snow on: the line waiting to be drawn, with the video mode it
    /// started in, and what the CPU put on the bus during each of its fetches.
    snow_line: Option<(usize, VideoMode)>,
    snow_bytes: [Option<u8>; FETCH_SLOTS as usize],
}

impl AtomBus {
    fn new(standard: Standard) -> Self {
        AtomBus {
            memory: Memory::new(RamPreset::With22KExpansion),
            ppi: Ppi::default(),
            via: Via::new(),
            vdg: Vdg::new(standard),
            framebuffer: Framebuffer::new(),
            cycles: 0,
            sound: Sampler::new(CLOCK_HZ, 44_100.0),
            sid: SidCard::new(44_100),
            tape: Tape::new(),
            fdc: Fdc8271::new(),
            fdc1770: Fdc1770::new(),
            atommc: AtoMmc::default(),
            fdc_nmi_was_asserted: false,
            debug: Debugger::default(),
            external_irq: false,
            nmi_edge: false,
            snow: false,
            snow_line: None,
            snow_bytes: [None; FETCH_SLOTS as usize],
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
        if self.sound.tick(self.ppi.speaker()) && self.memory.sid_fitted {
            let sid_level = self.sid.next_sample();
            self.sound.add_to_last_sample(sid_level);
        }
        self.fdc.tick();
        self.fdc1770.tick(&mut self.fdc.drives);
        // The controller's NMI output is level-sensitive, the CPU's input is
        // edge-sensitive: remember each time it goes active.
        let fdc_nmi = self.fdc.nmi_asserted() || self.fdc1770.nmi_asserted();
        if fdc_nmi && !self.fdc_nmi_was_asserted {
            self.nmi_edge = true;
        }
        self.fdc_nmi_was_asserted = fdc_nmi;
        self.vdg.tick();
        self.ppi.field_sync_low = self.vdg.field_sync_low();
        if self.vdg.at_line_start() {
            if self.snow {
                self.draw_scanline_with_snow();
            } else if self.vdg.line() < VISIBLE_LINES {
                self.draw_scanline();
            }
        }
    }

    /// With snow on, a line cannot be drawn until it has finished, because
    /// the CPU may still disturb it. So at the start of each line, draw the
    /// one before it, then start collecting for this one.
    fn draw_scanline_with_snow(&mut self) {
        if let Some((line, mode)) = self.snow_line.take() {
            // Put the bytes the CPU put on the bus where the 6847 read them,
            // draw, and put the real bytes back.
            let mut saved = Vec::new();
            for (slot, byte) in self.snow_bytes.iter().enumerate() {
                if let Some(byte) = *byte {
                    let at = fetch_address(mode, line, slot);
                    saved.push((at, self.memory.ram[at]));
                    self.memory.ram[at] = byte;
                }
            }
            self.framebuffer
                .render_line(line, &self.memory.ram[..], mode, self.ppi.colour_set());
            for (at, byte) in saved.into_iter().rev() {
                self.memory.ram[at] = byte;
            }
        }
        self.snow_bytes = [None; FETCH_SLOTS as usize];
        if self.vdg.line() < VISIBLE_LINES {
            let mode = VideoMode::from_port_a_high_nibble(self.ppi.video_mode_bits());
            self.snow_line = Some((self.vdg.line() as usize, mode));
        }
    }

    /// The CPU has just used video RAM. The 6847 reads the same RAM to draw
    /// the picture, and on the Atom they take turns on the one set of address
    /// and data lines, so if the CPU gets in while the 6847 is fetching, the
    /// 6847 can see the CPU's byte instead of its own: a flicker of wrong
    /// characters ("snow"). This is an approximation (the real timing of the
    /// hand-over is not documented well enough to be exact), so it is off by
    /// default: when a fetch and an access coincide, the fetch gets the byte
    /// that was transferred.
    fn note_snow(&mut self, byte: u8) {
        if !self.snow || self.snow_line.is_none() {
            return;
        }
        if let Some(slot) = fetch_slot(self.vdg.cycle_in_line()) {
            self.snow_bytes[slot] = Some(byte);
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
        self.fdc1770.reset();
        self.atommc.reset();
        self.sid.reset();
        self.nmi_edge = false;
        self.fdc_nmi_was_asserted = false;
    }

    fn read_board_register(&self, addr: u16) -> u8 {
        self.memory
            .board
            .as_ref()
            .map_or_else(|| open_bus(addr), |board| board.read_register(addr))
    }

    /// Tell the speaker and SID what rate the sound card runs at.
    pub fn set_audio_sample_rate(&mut self, sample_rate: u32) {
        self.sound.set_sample_rate(CLOCK_HZ, f64::from(sample_rate));
        self.sid.set_sample_rate(sample_rate);
    }

    /// Read without side effects or clock ticks, for the debugger.
    pub fn peek(&self, addr: u16) -> u8 {
        match self.memory.region(addr) {
            Region::Ram | Region::VideoRam => self.memory.ram[addr as usize],
            Region::Rom(slot) => self.memory.read_rom(slot, addr),
            Region::Sid => self.sid.read(addr),
            Region::RamRom => self.read_board_register(addr),
            Region::Ppi
            | Region::Via
            | Region::Fdc
            | Region::Gdos1770
            | Region::AtoMmc
            | Region::Unmapped => open_bus(addr),
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
            Region::Sid => self.sid.read(addr),
            Region::RamRom => self.read_board_register(addr),
            Region::Gdos1770 => self.fdc1770.read(addr),
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
            Region::Ram => self.memory.ram[addr as usize] = value,
            Region::VideoRam => {
                self.memory.ram[addr as usize] = value;
                self.note_snow(value);
            }
            Region::Ppi => self.ppi.write(addr, value),
            Region::Via => self.via.write(addr, value),
            Region::Fdc => self.fdc.write(addr, value),
            Region::AtoMmc => self.atommc.write(addr, value),
            Region::Sid => self.sid.write(addr, value),
            Region::Gdos1770 => self.fdc1770.write(addr, value, &mut self.fdc.drives),
            Region::RamRom => {
                if let Some(board) = &mut self.memory.board {
                    board.write_register(addr, value);
                }
                self.memory.load_bbc_ram();
            }
            // Writes to ROM go nowhere, unless the RAM/ROM board has put RAM there.
            Region::Rom(slot) => self.memory.write_rom_area(slot, addr, value),
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
        self.bus.memory.board = None;
        self.bus.memory.remove_gdos();
        self.bus.memory.set_ram_preset(RamPreset::With22KExpansion);
        self.bus.atommc.remove_card();
        self.reset();
        Ok(())
    }

    /// Fit a different amount of RAM, and restart as if the Atom had been
    /// switched off to do it (what is in RAM is kept). Not possible with the
    /// RAM/ROM board, which provides RAM everywhere.
    pub fn set_ram_preset(&mut self, preset: RamPreset) -> Result<(), String> {
        if self.bus.memory.board.is_some() {
            return Err("the RAM/ROM board provides RAM everywhere in SD card mode".to_string());
        }
        self.bus.memory.set_ram_preset(preset);
        self.reset();
        Ok(())
    }

    /// Fit the RAM/ROM board with the given jumpers, which gives RAM
    /// everywhere. The ROM image, `ramrom.rom`, must be in `rom_dir`.
    fn install_board(&mut self, rom_dir: &Path, jumpers: u8) -> Result<(), String> {
        let path = rom_dir.join("ramrom.rom");
        let image = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let board =
            RamRomBoard::new(&image, jumpers).map_err(|e| format!("{}: {e}", path.display()))?;
        let memory = &mut self.bus.memory;
        memory.board = Some(board);
        memory.set_ram_preset(RamPreset::With23KExpansion);
        Ok(())
    }

    /// Fit the GDOS/ADOS 2015 disk system: a WD1770 disk controller (at
    /// `BC10`) and its ROM, in place of the standard Atom DOS and 8271 (the
    /// disks in the drives are shared). The ROM, `gdos2015.rom`, must be in
    /// `rom_dir`. Not available in SD card or BBC mode, which replace the
    /// disk ROM themselves. Restarts the Atom.
    pub fn use_gdos(&mut self, rom_dir: &Path) -> Result<(), String> {
        if self.bus.memory.board.is_some() {
            return Err("GDOS is for the floppy disk Atom, not SD card or BBC mode".to_string());
        }
        let path = rom_dir.join("gdos2015.rom");
        let image = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.bus
            .memory
            .install_gdos(&image)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        self.reset();
        Ok(())
    }

    /// Which kind of Atom this is now.
    pub fn machine(&self) -> Machine {
        if self.bbc_basic_active() {
            Machine::BbcBasic
        } else if self.bus.memory.board.is_some() {
            Machine::SdCard
        } else if self.gdos_active() {
            Machine::GdosAtom
        } else {
            Machine::FloppyAtom
        }
    }

    /// Become a different kind of Atom, and restart. Settings that belong to
    /// the old kind (the RAM size, the card, GDOS) are given up cleanly.
    ///
    /// `rom_dir` holds the ROM images. `card_folder` is the folder to use as
    /// the SD card, which the SD card machine needs.
    pub fn set_machine(
        &mut self,
        machine: Machine,
        rom_dir: &Path,
        card_folder: Option<&Path>,
    ) -> Result<(), String> {
        match machine {
            Machine::FloppyAtom => self.use_floppy_disks(rom_dir),
            Machine::GdosAtom => {
                self.use_floppy_disks(rom_dir)?;
                self.use_gdos(rom_dir)
            }
            Machine::SdCard => {
                let folder = card_folder.ok_or("choose a folder to use as the SD card first")?;
                self.use_sd_card(rom_dir, folder.to_path_buf())
            }
            Machine::BbcBasic => {
                // GDOS cannot coexist with the board.
                self.bus.memory.remove_gdos();
                self.set_bbc_basic(true, rom_dir)
            }
        }
    }

    /// Is the GDOS disk system fitted?
    pub fn gdos_active(&self) -> bool {
        self.bus.memory.gdos_fitted()
    }

    /// Is this a BBC BASIC machine? (See [`Atom::set_bbc_basic`].)
    pub fn bbc_basic_active(&self) -> bool {
        self.bus
            .memory
            .board
            .as_ref()
            .is_some_and(RamRomBoard::bbc_mode)
    }

    /// Switch the Atom to BBC BASIC (or back) and restart it: the RAM/ROM
    /// board's BBC jumper. In SD card mode the board is already there and only
    /// the jumper changes; otherwise a board is fitted just for this (and the
    /// floppy disk system is not available, as on the real thing). Switching
    /// off from there goes back to the ordinary floppy-disk Atom.
    pub fn set_bbc_basic(&mut self, on: bool, rom_dir: &Path) -> Result<(), String> {
        let sd_card_mode = self.bus.atommc.card_folder().is_some();
        match (&mut self.bus.memory.board, on) {
            (Some(board), _) if sd_card_mode => {
                board.jumpers = if on {
                    board.jumpers | ramrom::flag::BBCMODE
                } else {
                    board.jumpers & !ramrom::flag::BBCMODE
                };
                self.reset();
                Ok(())
            }
            (_, true) => {
                self.install_board(rom_dir, ramrom::flag::BBCMODE)?;
                self.reset();
                Ok(())
            }
            (_, false) => self.use_floppy_disks(rom_dir),
        }
    }

    /// Plug in a joystick (or `None` to unplug it). It is seen through both
    /// the AtoMMC's port and the keyboard matrix. With none plugged in the
    /// AtoMMC port answers 0, which software reads as "no joystick".
    pub fn set_joystick(&mut self, stick: Option<Joystick>) {
        self.bus.ppi.keyboard.joystick = stick;
        self.bus.atommc.joystick = stick.map(Joystick::port_bits);
    }

    /// Turn this into an Atom with an AtoMMC SD card interface, using the
    /// folder `card_folder` as the card.
    ///
    /// This is the Atom as the AtoMMC is used: with a Ramoth RAM/ROM board
    /// (see [`RamRomBoard`]) whose 128K ROM image, `ramrom.rom`, must be in
    /// `rom_dir`. The board's `DISKROM` jumper is fitted, so the machine boots
    /// with the AtoMMC ROM in place of Atom DOS and its matching kernel and
    /// BASIC, and the page at `0A00` is RAM. Resets the machine.
    pub fn use_sd_card(&mut self, rom_dir: &Path, card_folder: PathBuf) -> Result<(), String> {
        self.install_board(rom_dir, ramrom::flag::DISKROM)?;

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
        self.bus.memory.load_bbc_ram();
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

    /// A board whose ROM image has block `n` (at `n * 0x1000`) filled with `n`.
    fn atom_with_a_numbered_board(jumpers: u8) -> Atom {
        let image: Vec<u8> = (0..ramrom::IMAGE_SIZE)
            .map(|i| (i / 0x1000) as u8)
            .collect();
        let mut atom = Atom::new();
        atom.bus.memory.board = Some(RamRomBoard::new(&image, jumpers).unwrap());
        atom.bus.memory.set_ram_preset(RamPreset::With23KExpansion);
        atom
    }

    #[test]
    fn the_board_registers_are_on_the_bus_only_when_a_board_is_fitted() {
        let mut atom = Atom::new();
        atom.bus.write(0xBFFF, 0x05);
        assert_eq!(atom.bus.read(0xBFFF), 0xBF, "no board: open bus");

        let mut atom = atom_with_a_numbered_board(0);
        atom.bus.write(0xBFFF, 0x05);
        atom.bus.write(0xBFFE, 0x02);
        assert_eq!(atom.bus.read(0xBFFF), 0xB5);
        assert_eq!(atom.bus.read(0xBFFE), 0xB2);
        assert_eq!(atom.bus.read(0xBC00), 0xBF, "the rest of the page");
    }

    #[test]
    fn the_utility_block_shows_a_rom_bank_or_the_ram_at_7000() {
        let mut atom = atom_with_a_numbered_board(0);
        // A ROM bank: block n of the image.
        atom.bus.write(0xBFFF, 3);
        assert_eq!(atom.bus.read(0xA123), 3);
        atom.bus.write(0xA123, 0x99);
        assert_eq!(atom.bus.read(0xA123), 3, "ROM cannot be written");

        // EXTRAM with bank 0: the RAM at 7000, writable through A000.
        atom.bus.write(0x7123, 0xAB);
        atom.bus.write(0xBFFF, 0);
        atom.bus.write(0xBFFE, 0x01);
        assert_eq!(atom.bus.read(0xA123), 0xAB);
        atom.bus.write(0xA124, 0x55);
        assert_eq!(atom.bus.peek(0x7124), 0x55);
        assert_eq!(atom.bus.read(0xA124), 0x55);

        // With bank 1 selected it is a ROM again.
        atom.bus.write(0xBFFF, 1);
        assert_eq!(atom.bus.read(0xA123), 1);
    }

    #[test]
    fn the_board_flags_choose_the_rom_set_and_what_is_at_0a00() {
        // DISKROM jumper, as with an AtoMMC: its ROM set, and RAM at 0A00.
        let mut atom = atom_with_a_numbered_board(ramrom::flag::DISKROM);
        atom.bus.memory.disk_interface_fitted = true;
        assert_eq!(atom.bus.read(0xE000), 0x12, "the AtoMMC ROM");
        assert_eq!(atom.bus.read(0xC000), 0x10);
        atom.bus.write(0x0A00, 0x42);
        assert_eq!(
            atom.bus.read(0x0A00),
            0x42,
            "RAM, not the floppy controller"
        );

        // Clearing the flag in the register cancels the jumper: the other
        // ROM set, and the floppy controller shows at 0A00.
        atom.bus.write(0xBFFE, ramrom::flag::DISKROM);
        assert_eq!(atom.bus.read(0xC000), 0x14);
        assert_eq!(atom.bus.memory.region(0x0A00), Region::Fdc);
    }

    #[test]
    fn break_does_not_clear_the_board_so_a_loaded_rom_survives() {
        let mut atom = atom_with_a_numbered_board(0);
        atom.bus.write(0xBFFE, 0x01);
        atom.bus.write(0xBFFF, 0x00);
        atom.reset();
        assert_eq!(atom.bus.read(0xBFFE), 0xB1);
        assert!(
            atom.bus
                .memory
                .board
                .as_ref()
                .unwrap()
                .utility_block_is_ram(),
            "still showing the RAM"
        );
    }

    #[test]
    fn a_saved_state_restores_the_board_registers() {
        let mut atom = atom_with_a_numbered_board(0);
        atom.bus.write(0xBFFE, 0x05);
        atom.bus.write(0xBFFF, 0x07);
        let state = atom.save_state().unwrap();
        atom.bus.write(0xBFFE, 0x00);
        atom.bus.write(0xBFFF, 0x00);
        atom.load_state(&state).unwrap();
        assert_eq!(atom.bus.read(0xBFFE), 0xB5);
        assert_eq!(atom.bus.read(0xBFFF), 0xB7);
    }

    #[test]
    fn fitting_different_ram_restarts_the_atom_and_keeps_what_was_in_it() {
        let mut atom = Atom::new();
        atom.bus.memory.ram[0x0300] = 0x5A;
        atom.cpu.regs.pc = 0x1234;
        atom.set_ram_preset(RamPreset::Minimal).unwrap();
        assert_eq!(atom.bus.memory.ram_preset(), RamPreset::Minimal);
        assert_eq!(atom.bus.memory.ram[0x0300], 0x5A, "chips are not wiped");
        assert_ne!(atom.cpu.regs.pc, 0x1234, "it restarted");
        assert_eq!(atom.bus.memory.region(0x2900), Region::Unmapped);
    }

    #[test]
    fn the_ram_cannot_be_changed_in_sd_card_mode() {
        let mut atom = atom_with_a_numbered_board(0);
        assert!(atom.set_ram_preset(RamPreset::Minimal).is_err());
        assert_eq!(
            atom.bus.memory.ram_preset(),
            RamPreset::With23KExpansion,
            "unchanged"
        );
    }

    #[test]
    fn a_saved_state_remembers_how_much_ram_there_was() {
        let mut atom = Atom::new();
        atom.set_ram_preset(RamPreset::WithDiskRam).unwrap();
        let state = atom.save_state().unwrap();
        atom.set_ram_preset(RamPreset::With22KExpansion).unwrap();
        atom.load_state(&state).unwrap();
        assert_eq!(atom.bus.memory.ram_preset(), RamPreset::WithDiskRam);
    }

    #[test]
    fn writes_beyond_the_fitted_video_ram_are_lost_and_reads_see_open_bus() {
        let mut atom = Atom::new();
        atom.bus.write(0x8400, 0x55);
        assert_eq!(atom.bus.read(0x8400), 0x55, "8K: there");

        atom.bus.memory.set_video_ram_kb(1);
        atom.bus.write(0x8200, 0x66);
        assert_eq!(atom.bus.read(0x8200), 0x66, "inside the first 1K");
        atom.bus.write(0x8400, 0x77);
        assert_eq!(
            atom.bus.read(0x8400),
            0x84,
            "nothing there: the address high byte"
        );
        atom.bus.memory.set_video_ram_kb(8);
        assert_eq!(
            atom.bus.read(0x8400),
            0x55,
            "and the old contents were not touched"
        );
    }

    #[test]
    fn a_saved_state_remembers_the_video_ram_size() {
        let mut atom = Atom::new();
        atom.bus.memory.set_video_ram_kb(3);
        let state = atom.save_state().unwrap();
        atom.bus.memory.set_video_ram_kb(8);
        atom.load_state(&state).unwrap();
        assert_eq!(atom.bus.memory.video_ram_kb(), 3);
    }

    /// A 64K GDOS ROM in which bank `n` is full of `n`.
    fn numbered_gdos_rom() -> Vec<u8> {
        (0..crate::machine::memory::GDOS_IMAGE_SIZE)
            .map(|i| (i / 0x1000) as u8)
            .collect()
    }

    #[test]
    fn the_gdos_controller_is_on_the_bus_only_when_gdos_is_fitted() {
        let mut atom = Atom::new();
        atom.bus.write(0xBC11, 0x55);
        assert_eq!(atom.bus.read(0xBC11), 0xBC, "no GDOS: open bus");

        atom.bus.memory.install_gdos(&numbered_gdos_rom()).unwrap();
        atom.bus.write(0xBC11, 0x55); // the track register
        assert_eq!(atom.bus.read(0xBC11), 0x55);
        atom.bus.write(0xBC12, 0x07); // the sector register
        assert_eq!(atom.bus.read(0xBC12), 0x07);
    }

    #[test]
    fn the_gdos_rom_replaces_the_disk_rom_and_its_bank_can_be_chosen() {
        let mut atom = Atom::new();
        atom.bus.memory.install_gdos(&numbered_gdos_rom()).unwrap();
        assert_eq!(atom.bus.read(0xE123), 0, "bank 0");
        atom.bus.memory.set_gdos_bank(5);
        assert_eq!(atom.bus.read(0xE123), 5);
        atom.bus.memory.set_gdos_bank(21);
        assert_eq!(atom.bus.memory.gdos_bank(), 5, "21 wraps to 5 of 16");
        atom.bus.memory.remove_gdos();
        assert_eq!(
            atom.bus.read(0xE123),
            0xE1,
            "open bus again, nothing in the socket"
        );
    }

    #[test]
    fn a_finished_gdos_command_interrupts_the_cpu_with_an_nmi() {
        let mut atom = Atom::new();
        atom.bus.memory.install_gdos(&numbered_gdos_rom()).unwrap();
        atom.bus.write(0xBC10, 0x00); // restore
        // The command takes a while; every access is one cycle.
        let mut interrupted = false;
        for _ in 0..5000 {
            atom.bus.read(0x0000);
            if atom.bus.take_nmi_edge() {
                interrupted = true;
                break;
            }
        }
        assert!(interrupted, "no NMI when the restore finished");
    }

    #[test]
    fn gdos_cannot_be_fitted_with_the_ram_rom_board() {
        let mut atom = atom_with_a_numbered_board(0);
        assert!(atom.use_gdos(Path::new("no-such-folder")).is_err());
    }

    #[test]
    fn a_joystick_is_seen_through_the_atommc_port() {
        let mut atom = Atom::new();
        // No stick plugged in: software reads 0, "no joystick".
        atom.bus.write(0xB400, 0xA2);
        assert_eq!(atom.bus.read(0xB401), 0);

        atom.set_joystick(Some(Joystick::default()));
        atom.bus.write(0xB400, 0xA2);
        assert_eq!(atom.bus.read(0xB401), 0xFF, "a stick at rest");

        atom.set_joystick(Some(Joystick {
            left: true,
            fire: true,
            ..Joystick::default()
        }));
        atom.bus.write(0xB400, 0xA2);
        assert_eq!(atom.bus.read(0xB401), 0b1110_1101);

        atom.set_joystick(None);
        atom.bus.write(0xB400, 0xA2);
        assert_eq!(atom.bus.read(0xB401), 0, "unplugged again");
    }

    #[test]
    fn a_joystick_is_seen_on_keyboard_row_zero_and_only_there() {
        let mut atom = Atom::new();
        atom.set_joystick(Some(Joystick {
            up: true,
            ..Joystick::default()
        }));
        atom.bus.write(0xB000, 0x00); // select row 0
        assert_eq!(atom.bus.read(0xB001) & 0x3F, 0b10_1111, "Q's bit is low");
        atom.bus.write(0xB000, 0x01); // another row
        assert_eq!(atom.bus.read(0xB001) & 0x3F, 0x3F, "nothing pressed there");
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
