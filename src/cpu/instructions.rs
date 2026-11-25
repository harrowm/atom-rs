//! What the instructions actually do to registers, flags and memory.

use super::addressing::Operand;
use super::opcodes::{Access, Mnemonic, Mode};
use super::{Bus, Cpu, flags};

/// Undocumented `XAA` and `LXA` OR this "magic" constant into A. The real
/// value varies between chips and temperature; 0xEE is the one the
/// SingleStepTests data was generated with.
const MAGIC_CONSTANT: u8 = 0xEE;

impl Cpu {
    // ---- Instructions without a memory operand ----------------------------

    /// Implied and accumulator instructions: 2 cycles, the second being a
    /// throwaway read of the byte after the opcode.
    pub(super) fn register_op(&mut self, bus: &mut impl Bus, mnemonic: Mnemonic) {
        use Mnemonic::*;
        self.dummy_read(bus, self.regs.pc);
        let r = &mut self.regs;
        match mnemonic {
            TAX => {
                r.x = r.a;
                r.set_zero_negative(r.x);
            }
            TAY => {
                r.y = r.a;
                r.set_zero_negative(r.y);
            }
            TXA => {
                r.a = r.x;
                r.set_zero_negative(r.a);
            }
            TYA => {
                r.a = r.y;
                r.set_zero_negative(r.a);
            }
            TSX => {
                r.x = r.sp;
                r.set_zero_negative(r.x);
            }
            TXS => r.sp = r.x, // the one transfer that sets no flags
            INX => {
                r.x = r.x.wrapping_add(1);
                r.set_zero_negative(r.x);
            }
            INY => {
                r.y = r.y.wrapping_add(1);
                r.set_zero_negative(r.y);
            }
            DEX => {
                r.x = r.x.wrapping_sub(1);
                r.set_zero_negative(r.x);
            }
            DEY => {
                r.y = r.y.wrapping_sub(1);
                r.set_zero_negative(r.y);
            }
            CLC => r.set_flag(flags::CARRY, false),
            SEC => r.set_flag(flags::CARRY, true),
            CLI => r.set_flag(flags::IRQ_DISABLE, false),
            SEI => r.set_flag(flags::IRQ_DISABLE, true),
            CLD => r.set_flag(flags::DECIMAL, false),
            SED => r.set_flag(flags::DECIMAL, true),
            CLV => r.set_flag(flags::OVERFLOW, false),
            ASL | LSR | ROL | ROR => {
                let a = self.regs.a;
                self.regs.a = self.shift(mnemonic, a);
            }
            NOP => {}
            _ => unreachable!("{mnemonic:?} is not an implied instruction"),
        }
    }

    // ---- Instructions with a memory operand -------------------------------

    pub(super) fn memory_op(&mut self, bus: &mut impl Bus, mnemonic: Mnemonic, mode: Mode) {
        let access = mnemonic.access();
        let operand = self.operand(bus, mode, access);
        match access {
            Access::Read => {
                let value = self.read(bus, operand.addr);
                self.apply_read(mnemonic, value);
            }
            Access::Write => self.store(bus, mnemonic, operand),
            Access::ReadModifyWrite => {
                let old = self.read(bus, operand.addr);
                // The 6502 writes the unmodified value back first, then the
                // result. Hardware registers can see both writes.
                self.write(bus, operand.addr, old);
                let new = self.modify(mnemonic, old);
                self.write(bus, operand.addr, new);
            }
        }
    }

    /// Instructions that consume a byte read from memory.
    fn apply_read(&mut self, mnemonic: Mnemonic, value: u8) {
        use Mnemonic::*;
        match mnemonic {
            LDA => self.load_a(value),
            LDX => {
                self.regs.x = value;
                self.regs.set_zero_negative(value);
            }
            LDY => {
                self.regs.y = value;
                self.regs.set_zero_negative(value);
            }
            LAX => {
                self.regs.x = value;
                self.load_a(value);
            }
            ADC => self.add_with_carry(value),
            SBC => self.subtract_with_carry(value),
            AND => self.load_a(self.regs.a & value),
            ORA => self.load_a(self.regs.a | value),
            EOR => self.load_a(self.regs.a ^ value),
            CMP => self.compare(self.regs.a, value),
            CPX => self.compare(self.regs.x, value),
            CPY => self.compare(self.regs.y, value),
            BIT => {
                self.regs.set_flag(flags::ZERO, self.regs.a & value == 0);
                self.regs.set_flag(flags::NEGATIVE, value & 0x80 != 0);
                self.regs.set_flag(flags::OVERFLOW, value & 0x40 != 0);
            }
            NOP => {} // undocumented NOPs with operands still do the read
            // ---- undocumented ----
            ANC => {
                self.load_a(self.regs.a & value);
                let negative = self.regs.flag(flags::NEGATIVE);
                self.regs.set_flag(flags::CARRY, negative);
            }
            ALR => {
                let anded = self.regs.a & value;
                let shifted = self.shift(LSR, anded);
                self.regs.a = shifted;
            }
            ARR => self.and_then_rotate_right(value),
            XAA => self.load_a((self.regs.a | MAGIC_CONSTANT) & self.regs.x & value),
            LXA => {
                let result = (self.regs.a | MAGIC_CONSTANT) & value;
                self.regs.x = result;
                self.load_a(result);
            }
            SBX => {
                // X = (A & X) - operand, with CMP-style flags and no borrow in.
                let anded = self.regs.a & self.regs.x;
                self.compare(anded, value);
                self.regs.x = anded.wrapping_sub(value);
            }
            LAS => {
                let result = value & self.regs.sp;
                self.regs.x = result;
                self.regs.sp = result;
                self.load_a(result);
            }
            _ => unreachable!("{mnemonic:?} is not a read instruction"),
        }
    }

