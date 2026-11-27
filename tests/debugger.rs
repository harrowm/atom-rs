//! The debugger driving the real Atom: write a program with the command box,
//! stop inside the real operating system, change a register and a byte of
//! memory while stopped, and see the change on the screen.

mod common;

use atom_rs::machine::Atom;
use atom_rs::machine::debug_console::Console;
use atom_rs::machine::debug_session::DebugSession;
use common::booted_atom;

/// The operating system's "print the character in A" routine.
const OSWRCH: u16 = 0xFFF4;

/// A debugger session on an Atom that has booted and then been paused.
struct Debugging {
    atom: Atom,
    session: DebugSession,
    console: Console,
}

impl Debugging {
    fn start() -> Option<Self> {
        let mut atom = booted_atom(120)?;
        let mut session = DebugSession::default();
        let mut console = Console::default();
        console.execute("p", &mut atom, &mut session);
        Some(Debugging {
            atom,
            session,
            console,
        })
    }

    /// Type a line into the command box.
    fn say(&mut self, line: &str) -> String {
        self.console
            .execute(line, &mut self.atom, &mut self.session)
    }

    /// Let the machine run until it stops (it must, within a second).
    fn run_until_it_stops(&mut self) {
        let stopped = self.session.run_cycles(&mut self.atom, 1_000_000);
        assert!(stopped, "the machine never stopped:\n{}", self.say("r"));
    }
}

/// `LDA #'Z' ; JSR OSWRCH ; LDA #'!' ; JSR OSWRCH ; JMP *` at $2900.
const PROGRAM: &str = "writem 2900 A9 5A 20 F4 FF A9 21 20 F4 FF 4C 0A 29";

#[test]
fn stop_in_the_real_os_change_a_register_and_the_screen_shows_it() {
    let Some(mut d) = Debugging::start() else {
        return;
    };
    assert!(d.say(PROGRAM).starts_with("Wrote A9 5A 20 F4 FF"));
    d.say("set pc 2900");
    d.say(&format!("break {OSWRCH:04X}"));

    // First call: stops at the entry to the OS routine with A = 'Z' ($5A).
    d.say("c");
    d.run_until_it_stops();
    let registers = d.say("r");
    assert!(
        registers.contains("A=5A") && registers.contains("PC=FFF4"),
        "{registers}"
    );
    assert_eq!(
        d.session.stop_message.as_deref(),
        Some("Breakpoint at $FFF4")
    );
    // The debugger can look at the ROM it is stopped in.
    let code = d.say("d");
    assert!(code.starts_with(">FFF4"), "{code}");

    // Change A to 'A' while stopped, and go on: the OS prints what A holds now.
    d.say("set a 41");
    d.say("c");
    d.run_until_it_stops();
    let registers = d.say("r");
    assert!(
        registers.contains("A=21"),
        "the second call, with '!': {registers}"
    );

    // Remove the breakpoint and let it finish. The OS printed "A!", not "Z!".
    assert_eq!(
        d.say(&format!("bclear {OSWRCH:04X}")),
        "Cleared the breakpoint at FFF4"
    );
    d.say("c");
    d.atom.run_cycles(200_000);
    let screen = d.atom.screen_text().join("\n");
    assert!(screen.contains("A!"), "{screen}");
    assert!(!screen.contains("Z!"), "{screen}");
}

#[test]
fn step_over_a_call_into_the_real_os_runs_it_as_one_step() {
    let Some(mut d) = Debugging::start() else {
        return;
    };
    d.say(PROGRAM);
    d.say("set pc 2902"); // at the first JSR OSWRCH, with A = whatever it is
    d.say("set a 51"); // 'Q'
    let report = d.say("over");
    assert!(report.contains("Stepping over"), "{report}");
    d.run_until_it_stops();
    assert_eq!(d.atom.cpu.regs.pc, 0x2905, "back after the call");
    assert_eq!(d.session.stop_message.as_deref(), Some("Stopped at $2905"));
    d.atom.run_cycles(1_000);
    let screen = d.atom.screen_text().join("\n");
    assert!(screen.contains('Q'), "the OS routine really ran: {screen}");
}

