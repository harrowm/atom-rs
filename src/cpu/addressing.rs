//! Working out where an instruction's operand lives, one bus cycle at a time.

use super::opcodes::{Access, Mode};
use super::{Bus, Cpu};

/// The result of resolving an addressing mode.
pub(super) struct Operand {
    pub addr: u16,
    /// High byte of the address *before* the index was added. A few
    /// undocumented store instructions (`AHX`, `SHX`, ...) use it.
    pub base_high: u8,
    /// Did adding the index carry into the high byte?
    pub page_crossed: bool,
}

impl Operand {
    fn at(addr: u16) -> Self {
        Operand {
            addr,
            base_high: (addr >> 8) as u8,
            page_crossed: false,
        }
    }
}

impl Cpu {
    /// Fetch the operand bytes for `mode` and return the effective address,
    /// making every bus access the real chip makes on the way.
    pub(super) fn operand(&mut self, bus: &mut impl Bus, mode: Mode, access: Access) -> Operand {
        match mode {
            Mode::Immediate => {
                let addr = self.regs.pc;
                self.regs.pc = addr.wrapping_add(1);
                Operand::at(addr)
            }
            Mode::ZeroPage => Operand::at(self.fetch(bus) as u16),
            Mode::ZeroPageX => self.zero_page_indexed(bus, self.regs.x),
            Mode::ZeroPageY => self.zero_page_indexed(bus, self.regs.y),
            Mode::Absolute => Operand::at(self.fetch_u16(bus)),
            Mode::AbsoluteX => {
                let base = self.fetch_u16(bus);
                self.add_index(bus, base, self.regs.x, access)
            }
            Mode::AbsoluteY => {
                let base = self.fetch_u16(bus);
                self.add_index(bus, base, self.regs.y, access)
            }
            Mode::IndirectX => {
                let pointer = self.fetch(bus);
                // While the CPU adds X it reads the unindexed pointer.
                self.dummy_read(bus, pointer as u16);
                let pointer = pointer.wrapping_add(self.regs.x);
                Operand::at(self.read_zero_page_u16(bus, pointer))
            }
            Mode::IndirectY => {
                let pointer = self.fetch(bus);
                let base = self.read_zero_page_u16(bus, pointer);
                self.add_index(bus, base, self.regs.y, access)
            }
            Mode::Implied | Mode::Accumulator | Mode::Relative | Mode::Indirect => {
                unreachable!("{mode:?} has no memory operand")
            }
        }
    }

    /// `zp,X` and `zp,Y`: the sum wraps within page zero.
    fn zero_page_indexed(&mut self, bus: &mut impl Bus, index: u8) -> Operand {
        let base = self.fetch(bus);
        self.dummy_read(bus, base as u16);
        Operand::at(base.wrapping_add(index) as u16)
    }

    /// Read a 16-bit pointer from zero page; the high byte wraps within page zero.
    fn read_zero_page_u16(&mut self, bus: &mut impl Bus, pointer: u8) -> u16 {
        let low = self.read(bus, pointer as u16);
        let high = self.read(bus, pointer.wrapping_add(1) as u16);
        u16::from_le_bytes([low, high])
    }

    /// Add an index register to a 16-bit base address.
    ///
    /// The CPU adds the low byte first and starts reading before it knows
    /// whether the high byte needs fixing. If the addition carried (a "page
    /// cross"), that first read hit the wrong address and one more cycle is
    /// needed. Reads skip that dummy cycle when there is no carry; writes and
    /// read-modify-writes always spend it.
    fn add_index(&mut self, bus: &mut impl Bus, base: u16, index: u8, access: Access) -> Operand {
        let addr = base.wrapping_add(index as u16);
        let page_crossed = (base ^ addr) & 0xFF00 != 0;
        if page_crossed || access != Access::Read {
            let uncorrected = (base & 0xFF00) | (addr & 0x00FF);
            self.dummy_read(bus, uncorrected);
        }
        Operand {
            addr,
            base_high: (base >> 8) as u8,
            page_crossed,
        }
    }
}
