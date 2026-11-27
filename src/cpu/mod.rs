//! The 6502 processor core (NMOS, including the undocumented opcodes).
//!
//! ## How cycle accuracy works
//!
//! [`Cpu::step`] runs one whole instruction. Inside, the code performs the
//! same bus accesses, in the same order, as the real chip, *including the
//! "dummy" reads and writes* that the hardware makes while it is busy (for
//! example the extra read when an indexed address crosses a page). Every
//! access goes through [`Bus`], and each access is one clock cycle.
//!
//! The module layout:
//!
//! * [`bus`]: the `Bus` trait the CPU uses to reach memory and devices
//! * [`registers`]: A, X, Y, SP, PC and the status flags
//! * [`opcodes`]: the 256-entry table of instruction and addressing mode
//! * `addressing`: works out the operand address, cycle by cycle
//! * `instructions`: what each instruction does (ALU, loads, shifts, ...)
//! * `control`: branches, jumps, stack operations and interrupts

use serde::{Deserialize, Serialize};

mod addressing;
pub mod bus;
mod control;
pub mod disasm;
mod instructions;
pub mod opcodes;
pub mod registers;

pub use bus::{Bus, RamBus};
pub use registers::{Registers, flags};

use opcodes::{Mnemonic, Mode, OPCODES};

/// Where the CPU finds the address to jump to for each kind of interrupt.
pub const NMI_VECTOR: u16 = 0xFFFA;
pub const RESET_VECTOR: u16 = 0xFFFC;
pub const IRQ_VECTOR: u16 = 0xFFFE;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cpu {
    pub regs: Registers,
    /// Total clock cycles executed since this `Cpu` was created.
    pub cycles: u64,
    /// The I flag as the interrupt logic sees it when it next polls. This
    /// differs from the real flag right after `CLI`, `SEI` and `PLP`: the 6502
    /// polls for interrupts before those instructions' last cycle, so they
    /// take effect one instruction late.
    irq_disable_at_poll: bool,
    /// Set by the `JAM` opcodes, which lock up the processor until reset.
    jammed: bool,
    /// Was the last [`Cpu::step`] an interrupt being taken, rather than an
    /// instruction? (For the debugger; not part of the machine's state.)
    #[serde(skip)]
    interrupt_taken: bool,
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            regs: Registers {
                sp: 0xFD,
                p: flags::IRQ_DISABLE | flags::UNUSED,
                ..Registers::default()
            },
            cycles: 0,
            irq_disable_at_poll: true,
            jammed: false,
            interrupt_taken: false,
        }
    }

    /// The 7-cycle reset sequence: like an interrupt, but nothing is written
    /// (the stack pointer drops by 3 and the pushes are done as reads).
    pub fn reset(&mut self, bus: &mut impl Bus) {
        self.jammed = false;
        self.read(bus, self.regs.pc);
        self.read(bus, self.regs.pc);
        for _ in 0..3 {
            self.read(bus, registers::STACK_PAGE | self.regs.sp as u16);
            self.regs.sp = self.regs.sp.wrapping_sub(1);
        }
        self.regs.set_flag(flags::IRQ_DISABLE, true);
        self.irq_disable_at_poll = true;
        self.regs.pc = self.read_u16(bus, RESET_VECTOR);
    }

    /// Run one instruction, or one interrupt-entry sequence if an interrupt
    /// is pending. Returns how many cycles it took.
    pub fn step(&mut self, bus: &mut impl Bus) -> u64 {
        let start = self.cycles;
        self.interrupt_taken = false;
        if self.jammed {
            self.read(bus, 0xFFFF);
        } else if bus.take_nmi_edge() {
            self.interrupt_taken = true;
            self.enter_interrupt(bus, NMI_VECTOR);
            self.irq_disable_at_poll = true;
        } else if bus.irq_asserted() && !self.irq_disable_at_poll {
            self.interrupt_taken = true;
            self.enter_interrupt(bus, IRQ_VECTOR);
            self.irq_disable_at_poll = true; // entering an interrupt sets the I flag
        } else {
            self.execute_next_instruction(bus);
        }
        self.cycles - start
    }

    /// Was the last [`Cpu::step`] an interrupt being taken (so no instruction
    /// ran)?
    pub fn took_interrupt(&self) -> bool {
        self.interrupt_taken
    }

    fn execute_next_instruction(&mut self, bus: &mut impl Bus) {
        let opcode = self.fetch(bus);
        let (mnemonic, mode) = OPCODES[opcode as usize];
        let irq_disable_before = self.regs.flag(flags::IRQ_DISABLE);

        self.execute(bus, mnemonic, mode);

        let changes_i_late = matches!(mnemonic, Mnemonic::CLI | Mnemonic::SEI | Mnemonic::PLP);
        self.irq_disable_at_poll = if changes_i_late {
            irq_disable_before
        } else {
            self.regs.flag(flags::IRQ_DISABLE)
        };
    }

    fn execute(&mut self, bus: &mut impl Bus, mnemonic: Mnemonic, mode: Mode) {
        use Mnemonic::*;
        match mnemonic {
            BPL | BMI | BVC | BVS | BCC | BCS | BNE | BEQ => self.branch(bus, mnemonic),
            BRK => self.brk(bus),
            JSR => self.jsr(bus),
            RTS => self.rts(bus),
            RTI => self.rti(bus),
            JMP => self.jmp(bus, mode),
            PHA | PHP | PLA | PLP => self.stack_op(bus, mnemonic),
            JAM => self.jam(bus),
            _ => match mode {
                Mode::Implied | Mode::Accumulator => self.register_op(bus, mnemonic),
                _ => self.memory_op(bus, mnemonic, mode),
            },
        }
    }

    // ---- Bus helpers: each one is exactly one clock cycle -----------------

    fn read(&mut self, bus: &mut impl Bus, addr: u16) -> u8 {
        self.cycles += 1;
        bus.read(addr)
    }

    fn write(&mut self, bus: &mut impl Bus, addr: u16, value: u8) {
        self.cycles += 1;
        bus.write(addr, value);
    }

    /// Read the byte at PC and advance PC (opcodes and operand bytes).
    fn fetch(&mut self, bus: &mut impl Bus) -> u8 {
        let value = self.read(bus, self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        value
    }

    /// Two fetches: little-endian 16-bit operand.
    fn fetch_u16(&mut self, bus: &mut impl Bus) -> u16 {
        let low = self.fetch(bus);
        let high = self.fetch(bus);
        u16::from_le_bytes([low, high])
    }

    fn read_u16(&mut self, bus: &mut impl Bus, addr: u16) -> u16 {
        let low = self.read(bus, addr);
        let high = self.read(bus, addr.wrapping_add(1));
        u16::from_le_bytes([low, high])
    }

    fn push(&mut self, bus: &mut impl Bus, value: u8) {
        self.write(bus, registers::STACK_PAGE | self.regs.sp as u16, value);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
    }

    fn pull(&mut self, bus: &mut impl Bus) -> u8 {
        self.regs.sp = self.regs.sp.wrapping_add(1);
        self.read(bus, registers::STACK_PAGE | self.regs.sp as u16)
    }

    /// A read whose result is thrown away. The CPU does these while it is
    /// busy internally, and they matter because they can touch hardware.
    fn dummy_read(&mut self, bus: &mut impl Bus, addr: u16) {
        self.read(bus, addr);
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}
