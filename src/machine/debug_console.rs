//! The debugger's command box: type `break 2900`, `s 10`, `writem 80 FF`...
//!
//! The commands follow Atomulator's debugger (`break`, `bclear`, `blist`,
//! `watchr`, `s [n]`, `c`, `d`, `m`, `r`, `writem`), with a few additions:
//! `set` to change a register or flag, `over` and `g` to step over and run to
//! an address, and `writem` taking several bytes. Numbers are hexadecimal, except
//! the number of steps in `s`, which is decimal.
//!
//! [`Console`] takes a line of text and an [`Atom`] and returns the text to
//! print. It does not draw anything, which keeps it easy to test.

use super::atom::Atom;
use super::debug::TraceHit;
use super::debug_session::{Command, DebugSession};
use crate::cpu::disasm::disassemble;
use crate::cpu::flags;

/// Lines of memory shown by `m` (16 bytes each) and of code shown by `d`.
const DUMP_ROWS: u16 = 8;
const DISASSEMBLY_LINES: u16 = 12;

pub const HELP: &str = "\
Debugger commands (numbers are hexadecimal except step counts; Enter repeats the last command):

  r              show the registers
  set REG VALUE  change a register (a x y sp pc p) or a flag (n v d i z c, 0 or 1);
                 only while the machine is stopped
  s [n]          step n instructions (n is decimal; 1 if no n)
  over           step over a JSR as one step
  c              continue running
  g ADDR         run until the PC reaches ADDR
  f              run for one video frame
  p              pause
  d [ADDR]       disassemble (from the PC, or ADDR; then carries on)
  m [ADDR]       dump memory (carries on from where it stopped)
  writem A V...  write one or more bytes to RAM starting at A

  break ADDR     stop before the instruction at ADDR      (bclear ADDR removes it)
  breakr ADDR    stop when ADDR is read                   (bclearr ADDR)
  breakw ADDR    stop when ADDR is written                (bclearw ADDR)
  blist          list the breakpoints
  watchr ADDR    log reads of ADDR, without stopping      (wclearr ADDR)
  watchw ADDR    log writes of ADDR, without stopping     (wclearw ADDR)
  wlist          list the logging watches

  help           this list";

/// Parse a hex number as typed by a person: `C000`, `$C000` or `0xC000`.
pub fn parse_hex(text: &str) -> Option<u16> {
    let text = text.trim();
    let text = text
        .strip_prefix('$')
        .or_else(|| text.strip_prefix("0x"))
        .unwrap_or(text);
    u16::from_str_radix(text, 16).ok()
}

/// What the box remembers between commands.
#[derive(Clone, Debug, Default)]
pub struct Console {
    /// Where `m` carries on from.
    dump_next: u16,
    /// Where `d` carries on from, if it has already been used since the machine
    /// last ran.
    disassemble_next: Option<u16>,
    last_line: String,
}

