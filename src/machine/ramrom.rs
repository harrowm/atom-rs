//! The Ramoth RAM/ROM board.
//!
//! A board that plugs into the Atom's ROM sockets and replaces them with a 128K
//! ROM chip and some RAM, so ROMs can be swapped by software instead of by
//! pulling chips out. It is what the AtoMMC SD card interface is used with:
//! the Atom boots with the AtoMMC's ROM and kernel in place of Atom DOS, and
//! utility ROMs (word processors, editors, toolkits) are loaded from the card
//! and switched in at `A000`.
//!
//! The board has three registers at the top of the I/O page:
//!
//! | Address | Write | Read |
//! |---------|-------|------|
//! | `BFFF`  | the ROM bank at `A000` (4 bits) | `B0` + the bank |
//! | `BFFE`  | the four enable flags below (4 bits) | `B0` + the flags |
//! | `BFFD`  | nothing | `B0` + the jumpers |
//!
//! Anything else from `BC00` to `BFFF` reads `BF`.
//!
//! The registers start at zero when the board is installed (power on), but
//! BREAK does **not** clear them. That matters: a utility ROM is loaded into
//! RAM, switched in with `EXTRAM`, and then BREAK is pressed to start it, so
//! it has to survive the BREAK. (Atomulator behaves the same way.)
//!
//! ## The enable flags
//!
//! The flag in force is the flag register **exclusive-or the jumpers** (the
//! jumpers are links on the board, which a program cannot change). For
//! example the AtoMMC setup has the `DISKROM` jumper fitted.
//!
//! * `EXTRAM` (1): with bank 0 selected, `A000-AFFF` is RAM (the same RAM that
//!   is at `7000-7FFF`) instead of ROM. A ROM image read from the SD card is
//!   put into RAM at `7000` and then switched in this way.
//! * `BLKA_RAM` (2): see `DISKROM`.
//! * `DISKROM` (4): which set of ROMs is in use. With it set, the BASIC,
//!   floating point and kernel ROMs come from the board, the `E000` socket
//!   holds the AtoMMC ROM, and (unless `BLKA_RAM` is also set) the page at
//!   `0A00` is RAM instead of the floppy controller. With it clear, the
//!   board's other copy of the BASIC, floating point and kernel ROMs is used
//!   and `E000` is left to whatever Atom DOS ROM is fitted.
//! * `BBCMODE` (8): swaps the whole machine over to BBC BASIC: the five ROM
//!   sockets show BBC BASIC (in four 4K pieces, at `A000`, `C000`, `D000` and
//!   `E000`) and the BBC operating system (at `F000`), and two more ROM
//!   images are copied into RAM at `6000` and `7000` (the first of them
//!   chosen by the bank register). Normally this is a jumper on the board
//!   rather than something a program sets.
//!
//! ## The 128K ROM image
//!
//! | Offset | What |
//! |--------|------|
//! | `00000-07FFF` | the eight banks of `A000` |
//! | `08000-0FFFF` | BBC mode: the eight banks of `6000` |
//! | `10000` `11000` `12000` `13000` | BASIC, floating point, AtoMMC ROM, kernel (`DISKROM` set) |
//! | `14000` `15000` `17000` | BASIC, floating point, kernel (`DISKROM` clear) |
//! | `19000` | BBC mode: the ROM copied to RAM at `7000` |
//! | `1A000` `1C000` `1D000` `1E000` | BBC mode: BBC BASIC, four 4K pieces |
//! | `1F000` | BBC mode: the BBC operating system |
//!
//! Following Atomulator, which documents the layout in its `roms.h`.

use super::memory::{ROM_SIZE, RomSlot};

/// The ROM image is this big.
pub const IMAGE_SIZE: usize = 0x20000;

/// The first and last addresses of the three registers.
const BANK_REGISTER: u16 = 0xBFFF;
const FLAGS_REGISTER: u16 = 0xBFFE;
const JUMPERS_REGISTER: u16 = 0xBFFD;
/// What the unused addresses read as, and what the top nibble of a register
/// reads as.
const UNUSED: u8 = 0xBF;
const HIGH_NIBBLE: u8 = 0xB0;

