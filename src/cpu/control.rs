//! Instructions that change the flow of control or use the stack:
//! branches, jumps, subroutines, `BRK`, `RTI`, interrupts and `JAM`.

use super::opcodes::{Mnemonic, Mode};
use super::registers::STACK_PAGE;
use super::{Bus, Cpu, IRQ_VECTOR, NMI_VECTOR, flags};

impl Cpu {
    /// Relative branches. 2 cycles if not taken, 3 if taken, 4 if taken and
    /// the destination is on a different page.
    pub(super) fn branch(&mut self, bus: &mut impl Bus, mnemonic: Mnemonic) {
        use Mnemonic::*;
        let regs = &self.regs;
        let taken = match mnemonic {
            BPL => !regs.flag(flags::NEGATIVE),
            BMI => regs.flag(flags::NEGATIVE),
            BVC => !regs.flag(flags::OVERFLOW),
            BVS => regs.flag(flags::OVERFLOW),
            BCC => !regs.flag(flags::CARRY),
            BCS => regs.flag(flags::CARRY),
            BNE => !regs.flag(flags::ZERO),
            BEQ => regs.flag(flags::ZERO),
            _ => unreachable!("{mnemonic:?} is not a branch"),
        };
        let offset = self.fetch(bus) as i8;
        if !taken {
            return;
        }
        // Taken: spend a cycle adding the offset to PC's low byte.
        self.dummy_read(bus, self.regs.pc);
        let target = self.regs.pc.wrapping_add(offset as u16);
        if (target ^ self.regs.pc) & 0xFF00 != 0 {
            // The high byte was wrong for that read; fixing it costs a cycle.
            self.dummy_read(bus, (self.regs.pc & 0xFF00) | (target & 0x00FF));
        }
        self.regs.pc = target;
    }

    pub(super) fn jmp(&mut self, bus: &mut impl Bus, mode: Mode) {
        let target = self.fetch_u16(bus);
        self.regs.pc = if mode == Mode::Indirect {
            // Hardware bug: the pointer's high byte is read without carrying
            // into the next page, so `JMP ($12FF)` reads $12FF and $1200.
            let low = self.read(bus, target);
            let high_addr = (target & 0xFF00) | (target.wrapping_add(1) & 0x00FF);
            let high = self.read(bus, high_addr);
            u16::from_le_bytes([low, high])
        } else {
            target
        };
    }

    pub(super) fn jsr(&mut self, bus: &mut impl Bus) {
        let low = self.fetch(bus);
        self.dummy_read(bus, STACK_PAGE | self.regs.sp as u16);
        // The pushed address is that of the *last byte* of the JSR instruction.
        self.push_pc(bus);
        let high = self.fetch(bus);
        self.regs.pc = u16::from_le_bytes([low, high]);
    }

    pub(super) fn rts(&mut self, bus: &mut impl Bus) {
        self.dummy_read(bus, self.regs.pc);
        self.dummy_read(bus, STACK_PAGE | self.regs.sp as u16);
        let low = self.pull(bus);
        let high = self.pull(bus);
        self.regs.pc = u16::from_le_bytes([low, high]);
        // RTS returns to the last byte of the JSR, so step past it.
        self.dummy_read(bus, self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
    }

    pub(super) fn rti(&mut self, bus: &mut impl Bus) {
        self.dummy_read(bus, self.regs.pc);
        self.dummy_read(bus, STACK_PAGE | self.regs.sp as u16);
        let pulled = self.pull(bus);
        self.regs.p = Self::status_from_stack(pulled);
        let low = self.pull(bus);
        let high = self.pull(bus);
        self.regs.pc = u16::from_le_bytes([low, high]);
    }

    pub(super) fn stack_op(&mut self, bus: &mut impl Bus, mnemonic: Mnemonic) {
        self.dummy_read(bus, self.regs.pc);
        match mnemonic {
            Mnemonic::PHA => self.push(bus, self.regs.a),
            Mnemonic::PHP => self.push(bus, self.regs.p | flags::BREAK | flags::UNUSED),
            Mnemonic::PLA | Mnemonic::PLP => {
                // Pulling costs an extra cycle while SP is incremented.
                self.dummy_read(bus, STACK_PAGE | self.regs.sp as u16);
                let value = self.pull(bus);
                if mnemonic == Mnemonic::PLA {
                    self.regs.a = value;
                    self.regs.set_zero_negative(value);
                } else {
                    self.regs.p = Self::status_from_stack(value);
                }
            }
            _ => unreachable!("{mnemonic:?} is not a stack instruction"),
        }
    }

    // ---- Interrupts -------------------------------------------------------

    /// `BRK`: a software interrupt. It is a 2-byte instruction; the byte after
    /// the opcode is skipped over (it is meant as a "reason" code).
    pub(super) fn brk(&mut self, bus: &mut impl Bus) {
        self.fetch(bus);
        self.push_pc(bus);
        self.push(bus, self.regs.p | flags::BREAK | flags::UNUSED);
        self.jump_through_vector(bus, IRQ_VECTOR);
    }

    /// Hardware interrupt entry (IRQ or NMI): 7 cycles, like `BRK` but with
    /// the B flag clear in the pushed status and no operand byte skipped.
    pub(super) fn enter_interrupt(&mut self, bus: &mut impl Bus, vector: u16) {
        self.dummy_read(bus, self.regs.pc);
        self.dummy_read(bus, self.regs.pc);
        self.push_pc(bus);
        self.push(bus, (self.regs.p | flags::UNUSED) & !flags::BREAK);
        self.jump_through_vector(bus, vector);
    }

    /// B and the unused bit are not real flags, so pulling P ignores B and
    /// always reads the unused bit as 1.
    fn status_from_stack(value: u8) -> u8 {
        (value & !flags::BREAK) | flags::UNUSED
    }

    fn push_pc(&mut self, bus: &mut impl Bus) {
        let [low, high] = self.regs.pc.to_le_bytes();
        self.push(bus, high);
        self.push(bus, low);
    }

    /// Load PC from `vector` and mask IRQs. An NMI arriving while a `BRK` or
    /// IRQ is being entered "hijacks" it: the pushes are already done, but the
    /// NMI vector is used instead.
    fn jump_through_vector(&mut self, bus: &mut impl Bus, vector: u16) {
        let vector = if vector != NMI_VECTOR && bus.take_nmi_edge() {
            NMI_VECTOR
        } else {
            vector
        };
        self.regs.set_flag(flags::IRQ_DISABLE, true);
        self.regs.pc = self.read_u16(bus, vector);
    }

    /// The `JAM` (a.k.a. `KIL`/`HLT`) opcodes lock the CPU up. This is the bus
    /// traffic the hardware shows while it does so, then it stays jammed.
    pub(super) fn jam(&mut self, bus: &mut impl Bus) {
        const JAM_BUS_PATTERN: [u16; 9] = [
            0xFFFF, 0xFFFE, 0xFFFE, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF,
        ];
        self.dummy_read(bus, self.regs.pc);
        for addr in JAM_BUS_PATTERN {
            self.dummy_read(bus, addr);
        }
        self.jammed = true;
    }
}
