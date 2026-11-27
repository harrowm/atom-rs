//! Conditions for conditional breakpoints: `a==5`, `x>=10 && [0200]!=0`.
//!
//! A condition is one or more comparisons joined by `&&` (all must be true).
//! Each side of a comparison is one of:
//!
//! * a register: `a x y sp pc p`
//! * a flag, which is 0 or 1: `n v d i z c`
//! * a byte of memory: `[2900]` (hexadecimal address)
//! * a hexadecimal number: `5`, `ff`, `$ff`, `0xff`
//!
//! A bare letter that is a register or flag name means that register or flag,
//! so a number like `c` or `d` has to be written `$c` or `$d`.
//! The comparisons are `==` (or `=`), `!=`, `<`, `<=`, `>` and `>=`.

use std::fmt;

use super::debug_console::parse_hex;
use crate::cpu::{Registers, flags};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operand {
    A,
    X,
    Y,
    Sp,
    P,
    Pc,
    Flag(u8),
    Memory(u16),
    Number(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Term {
    left: Operand,
    comparison: Comparison,
    right: Operand,
}

/// A parsed condition, ready to be tested at every pass of a breakpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Condition {
    terms: Vec<Term>,
    /// What was typed, tidied, for showing in lists.
    text: String,
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// The comparison symbols, longest first so `<=` is not read as `<`.
const SYMBOLS: [(&str, Comparison); 7] = [
    ("==", Comparison::Equal),
    ("!=", Comparison::NotEqual),
    ("<=", Comparison::LessOrEqual),
    (">=", Comparison::GreaterOrEqual),
    ("=", Comparison::Equal),
    ("<", Comparison::Less),
    (">", Comparison::Greater),
];

impl Condition {
    pub fn parse(text: &str) -> Result<Condition, String> {
        let mut terms = Vec::new();
        for part in text.split("&&") {
            terms.push(parse_term(part)?);
        }
        let tidy: Vec<String> = text
            .split("&&")
            .map(|p| p.split_whitespace().collect())
            .collect();
        Ok(Condition {
            terms,
            text: tidy.join(" && "),
        })
    }

    /// Is the condition true with these registers, reading memory with `read`?
    pub fn holds(&self, regs: &Registers, read: impl Fn(u16) -> u8) -> bool {
        self.terms.iter().all(|term| {
            let left = value(term.left, regs, &read);
            let right = value(term.right, regs, &read);
            match term.comparison {
                Comparison::Equal => left == right,
                Comparison::NotEqual => left != right,
                Comparison::Less => left < right,
                Comparison::LessOrEqual => left <= right,
                Comparison::Greater => left > right,
                Comparison::GreaterOrEqual => left >= right,
            }
        })
    }
}

fn value(operand: Operand, regs: &Registers, read: &impl Fn(u16) -> u8) -> u16 {
    match operand {
        Operand::A => regs.a.into(),
        Operand::X => regs.x.into(),
        Operand::Y => regs.y.into(),
        Operand::Sp => regs.sp.into(),
        Operand::P => regs.p.into(),
        Operand::Pc => regs.pc,
        Operand::Flag(mask) => u16::from(regs.flag(mask)),
        Operand::Memory(addr) => read(addr).into(),
        Operand::Number(n) => n,
    }
}

fn parse_term(text: &str) -> Result<Term, String> {
    // The earliest comparison symbol, preferring the longer one at a position.
    let found = (0..text.len())
        .filter(|&i| text.is_char_boundary(i))
        .find_map(|i| {
            SYMBOLS
                .iter()
                .find(|(symbol, _)| text[i..].starts_with(symbol))
                .map(|&(symbol, comparison)| (i, symbol.len(), comparison))
        });
    let Some((at, length, comparison)) = found else {
        return Err(format!(
            "'{}' needs a comparison (==, !=, <, <=, >, >=)",
            text.trim()
        ));
    };
    Ok(Term {
        left: parse_operand(&text[..at])?,
        comparison,
        right: parse_operand(&text[at + length..])?,
    })
}

fn parse_operand(text: &str) -> Result<Operand, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a comparison is missing one side".to_string());
    }
    if let Some(inner) = text.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or_else(|| format!("'{text}' is missing its closing ]"))?;
        return parse_hex(inner)
            .map(Operand::Memory)
            .ok_or_else(|| format!("'{inner}' is not a hex address"));
    }
    Ok(match text.to_ascii_lowercase().as_str() {
        "a" => Operand::A,
        "x" => Operand::X,
        "y" => Operand::Y,
        "sp" => Operand::Sp,
        "p" => Operand::P,
        "pc" => Operand::Pc,
        "n" => Operand::Flag(flags::NEGATIVE),
        "v" => Operand::Flag(flags::OVERFLOW),
        "d" => Operand::Flag(flags::DECIMAL),
        "i" => Operand::Flag(flags::IRQ_DISABLE),
        "z" => Operand::Flag(flags::ZERO),
        "c" => Operand::Flag(flags::CARRY),
        _ => Operand::Number(
            parse_hex(text)
                .ok_or_else(|| format!("'{text}' is not a register, flag or hex number"))?,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regs(a: u8, x: u8, y: u8) -> Registers {
        Registers {
            a,
            x,
            y,
            sp: 0xF0,
            pc: 0x2900,
            p: 0x24,
        }
    }

    /// Memory where every address holds the low byte of its address.
    fn memory(addr: u16) -> u8 {
        addr as u8
    }

    fn holds(condition: &str, regs: &Registers) -> bool {
        Condition::parse(condition)
            .unwrap_or_else(|e| panic!("{condition}: {e}"))
            .holds(regs, memory)
    }

    #[test]
    fn registers_compare_with_numbers() {
        let r = regs(5, 0x10, 0xFF);
        assert!(holds("a==5", &r));
        assert!(!holds("a==6", &r));
        assert!(
            holds("a = 5", &r),
            "a single = is equality, and spaces are fine"
        );
        assert!(holds("x==10", &r), "numbers are hex");
        assert!(holds("y==$ff", &r));
        assert!(holds("y==0xFF", &r));
        assert!(holds("pc==2900", &r));
        assert!(holds("sp==f0", &r));
        assert!(holds("p==24", &r));
    }

    #[test]
    fn all_six_comparisons() {
        let r = regs(5, 0, 0);
        assert!(holds("a!=4", &r) && !holds("a!=5", &r));
        assert!(holds("a<6", &r) && !holds("a<5", &r));
        assert!(holds("a<=5", &r) && !holds("a<=4", &r));
        assert!(holds("a>4", &r) && !holds("a>5", &r));
        assert!(holds("a>=5", &r) && !holds("a>=6", &r));
    }

    #[test]
    fn a_comparison_can_have_registers_on_both_sides() {
        assert!(holds("a==x", &regs(7, 7, 0)));
        assert!(!holds("a==x", &regs(7, 8, 0)));
        assert!(holds("a<y", &regs(1, 0, 2)));
    }

    #[test]
    fn flags_are_zero_or_one() {
        // P = $24: interrupts disabled (I) and the unused bit.
        let r = regs(0, 0, 0);
        assert!(holds("i==1", &r));
        assert!(holds("c==0", &r));
        assert!(holds("z==0 && n==0 && v==0 && d==0", &r));
        let mut set = r.clone();
        set.p |= flags::CARRY | flags::ZERO | flags::NEGATIVE;
        assert!(holds("c==1 && z==1 && n==1", &set));
    }

    #[test]
    fn memory_is_read_through_the_function_given() {
        let r = regs(0, 0, 0);
        assert!(
            holds("[2905]==5", &r),
            "memory() returns the low byte of the address"
        );
        assert!(holds("[0200]==0", &r));
        assert!(holds("[12]==$12", &r));
        assert!(holds("a==[0000]", &r));
        assert!(!holds("[0201]==0", &r));
    }

    #[test]
    fn terms_joined_by_and_must_all_hold() {
        let r = regs(5, 6, 7);
        assert!(holds("a==5 && x==6 && y==7", &r));
        assert!(!holds("a==5 && x==6 && y==8", &r));
        assert!(!holds("a==4 && x==6", &r));
    }

    #[test]
    fn a_bare_letter_is_a_register_so_hex_c_needs_a_dollar_sign() {
        let r = regs(0x0C, 0, 0);
        assert!(holds("a==$c", &r), "$c is the number twelve");
        assert!(!holds("a==c", &r), "c is the carry flag (0)");
        assert!(holds("a==0c", &r));
    }

    #[test]
    fn the_text_is_tidied_for_display() {
        let c = Condition::parse("  a == 5   &&  x>=  10 ").unwrap();
        assert_eq!(c.to_string(), "a==5 && x>=10");
    }

    #[test]
    fn nonsense_is_refused_with_a_reason() {
        let err = |text: &str| Condition::parse(text).unwrap_err();
        assert!(err("a").contains("needs a comparison"));
        assert!(err("").contains("needs a comparison"));
        assert!(err("a==").contains("missing one side"));
        assert!(err("==5").contains("missing one side"));
        assert!(err("q==5").contains("not a register, flag or hex number"));
        assert!(err("a==zz").contains("not a register, flag or hex number"));
        assert!(err("[2900==5").contains("closing ]"));
        assert!(err("[zz]==5").contains("not a hex address"));
        assert!(err("a==5 &&").contains("needs a comparison"));
        assert!(err("a==5 && x").contains("needs a comparison"));
        assert!(
            err("a==10000").contains("not a register, flag or hex number"),
            "too big for 16 bits"
        );
    }
}
