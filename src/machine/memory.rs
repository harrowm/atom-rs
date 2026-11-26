//! The Atom's memory map: which chip or memory answers at each address.
//!
//! ```text
//! 0000-09FF  RAM (zero page, stack, OS workspace, then the RAM expansion)
//! 0B00-7FFF  RAM (about 31K in all; `RamLayout::standard()` is the smaller
//!            25K Atom with RAM only at 0000-03FF and 2000-7FFF)
//! 8000-9FFF  Video RAM, read by the 6847
//! 0A00-0AFF  8271 disk controller (only when the disk interface is fitted)
//! A000-AFFF  Utility ROM socket
//! B000-B3FF  8255 PPI  (keyboard, video mode, speaker, tape)
//! B400-B7FF  AtoMMC (only its joystick/port commands are emulated)
//! B800-BBFF  6522 VIA
//! BC00-BFFF  Other expansion (not emulated yet)
//! C000-CFFF  BASIC ROM
//! D000-DFFF  Floating point ROM
//! E000-EFFF  Disk ROM socket
//! F000-FFFF  MOS kernel ROM
//! ```
//!
//! This follows Atomulator. Anything not listed is "unmapped": writes do
//! nothing and reads return the high byte of the address (the value left
//! floating on the data bus, which Atomulator mimics).

use std::ops::Range;

use super::ramrom::RamRomBoard;

/// The five 4 KiB ROM sockets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RomSlot {
    /// `A000-AFFF`
    Utility,
    /// `C000-CFFF`
    Basic,
    /// `D000-DFFF`
    FloatingPoint,
    /// `E000-EFFF`
    Disk,
    /// `F000-FFFF`
    Kernel,
}

impl RomSlot {
    pub const ALL: [RomSlot; 5] = [
        RomSlot::Utility,
        RomSlot::Basic,
        RomSlot::FloatingPoint,
        RomSlot::Disk,
        RomSlot::Kernel,
    ];

    fn index(self) -> usize {
        self as usize
    }
}

pub const ROM_SIZE: usize = 0x1000;

/// What a given address is connected to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Ram,
    VideoRam,
    Fdc,
    AtoMmc,
    Sid,
    /// The RAM/ROM board's three registers (`BFFD-BFFF`) and the rest of that page.
    RamRom,
    Ppi,
    Via,
    Rom(RomSlot),
    Unmapped,
}

/// Which parts of `0000-7FFF` have RAM chips fitted.
#[derive(Clone, Debug)]
pub struct RamLayout {
    ranges: Vec<Range<u16>>,
}

impl RamLayout {
    /// Atomulator's default: 1K at the bottom plus 24K from `2000`, 25K in all.
    pub fn standard() -> Self {
        RamLayout {
            ranges: vec![0x0000..0x0400, 0x2000..0x8000],
        }
    }

    /// RAM everywhere from `0000` to `7FFF`, including the page at `0A00`
    /// where the 8271 disk controller would otherwise be. This is what a machine
    /// with an AtoMMC (and no floppy controller) has.
    pub fn full() -> Self {
        RamLayout {
            ranges: std::iter::once(0x0000..0x8000).collect(),
        }
    }

    /// The Atom with its RAM expansion: everything from `0000` to `7FFF`
    /// except the disk controller's page at `0A00-0AFF`, about 31K. Much
    /// software (Chuckie Egg, for one) keeps its variables in `0400-07FF`.
    pub fn expanded() -> Self {
        RamLayout {
            ranges: vec![0x0000..0x0A00, 0x0B00..0x8000],
        }
    }

    fn contains(&self, addr: u16) -> bool {
        self.ranges.iter().any(|range| range.contains(&addr))
    }
}

/// The video RAM window.
const VIDEO_RAM: Range<u16> = 0x8000..0xA000;