#[test]
fn a_write_watch_stops_on_the_video_memory_the_os_draws_into() {
    let Some(mut d) = Debugging::start() else {
        return;
    };
    d.say(PROGRAM);
    d.say("set pc 2900");
    // Anything the OS draws goes into video RAM at $8000-$81FF.
    for addr in 0x8000..0x8200u16 {
        d.say(&format!("breakw {addr:04X}"));
    }
    // Each stop is one write into the screen. The OS first rubs out the cursor
    // (a space), then draws the letter; the 6847 numbers letters A=1 ... Z=26.
    let mut written = Vec::new();
    for _ in 0..4 {
        d.say("c");
        d.run_until_it_stops();
        let stop = d.session.stop_message.clone().unwrap();
        assert!(stop.starts_with("Watchpoint: write to $8"), "{stop}");
        let addr = u16::from_str_radix(stop.rsplit('$').next().unwrap(), 16).unwrap();
        assert!((0x8000..0x8200).contains(&addr), "{stop}");
        written.push(d.atom.bus.peek(addr));
        if written.last() == Some(&26) {
            break;
        }
    }
    assert!(
        written.contains(&26),
        "the 'Z' was never drawn; saw {written:?}"
    );
    assert_eq!(written[0], 0x20, "the first write rubs out the cursor");
}

#[test]
fn single_stepping_through_the_program_changes_the_registers_as_the_cpu_would() {
    let Some(mut d) = Debugging::start() else {
        return;
    };
    d.say(PROGRAM);
    d.say("set pc 2900");
    let after_lda = d.say("s");
    assert!(
        after_lda.contains("A=5A") && after_lda.contains("PC=2902"),
        "{after_lda}"
    );
    assert!(
        after_lda.contains(">2902  20 F4 FF  JSR $FFF4"),
        "{after_lda}"
    );
    let after_jsr = d.say("s");
    assert!(after_jsr.contains("PC=FFF4"), "{after_jsr}");
    // The return address is on the stack: JSR pushes the address of its last
    // byte ($2904), high byte first.
    let sp = d.atom.cpu.regs.sp;
    let (low, high) = (
        d.atom.bus.peek(0x0100 + sp as u16 + 1),
        d.atom.bus.peek(0x0100 + sp as u16 + 2),
    );
    assert_eq!(u16::from_le_bytes([low, high]), 0x2904);
}

#[test]
fn the_nmi_and_irq_buttons_reach_the_real_operating_system() {
    use atom_rs::machine::debug_session::Command;
    let Some(mut d) = Debugging::start() else {
        return;
    };
    d.say(PROGRAM);
    d.say("set pc 290A"); // the JMP * loop: always the same instruction
    d.session.execute(&mut d.atom, Command::Nmi);
    d.session.execute(&mut d.atom, Command::Step);
    let nmi_vector = u16::from_le_bytes([d.atom.bus.peek(0xFFFA), d.atom.bus.peek(0xFFFB)]);
    assert_eq!(
        d.atom.cpu.regs.pc, nmi_vector,
        "the CPU is in the NMI handler"
    );
    assert!(
        d.atom.cpu.regs.sp < 0xFD,
        "and pushed PC and P on the stack"
    );

    let mut d = Debugging::start().unwrap();
    d.say(PROGRAM);
    d.say("set pc 290A");
    d.say("set i 0"); // allow interrupts
    d.session.execute(&mut d.atom, Command::ToggleIrq);
    d.session.execute(&mut d.atom, Command::Step);
    let irq_vector = u16::from_le_bytes([d.atom.bus.peek(0xFFFE), d.atom.bus.peek(0xFFFF)]);
    assert_eq!(d.atom.cpu.regs.pc, irq_vector, "the CPU took the IRQ");
}