pub mod flag {
    pub const EXTRAM: u8 = 0x01;
    pub const BLKA_RAM: u8 = 0x02;
    pub const DISKROM: u8 = 0x04;
    pub const BBCMODE: u8 = 0x08;
}

/// Where each ROM set starts in the image.
mod offset {
    pub const UTILITY_BANKS: usize = 0x00000;
    pub const BASIC_WITH_DISKROM: usize = 0x10000;
    pub const FLOAT_WITH_DISKROM: usize = 0x11000;
    pub const ATOMMC_ROM: usize = 0x12000;
    pub const KERNEL_WITH_DISKROM: usize = 0x13000;
    pub const BASIC_WITHOUT: usize = 0x14000;
    pub const FLOAT_WITHOUT: usize = 0x15000;
    pub const KERNEL_WITHOUT: usize = 0x17000;
    /// BBC mode: ROMs copied into RAM at 6000 (one of eight banks) and 7000.
    pub const BBC_RAM_AT_6000_BANKS: usize = 0x08000;
    pub const BBC_RAM_AT_7000: usize = 0x19000;
    /// BBC mode: what the five sockets show.
    pub const BBC_BASIC_1: usize = 0x1A000;
    pub const BBC_BASIC_2: usize = 0x1C000;
    pub const BBC_BASIC_3: usize = 0x1D000;
    pub const BBC_BASIC_4: usize = 0x1E000;
    pub const BBC_OS: usize = 0x1F000;
}

#[derive(Clone, Debug)]
pub struct RamRomBoard {
    image: Box<[u8; IMAGE_SIZE]>,
    /// The bank register (`BFFF`), 4 bits.
    bank: u8,
    /// The flags register (`BFFE`), 4 bits.
    flags: u8,
    /// The links fitted on the board, 4 bits (`BFFD`).
    pub jumpers: u8,
}

impl RamRomBoard {
    /// A board holding `image`, which must be the 128K ROM image.
    pub fn new(image: &[u8], jumpers: u8) -> Result<Self, String> {
        let image: Box<[u8; IMAGE_SIZE]> = Box::<[u8]>::from(image)
            .try_into()
            .map_err(|_| format!("the RAM/ROM image must be {IMAGE_SIZE} bytes"))?;
        Ok(RamRomBoard {
            image,
            bank: 0,
            flags: 0,
            jumpers: jumpers & 0x0F,
        })
    }

    /// The registers, for saving a state: (bank, flags).
    pub fn registers(&self) -> (u8, u8) {
        (self.bank, self.flags)
    }

    pub fn set_registers(&mut self, bank: u8, flags: u8) {
        self.bank = bank & 0x0F;
        self.flags = flags & 0x0F;
    }