/// What the RAM holds when the Atom is switched on.
///
/// Real RAM chips power up with junk in them, and software relies on it: Atom
/// BASIC keeps its `RND` seed in the zero page and never sets it, so a
/// programme like Acornsoft's Asteroids, which places its rocks with `RND`,
/// gets none at all if the seed starts as zero. A fixed pseudo-random pattern
/// gives the same "junk" on every run, so the emulator stays repeatable.
fn power_on_ram() -> [u8; 0x10000] {
    let mut ram = [0; 0x10000];
    let mut state: u32 = 0x2545_F491;
    for byte in ram.iter_mut() {
        // xorshift32: a few shifts and xors give a decent scramble.
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *byte = (state >> 24) as u8;
    }
    ram
}

#[derive(Clone, Debug)]
pub struct Memory {
    /// All of the RAM, video RAM included, as one 64 KiB array. Only the
    /// addresses that [`Memory::region`] says are RAM are ever used.
    pub ram: Box<[u8; 0x10000]>,
    roms: [Option<Box<[u8; ROM_SIZE]>>; 5],
    layout: RamLayout,
    /// Is the disk interface fitted? (True once a disk ROM is loaded.)
    pub disk_interface_fitted: bool,
    /// Is a SID sound card fitted (at `BDC0-BDDF`)? Atomulator has one by
    /// default, so it is here too; programs that do not use it never notice.
    pub sid_fitted: bool,
    /// The Ramoth RAM/ROM board, if fitted. It takes over the ROM sockets, the
    /// page at `0A00` and the top of the I/O page (see [`RamRomBoard`]).
    pub board: Option<RamRomBoard>,
}

impl Memory {
    pub fn new(layout: RamLayout) -> Self {
        Memory {
            ram: Box::new(power_on_ram()),
            roms: Default::default(),
            layout,
            disk_interface_fitted: false,
            sid_fitted: true,
            board: None,
        }
    }

    /// Change which parts of `0000-7FFF` have RAM.
    pub fn set_layout(&mut self, layout: RamLayout) {
        self.layout = layout;
    }

    pub fn load_rom(&mut self, slot: RomSlot, data: &[u8]) -> Result<(), String> {
        let contents: [u8; ROM_SIZE] = data
            .try_into()
            .map_err(|_| format!("{slot:?} ROM must be {ROM_SIZE} bytes, got {}", data.len()))?;
        self.roms[slot.index()] = Some(Box::new(contents));
        Ok(())
    }

    /// A write into a ROM socket. Normally it goes nowhere, but with the
    /// board's RAM switched into the utility block it changes that RAM.
    pub fn write_rom_area(&mut self, slot: RomSlot, addr: u16, value: u8) {
        let ram_is_mapped = self
            .board
            .as_ref()
            .is_some_and(|board| board.utility_block_is_ram());
        if slot == RomSlot::Utility && ram_is_mapped {
            self.ram[utility_ram_address(addr)] = value;
        }
    }

    /// Decode an address into the device that answers it.
    pub fn region(&self, addr: u16) -> Region {
        match addr {
            // With the RAM/ROM board its flags decide whether 0A00 is RAM or the
            // floppy controller, whatever the RAM layout says.
            0x0A00..=0x0AFF if self.board_selects_floppy_controller() => Region::Fdc,
            0x0000..=0x7FFF if self.layout.contains(addr) => Region::Ram,
            0x0A00..=0x0AFF if self.board.is_none() && self.disk_interface_fitted => Region::Fdc,
            a if VIDEO_RAM.contains(&a) => Region::VideoRam,
            0xA000..=0xAFFF => Region::Rom(RomSlot::Utility),
            0xB000..=0xB3FF => Region::Ppi,
            0xB400..=0xB7FF => Region::AtoMmc,
            0xB800..=0xBBFF => Region::Via,
            0xBDC0..=0xBDDF if self.sid_fitted => Region::Sid,
            0xBC00..=0xBFFF if self.board.is_some() => Region::RamRom,
            0xC000..=0xCFFF => Region::Rom(RomSlot::Basic),
            0xD000..=0xDFFF => Region::Rom(RomSlot::FloatingPoint),
            0xE000..=0xEFFF => Region::Rom(RomSlot::Disk),
            0xF000..=0xFFFF => Region::Rom(RomSlot::Kernel),
            _ => Region::Unmapped,
        }
    }

