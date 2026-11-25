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

#[derive(Clone, Debug)]
pub struct Memory {
    /// All of the RAM, video RAM included, as one 64 KiB array. Only the
    /// addresses that [`Memory::region`] says are RAM are ever used.
    pub ram: Box<[u8; 0x10000]>,
    roms: [Option<Box<[u8; ROM_SIZE]>>; 5],
    layout: RamLayout,
    /// Is the disk interface fitted? (True once a disk ROM is loaded.)
    pub disk_interface_fitted: bool,
}

impl Memory {
    pub fn new(layout: RamLayout) -> Self {
        Memory {
            ram: Box::new([0; 0x10000]),
            roms: Default::default(),
            layout,
            disk_interface_fitted: false,
        }
    }

    pub fn load_rom(&mut self, slot: RomSlot, data: &[u8]) -> Result<(), String> {
        let contents: [u8; ROM_SIZE] = data
            .try_into()
            .map_err(|_| format!("{slot:?} ROM must be {ROM_SIZE} bytes, got {}", data.len()))?;
        self.roms[slot.index()] = Some(Box::new(contents));
        Ok(())
    }

    /// Decode an address into the device that answers it.
    pub fn region(&self, addr: u16) -> Region {
        match addr {
            0x0000..=0x7FFF if self.layout.contains(addr) => Region::Ram,
            0x0A00..=0x0AFF if self.disk_interface_fitted => Region::Fdc,
            a if VIDEO_RAM.contains(&a) => Region::VideoRam,
            0xA000..=0xAFFF => Region::Rom(RomSlot::Utility),
            0xB000..=0xB3FF => Region::Ppi,
            0xB400..=0xB7FF => Region::AtoMmc,
            0xB800..=0xBBFF => Region::Via,
            0xC000..=0xCFFF => Region::Rom(RomSlot::Basic),
            0xD000..=0xDFFF => Region::Rom(RomSlot::FloatingPoint),
            0xE000..=0xEFFF => Region::Rom(RomSlot::Disk),
            0xF000..=0xFFFF => Region::Rom(RomSlot::Kernel),
            _ => Region::Unmapped,
        }
    }

    /// Read a byte from a ROM socket; an empty socket reads as open bus.
    pub fn read_rom(&self, slot: RomSlot, addr: u16) -> u8 {
        match &self.roms[slot.index()] {
            Some(rom) => rom[addr as usize & (ROM_SIZE - 1)],
            None => open_bus(addr),
        }
    }
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
