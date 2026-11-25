//! The programmer-visible registers of the 6502.

use serde::{Deserialize, Serialize};

/// Bit masks for the processor status register (`P`).
///
/// ```text
///   bit:  7 6 5 4 3 2 1 0
///         N V - B D I Z C
/// ```
///
/// Bits 4 (`B`) and 5 (unused) do not really exist inside the chip. They only
/// show up in copies of `P` pushed to the stack, which is why they are kept
/// here as plain stored bits.
pub mod flags {
    pub const CARRY: u8 = 0x01;
    pub const ZERO: u8 = 0x02;
    pub const IRQ_DISABLE: u8 = 0x04;
    pub const DECIMAL: u8 = 0x08;
    pub const BREAK: u8 = 0x10;
    pub const UNUSED: u8 = 0x20;
    pub const OVERFLOW: u8 = 0x40;
    pub const NEGATIVE: u8 = 0x80;
}

/// Where the stack lives: page one, `$0100-$01FF`.
pub const STACK_PAGE: u16 = 0x0100;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registers {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    /// Stack pointer: the low byte of the next free stack address.
    pub sp: u8,
    pub pc: u16,
    /// Processor status; see [`flags`].
    pub p: u8,
}

impl Registers {
    pub fn flag(&self, mask: u8) -> bool {
        self.p & mask != 0
    }

    pub fn set_flag(&mut self, mask: u8, on: bool) {
        if on {
            self.p |= mask;
        } else {
            self.p &= !mask;
        }
    }

    /// Set Zero and Negative from a result byte (done by most instructions).
    pub fn set_zero_negative(&mut self, value: u8) {
        self.set_flag(flags::ZERO, value == 0);
        self.set_flag(flags::NEGATIVE, value & 0x80 != 0);
    }
}