    fn board_selects_floppy_controller(&self) -> bool {
        self.disk_interface_fitted
            && self
                .board
                .as_ref()
                .is_some_and(|board| !board.page_a_is_ram())
    }

    /// Read a byte from a ROM socket; an empty socket reads as open bus.
    ///
    /// With the RAM/ROM board fitted it supplies most of the sockets: see
    /// [`RamRomBoard::read_rom`]. A socket it leaves alone reads what is
    /// fitted in it.
    pub fn read_rom(&self, slot: RomSlot, addr: u16) -> u8 {
        if let Some(board) = &self.board {
            if slot == RomSlot::Utility && board.utility_block_is_ram() {
                return self.ram[utility_ram_address(addr)];
            }
            if let Some(byte) = board.read_rom(slot, addr) {
                return byte;
            }
        }
        match &self.roms[slot.index()] {
            Some(rom) => rom[addr as usize & (ROM_SIZE - 1)],
            None => open_bus(addr),
        }
    }
}

/// The RAM that the board maps into the utility block `A000-AFFF` is the RAM
/// at `7000-7FFF`: the byte at `A123` is the one at `7123`.
fn utility_ram_address(addr: u16) -> usize {
    0x7000 + (usize::from(addr) & 0x0FFF)
}

/// The value seen when nothing drives the data bus.
pub fn open_bus(addr: u16) -> u8 {
    (addr >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_main_regions() {
        let memory = Memory::new(RamLayout::standard());
        assert_eq!(memory.region(0x0000), Region::Ram);
        assert_eq!(memory.region(0x03FF), Region::Ram);
        assert_eq!(memory.region(0x0400), Region::Unmapped);
        assert_eq!(memory.region(0x2000), Region::Ram);
        assert_eq!(memory.region(0x8000), Region::VideoRam);
        assert_eq!(memory.region(0xB002), Region::Ppi);
        assert_eq!(memory.region(0xB800), Region::Via);
        assert_eq!(memory.region(0xB401), Region::AtoMmc);
        assert_eq!(memory.region(0xC000), Region::Rom(RomSlot::Basic));
        assert_eq!(memory.region(0xFFFC), Region::Rom(RomSlot::Kernel));
    }

    #[test]
    fn ram_powers_up_holding_junk_not_zeros() {
        // The zero page holds BASIC's RND seed (08-0B), which must not be zero.
        let memory = Memory::new(RamLayout::expanded());
        assert!(memory.ram[8..12].iter().any(|&b| b != 0));
        let zeros = memory.ram.iter().filter(|&&b| b == 0).count();
        assert!(
            zeros < 1000,
            "{zeros} zero bytes out of 65536 looks like a blank memory"
        );
    }

    #[test]
    fn expanded_layout_has_ram_in_the_low_pages_around_the_disk_controller() {
        // Software such as Chuckie Egg keeps its variables here.
        let mut memory = Memory::new(RamLayout::expanded());
        memory.disk_interface_fitted = true;
        for addr in [0x0400, 0x07FF, 0x09FF, 0x0B00, 0x1FFF, 0x2000, 0x7FFF] {
            assert_eq!(memory.region(addr), Region::Ram, "{addr:04X}");
        }
        assert_eq!(memory.region(0x0A00), Region::Fdc);
        assert_eq!(
            Memory::new(RamLayout::standard()).region(0x0400),
            Region::Unmapped
        );
    }

    #[test]
    fn disk_controller_appears_only_when_fitted() {
        let mut memory = Memory::new(RamLayout::standard());
        assert_eq!(memory.region(0x0A04), Region::Unmapped);
        memory.disk_interface_fitted = true;
        assert_eq!(memory.region(0x0A04), Region::Fdc);
    }

    #[test]
    fn empty_rom_socket_reads_open_bus() {
        let memory = Memory::new(RamLayout::standard());
        assert_eq!(memory.read_rom(RomSlot::Basic, 0xC123), 0xC1);
    }
}
