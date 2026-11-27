//! Debugger support: breakpoints, watchpoints, access logging and the reason
//! a run stopped.

use std::collections::{BTreeMap, BTreeSet};

/// Break when the CPU reads and/or writes an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watchpoint {
    pub addr: u16,
    pub on_read: bool,
    pub on_write: bool,
}

/// One access noted by a logging watch (see [`Debugger::traces`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceHit {
    pub addr: u16,
    pub write: bool,
    /// The byte written. (Reads are noted before the byte is known.)
    pub value: Option<u8>,
    /// The cycle the access happened on.
    pub cycle: u64,
}

/// The most trace hits kept waiting to be shown.
const MAX_TRACE_LOG: usize = 1000;

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
    /// Watches that stop the machine when the address is used.
    pub watchpoints: Vec<Watchpoint>,
    /// Watches that only record the access, and let the program carry on.
    pub traces: Vec<Watchpoint>,
    trace_log: Vec<TraceHit>,
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

/// Add a watch for the reads and/or writes of `addr` to `list`. Watching what
/// is already watched changes nothing.
fn add_watch(list: &mut Vec<Watchpoint>, addr: u16, on_read: bool, on_write: bool) {
    match list.iter_mut().find(|w| w.addr == addr) {
        Some(existing) => {
            existing.on_read |= on_read;
            existing.on_write |= on_write;
        }
        None => list.push(Watchpoint {
            addr,
            on_read,
            on_write,
        }),
    }
}

/// Stop watching the reads and/or writes of `addr`. A watch left watching
/// nothing is dropped. Returns whether anything was removed.
fn remove_watch(list: &mut Vec<Watchpoint>, addr: u16, on_read: bool, on_write: bool) -> bool {
    let Some(index) = list.iter().position(|w| w.addr == addr) else {
        return false;
    };
    let watch = &mut list[index];
    let removed = (on_read && watch.on_read) || (on_write && watch.on_write);
    watch.on_read &= !on_read;
    watch.on_write &= !on_write;
    if !watch.on_read && !watch.on_write {
        list.remove(index);
    }
    removed
}

impl Debugger {
    pub fn toggle_breakpoint(&mut self, addr: u16) {
        if !self.breakpoints.remove(&addr) {
            self.breakpoints.insert(addr);
        }
    }

    pub fn add_breakpoint(&mut self, addr: u16) {
        self.breakpoints.insert(addr);
    }

    /// Returns whether there was a breakpoint to remove.
    pub fn remove_breakpoint(&mut self, addr: u16) -> bool {
        self.breakpoints.remove(&addr)
    }

    /// Stop when `addr` is read and/or written.
    pub fn add_watchpoint(&mut self, addr: u16, on_read: bool, on_write: bool) {
        add_watch(&mut self.watchpoints, addr, on_read, on_write);
    }

    pub fn remove_watchpoint(&mut self, addr: u16, on_read: bool, on_write: bool) -> bool {
        remove_watch(&mut self.watchpoints, addr, on_read, on_write)
    }

    /// Record (but do not stop for) reads and/or writes of `addr`.
    pub fn add_trace(&mut self, addr: u16, on_read: bool, on_write: bool) {
        add_watch(&mut self.traces, addr, on_read, on_write);
    }

    pub fn remove_trace(&mut self, addr: u16, on_read: bool, on_write: bool) -> bool {
        remove_watch(&mut self.traces, addr, on_read, on_write)
    }

    /// Is anything watching accesses? (The bus skips the checks if not.)
    pub fn watching(&self) -> bool {
        !self.watchpoints.is_empty() || !self.traces.is_empty()
    }

