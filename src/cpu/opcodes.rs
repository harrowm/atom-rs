//! The opcode table: for each of the 256 opcode bytes, what the instruction is
//! and how it finds its operand. Cycle counts are not stored here; they come
//! out of the bus accesses each instruction makes (see `addressing.rs` and
//! `instructions.rs`).

/// How an instruction finds its operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// No operand (`INX`).
    Implied,
    /// Operates on A (`ASL A`).
    Accumulator,
    /// Operand is the byte after the opcode (`LDA #$12`).
    Immediate,
    ZeroPage,
    /// Zero page plus X, wrapping inside page zero (`LDA $12,X`).
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    /// Only used by `JMP ($1234)`.
    Indirect,
    /// `LDA ($12,X)`: pointer in zero page at (operand + X).
    IndirectX,
    /// `LDA ($12),Y`: pointer in zero page at operand, then add Y.
    IndirectY,
    /// Branches: signed 8-bit offset from the next instruction.
    Relative,
}

/// Instruction names, including the undocumented NMOS ones (the last group).
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mnemonic {
    // Loads, stores and register transfers
    LDA,
    LDX,
    LDY,
    STA,
    STX,
    STY,
    TAX,
    TAY,
    TXA,
    TYA,
    TSX,
    TXS,
    // Stack
    PHA,
    PHP,
    PLA,
    PLP,
    // Arithmetic and logic
    ADC,
    SBC,
    AND,
    ORA,
    EOR,
    CMP,
    CPX,
    CPY,
    BIT,
    // Increments, decrements and shifts
    INC,
    INX,
    INY,
    DEC,
    DEX,
    DEY,
    ASL,
    LSR,
    ROL,
    ROR,
    // Jumps and subroutines
    JMP,
    JSR,
    RTS,
    RTI,
    BRK,
    // Branches
    BCC,
    BCS,
    BEQ,
    BNE,
    BMI,
    BPL,
    BVC,
    BVS,
    // Flags
    CLC,
    CLD,
    CLI,
    CLV,
    SEC,
    SED,
    SEI,
    NOP,
    // Undocumented
    SLO,
    RLA,
    SRE,
    RRA,
    SAX,
    LAX,
    DCP,
    ISC,
    ANC,
    ALR,
    ARR,
    XAA,
    LXA,
    SBX,
    AHX,
    TAS,
    SHX,
    SHY,
    LAS,
    JAM,
}

use Mnemonic::*;
use Mode::*;

const fn op(mnemonic: Mnemonic, mode: Mode) -> (Mnemonic, Mode) {
    (mnemonic, mode)
}

