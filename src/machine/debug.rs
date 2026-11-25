//! Debugger support: breakpoints, watchpoints and the reason a run stopped.

use std::collections::{BTreeMap, BTreeSet};

/// Break when the CPU reads and/or writes an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watchpoint {
    pub addr: u16,
    pub on_read: bool,
    pub on_write: bool,
}

/// Why the emulator stopped running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// About to execute an instruction at a breakpoint address.
    Breakpoint(u16),
    /// A watched address was read or written.
    Watchpoint { addr: u16, write: bool },
    /// A "step over" or "run to" target was reached.
    ReachedTarget(u16),
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopReason::Breakpoint(addr) => write!(f, "Breakpoint at ${addr:04X}"),
            StopReason::Watchpoint { addr, write } => {
                let kind = if *write { "write to" } else { "read of" };
                write!(f, "Watchpoint: {kind} ${addr:04X}")
            }
            StopReason::ReachedTarget(addr) => write!(f, "Stopped at ${addr:04X}"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Debugger {
    pub breakpoints: BTreeSet<u16>,
    pub watchpoints: Vec<Watchpoint>,
    /// A one-shot breakpoint (used by "step over").
    pub target: Option<u16>,
    /// Set when the machine stops; the front end reads and clears it.
    pub stop: Option<StopReason>,
    /// Count accesses to addresses that nothing answers (see [`Debugger::unmapped`]).
    pub log_unmapped: bool,
    /// For each unmapped address touched while `log_unmapped` was on:
    /// `(reads, writes)`. Software that expects hardware or RAM we do not
    /// provide shows up here.
    pub unmapped: BTreeMap<u16, (u32, u32)>,
    /// When set, the next instruction runs even if it is at a breakpoint.
    /// Without this you could never resume from a breakpoint.
    pub skip_breakpoint_once: bool,
}

impl Debugger {
    pub fn toggle_breakpoint(&mut self, addr: u16) {
        if !self.breakpoints.remove(&addr) {
            self.breakpoints.insert(addr);
        }
    }

    /// Called by the bus for an access to unmapped memory (only when logging).
    pub fn note_unmapped(&mut self, addr: u16, write: bool) {
        let counts = self.unmapped.entry(addr).or_default();
        if write {
            counts.1 += 1;
        } else {
            counts.0 += 1;
        }
    }

    /// Called by the bus on every access (only if there are watchpoints).
    pub fn note_access(&mut self, addr: u16, write: bool) {
        let hit = self
            .watchpoints
            .iter()
            .any(|w| w.addr == addr && if write { w.on_write } else { w.on_read });
        if hit && self.stop.is_none() {
            self.stop = Some(StopReason::Watchpoint { addr, write });
        }
    }

    /// Is a stop due before executing the instruction at `pc`?
    pub fn check_before_instruction(&mut self, pc: u16) -> Option<StopReason> {
        if std::mem::take(&mut self.skip_breakpoint_once) {
            return None;
        }
        if self.target == Some(pc) {
            self.target = None;
            return Some(StopReason::ReachedTarget(pc));
        }
        self.breakpoints
            .contains(&pc)
            .then_some(StopReason::Breakpoint(pc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoint_stops_once_then_can_be_skipped() {
        let mut debugger = Debugger::default();
        debugger.toggle_breakpoint(0xC000);
        assert_eq!(
            debugger.check_before_instruction(0xC000),
            Some(StopReason::Breakpoint(0xC000))
        );
        debugger.skip_breakpoint_once = true;
        assert_eq!(debugger.check_before_instruction(0xC000), None);
        assert!(debugger.check_before_instruction(0xC000).is_some());
    }

    #[test]
    fn watchpoints_distinguish_reads_and_writes() {
        let mut debugger = Debugger::default();
        debugger.watchpoints.push(Watchpoint {
            addr: 0x10,
            on_read: false,
            on_write: true,
        });
        debugger.note_access(0x10, false);
        assert_eq!(debugger.stop, None);
        debugger.note_access(0x10, true);
        assert_eq!(
            debugger.stop,
            Some(StopReason::Watchpoint {
                addr: 0x10,
                write: true
            })
        );
    }
}
