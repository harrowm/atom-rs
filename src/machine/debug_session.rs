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
                // A plain Run is just that: forget any "run to" or "step over"
                // that was waiting from before.
                atom.bus.debug.target = None;
                self.resume(atom);
            }
            Command::Pause => {
                self.running = false;
                atom.bus.debug.target = None;
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
        atom.bus.debug.target = None;
    }

    /// The run ended: if there was a reason, stop and say why. Whatever the
    /// reason, a one-shot target (from step over or run to) is spent: a
    /// breakpoint met on the way must not leave it waiting to stop the next
    /// Run at a place the user has forgotten about.
    fn note_stop(&mut self, atom: &mut Atom, reason: Option<StopReason>) {
        if let Some(reason) = reason {
            self.running = false;
            self.stop_message = Some(reason.to_string());
            atom.bus.debug.target = None;
        }
    }
}

#[cfg(test)]
mod tests {
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
}