    // ---- The registers ------------------------------------------------------------

    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            BANK_REGISTER => HIGH_NIBBLE | self.bank,
            FLAGS_REGISTER => HIGH_NIBBLE | self.flags,
            JUMPERS_REGISTER => HIGH_NIBBLE | self.jumpers,
            _ => UNUSED,
        }
    }

    pub fn write_register(&mut self, addr: u16, value: u8) {
        match addr {
            BANK_REGISTER => self.bank = value & 0x0F,
            FLAGS_REGISTER => self.flags = value & 0x0F,
            _ => {}
        }
    }

    // ---- What the flags mean ------------------------------------------------------

    /// Is this flag in force? (The register exclusive-or the jumpers.)
    pub fn flag(&self, flag: u8) -> bool {
        (self.flags ^ self.jumpers) & flag != 0
    }

    /// Is the page at `0A00` RAM (rather than the floppy controller)?
    pub fn page_a_is_ram(&self) -> bool {
        !self.flag(flag::BLKA_RAM) && self.flag(flag::DISKROM)
    }

    /// Is `A000-AFFF` the RAM from `7000-7FFF` (rather than a ROM bank)?
    pub fn utility_block_is_ram(&self) -> bool {
        self.flag(flag::EXTRAM) && self.bank == 0 && !self.bbc_mode()
    }

    /// Is the machine a BBC BASIC machine?
    pub fn bbc_mode(&self) -> bool {
        self.flag(flag::BBCMODE)
    }

    /// In BBC mode, the two ROMs copied into RAM: the 4K for `6000` (which
    /// one depends on the bank register) and the 4K for `7000`.
    pub fn bbc_ram_images(&self) -> (&[u8], &[u8]) {
        let first = offset::BBC_RAM_AT_6000_BANKS + usize::from(self.bank) * ROM_SIZE;
        let second = offset::BBC_RAM_AT_7000;
        (
            &self.image[first..first + ROM_SIZE],
            &self.image[second..second + ROM_SIZE],
        )
    }

    /// A byte of the ROM the board puts in `slot`, or `None` if the board
    /// leaves that socket to whatever ROM is fitted there.
    ///
    /// (The utility socket is handled by the caller when it is RAM.)
    pub fn read_rom(&self, slot: RomSlot, addr: u16) -> Option<u8> {
        let disk_rom = self.flag(flag::DISKROM);
        if self.bbc_mode() {
            let start = match slot {
                RomSlot::Utility => offset::BBC_BASIC_1,
                RomSlot::Basic => offset::BBC_BASIC_2,
                RomSlot::FloatingPoint => offset::BBC_BASIC_3,
                RomSlot::Disk => offset::BBC_BASIC_4,
                RomSlot::Kernel => offset::BBC_OS,
            };
            return Some(self.image[start + usize::from(addr) % ROM_SIZE]);
        }
        let start = match slot {
            RomSlot::Utility => offset::UTILITY_BANKS + usize::from(self.bank) * ROM_SIZE,
            RomSlot::Basic if disk_rom => offset::BASIC_WITH_DISKROM,
            RomSlot::Basic => offset::BASIC_WITHOUT,
            RomSlot::FloatingPoint if disk_rom => offset::FLOAT_WITH_DISKROM,
            RomSlot::FloatingPoint => offset::FLOAT_WITHOUT,
            RomSlot::Disk if disk_rom => offset::ATOMMC_ROM,
            RomSlot::Disk => return None,
            RomSlot::Kernel if disk_rom => offset::KERNEL_WITH_DISKROM,
            RomSlot::Kernel => offset::KERNEL_WITHOUT,
        };
        Some(self.image[start + usize::from(addr) % ROM_SIZE])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image where each 4K block is filled with a different value, so a
    /// read shows which block it came from: block `n` (at `n * 0x1000`) is `n`.
    fn numbered_image() -> Vec<u8> {
        (0..IMAGE_SIZE).map(|i| (i / ROM_SIZE) as u8).collect()
    }

    fn board(jumpers: u8) -> RamRomBoard {
        RamRomBoard::new(&numbered_image(), jumpers).unwrap()
    }

    #[test]
    fn the_image_must_be_128k() {
        assert!(RamRomBoard::new(&[0; 1000], 0).is_err());
        assert!(RamRomBoard::new(&numbered_image(), 0).is_ok());
    }

    #[test]
    fn the_registers_read_back_with_b0_in_the_top_nibble() {
        let mut board = board(0b0100);
        board.write_register(BANK_REGISTER, 0x35);
        board.write_register(FLAGS_REGISTER, 0xA9);
        assert_eq!(
            board.read_register(BANK_REGISTER),
            0xB5,
            "only 4 bits are kept"
        );
        assert_eq!(board.read_register(FLAGS_REGISTER), 0xB9);
        assert_eq!(board.read_register(JUMPERS_REGISTER), 0xB4);
        assert_eq!(board.read_register(0xBFF0), 0xBF, "anywhere else");
    }

    #[test]
    fn the_jumpers_cannot_be_written() {
        let mut board = board(0b0100);
        board.write_register(JUMPERS_REGISTER, 0x0F);
        assert_eq!(board.read_register(JUMPERS_REGISTER), 0xB4);
    }

    #[test]
    fn a_flag_in_force_is_the_register_xor_the_jumpers() {
        let mut board = board(flag::DISKROM);
        assert!(board.flag(flag::DISKROM), "set by the jumper alone");
        board.write_register(FLAGS_REGISTER, flag::DISKROM);
        assert!(
            !board.flag(flag::DISKROM),
            "the register cancels the jumper"
        );
        board.write_register(FLAGS_REGISTER, flag::EXTRAM);
        assert!(board.flag(flag::EXTRAM) && board.flag(flag::DISKROM));
    }

    #[test]
    fn the_utility_socket_shows_the_selected_bank() {
        let mut board = board(0);
        for bank in 0..8u8 {
            board.write_register(BANK_REGISTER, bank);
            assert_eq!(board.read_rom(RomSlot::Utility, 0xA123), Some(bank));
        }
    }

    #[test]
    fn the_disk_rom_flag_chooses_the_rom_set() {
        let with = board(flag::DISKROM);
        assert_eq!(with.read_rom(RomSlot::Basic, 0xC000), Some(0x10));
        assert_eq!(with.read_rom(RomSlot::FloatingPoint, 0xD000), Some(0x11));
        assert_eq!(
            with.read_rom(RomSlot::Disk, 0xE000),
            Some(0x12),
            "the AtoMMC ROM"
        );
        assert_eq!(with.read_rom(RomSlot::Kernel, 0xF000), Some(0x13));

        let without = board(0);
        assert_eq!(without.read_rom(RomSlot::Basic, 0xC000), Some(0x14));
        assert_eq!(without.read_rom(RomSlot::FloatingPoint, 0xD000), Some(0x15));
        assert_eq!(
            without.read_rom(RomSlot::Disk, 0xE000),
            None,
            "left to the fitted ROM"
        );
        assert_eq!(without.read_rom(RomSlot::Kernel, 0xF000), Some(0x17));
    }

    #[test]
    fn page_a_is_ram_only_with_the_disk_rom_and_without_blka() {
        assert!(board(flag::DISKROM).page_a_is_ram());
        assert!(
            !board(0).page_a_is_ram(),
            "no AtoMMC: the floppy controller is there"
        );
        assert!(!board(flag::DISKROM | flag::BLKA_RAM).page_a_is_ram());
    }

    #[test]
    fn the_utility_block_is_ram_with_extram_and_bank_0() {
        let mut board = board(0);
        assert!(!board.utility_block_is_ram());
        board.write_register(FLAGS_REGISTER, flag::EXTRAM);
        assert!(board.utility_block_is_ram());
        board.write_register(BANK_REGISTER, 3);
        assert!(!board.utility_block_is_ram(), "a ROM bank is selected");
    }

    #[test]
    fn bbc_mode_puts_bbc_basic_and_the_bbc_os_in_the_sockets() {
        let bbc = board(flag::BBCMODE);
        assert!(bbc.bbc_mode());
        assert_eq!(bbc.read_rom(RomSlot::Utility, 0xA000), Some(0x1A));
        assert_eq!(bbc.read_rom(RomSlot::Basic, 0xC000), Some(0x1C));
        assert_eq!(bbc.read_rom(RomSlot::FloatingPoint, 0xD000), Some(0x1D));
        assert_eq!(bbc.read_rom(RomSlot::Disk, 0xE000), Some(0x1E));
        assert_eq!(bbc.read_rom(RomSlot::Kernel, 0xF000), Some(0x1F));
    }

    #[test]
    fn bbc_mode_has_no_ram_in_the_utility_socket_and_chooses_the_6000_rom_by_bank() {
        let mut board = board(flag::BBCMODE);
        board.write_register(FLAGS_REGISTER, flag::EXTRAM);
        assert!(!board.utility_block_is_ram(), "EXTRAM is for Atom mode");
        let (at_6000, at_7000) = board.bbc_ram_images();
        assert_eq!((at_6000[0], at_7000[0]), (0x08, 0x19), "bank 0");
        board.write_register(BANK_REGISTER, 3);
        assert_eq!(board.bbc_ram_images().0[0], 0x0B, "bank 3 is the 4th block");
        assert_eq!(
            board.bbc_ram_images().1[0],
            0x19,
            "7000 does not depend on it"
        );
    }
}