// ---- Conditional breakpoints, step out and the history, on the real Atom ----------

#[test]
fn a_conditional_breakpoint_in_the_real_os_skips_the_calls_that_do_not_match() {
    let Some(mut d) = Debugging::start() else {
        return;
    };
    d.say(PROGRAM); // prints 'Z' (A=$5A) and then '!' (A=$21)
    d.say("set pc 2900");
    assert_eq!(
        d.say(&format!("break {OSWRCH:04X} if a==21")),
        "Breakpoint at FFF4 if a==21"
    );
    d.say("c");
    d.run_until_it_stops();
    let registers = d.say("r");
    assert!(
        registers.contains("A=21"),
        "it let the 'Z' go by: {registers}"
    );
    d.say("c"); // carries on from the breakpoint
    d.atom.run_cycles(200_000);
    let screen = d.atom.screen_text().join("\n");
    assert!(screen.contains("Z!"), "both letters were printed: {screen}");
}

#[test]
fn step_out_of_a_real_os_routine_returns_to_the_program_that_called_it() {
    let Some(mut d) = Debugging::start() else {
        return;
    };
    d.say(PROGRAM);
    d.say("set pc 2900");
    d.say("s 2"); // LDA #'Z', then JSR: now at the first instruction of OSWRCH
    assert_eq!(d.atom.cpu.regs.pc, OSWRCH);
    assert_eq!(d.say("out"), "Running until this routine returns");
    d.run_until_it_stops();
    assert_eq!(d.atom.cpu.regs.pc, 0x2905, "back in our program");
    assert_eq!(
        d.session.stop_message.as_deref(),
        Some("Stepped out to $2905")
    );
    d.atom.run_cycles(1_000);
    assert!(
        d.atom.screen_text().join("\n").contains('Z'),
        "the OS drew the letter"
    );
}

/// An interrupt arriving in the middle of the routine must not be mistaken for
/// the routine returning: the handler's own RTI is not our way out.
#[test]
fn step_out_is_not_fooled_by_an_interrupt_handler_finishing() {
    use atom_rs::machine::debug_session::Command;
    let Some(mut d) = Debugging::start() else {
        return;
    };
    d.say(PROGRAM);
    d.say("set pc 2900");
    d.say("s 2");
    assert_eq!(d.atom.cpu.regs.pc, OSWRCH);
    // The OS's NMI handler is `PHA ; JMP ($0200)`: give it somewhere to go,
    // as a program using NMI would: a handler at $2930 that is `PLA ; RTI`.
    d.say("writem 2930 68 40");
    d.say("writem 0200 30 29");
    d.say("trace on");
    d.session.execute(&mut d.atom, Command::Nmi); // taken at the very next step
    d.say("out");
    d.run_until_it_stops();
    assert_eq!(
        d.atom.cpu.regs.pc, 0x2905,
        "it came all the way back to our program"
    );
    let history = d.say("history 100000");
    assert!(
        history.contains("interrupt taken"),
        "the NMI really happened: {history}"
    );
    assert!(
        history.contains("RTI"),
        "and its handler returned on the way: {history}"
    );
}

#[test]
fn the_history_shows_how_the_machine_got_to_the_breakpoint() {
    let Some(mut d) = Debugging::start() else {
        return;
    };
    d.say(PROGRAM);
    d.say("set pc 2900");
    d.say(&format!("break {OSWRCH:04X}"));
    d.say("trace on");
    d.say("c");
    d.run_until_it_stops();
    let last_two = d.say("history 2");
    let lines: Vec<&str> = last_two.lines().collect();
    assert_eq!(lines.len(), 2, "{last_two}");
    assert!(
        lines[0].contains("2900") && lines[0].contains("LDA #$5A"),
        "{last_two}"
    );
    assert!(
        lines[1].contains("2902") && lines[1].contains("JSR $FFF4"),
        "{last_two}"
    );
    assert!(
        lines[1].contains("A=5A"),
        "the registers before the JSR: {last_two}"
    );
}
