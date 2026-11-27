//! Running and stepping under the debugger's control.
//!
//! The debugger windows and the command box both ask for things to happen (run,
//! step, run to an address...) as [`Command`]s. [`DebugSession`] carries them
//! out on an [`Atom`] and remembers whether the machine is running and why it
//! last stopped. It lives here, rather than in the window code, so that it can
//! be tested without a window.

use super::atom::Atom;
use super::debug::StopReason;
use super::vdg6847::CYCLES_PER_LINE;

/// Something the user asked the debugger to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Run,
    Pause,
    /// Execute one instruction.
    Step,
    /// Execute this many instructions (stopping early at a watchpoint).
    StepN(u32),
    /// Like `Step`, but run a whole `JSR` subroutine as one step.
    StepOver,
    /// Run until the subroutine the PC is in returns to its caller.
    StepOut,
    /// Run for one video frame.
    StepFrame,
    /// Run until the PC reaches this address.
    RunTo(u16),
    Reset,
    Nmi,
    ToggleIrq,
}

/// Cycles in one video frame (the length of the "Frame" step).
pub const CYCLES_PER_FRAME: u64 = CYCLES_PER_LINE as u64 * 262;

/// Is the machine running, and why did it last stop?
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugSession {
    pub running: bool,
    /// Why the last run stopped (breakpoint, watchpoint...), if it did.
    pub stop_message: Option<String>,
}

impl Default for DebugSession {
    /// A new session is running, as the emulator is when it starts.
    fn default() -> Self {
        DebugSession {
            running: true,
            stop_message: None,
        }
    }
}

impl DebugSession {
    pub fn execute(&mut self, atom: &mut Atom, command: Command) {
        match command {
            Command::Run => {
                // A plain Run is just that: forget any "run to", "step over"
                // or "step out" that was waiting from before.
                forget_pending_stops(atom);
                self.resume(atom);
            }
            Command::Pause => {
                self.running = false;
                forget_pending_stops(atom);
            }
            Command::Step => self.step_n(atom, 1),
            Command::StepN(count) => self.step_n(atom, count),
            Command::StepOver => match atom.step_over_target() {
                Some(target) => {
                    atom.bus.debug.target = Some(target);
                    self.resume(atom);
                }
                None => self.step_n(atom, 1),
            },
            Command::StepOut => {
                atom.bus.debug.start_step_out();
                self.resume(atom);
            }
            Command::StepFrame => {
                self.running = false;
                self.stop_message = None;
                atom.bus.debug.skip_breakpoint_once = true;
                let stop = atom.run_cycles(CYCLES_PER_FRAME);
                self.note_stop(atom, stop);
            }
            Command::RunTo(addr) => {
                atom.bus.debug.target = Some(addr);
                self.resume(atom);
            }
            Command::Reset => {
                atom.reset();
                self.stop_message = None;
            }
            Command::Nmi => atom.bus.nmi_edge = true,
            Command::ToggleIrq => atom.bus.external_irq = !atom.bus.external_irq,
        }
    }

    /// Run the machine for about `cycles` (what the main loop does while the
    /// machine is running). Returns true if it stopped.
    pub fn run_cycles(&mut self, atom: &mut Atom, cycles: u64) -> bool {
        let stop = atom.run_cycles(cycles);
        self.note_stop(atom, stop);
        stop.is_some()
    }

    /// Start running. The first instruction runs even if it sits on a
    /// breakpoint, otherwise you could never continue from one.
    fn resume(&mut self, atom: &mut Atom) {
        self.running = true;
        self.stop_message = None;
        atom.bus.debug.skip_breakpoint_once = true;
    }

    /// Execute instructions one at a time. Breakpoints do not stop this (you
    /// asked for these steps), but a watchpoint does.
    fn step_n(&mut self, atom: &mut Atom, count: u32) {
        self.running = false;
        self.stop_message = None;
        for _ in 0..count {
            atom.step();
            let stop = atom.bus.debug.stop.take();
            if stop.is_some() {
                self.note_stop(atom, stop);
                return;
            }
        }
        forget_pending_stops(atom);
    }