    /// The accesses recorded since the last call, oldest first.
    pub fn take_trace_log(&mut self) -> Vec<TraceHit> {
        std::mem::take(&mut self.trace_log)
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

    /// Called by the bus on every access (only if something is watching).
    /// `value` is the byte being written, for a write.
    pub fn note_access(&mut self, addr: u16, write: bool, value: Option<u8>, cycle: u64) {
        let matches = |w: &Watchpoint| w.addr == addr && if write { w.on_write } else { w.on_read };
        if self.watchpoints.iter().any(matches) && self.stop.is_none() {
            self.stop = Some(StopReason::Watchpoint { addr, write });
        }
        if self.traces.iter().any(matches) {
            if self.trace_log.len() == MAX_TRACE_LOG {
                self.trace_log.remove(0); // keep the newest
            }
            self.trace_log.push(TraceHit {
                addr,
                write,
                value,
                cycle,
            });
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
        debugger.note_access(0x10, false, None, 0);
        assert_eq!(debugger.stop, None);
        debugger.note_access(0x10, true, Some(1), 0);
        assert_eq!(
            debugger.stop,
            Some(StopReason::Watchpoint {
                addr: 0x10,
                write: true
            })
        );
    }

    #[test]
    fn breakpoints_are_added_removed_and_toggled() {
        let mut debugger = Debugger::default();
        debugger.add_breakpoint(0x2900);
        debugger.add_breakpoint(0x2900); // twice is still one
        assert_eq!(debugger.breakpoints.len(), 1);
        assert!(debugger.remove_breakpoint(0x2900));
        assert!(!debugger.remove_breakpoint(0x2900), "already gone");
        debugger.toggle_breakpoint(0x10);
        assert!(debugger.breakpoints.contains(&0x10));
        debugger.toggle_breakpoint(0x10);
        assert!(debugger.breakpoints.is_empty());
    }

    #[test]
    fn a_target_stops_once_and_beats_a_breakpoint_at_the_same_place() {
        let mut debugger = Debugger::default();
        debugger.add_breakpoint(0x2908);
        debugger.target = Some(0x2908);
        assert_eq!(
            debugger.check_before_instruction(0x2908),
            Some(StopReason::ReachedTarget(0x2908))
        );
        assert_eq!(debugger.target, None, "a target is used up");
        assert_eq!(
            debugger.check_before_instruction(0x2908),
            Some(StopReason::Breakpoint(0x2908))
        );
        assert_eq!(debugger.check_before_instruction(0x2909), None);
    }

    #[test]
    fn watching_the_same_address_again_merges_and_clearing_takes_one_side_at_a_time() {
        let mut debugger = Debugger::default();
        debugger.add_watchpoint(0x10, true, false);
        debugger.add_watchpoint(0x10, false, true);
        assert_eq!(
            debugger.watchpoints,
            vec![Watchpoint {
                addr: 0x10,
                on_read: true,
                on_write: true
            }]
        );
        assert!(debugger.remove_watchpoint(0x10, true, false));
        assert!(
            !debugger.remove_watchpoint(0x10, true, false),
            "the read side is gone"
        );
        assert!(debugger.watching(), "writes are still watched");
        assert!(debugger.remove_watchpoint(0x10, false, true));
        assert!(
            debugger.watchpoints.is_empty(),
            "nothing left, so the entry goes"
        );
        assert!(!debugger.watching());
        assert!(
            !debugger.remove_watchpoint(0x99, true, true),
            "never watched"
        );
    }

    #[test]
    fn a_watchpoint_reports_only_the_first_hit_until_the_stop_is_taken() {
        let mut debugger = Debugger::default();
        debugger.add_watchpoint(0x10, true, true);
        debugger.note_access(0x10, false, None, 1);
        debugger.note_access(0x10, true, Some(5), 2);
        assert_eq!(
            debugger.stop,
            Some(StopReason::Watchpoint {
                addr: 0x10,
                write: false
            }),
            "the first one wins"
        );
        debugger.note_access(0x11, true, Some(5), 3);
        assert_eq!(
            debugger.stop.take().map(|s| s.to_string()).as_deref(),
            Some("Watchpoint: read of $0010")
        );
    }

    #[test]
    fn traces_record_without_stopping_and_the_log_is_taken_in_order() {
        let mut debugger = Debugger::default();
        debugger.add_trace(0x0200, false, true);
        assert!(
            debugger.watching(),
            "a trace needs the bus to report accesses"
        );
        debugger.note_access(0x0200, true, Some(1), 10);
        debugger.note_access(0x0200, false, None, 11); // reads are not traced
        debugger.note_access(0x0200, true, Some(2), 12);
        assert_eq!(debugger.stop, None, "a trace never stops the machine");
        let log = debugger.take_trace_log();
        assert_eq!(
            log,
            vec![
                TraceHit {
                    addr: 0x0200,
                    write: true,
                    value: Some(1),
                    cycle: 10
                },
                TraceHit {
                    addr: 0x0200,
                    write: true,
                    value: Some(2),
                    cycle: 12
                },
            ]
        );
        assert!(debugger.take_trace_log().is_empty(), "taking empties it");
        assert!(debugger.remove_trace(0x0200, false, true));
        assert!(!debugger.watching());
    }

    #[test]
    fn the_trace_log_keeps_the_newest_when_it_fills() {
        let mut debugger = Debugger::default();
        debugger.add_trace(0x10, true, true);
        for cycle in 0..(MAX_TRACE_LOG as u64 + 50) {
            debugger.note_access(0x10, true, Some(0), cycle);
        }
        let log = debugger.take_trace_log();
        assert_eq!(log.len(), MAX_TRACE_LOG);
        assert_eq!(log.first().unwrap().cycle, 50);
        assert_eq!(log.last().unwrap().cycle, MAX_TRACE_LOG as u64 + 49);
    }

    #[test]
    fn unmapped_accesses_are_counted_per_address_and_direction() {
        let mut debugger = Debugger::default();
        debugger.note_unmapped(0x9F00, false);
        debugger.note_unmapped(0x9F00, false);
        debugger.note_unmapped(0x9F00, true);
        debugger.note_unmapped(0x9F01, true);
        assert_eq!(debugger.unmapped[&0x9F00], (2, 1));
        assert_eq!(debugger.unmapped[&0x9F01], (0, 1));
    }

    #[test]
    fn stop_reasons_read_well() {
        assert_eq!(
            StopReason::Breakpoint(0xC000).to_string(),
            "Breakpoint at $C000"
        );
        assert_eq!(
            StopReason::Watchpoint {
                addr: 0x10,
                write: false
            }
            .to_string(),
            "Watchpoint: read of $0010"
        );
        assert_eq!(
            StopReason::ReachedTarget(0x2908).to_string(),
            "Stopped at $2908"
        );
    }
}