/// Indexed by opcode byte. Read it as 16 rows of 16: the row is the high
/// nibble of the opcode and the column is the low nibble.
#[rustfmt::skip]
pub const OPCODES: [(Mnemonic, Mode); 256] = [
    // 0x00
    op(BRK, Implied), op(ORA, IndirectX), op(JAM, Implied), op(SLO, IndirectX),
    op(NOP, ZeroPage), op(ORA, ZeroPage), op(ASL, ZeroPage), op(SLO, ZeroPage),
    op(PHP, Implied), op(ORA, Immediate), op(ASL, Accumulator), op(ANC, Immediate),
    op(NOP, Absolute), op(ORA, Absolute), op(ASL, Absolute), op(SLO, Absolute),
    // 0x10
    op(BPL, Relative), op(ORA, IndirectY), op(JAM, Implied), op(SLO, IndirectY),
    op(NOP, ZeroPageX), op(ORA, ZeroPageX), op(ASL, ZeroPageX), op(SLO, ZeroPageX),
    op(CLC, Implied), op(ORA, AbsoluteY), op(NOP, Implied), op(SLO, AbsoluteY),
    op(NOP, AbsoluteX), op(ORA, AbsoluteX), op(ASL, AbsoluteX), op(SLO, AbsoluteX),
    // 0x20
    op(JSR, Absolute), op(AND, IndirectX), op(JAM, Implied), op(RLA, IndirectX),
    op(BIT, ZeroPage), op(AND, ZeroPage), op(ROL, ZeroPage), op(RLA, ZeroPage),
    op(PLP, Implied), op(AND, Immediate), op(ROL, Accumulator), op(ANC, Immediate),
    op(BIT, Absolute), op(AND, Absolute), op(ROL, Absolute), op(RLA, Absolute),
    // 0x30
    op(BMI, Relative), op(AND, IndirectY), op(JAM, Implied), op(RLA, IndirectY),
    op(NOP, ZeroPageX), op(AND, ZeroPageX), op(ROL, ZeroPageX), op(RLA, ZeroPageX),
    op(SEC, Implied), op(AND, AbsoluteY), op(NOP, Implied), op(RLA, AbsoluteY),
    op(NOP, AbsoluteX), op(AND, AbsoluteX), op(ROL, AbsoluteX), op(RLA, AbsoluteX),
    // 0x40
    op(RTI, Implied), op(EOR, IndirectX), op(JAM, Implied), op(SRE, IndirectX),
    op(NOP, ZeroPage), op(EOR, ZeroPage), op(LSR, ZeroPage), op(SRE, ZeroPage),
    op(PHA, Implied), op(EOR, Immediate), op(LSR, Accumulator), op(ALR, Immediate),
    op(JMP, Absolute), op(EOR, Absolute), op(LSR, Absolute), op(SRE, Absolute),
    // 0x50
    op(BVC, Relative), op(EOR, IndirectY), op(JAM, Implied), op(SRE, IndirectY),
    op(NOP, ZeroPageX), op(EOR, ZeroPageX), op(LSR, ZeroPageX), op(SRE, ZeroPageX),
    op(CLI, Implied), op(EOR, AbsoluteY), op(NOP, Implied), op(SRE, AbsoluteY),
    op(NOP, AbsoluteX), op(EOR, AbsoluteX), op(LSR, AbsoluteX), op(SRE, AbsoluteX),
    // 0x60
    op(RTS, Implied), op(ADC, IndirectX), op(JAM, Implied), op(RRA, IndirectX),
    op(NOP, ZeroPage), op(ADC, ZeroPage), op(ROR, ZeroPage), op(RRA, ZeroPage),
    op(PLA, Implied), op(ADC, Immediate), op(ROR, Accumulator), op(ARR, Immediate),
    op(JMP, Indirect), op(ADC, Absolute), op(ROR, Absolute), op(RRA, Absolute),
    // 0x70
    op(BVS, Relative), op(ADC, IndirectY), op(JAM, Implied), op(RRA, IndirectY),
    op(NOP, ZeroPageX), op(ADC, ZeroPageX), op(ROR, ZeroPageX), op(RRA, ZeroPageX),
    op(SEI, Implied), op(ADC, AbsoluteY), op(NOP, Implied), op(RRA, AbsoluteY),
    op(NOP, AbsoluteX), op(ADC, AbsoluteX), op(ROR, AbsoluteX), op(RRA, AbsoluteX),
    // 0x80
    op(NOP, Immediate), op(STA, IndirectX), op(NOP, Immediate), op(SAX, IndirectX),
    op(STY, ZeroPage), op(STA, ZeroPage), op(STX, ZeroPage), op(SAX, ZeroPage),
    op(DEY, Implied), op(NOP, Immediate), op(TXA, Implied), op(XAA, Immediate),
    op(STY, Absolute), op(STA, Absolute), op(STX, Absolute), op(SAX, Absolute),
    // 0x90
    op(BCC, Relative), op(STA, IndirectY), op(JAM, Implied), op(AHX, IndirectY),
    op(STY, ZeroPageX), op(STA, ZeroPageX), op(STX, ZeroPageY), op(SAX, ZeroPageY),
    op(TYA, Implied), op(STA, AbsoluteY), op(TXS, Implied), op(TAS, AbsoluteY),
    op(SHY, AbsoluteX), op(STA, AbsoluteX), op(SHX, AbsoluteY), op(AHX, AbsoluteY),
    // 0xA0
    op(LDY, Immediate), op(LDA, IndirectX), op(LDX, Immediate), op(LAX, IndirectX),
    op(LDY, ZeroPage), op(LDA, ZeroPage), op(LDX, ZeroPage), op(LAX, ZeroPage),
    op(TAY, Implied), op(LDA, Immediate), op(TAX, Implied), op(LXA, Immediate),
    op(LDY, Absolute), op(LDA, Absolute), op(LDX, Absolute), op(LAX, Absolute),
    // 0xB0
    op(BCS, Relative), op(LDA, IndirectY), op(JAM, Implied), op(LAX, IndirectY),
    op(LDY, ZeroPageX), op(LDA, ZeroPageX), op(LDX, ZeroPageY), op(LAX, ZeroPageY),
    op(CLV, Implied), op(LDA, AbsoluteY), op(TSX, Implied), op(LAS, AbsoluteY),
    op(LDY, AbsoluteX), op(LDA, AbsoluteX), op(LDX, AbsoluteY), op(LAX, AbsoluteY),
    // 0xC0
    op(CPY, Immediate), op(CMP, IndirectX), op(NOP, Immediate), op(DCP, IndirectX),
    op(CPY, ZeroPage), op(CMP, ZeroPage), op(DEC, ZeroPage), op(DCP, ZeroPage),
    op(INY, Implied), op(CMP, Immediate), op(DEX, Implied), op(SBX, Immediate),
    op(CPY, Absolute), op(CMP, Absolute), op(DEC, Absolute), op(DCP, Absolute),
    // 0xD0
    op(BNE, Relative), op(CMP, IndirectY), op(JAM, Implied), op(DCP, IndirectY),
    op(NOP, ZeroPageX), op(CMP, ZeroPageX), op(DEC, ZeroPageX), op(DCP, ZeroPageX),
    op(CLD, Implied), op(CMP, AbsoluteY), op(NOP, Implied), op(DCP, AbsoluteY),
    op(NOP, AbsoluteX), op(CMP, AbsoluteX), op(DEC, AbsoluteX), op(DCP, AbsoluteX),
    // 0xE0
    op(CPX, Immediate), op(SBC, IndirectX), op(NOP, Immediate), op(ISC, IndirectX),
    op(CPX, ZeroPage), op(SBC, ZeroPage), op(INC, ZeroPage), op(ISC, ZeroPage),
    op(INX, Implied), op(SBC, Immediate), op(NOP, Implied), op(SBC, Immediate),
    op(CPX, Absolute), op(SBC, Absolute), op(INC, Absolute), op(ISC, Absolute),
    // 0xF0
    op(BEQ, Relative), op(SBC, IndirectY), op(JAM, Implied), op(ISC, IndirectY),
    op(NOP, ZeroPageX), op(SBC, ZeroPageX), op(INC, ZeroPageX), op(ISC, ZeroPageX),
    op(SED, Implied), op(SBC, AbsoluteY), op(NOP, Implied), op(ISC, AbsoluteY),
    op(NOP, AbsoluteX), op(SBC, AbsoluteX), op(INC, AbsoluteX), op(ISC, AbsoluteX),
];

/// What an instruction does with the memory location it addresses. This
/// decides the exact sequence of bus cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Reads the operand (`LDA`, `ADC`, ...).
    Read,
    /// Only writes it (`STA`, ...).
    Write,
    /// Reads, then writes a modified value back (`INC`, `ASL`, ...).
    ReadModifyWrite,
}

impl Mnemonic {
    /// Only meaningful for instructions that use a memory operand.
    pub fn access(self) -> Access {
        match self {
            STA | STX | STY | SAX | AHX | TAS | SHX | SHY => Access::Write,
            INC | DEC | ASL | LSR | ROL | ROR | SLO | RLA | SRE | RRA | DCP | ISC => {
                Access::ReadModifyWrite
            }
            _ => Access::Read,
        }
    }
}