    /// The run ended: if there was a reason, stop and say why. Whatever the
    /// reason, a one-shot target (from step over, step out or run to) is spent:
    /// a breakpoint met on the way must not leave it waiting to stop the next
    /// Run at a place the user has forgotten about.
    fn note_stop(&mut self, atom: &mut Atom, reason: Option<StopReason>) {
        if let Some(reason) = reason {
            self.running = false;
            self.stop_message = Some(reason.to_string());
            forget_pending_stops(atom);
        }
    }
}

/// Drop the one-shot "stop when we get there" requests: run to an address,
/// step over, step out.
fn forget_pending_stops(atom: &mut Atom) {
    atom.bus.debug.target = None;
    atom.bus.debug.step_out = None;
}

#[cfg(test)]
mod tests {
    use super::super::condition::Condition;
    use super::*;

    /// A tiny program at $2900:
    ///
    /// ```text
    /// 2900  LDA #$05
    /// 2902  STA $0200
    /// 2905  JSR $2910
    /// 2908  INC $0200
    /// 290B  NOP
    /// 290C  JMP $290C      (stay here)
    ///
    /// 2910  LDX #$07       (the subroutine)
    /// 2912  STX $0201
    /// 2915  RTS
    /// ```
    fn program() -> Atom {
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
                assert!(atom.bus.poke(start + i as u16, byte));
            }
        }
        atom.cpu.regs.pc = 0x2900;
        atom
    }

    fn at_rest() -> (Atom, DebugSession) {
        let mut session = DebugSession::default();
        session.execute(&mut program(), Command::Pause);
        (
            program(),
            DebugSession {
                running: false,
                ..session
            },
        )
    }

    #[test]
    fn a_new_session_is_running() {
        assert!(DebugSession::default().running);
    }

    #[test]
    fn step_runs_exactly_one_instruction_and_stays_paused() {
        let (mut atom, mut session) = at_rest();
        session.execute(&mut atom, Command::Step);
        assert_eq!((atom.cpu.regs.pc, atom.cpu.regs.a), (0x2902, 5));
        session.execute(&mut atom, Command::Step);
        assert_eq!(atom.cpu.regs.pc, 0x2905);
        assert_eq!(atom.bus.peek(0x0200), 5, "the STA happened");
        assert!(!session.running);
    }

    #[test]
    fn step_n_runs_that_many_instructions() {
        let (mut atom, mut session) = at_rest();
        session.execute(&mut atom, Command::StepN(3));
        assert_eq!(atom.cpu.regs.pc, 0x2910, "LDA, STA, JSR");
        session.execute(&mut atom, Command::StepN(0));
        assert_eq!(atom.cpu.regs.pc, 0x2910, "none at all");
    }

    #[test]
    fn step_n_goes_through_breakpoints_but_a_watchpoint_stops_it() {
        let (mut atom, mut session) = at_rest();
        atom.bus.debug.add_breakpoint(0x2902);
        session.execute(&mut atom, Command::StepN(2));
        assert_eq!(atom.cpu.regs.pc, 0x2905, "stepping ignores the breakpoint");

        let (mut atom, mut session) = at_rest();
        atom.bus.debug.add_watchpoint(0x0200, false, true);
        session.execute(&mut atom, Command::StepN(10));
        assert_eq!(atom.cpu.regs.pc, 0x2905, "stopped after the STA");
        assert_eq!(
            session.stop_message.as_deref(),
            Some("Watchpoint: write to $0200")
        );
    }

    #[test]
    fn run_stops_at_a_breakpoint_before_the_instruction() {
        let (mut atom, mut session) = at_rest();
        atom.bus.debug.add_breakpoint(0x2905);
        session.execute(&mut atom, Command::Run);
        assert!(session.running);
        assert!(session.run_cycles(&mut atom, 10_000));
        assert!(!session.running);
        assert_eq!(atom.cpu.regs.pc, 0x2905);
        assert_eq!(atom.bus.peek(0x0201), 0, "the JSR has not run");
        assert_eq!(session.stop_message.as_deref(), Some("Breakpoint at $2905"));
    }

    #[test]
    fn run_carries_on_from_a_breakpoint_and_stops_at_the_next_one() {
        let (mut atom, mut session) = at_rest();
        atom.bus.debug.add_breakpoint(0x2905);
        atom.bus.debug.add_breakpoint(0x290B);
        session.execute(&mut atom, Command::Run);
        session.run_cycles(&mut atom, 10_000);
        assert_eq!(atom.cpu.regs.pc, 0x2905);
        session.execute(&mut atom, Command::Run);
        assert!(session.stop_message.is_none(), "the message clears on Run");
        session.run_cycles(&mut atom, 10_000);
        assert_eq!(atom.cpu.regs.pc, 0x290B);
        assert_eq!(atom.bus.peek(0x0200), 6, "INC ran on the way");
        assert_eq!(atom.bus.peek(0x0201), 7, "and so did the subroutine");
    }

    #[test]
    fn a_cleared_breakpoint_no_longer_stops() {
        let (mut atom, mut session) = at_rest();
        atom.bus.debug.add_breakpoint(0x2905);
        assert!(atom.bus.debug.remove_breakpoint(0x2905));
        session.execute(&mut atom, Command::Run);
        assert!(!session.run_cycles(&mut atom, 10_000), "nothing to stop it");
        assert_eq!(atom.cpu.regs.pc, 0x290C, "it ends up in the JMP loop");
        assert!(
            !atom.bus.debug.remove_breakpoint(0x2905),
            "nothing left to remove"
        );
    }

    #[test]
    fn step_over_runs_a_whole_subroutine() {
        let (mut atom, mut session) = at_rest();
        session.execute(&mut atom, Command::StepN(2)); // up to the JSR
        session.execute(&mut atom, Command::StepOver);
        assert!(session.running, "step over runs until it gets back");
        session.run_cycles(&mut atom, 10_000);
        assert_eq!(atom.cpu.regs.pc, 0x2908, "the instruction after the JSR");
        assert_eq!(atom.bus.peek(0x0201), 7, "the subroutine ran");
        assert_eq!(session.stop_message.as_deref(), Some("Stopped at $2908"));
    }

    #[test]
    fn step_over_an_ordinary_instruction_is_just_a_step() {
        let (mut atom, mut session) = at_rest();
        session.execute(&mut atom, Command::StepOver);
        assert_eq!(atom.cpu.regs.pc, 0x2902);
        assert!(!session.running);
    }

    #[test]
    fn a_breakpoint_inside_a_stepped_over_subroutine_stops_there_and_cancels_the_step_over() {
        let (mut atom, mut session) = at_rest();
        session.execute(&mut atom, Command::StepN(2));
        atom.bus.debug.add_breakpoint(0x2912);
        session.execute(&mut atom, Command::StepOver);
        session.run_cycles(&mut atom, 10_000);
        assert_eq!(atom.cpu.regs.pc, 0x2912, "stopped inside the subroutine");
        // Carry on with a plain Run: it must NOT stop again at $2908, where
        // the abandoned step over would have ended.
        session.execute(&mut atom, Command::Run);
        assert!(!session.run_cycles(&mut atom, 10_000));
        assert_eq!(atom.cpu.regs.pc, 0x290C);
    }

    #[test]
    fn run_to_stops_at_the_address() {
        let (mut atom, mut session) = at_rest();
        session.execute(&mut atom, Command::RunTo(0x290B));
        session.run_cycles(&mut atom, 10_000);
        assert_eq!(atom.cpu.regs.pc, 0x290B);
        assert_eq!(atom.bus.peek(0x0200), 6);
        assert_eq!(session.stop_message.as_deref(), Some("Stopped at $290B"));
    }

    #[test]
    fn pause_cancels_a_run_to() {
        let (mut atom, mut session) = at_rest();
        session.execute(&mut atom, Command::RunTo(0x290B));
        session.execute(&mut atom, Command::Pause);
        assert!(!session.running);
        session.execute(&mut atom, Command::Run);
        assert!(!session.run_cycles(&mut atom, 10_000), "the target is gone");
    }

    #[test]
    fn frame_runs_about_one_video_frame_and_stops_at_a_breakpoint() {
        let (mut atom, mut session) = at_rest();
        let start = atom.bus.cycles;
        session.execute(&mut atom, Command::StepFrame);
        let used = atom.bus.cycles - start;
        assert!(
            (CYCLES_PER_FRAME..CYCLES_PER_FRAME + 10).contains(&used),
            "{used} cycles"
        );
        assert!(!session.running);

        let (mut atom, mut session) = at_rest();
        atom.bus.debug.add_breakpoint(0x2908);
        session.execute(&mut atom, Command::StepFrame);
        assert_eq!(atom.cpu.regs.pc, 0x2908);
        assert_eq!(session.stop_message.as_deref(), Some("Breakpoint at $2908"));
    }

    #[test]
    fn the_nmi_command_makes_the_cpu_take_the_nmi() {
        let (mut atom, mut session) = at_rest();
        // (There are no ROMs here, so the NMI vector is open bus: check only
        // that the request reaches the CPU. The handler is entered in
        // `nmi_button_makes_the_cpu_enter_the_nmi_handler`, in atom.rs.)
        session.execute(&mut atom, Command::Nmi);
        assert!(atom.bus.nmi_edge);
        session.execute(&mut atom, Command::Step);
        assert!(!atom.bus.nmi_edge, "the CPU saw it");
    }

    #[test]
    fn the_irq_button_holds_the_line_and_releases_it() {
        let (mut atom, mut session) = at_rest();
        assert!(!atom.bus.external_irq);
        session.execute(&mut atom, Command::ToggleIrq);
        assert!(atom.bus.external_irq);
        session.execute(&mut atom, Command::ToggleIrq);
        assert!(!atom.bus.external_irq);
    }

    #[test]
    fn reset_clears_the_stop_message() {
        let (mut atom, mut session) = at_rest();
        atom.bus.debug.add_breakpoint(0x2905);
        session.execute(&mut atom, Command::Run);
        session.run_cycles(&mut atom, 10_000);
        assert!(session.stop_message.is_some());
        session.execute(&mut atom, Command::Reset);
        assert!(session.stop_message.is_none());
    }

    // ---- Step out and conditional breakpoints ---------------------------------------

    /// Load bytes into RAM at the given addresses.
    fn load(atom: &mut Atom, code: &[(u16, &[u8])]) {
        for &(start, bytes) in code {
            for (i, &byte) in bytes.iter().enumerate() {
                assert!(atom.bus.poke(start + i as u16, byte));
            }
        }
    }

    /// Two routines, one calling the other:
    ///
    /// ```text
    /// 2900  JSR $2920
    /// 2903  NOP
    /// 2904  JMP $2904
    ///
    /// 2920  LDX #$01     routine A
    /// 2922  JSR $2930
    /// 2925  INX
    /// 2926  RTS
    ///
    /// 2930  LDY #$02     routine B
    /// 2932  RTS
    /// ```
    fn nested_calls() -> (Atom, DebugSession) {
        let mut atom = Atom::new();
        load(
            &mut atom,
            &[
                (0x2900, &[0x20, 0x20, 0x29, 0xEA, 0x4C, 0x04, 0x29]),
                (0x2920, &[0xA2, 0x01, 0x20, 0x30, 0x29, 0xE8, 0x60]),
                (0x2930, &[0xA0, 0x02, 0x60]),
            ],
        );
        atom.cpu.regs.pc = 0x2900;
        let session = DebugSession {
            running: false,
            stop_message: None,
        };
        (atom, session)
    }

    #[test]
    fn step_out_from_the_start_of_a_routine_runs_it_and_its_calls_and_stops_in_the_caller() {
        let (mut atom, mut session) = nested_calls();
        atom.cpu.regs.pc = 0x2920; // as if we had just stepped into routine A
        atom.cpu.regs.sp = 0xFB; // (a return address is on the stack: not needed to run)
        atom.bus.poke(0x01FC, 0x02);
        atom.bus.poke(0x01FD, 0x29);
        session.execute(&mut atom, Command::StepOut);
        assert!(session.running);
        assert!(session.run_cycles(&mut atom, 10_000));
        assert_eq!(atom.cpu.regs.pc, 0x2903, "back in the caller");
        assert_eq!(
            session.stop_message.as_deref(),
            Some("Stepped out to $2903")
        );
        assert_eq!(
            (atom.cpu.regs.x, atom.cpu.regs.y),
            (2, 2),
            "A ran, and so did B"
        );
        assert!(!session.running);
    }

    #[test]
    fn step_out_from_the_middle_of_a_routine() {
        let (mut atom, mut session) = nested_calls();
        session.execute(&mut atom, Command::StepN(4)); // JSR A, LDX, JSR B, LDY: now in B
        assert_eq!(atom.cpu.regs.pc, 0x2932);
        session.execute(&mut atom, Command::StepOut);
        session.run_cycles(&mut atom, 10_000);
        assert_eq!(atom.cpu.regs.pc, 0x2925, "out of B, back in A");
        session.execute(&mut atom, Command::StepOut);
        session.run_cycles(&mut atom, 10_000);
        assert_eq!(
            atom.cpu.regs.pc, 0x2903,
            "out of A, back in the main program"
        );
        assert_eq!(atom.cpu.regs.x, 2, "A finished its INX before returning");
    }

    #[test]
    fn step_out_at_the_rts_itself_stops_straight_after_it() {
        let (mut atom, mut session) = nested_calls();
        session.execute(&mut atom, Command::StepN(4)); // JSR, LDX, JSR, LDY
        assert_eq!(atom.cpu.regs.pc, 0x2932, "at the RTS of B");
        session.execute(&mut atom, Command::StepN(2)); // that RTS, then INX in A
        assert_eq!(atom.cpu.regs.pc, 0x2926);
        let before = atom.bus.cycles;
        session.execute(&mut atom, Command::StepOut);
        session.run_cycles(&mut atom, 10_000);
        assert_eq!(atom.cpu.regs.pc, 0x2903);
        assert!(atom.bus.cycles - before < 20, "just the RTS, no more");
    }

    #[test]
    fn step_out_with_no_caller_keeps_running_until_paused() {
        let (mut atom, mut session) = nested_calls();
        atom.cpu.regs.pc = 0x2903; // the main program: nothing to return to
        session.execute(&mut atom, Command::StepOut);
        assert!(!session.run_cycles(&mut atom, 10_000), "nothing to stop at");
        assert!(session.running);
        session.execute(&mut atom, Command::Pause);
        assert!(atom.bus.debug.step_out.is_none(), "Pause cancels it");
    }

    #[test]
    fn a_breakpoint_in_a_nested_call_stops_there_and_cancels_the_step_out() {
        let (mut atom, mut session) = nested_calls();
        session.execute(&mut atom, Command::StepN(2)); // JSR A, LDX #1: in A
        atom.bus.debug.add_breakpoint(0x2930);
        session.execute(&mut atom, Command::StepOut);
        session.run_cycles(&mut atom, 10_000);
        assert_eq!(atom.cpu.regs.pc, 0x2930, "stopped in B");
        assert!(
            atom.bus.debug.step_out.is_none(),
            "the step out is abandoned"
        );
        session.execute(&mut atom, Command::Run);
        assert!(
            !session.run_cycles(&mut atom, 10_000),
            "a plain Run does not stop at the old place"
        );
        assert_eq!(atom.cpu.regs.pc, 0x2904);
    }

    /// A loop that counts X up to 5 and keeps the count in memory:
    ///
    /// ```text
    /// 2900  LDX #$00
    /// 2902  INX
    /// 2903  STX $0200
    /// 2906  CPX #$05
    /// 2908  BNE $2902
    /// 290A  JMP $290A
    /// ```
    fn counting_loop() -> (Atom, DebugSession) {
        let mut atom = Atom::new();
        load(
            &mut atom,
            &[(
                0x2900,
                &[
                    0xA2, 0x00, 0xE8, 0x8E, 0x00, 0x02, 0xE0, 0x05, 0xD0, 0xF8, 0x4C, 0x0A, 0x29,
                ],
            )],
        );
        atom.cpu.regs.pc = 0x2900;
        (
            atom,
            DebugSession {
                running: false,
                stop_message: None,
            },
        )
    }

    #[test]
    fn a_conditional_breakpoint_stops_only_when_its_condition_holds() {
        let (mut atom, mut session) = counting_loop();
        atom.bus
            .debug
            .add_conditional_breakpoint(0x2906, Condition::parse("x==3").unwrap());
        session.execute(&mut atom, Command::Run);
        assert!(session.run_cycles(&mut atom, 10_000));
        assert_eq!(atom.cpu.regs.pc, 0x2906);
        assert_eq!(atom.cpu.regs.x, 3, "not the first or second pass");
        assert_eq!(session.stop_message.as_deref(), Some("Breakpoint at $2906"));
        // Carry on: x is never 3 again, so the loop runs out.
        session.execute(&mut atom, Command::Run);
        assert!(!session.run_cycles(&mut atom, 10_000));
        assert_eq!(atom.cpu.regs.pc, 0x290A);
    }

    #[test]
    fn a_condition_can_look_at_memory_and_combine_tests() {
        let (mut atom, mut session) = counting_loop();
        let condition = Condition::parse("[0200]==4 && x>=4").unwrap();
        atom.bus.debug.add_conditional_breakpoint(0x2906, condition);
        session.execute(&mut atom, Command::Run);
        assert!(session.run_cycles(&mut atom, 10_000));
        assert_eq!(atom.cpu.regs.x, 4);
        assert_eq!(atom.bus.peek(0x0200), 4);
    }

    #[test]
    fn a_condition_that_never_holds_never_stops() {
        let (mut atom, mut session) = counting_loop();
        atom.bus
            .debug
            .add_conditional_breakpoint(0x2906, Condition::parse("x==9").unwrap());
        session.execute(&mut atom, Command::Run);
        assert!(!session.run_cycles(&mut atom, 10_000));
        assert_eq!(atom.cpu.regs.pc, 0x290A);
    }

    #[test]
    fn a_plain_breakpoint_replaces_a_conditional_one_and_removing_clears_both() {
        let mut debugger = crate::machine::debug::Debugger::default();
        debugger.add_conditional_breakpoint(0x2906, Condition::parse("x==3").unwrap());
        assert_eq!(debugger.conditions.len(), 1);
        debugger.add_breakpoint(0x2906);
        assert!(debugger.conditions.is_empty(), "now it always stops");
        assert!(debugger.breakpoints.contains(&0x2906));
        debugger.add_conditional_breakpoint(0x2906, Condition::parse("x==3").unwrap());
        assert!(debugger.remove_breakpoint(0x2906));
        assert!(debugger.conditions.is_empty() && debugger.breakpoints.is_empty());
        debugger.add_conditional_breakpoint(0x10, Condition::parse("a==1").unwrap());
        debugger.toggle_breakpoint(0x10); // clicking the dot removes it
        assert!(debugger.conditions.is_empty() && debugger.breakpoints.is_empty());
    }

    // ---- The history of executed instructions ----------------------------------------

    #[test]
    fn nothing_is_recorded_unless_recording_is_on() {
        let (mut atom, mut session) = counting_loop();
        session.execute(&mut atom, Command::StepN(5));
        assert_eq!(atom.bus.debug.history_len(), 0);
    }

    #[test]
    fn the_history_remembers_each_instruction_with_the_registers_before_it() {
        let (mut atom, mut session) = counting_loop();
        atom.bus.debug.recording = true;
        session.execute(&mut atom, Command::StepN(4));
        let history: Vec<_> = atom.bus.debug.recent_history(100).cloned().collect();
        let addresses: Vec<u16> = history.iter().map(|e| e.regs.pc).collect();
        assert_eq!(addresses, [0x2900, 0x2902, 0x2903, 0x2906]);
        assert_eq!(
            history[0].bytes,
            [0xA2, 0x00, 0xE8],
            "LDX #0, and the byte after"
        );
        assert_eq!(history[2].regs.x, 1, "STX ran with X already 1");
        assert_eq!(
            history[1].regs.x, 0,
            "the registers are from before the INX"
        );
        assert!(history.windows(2).all(|w| w[0].cycle < w[1].cycle));
        assert!(history.iter().all(|e| !e.interrupt));
    }

    #[test]
    fn recent_history_gives_the_newest_few_oldest_first() {
        let (mut atom, mut session) = counting_loop();
        atom.bus.debug.recording = true;
        session.execute(&mut atom, Command::StepN(10));
        let last3: Vec<u16> = atom
            .bus
            .debug
            .recent_history(3)
            .map(|e| e.regs.pc)
            .collect();
        assert_eq!(last3.len(), 3);
        let all: Vec<u16> = atom
            .bus
            .debug
            .recent_history(1000)
            .map(|e| e.regs.pc)
            .collect();
        assert_eq!(all.len(), 10);
        assert_eq!(last3, all[7..], "the last three, in order");
        atom.bus.debug.clear_history();
        assert_eq!(atom.bus.debug.history_len(), 0);
    }

    #[test]
    fn an_interrupt_is_in_the_history_marked_as_one() {
        let (mut atom, mut session) = counting_loop();
        atom.bus.debug.recording = true;
        session.execute(&mut atom, Command::StepN(2));
        session.execute(&mut atom, Command::Nmi);
        session.execute(&mut atom, Command::StepN(1)); // takes the NMI, runs no instruction
        let last = atom.bus.debug.recent_history(1).next().unwrap().clone();
        assert!(last.interrupt, "the NMI entry is marked");
        assert_eq!(
            last.regs.pc, 0x2903,
            "it interrupted the instruction at $2903"
        );
    }

    #[test]
    fn the_history_keeps_only_the_most_recent_steps() {
        use crate::machine::debug::HISTORY_LENGTH;
        let mut atom = Atom::new();
        load(&mut atom, &[(0x2900, &[0x4C, 0x00, 0x29])]); // JMP $2900 forever
        atom.cpu.regs.pc = 0x2900;
        atom.bus.debug.recording = true;
        let mut session = DebugSession {
            running: false,
            stop_message: None,
        };
        session.execute(&mut atom, Command::StepN(HISTORY_LENGTH as u32 + 500));
        assert_eq!(atom.bus.debug.history_len(), HISTORY_LENGTH);
        let oldest = atom
            .bus
            .debug
            .recent_history(HISTORY_LENGTH)
            .next()
            .unwrap();
        assert!(oldest.cycle >= 500 * 3, "the first 500 were forgotten");
    }
}