impl Console {
    /// Carry out one line and return what to print (possibly nothing).
    /// An empty line repeats the previous one, like Atomulator's.
    pub fn execute(&mut self, line: &str, atom: &mut Atom, session: &mut DebugSession) -> String {
        let line = if line.trim().is_empty() {
            self.last_line.clone()
        } else {
            self.last_line = line.trim().to_string();
            line.trim().to_string()
        };
        let mut words = line.split_whitespace();
        let Some(command) = words.next() else {
            return String::new();
        };
        let args: Vec<&str> = words.collect();
        match command.to_ascii_lowercase().as_str() {
            "help" | "h" | "?" => HELP.to_string(),
            "r" => registers_report(atom),
            "set" => set(&args, atom, session),
            "writem" | "w" => self.writem(&args, atom),
            "m" => self.dump(&args, atom),
            "d" => self.disassemble(&args, atom),
            "s" => self.step(&args, atom, session),
            "over" => self.control(Command::StepOver, atom, session),
            "c" => self.control(Command::Run, atom, session),
            "f" => self.control(Command::StepFrame, atom, session),
            "p" => self.control(Command::Pause, atom, session),
            "g" => match one_address(&args, "g") {
                Ok(addr) => self.control(Command::RunTo(addr), atom, session),
                Err(message) => message,
            },
            "break" => change_breakpoint(&args, "break", atom, |d, a| {
                d.add_breakpoint(a);
                format!("Breakpoint at {a:04X}")
            }),
            "bclear" => change_breakpoint(&args, "bclear", atom, |d, a| {
                if d.remove_breakpoint(a) {
                    format!("Cleared the breakpoint at {a:04X}")
                } else {
                    format!("No breakpoint at {a:04X}")
                }
            }),
            "breakr" => watch(&args, "breakr", atom, Kind::Stop, Access::Read, true),
            "breakw" => watch(&args, "breakw", atom, Kind::Stop, Access::Write, true),
            "bclearr" => watch(&args, "bclearr", atom, Kind::Stop, Access::Read, false),
            "bclearw" => watch(&args, "bclearw", atom, Kind::Stop, Access::Write, false),
            "watchr" => watch(&args, "watchr", atom, Kind::Log, Access::Read, true),
            "watchw" => watch(&args, "watchw", atom, Kind::Log, Access::Write, true),
            "wclearr" => watch(&args, "wclearr", atom, Kind::Log, Access::Read, false),
            "wclearw" => watch(&args, "wclearw", atom, Kind::Log, Access::Write, false),
            "blist" => breakpoint_list(atom),
            "wlist" => watch_list(atom),
            other => format!("Unknown command '{other}'. Type help for the commands."),
        }
    }

    fn writem(&mut self, args: &[&str], atom: &mut Atom) -> String {
        let Some((first, values)) = args.split_first().filter(|(_, v)| !v.is_empty()) else {
            return "Usage: writem ADDR VALUE [VALUE...]".to_string();
        };
        let Some(start) = parse_hex(first) else {
            return not_hex(first);
        };
        let mut bytes = Vec::new();
        for value in values {
            match parse_hex(value) {
                Some(byte) if byte <= 0xFF => bytes.push(byte as u8),
                _ => return format!("'{value}' is not a byte (00-FF)"),
            }
        }
        for (offset, &byte) in bytes.iter().enumerate() {
            let addr = start.wrapping_add(offset as u16);
            if !atom.bus.poke(addr, byte) {
                return format!("{addr:04X} is not RAM, so it cannot be written");
            }
        }
        let list: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
        format!("Wrote {} at {start:04X}", list.join(" "))
    }