    /// Stores. A few undocumented ones AND the value with "high byte + 1".
    fn store(&mut self, bus: &mut impl Bus, mnemonic: Mnemonic, operand: Operand) {
        use Mnemonic::*;
        let r = &self.regs;
        let high_plus_one = operand.base_high.wrapping_add(1);
        let value = match mnemonic {
            STA => r.a,
            STX => r.x,
            STY => r.y,
            SAX => r.a & r.x,
            AHX => r.a & r.x & high_plus_one,
            SHX => r.x & high_plus_one,
            SHY => r.y & high_plus_one,
            TAS => {
                self.regs.sp = r.a & r.x;
                self.regs.sp & high_plus_one
            }
            _ => unreachable!("{mnemonic:?} is not a store"),
        };
        let mut addr = operand.addr;
        let unstable = matches!(mnemonic, AHX | SHX | SHY | TAS);
        if unstable && operand.page_crossed {
            // The corrupted high byte of the address is the stored value.
            addr = ((value as u16) << 8) | (addr & 0x00FF);
        }
        self.write(bus, addr, value);
    }

    /// The "modify" step of read-modify-write instructions. The undocumented
    /// ones are a shift/inc/dec followed by an ALU operation on A.
    fn modify(&mut self, mnemonic: Mnemonic, value: u8) -> u8 {
        use Mnemonic::*;
        match mnemonic {
            ASL | LSR | ROL | ROR => self.shift(mnemonic, value),
            INC => self.increment(value),
            DEC => self.decrement(value),
            SLO => {
                let r = self.shift(ASL, value);
                self.load_a(self.regs.a | r);
                r
            }
            RLA => {
                let r = self.shift(ROL, value);
                self.load_a(self.regs.a & r);
                r
            }
            SRE => {
                let r = self.shift(LSR, value);
                self.load_a(self.regs.a ^ r);
                r
            }
            RRA => {
                let r = self.shift(ROR, value);
                self.add_with_carry(r);
                r
            }
            DCP => {
                let r = value.wrapping_sub(1);
                self.compare(self.regs.a, r);
                r
            }
            ISC => {
                let r = value.wrapping_add(1);
                self.subtract_with_carry(r);
                r
            }
            _ => unreachable!("{mnemonic:?} is not read-modify-write"),
        }
    }

    // ---- Building blocks ---------------------------------------------------

    fn load_a(&mut self, value: u8) {
        self.regs.a = value;
        self.regs.set_zero_negative(value);
    }

    fn increment(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        self.regs.set_zero_negative(result);
        result
    }

    fn decrement(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        self.regs.set_zero_negative(result);
        result
    }

    /// ASL, LSR, ROL, ROR: the bit shifted out goes to Carry.
    fn shift(&mut self, mnemonic: Mnemonic, value: u8) -> u8 {
        let carry_in = self.regs.flag(flags::CARRY) as u8;
        let (result, carry_out) = match mnemonic {
            Mnemonic::ASL => (value << 1, value & 0x80),
            Mnemonic::LSR => (value >> 1, value & 0x01),
            Mnemonic::ROL => ((value << 1) | carry_in, value & 0x80),
            Mnemonic::ROR => ((value >> 1) | (carry_in << 7), value & 0x01),
            _ => unreachable!("{mnemonic:?} is not a shift"),
        };
        self.regs.set_flag(flags::CARRY, carry_out != 0);
        self.regs.set_zero_negative(result);
        result
    }

