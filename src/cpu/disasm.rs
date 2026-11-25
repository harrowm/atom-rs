//! A disassembler, used by the debugger.

use super::opcodes::{Mode, OPCODES};

/// One disassembled instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    /// e.g. `LDA ($20),Y`
    pub text: String,
    /// How many bytes the instruction occupies (1-3).
    pub length: u16,
}

/// Number of bytes an instruction using `mode` occupies, opcode included.
pub fn instruction_length(mode: Mode) -> u16 {
    match mode {
        Mode::Implied | Mode::Accumulator => 1,
        Mode::Immediate
        | Mode::ZeroPage
        | Mode::ZeroPageX
        | Mode::ZeroPageY
        | Mode::IndirectX
        | Mode::IndirectY
        | Mode::Relative => 2,
        Mode::Absolute | Mode::AbsoluteX | Mode::AbsoluteY | Mode::Indirect => 3,
    }
}

/// Disassemble the instruction at `addr`, fetching bytes with `read`.
pub fn disassemble(addr: u16, read: impl Fn(u16) -> u8) -> Instruction {
    let (mnemonic, mode) = OPCODES[read(addr) as usize];
    let byte = read(addr.wrapping_add(1));
    let word = u16::from_le_bytes([byte, read(addr.wrapping_add(2))]);

    let operand = match mode {
        Mode::Implied => String::new(),
        Mode::Accumulator => " A".to_string(),
        Mode::Immediate => format!(" #${byte:02X}"),
        Mode::ZeroPage => format!(" ${byte:02X}"),
        Mode::ZeroPageX => format!(" ${byte:02X},X"),
        Mode::ZeroPageY => format!(" ${byte:02X},Y"),
        Mode::Absolute => format!(" ${word:04X}"),
        Mode::AbsoluteX => format!(" ${word:04X},X"),
        Mode::AbsoluteY => format!(" ${word:04X},Y"),
        Mode::Indirect => format!(" (${word:04X})"),
        Mode::IndirectX => format!(" (${byte:02X},X)"),
        Mode::IndirectY => format!(" (${byte:02X}),Y"),
        Mode::Relative => {
            // Show the destination rather than the raw offset.
            let target = addr.wrapping_add(2).wrapping_add(byte as i8 as u16);
            format!(" ${target:04X}")
        }
    };
    Instruction {
        text: format!("{mnemonic:?}{operand}"),
        length: instruction_length(mode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disassemble_bytes(addr: u16, bytes: &[u8]) -> Instruction {
        disassemble(addr, |a| {
            bytes
                .get(a.wrapping_sub(addr) as usize)
                .copied()
                .unwrap_or(0)
        })
    }

    #[test]
    fn formats_each_addressing_mode() {
        let cases: [(&[u8], &str, u16); 10] = [
            (&[0xEA], "NOP", 1),
            (&[0x0A], "ASL A", 1),
            (&[0xA9, 0x12], "LDA #$12", 2),
            (&[0xA5, 0x20], "LDA $20", 2),
            (&[0xB5, 0x20], "LDA $20,X", 2),
            (&[0x8D, 0x34, 0x12], "STA $1234", 3),
            (&[0xB9, 0x00, 0xB0], "LDA $B000,Y", 3),
            (&[0x6C, 0xFC, 0xFF], "JMP ($FFFC)", 3),
            (&[0xA1, 0x20], "LDA ($20,X)", 2),
            (&[0xB1, 0x20], "LDA ($20),Y", 2),
        ];
        for (bytes, text, length) in cases {
            let instruction = disassemble_bytes(0x1000, bytes);
            assert_eq!(
                (instruction.text.as_str(), instruction.length),
                (text, length)
            );
        }
    }

    #[test]
    fn branches_show_their_destination() {
        // BNE back 4 bytes from $1000: target = $1002 - 4 = $0FFE.
        assert_eq!(disassemble_bytes(0x1000, &[0xD0, 0xFC]).text, "BNE $0FFE");
        assert_eq!(disassemble_bytes(0x1000, &[0xF0, 0x10]).text, "BEQ $1012");
    }
}