    fn dump(&mut self, args: &[&str], atom: &Atom) -> String {
        if let Some(arg) = args.first() {
            match parse_hex(arg) {
                Some(addr) => self.dump_next = addr,
                None => return not_hex(arg),
            }
        }
        let mut lines = Vec::new();
        for _ in 0..DUMP_ROWS {
            let base = self.dump_next;
            let bytes: Vec<u8> = (0..16)
                .map(|i| atom.bus.peek(base.wrapping_add(i)))
                .collect();
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
            let text: String = bytes
                .iter()
                .map(|&b| {
                    if (0x20..0x7F).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            lines.push(format!("{base:04X}  {}  {text}", hex.join(" ")));
            self.dump_next = base.wrapping_add(16);
        }
        lines.join("\n")
    }

    fn disassemble(&mut self, args: &[&str], atom: &Atom) -> String {
        let mut addr = match args.first() {
            Some(arg) => match parse_hex(arg) {
                Some(addr) => addr,
                None => return not_hex(arg),
            },
            None => self.disassemble_next.unwrap_or(atom.cpu.regs.pc),
        };
        let mut lines = Vec::new();
        for _ in 0..DISASSEMBLY_LINES {
            lines.push(instruction_line(atom, addr, addr == atom.cpu.regs.pc));
            addr = addr.wrapping_add(disassemble(addr, |a| atom.bus.peek(a)).length);
        }
        self.disassemble_next = Some(addr);
        lines.join("\n")
    }

    fn step(&mut self, args: &[&str], atom: &mut Atom, session: &mut DebugSession) -> String {
        // The count is the one decimal number in the box (the rest are hex).
        let count = match args.first() {
            None => 1,
            Some(arg) => match arg.parse::<u32>() {
                Ok(count) => count,
                Err(_) => return format!("'{arg}' is not a number of steps (decimal)"),
            },
        };
        self.control(Command::StepN(count), atom, session)
    }

    /// Carry out a control command, and say where the machine is now.
    fn control(&mut self, command: Command, atom: &mut Atom, session: &mut DebugSession) -> String {
        self.disassemble_next = None; // the machine has moved: `d` starts at the PC again
        session.execute(atom, command);
        match command {
            _ if session.running => match command {
                Command::StepOver => "Stepping over the call; running until it returns".to_string(),
                Command::RunTo(addr) => format!("Running to {addr:04X}"),
                _ => "Running".to_string(),
            },
            _ => {
                let mut report = state_report(atom);
                if let Some(message) = &session.stop_message {
                    report = format!("{message}\n{report}");
                }
                report
            }
        }
    }
}

/// `set a 42`, `set pc C000`, `set z 1`.
fn set(args: &[&str], atom: &mut Atom, session: &DebugSession) -> String {
    let [name, value] = args else {
        return "Usage: set REGISTER VALUE   (a x y sp pc p, or a flag: n v d i z c)".to_string();
    };
    if session.running {
        return "Pause first (p): a running program would change it straight back".to_string();
    }
    let Some(number) = parse_hex(value) else {
        return not_hex(value);
    };
    let regs = &mut atom.cpu.regs;
    let name = name.to_ascii_lowercase();
    let byte = || u8::try_from(number).map_err(|_| format!("{name} is one byte (00-FF)"));
    let result = match name.as_str() {
        "a" => byte().map(|b| regs.a = b),
        "x" => byte().map(|b| regs.x = b),
        "y" => byte().map(|b| regs.y = b),
        "sp" => byte().map(|b| regs.sp = b),
        "p" => byte().map(|b| regs.p = b),
        "pc" => {
            regs.pc = number;
            Ok(())
        }
        flag if flag.len() == 1 && "nvdizc".contains(flag) => {
            let mask = match flag {
                "n" => flags::NEGATIVE,
                "v" => flags::OVERFLOW,
                "d" => flags::DECIMAL,
                "i" => flags::IRQ_DISABLE,
                "z" => flags::ZERO,
                _ => flags::CARRY,
            };
            match number {
                0 | 1 => {
                    regs.set_flag(mask, number == 1);
                    Ok(())
                }
                _ => Err(format!("a flag is 0 or 1, not {value}")),
            }
        }
        other => Err(format!("'{other}' is not a register or flag")),
    };
    match result {
        Ok(()) => registers_report(atom),
        Err(message) => message,
    }
}

#[derive(Clone, Copy)]
enum Kind {
    /// Stops the machine.
    Stop,
    /// Only records the access.
    Log,
}

#[derive(Clone, Copy)]
enum Access {
    Read,
    Write,
}

/// `breakr`, `watchw`, `bclearr`... : add (`add`) or remove a watch.
fn watch(
    args: &[&str],
    name: &str,
    atom: &mut Atom,
    kind: Kind,
    access: Access,
    add: bool,
) -> String {
    let addr = match one_address(args, name) {
        Ok(addr) => addr,
        Err(message) => return message,
    };
    let (on_read, on_write) = match access {
        Access::Read => (true, false),
        Access::Write => (false, true),
    };
    let verb = if on_read { "reads of" } else { "writes to" };
    let debug = &mut atom.bus.debug;
    let (what, changed) = match (kind, add) {
        (Kind::Stop, true) => {
            debug.add_watchpoint(addr, on_read, on_write);
            ("Stopping on", true)
        }
        (Kind::Stop, false) => (
            "Stopped stopping on",
            debug.remove_watchpoint(addr, on_read, on_write),
        ),
        (Kind::Log, true) => {
            debug.add_trace(addr, on_read, on_write);
            ("Logging", true)
        }
        (Kind::Log, false) => (
            "Stopped logging",
            debug.remove_trace(addr, on_read, on_write),
        ),
    };
    if changed {
        format!("{what} {verb} {addr:04X}")
    } else {
        format!("Nothing was set for {verb} {addr:04X}")
    }
}

fn change_breakpoint(
    args: &[&str],
    name: &str,
    atom: &mut Atom,
    change: impl FnOnce(&mut super::debug::Debugger, u16) -> String,
) -> String {
    match one_address(args, name) {
        Ok(addr) => change(&mut atom.bus.debug, addr),
        Err(message) => message,
    }
}

fn breakpoint_list(atom: &Atom) -> String {
    let debug = &atom.bus.debug;
    let mut lines = Vec::new();
    if !debug.breakpoints.is_empty() {
        let list: Vec<String> = debug
            .breakpoints
            .iter()
            .map(|a| format!("{a:04X}"))
            .collect();
        lines.push(format!("Breakpoints: {}", list.join(" ")));
    }
    lines.extend(watch_lines(&debug.watchpoints, "Break on"));
    if lines.is_empty() {
        "No breakpoints".to_string()
    } else {
        lines.join("\n")
    }
}

fn watch_list(atom: &Atom) -> String {
    let lines = watch_lines(&atom.bus.debug.traces, "Log");
    if lines.is_empty() {
        "No logging watches".to_string()
    } else {
        lines.join("\n")
    }
}

fn watch_lines(list: &[super::debug::Watchpoint], label: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for access in [Access::Read, Access::Write] {
        let (word, addrs): (&str, Vec<String>) = match access {
            Access::Read => (
                "reads",
                list.iter()
                    .filter(|w| w.on_read)
                    .map(|w| format!("{:04X}", w.addr))
                    .collect(),
            ),
            Access::Write => (
                "writes",
                list.iter()
                    .filter(|w| w.on_write)
                    .map(|w| format!("{:04X}", w.addr))
                    .collect(),
            ),
        };
        if !addrs.is_empty() {
            lines.push(format!("{label} {word}: {}", addrs.join(" ")));
        }
    }
    lines
}

/// One line describing a logged access, for the command box to print.
pub fn describe_trace(hit: &TraceHit) -> String {
    match (hit.write, hit.value) {
        (true, Some(value)) => format!(
            "Write {value:02X} to {:04X}  (cycle {})",
            hit.addr, hit.cycle
        ),
        (true, None) => format!("Write to {:04X}  (cycle {})", hit.addr, hit.cycle),
        (false, _) => format!("Read from {:04X}  (cycle {})", hit.addr, hit.cycle),
    }
}

// ---- Reports ----------------------------------------------------------------------

/// `A=05 X=00 Y=00 S=FF PC=2902 P=34 (.V-B.I..)`: a flag shows as its letter
/// when set and `.` when clear.
fn registers_report(atom: &Atom) -> String {
    let r = &atom.cpu.regs;
    let letters: String = [
        ('N', flags::NEGATIVE),
        ('V', flags::OVERFLOW),
        ('-', flags::UNUSED),
        ('B', flags::BREAK),
        ('D', flags::DECIMAL),
        ('I', flags::IRQ_DISABLE),
        ('Z', flags::ZERO),
        ('C', flags::CARRY),
    ]
    .iter()
    .map(|&(letter, mask)| if r.flag(mask) { letter } else { '.' })
    .collect();
    format!(
        "A={:02X} X={:02X} Y={:02X} S={:02X} PC={:04X} P={:02X} ({letters})",
        r.a, r.x, r.y, r.sp, r.pc, r.p
    )
}

/// The registers and the instruction about to run.
pub fn state_report(atom: &Atom) -> String {
    format!(
        "{}\n{}",
        registers_report(atom),
        instruction_line(atom, atom.cpu.regs.pc, true)
    )
}

/// `2902  8D 00 02  STA $0200`, with `>` in front of the one at the PC.
fn instruction_line(atom: &Atom, addr: u16, at_pc: bool) -> String {
    let instruction = disassemble(addr, |a| atom.bus.peek(a));
    let bytes: Vec<String> = (0..instruction.length)
        .map(|i| format!("{:02X}", atom.bus.peek(addr.wrapping_add(i))))
        .collect();
    let marker = if at_pc { '>' } else { ' ' };
    format!(
        "{marker}{addr:04X}  {:<8}  {}",
        bytes.join(" "),
        instruction.text
    )
}

fn not_hex(text: &str) -> String {
    format!("'{text}' is not a hex number")
}

/// The single address argument of a command like `break ADDR`.
fn one_address(args: &[&str], name: &str) -> Result<u16, String> {
    match args {
        [arg] => parse_hex(arg).ok_or_else(|| not_hex(arg)),
        _ => Err(format!("Usage: {name} ADDR")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same little program as in the session tests, at $2900:
    /// `LDA #5 ; STA $200 ; JSR $2910 ; INC $200 ; NOP ; JMP $290C`, and the
    /// subroutine `LDX #7 ; STX $201 ; RTS` at $2910.
    fn machine() -> (Atom, DebugSession, Console) {
        let mut atom = Atom::new();
        let code: [(u16, &[u8]); 2] = [
            (
                0x2900,
                &[
                    0xA9, 0x05, 0x8D, 0x00, 0x02, 0x20, 0x10, 0x29, 0xEE, 0x00, 0x02, 0xEA, 0x4C,
                    0x0C, 0x29,
                ],
            ),
            (0x2910, &[0xA2, 0x07, 0x8E, 0x01, 0x02, 0x60]),
        ];
        for (start, bytes) in code {
            for (i, &byte) in bytes.iter().enumerate() {
                atom.bus.poke(start + i as u16, byte);
            }
        }
        atom.cpu.regs.pc = 0x2900;
        let mut session = DebugSession::default();
        session.execute(&mut atom, Command::Pause);
        (atom, session, Console::default())
    }

    fn run(
        console: &mut Console,
        atom: &mut Atom,
        session: &mut DebugSession,
        line: &str,
    ) -> String {
        console.execute(line, atom, session)
    }

    macro_rules! cmd {
        ($m:expr, $line:expr) => {{
            let (atom, session, console) = &mut $m;
            run(console, atom, session, $line)
        }};
    }

    #[test]
    fn parses_hex_in_common_notations() {
        assert_eq!(parse_hex("C000"), Some(0xC000));
        assert_eq!(parse_hex(" $b800 "), Some(0xB800));
        assert_eq!(parse_hex("0xFF"), Some(0xFF));
        assert_eq!(parse_hex("zz"), None);
        assert_eq!(parse_hex("12345"), None, "too big for 16 bits");
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn r_shows_the_registers_and_flags() {
        let mut m = machine();
        m.0.cpu.regs.a = 0x12;
        m.0.cpu.regs.sp = 0x00;
        m.0.cpu.regs.p = flags::CARRY | flags::ZERO | flags::UNUSED;
        assert_eq!(cmd!(m, "r"), "A=12 X=00 Y=00 S=00 PC=2900 P=23 (..-...ZC)");
    }

    #[test]
    fn set_changes_registers_and_flags() {
        let mut m = machine();
        assert!(cmd!(m, "set a 42").contains("A=42"));
        assert!(cmd!(m, "set x ff").contains("X=FF"));
        assert!(cmd!(m, "set y $10").contains("Y=10"));
        assert!(cmd!(m, "SET PC 2910").contains("PC=2910"));
        assert!(cmd!(m, "set sp f0").contains("S=F0"));
        assert_eq!(
            (m.0.cpu.regs.a, m.0.cpu.regs.x, m.0.cpu.regs.pc),
            (0x42, 0xFF, 0x2910)
        );
        assert!(cmd!(m, "set c 1").contains("C)"), "carry shows as C");
        assert!(m.0.cpu.regs.flag(flags::CARRY));
        cmd!(m, "set c 0");
        assert!(!m.0.cpu.regs.flag(flags::CARRY));
        cmd!(m, "set z 1");
        cmd!(m, "set n 1");
        assert!(m.0.cpu.regs.flag(flags::ZERO) && m.0.cpu.regs.flag(flags::NEGATIVE));
        cmd!(m, "set p 04");
        assert_eq!(m.0.cpu.regs.p, 0x04);
    }

    #[test]
    fn set_rejects_what_makes_no_sense() {
        let mut m = machine();
        assert!(cmd!(m, "set a 100").contains("one byte"));
        assert!(cmd!(m, "set c 2").contains("0 or 1"));
        assert!(cmd!(m, "set q 1").contains("not a register"));
        assert!(cmd!(m, "set a zz").contains("not a hex number"));
        assert!(cmd!(m, "set a").contains("Usage"));
        assert_eq!(m.0.cpu.regs.a, 0, "nothing changed");
    }

    #[test]
    fn set_needs_the_machine_stopped() {
        let mut m = machine();
        m.1.running = true;
        assert!(cmd!(m, "set a 1").contains("Pause first"));
        assert_eq!(m.0.cpu.regs.a, 0);
    }

    #[test]
    fn s_steps_and_reports_where_the_machine_is_now() {
        let mut m = machine();
        let out = cmd!(m, "s");
        assert!(out.contains("A=05"), "{out}");
        assert!(out.contains("PC=2902"), "{out}");
        assert!(out.contains(">2902  8D 00 02  STA $0200"), "{out}");
        assert!(cmd!(m, "s 2").lines().next().unwrap().contains("PC=2910"));
        assert!(!m.1.running);
    }

    #[test]
    fn enter_repeats_the_last_command() {
        let mut m = machine();
        cmd!(m, "s");
        assert_eq!(m.0.cpu.regs.pc, 0x2902);
        cmd!(m, "");
        assert_eq!(m.0.cpu.regs.pc, 0x2905);
        cmd!(m, "   ");
        assert_eq!(m.0.cpu.regs.pc, 0x2910);
        let mut fresh = machine();
        assert_eq!(cmd!(fresh, ""), "", "nothing to repeat yet");
    }

    #[test]
    fn break_c_and_the_stop_report() {
        let mut m = machine();
        assert_eq!(cmd!(m, "break 2905"), "Breakpoint at 2905");
        assert_eq!(cmd!(m, "c"), "Running");
        assert!(m.1.running);
        m.1.run_cycles(&mut m.0, 10_000);
        assert_eq!(m.0.cpu.regs.pc, 0x2905);
        assert!(!m.1.running);
        // Another command now reports the stop and the state.
        let out = cmd!(m, "p");
        assert!(out.contains("PC=2905"), "{out}");
    }

    #[test]
    fn bclear_removes_and_blist_lists() {
        let mut m = machine();
        assert_eq!(cmd!(m, "blist"), "No breakpoints");
        cmd!(m, "break 2905");
        cmd!(m, "break $2911");
        cmd!(m, "breakw 0200");
        cmd!(m, "breakr 0201");
        assert_eq!(
            cmd!(m, "blist"),
            "Breakpoints: 2905 2911\nBreak on reads: 0201\nBreak on writes: 0200"
        );
        assert_eq!(cmd!(m, "bclear 2905"), "Cleared the breakpoint at 2905");
        assert_eq!(cmd!(m, "bclear 2905"), "No breakpoint at 2905");
        assert_eq!(
            cmd!(m, "bclearw 0200"),
            "Stopped stopping on writes to 0200"
        );
        assert_eq!(
            cmd!(m, "bclearw 0200"),
            "Nothing was set for writes to 0200"
        );
        assert_eq!(cmd!(m, "bclearr 0201"), "Stopped stopping on reads of 0201");
        assert_eq!(cmd!(m, "blist"), "Breakpoints: 2911");
    }

    #[test]
    fn breakw_stops_the_machine_after_the_write() {
        let mut m = machine();
        cmd!(m, "breakw 0200");
        cmd!(m, "c");
        m.1.run_cycles(&mut m.0, 10_000);
        assert_eq!(m.0.cpu.regs.pc, 0x2905, "after the STA");
        assert!(
            m.1.stop_message
                .as_deref()
                .unwrap()
                .contains("write to $0200")
        );
    }

    #[test]
    fn watchw_logs_without_stopping() {
        let mut m = machine();
        assert_eq!(cmd!(m, "wlist"), "No logging watches");
        assert_eq!(cmd!(m, "watchw 0200"), "Logging writes to 0200");
        assert_eq!(cmd!(m, "watchr 0200"), "Logging reads of 0200");
        assert_eq!(cmd!(m, "wlist"), "Log reads: 0200\nLog writes: 0200");
        cmd!(m, "g 290C");
        assert!(m.1.run_cycles(&mut m.0, 10_000), "stops at the address");
        assert_eq!(m.0.cpu.regs.pc, 0x290C, "the watch did not stop it");
        let log = m.0.bus.debug.take_trace_log();
        let described: Vec<String> = log.iter().map(describe_trace).collect();
        // The STA, then the INC: a read, a write of the old value back (the
        // real 6502 does this while it works out the new one) and the write.
        assert_eq!(described.len(), 4, "{described:?}");
        assert!(
            described[0].starts_with("Write 05 to 0200"),
            "{described:?}"
        );
        assert!(described[1].starts_with("Read from 0200"), "{described:?}");
        assert!(
            described[2].starts_with("Write 05 to 0200"),
            "{described:?}"
        );
        assert!(
            described[3].starts_with("Write 06 to 0200"),
            "{described:?}"
        );
        assert_eq!(cmd!(m, "wclearw 0200"), "Stopped logging writes to 0200");
        assert_eq!(cmd!(m, "wclearr 0200"), "Stopped logging reads of 0200");
        assert_eq!(cmd!(m, "wlist"), "No logging watches");
    }

    #[test]
    fn g_and_over_and_f() {
        let mut m = machine();
        cmd!(m, "s 2");
        assert_eq!(
            cmd!(m, "over"),
            "Stepping over the call; running until it returns"
        );
        m.1.run_cycles(&mut m.0, 10_000);
        assert_eq!(m.0.cpu.regs.pc, 0x2908);
        assert_eq!(cmd!(m, "g 290B"), "Running to 290B");
        m.1.run_cycles(&mut m.0, 10_000);
        assert_eq!(m.0.cpu.regs.pc, 0x290B);
        let out = cmd!(m, "f");
        assert!(out.contains("PC=290C") || out.contains("PC=290B"), "{out}");
    }

    #[test]
    fn writem_writes_bytes_to_ram() {
        let mut m = machine();
        assert_eq!(cmd!(m, "writem 0300 12"), "Wrote 12 at 0300");
        assert_eq!(m.0.bus.peek(0x0300), 0x12);
        assert_eq!(cmd!(m, "writem 0300 A9 05 8D"), "Wrote A9 05 8D at 0300");
        assert_eq!(
            (
                m.0.bus.peek(0x0300),
                m.0.bus.peek(0x0301),
                m.0.bus.peek(0x0302)
            ),
            (0xA9, 0x05, 0x8D)
        );
        assert_eq!(
            cmd!(m, "w 0310 ff"),
            "Wrote FF at 0310",
            "w is short for writem"
        );
    }

    #[test]
    fn writem_refuses_rom_and_nonsense() {
        let mut m = machine();
        assert!(cmd!(m, "writem C000 12").contains("not RAM"));
        assert!(cmd!(m, "writem 0300 100").contains("not a byte"));
        assert!(cmd!(m, "writem 0300").contains("Usage"));
        assert!(cmd!(m, "writem zz 1").contains("not a hex number"));
    }

    #[test]
    fn a_program_poked_in_with_writem_runs() {
        let mut m = machine();
        // LDA #$2A ; STA $0300 ; BRK-free: JMP $0308 (a loop)
        cmd!(m, "writem 0300 A9 2A 8D 20 03 4C 05 03");
        cmd!(m, "set pc 0300");
        cmd!(m, "g 0305");
        m.1.run_cycles(&mut m.0, 10_000);
        assert_eq!(m.0.bus.peek(0x0320), 0x2A);
    }

    #[test]
    fn m_dumps_memory_and_carries_on() {
        let mut m = machine();
        let first = cmd!(m, "m 2900");
        let lines: Vec<&str> = first.lines().collect();
        assert_eq!(lines.len(), DUMP_ROWS as usize);
        assert!(
            lines[0].starts_with("2900  A9 05 8D 00 02 20 10 29 EE 00 02 EA 4C 0C 29 00"),
            "{first}"
        );
        assert!(lines[1].starts_with("2910  A2 07 8E 01 02 60"), "{first}");
        let next = cmd!(m, "m");
        assert!(
            next.starts_with(&format!("{:04X}", 0x2900 + 16 * DUMP_ROWS)),
            "{next}"
        );
        assert!(cmd!(m, "m zz").contains("not a hex number"));
    }

    #[test]
    fn m_shows_printable_characters() {
        let mut m = machine();
        cmd!(m, "writem 0400 48 49 00");
        let out = cmd!(m, "m 0400");
        assert!(
            out.lines().next().unwrap().ends_with("HI.............."),
            "{out}"
        );
    }

    #[test]
    fn d_disassembles_from_the_pc_and_carries_on() {
        let mut m = machine();
        let out = cmd!(m, "d");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), DISASSEMBLY_LINES as usize);
        assert_eq!(lines[0], ">2900  A9 05     LDA #$05", "the PC is marked");
        assert_eq!(lines[1], " 2902  8D 00 02  STA $0200");
        assert_eq!(lines[2], " 2905  20 10 29  JSR $2910");
        // The twelve lines ran from $2900 to $2917 (the zeros after the
        // program are one-byte BRKs); `d` carries on at $2918.
        assert!(out.lines().last().unwrap().starts_with(" 2917"), "{out}");
        assert!(cmd!(m, "d").starts_with(" 2918"));
        // d ADDR starts there.
        assert!(cmd!(m, "d 2910").starts_with(" 2910  A2 07     LDX #$07"));
        // After the machine moves, d starts at the PC again.
        cmd!(m, "s");
        assert!(cmd!(m, "d").starts_with(">2902"));
    }

    #[test]
    fn help_and_unknown_commands() {
        let mut m = machine();
        assert!(cmd!(m, "help").contains("writem"));
        assert_eq!(cmd!(m, "?"), cmd!(m, "help"));
        assert_eq!(cmd!(m, "h"), cmd!(m, "help"));
        assert!(cmd!(m, "frobnicate").contains("Unknown command 'frobnicate'"));
        // Every command the help lists is one the box knows.
        let entries = HELP
            .lines()
            .filter(|l| l.starts_with("  ") && !l.starts_with("   "));
        for word in entries.filter_map(|l| l.split_whitespace().next()) {
            let word = word.to_ascii_lowercase();
            let reply = cmd!(machine(), &word);
            assert!(
                !reply.contains("Unknown command"),
                "help lists '{word}': {reply}"
            );
        }
    }

    #[test]
    fn commands_are_not_case_sensitive_and_take_addresses_in_any_notation() {
        let mut m = machine();
        assert_eq!(cmd!(m, "BREAK $2905"), "Breakpoint at 2905");
        assert_eq!(cmd!(m, "Break 0x2911"), "Breakpoint at 2911");
        assert_eq!(cmd!(m, "blist"), "Breakpoints: 2905 2911");
    }

    #[test]
    fn missing_or_bad_addresses_are_explained() {
        let mut m = machine();
        assert_eq!(cmd!(m, "break"), "Usage: break ADDR");
        assert_eq!(cmd!(m, "break 1 2"), "Usage: break ADDR");
        assert!(cmd!(m, "break xyz").contains("not a hex number"));
        assert!(cmd!(m, "g").contains("Usage: g ADDR"));
        assert!(cmd!(m, "s abc").contains("not a number of steps"));
        assert_eq!(m.0.cpu.regs.pc, 0x2900, "nothing ran");
    }
}