    /// CMP/CPX/CPY: a subtraction that only sets flags.
    fn compare(&mut self, register: u8, value: u8) {
        self.regs.set_flag(flags::CARRY, register >= value);
        self.regs.set_zero_negative(register.wrapping_sub(value));
    }

    /// ADC. In decimal mode (NMOS behaviour) the result is BCD-corrected;
    /// N and V are taken part-way through the correction, and Z comes from
    /// the plain binary sum.
    fn add_with_carry(&mut self, value: u8) {
        if self.regs.flag(flags::DECIMAL) {
            self.add_decimal(value);
        } else {
            self.add_binary(value);
        }
    }

    /// Binary add; sets C, V, N, Z and returns the result.
    fn add_binary(&mut self, value: u8) -> u8 {
        let a = self.regs.a;
        let carry_in = self.regs.flag(flags::CARRY) as u16;
        let sum = a as u16 + value as u16 + carry_in;
        let result = sum as u8;
        // Overflow: both inputs had the same sign but the result differs.
        let overflow = !(a ^ value) & (a ^ result) & 0x80 != 0;
        self.regs.set_flag(flags::CARRY, sum > 0xFF);
        self.regs.set_flag(flags::OVERFLOW, overflow);
        self.regs.set_zero_negative(result);
        self.regs.a = result;
        result
    }

    fn add_decimal(&mut self, value: u8) {
        let a = self.regs.a;
        let carry_in = self.regs.flag(flags::CARRY) as u8;
        let binary_result = a.wrapping_add(value).wrapping_add(carry_in);

        // Low digit, corrected if it went past 9.
        let mut low = (a & 0x0F) + (value & 0x0F) + carry_in;
        if low >= 0x0A {
            low = ((low + 0x06) & 0x0F) + 0x10;
        }
        // High digit, added before it is corrected.
        let mut sum = (a & 0xF0) as u16 + (value & 0xF0) as u16 + low as u16;

        self.regs.set_flag(flags::ZERO, binary_result == 0);
        self.regs.set_flag(flags::NEGATIVE, sum & 0x80 != 0);
        self.regs
            .set_flag(flags::OVERFLOW, !(a ^ value) & (a ^ sum as u8) & 0x80 != 0);
        if sum >= 0xA0 {
            sum += 0x60;
        }
        self.regs.set_flag(flags::CARRY, sum >= 0x100);
        self.regs.a = sum as u8;
    }

    /// SBC. Flags always come from the binary subtraction (A + !value + C);
    /// in decimal mode only the result in A is BCD-corrected.
    fn subtract_with_carry(&mut self, value: u8) {
        let a = self.regs.a;
        let borrow_in = !self.regs.flag(flags::CARRY) as i16;
        let binary_result = self.add_binary(!value);
        if self.regs.flag(flags::DECIMAL) {
            let mut low = (a & 0x0F) as i16 - (value & 0x0F) as i16 - borrow_in;
            if low < 0 {
                low = ((low - 6) & 0x0F) - 0x10;
            }
            let mut diff = (a & 0xF0) as i16 - (value & 0xF0) as i16 + low;
            if diff < 0 {
                diff -= 0x60;
            }
            self.regs.a = diff as u8;
        } else {
            self.regs.a = binary_result;
        }
    }

    /// Undocumented ARR: AND with the operand, then rotate A right. The flags
    /// are unusual, and decimal mode adds its own BCD fix-up.
    fn and_then_rotate_right(&mut self, value: u8) {
        let anded = self.regs.a & value;
        let carry_in = self.regs.flag(flags::CARRY);
        let mut result = (anded >> 1) | ((carry_in as u8) << 7);

        if !self.regs.flag(flags::DECIMAL) {
            self.regs.set_zero_negative(result);
            self.regs.set_flag(flags::CARRY, result & 0x40 != 0);
            self.regs
                .set_flag(flags::OVERFLOW, (result ^ (result << 1)) & 0x40 != 0);
        } else {
            self.regs.set_flag(flags::NEGATIVE, carry_in);
            self.regs.set_flag(flags::ZERO, result == 0);
            self.regs
                .set_flag(flags::OVERFLOW, (anded ^ result) & 0x40 != 0);
            if (anded & 0x0F) + (anded & 0x01) > 5 {
                result = (result & 0xF0) | (result.wrapping_add(0x06) & 0x0F);
            }
            let high_fixup = (anded & 0xF0) as u16 + (anded & 0x10) as u16 > 0x50;
            if high_fixup {
                result = result.wrapping_add(0x60);
            }
            self.regs.set_flag(flags::CARRY, high_fixup);
        }
        self.regs.a = result;
    }
}
