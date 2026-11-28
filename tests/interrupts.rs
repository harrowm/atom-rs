//! How the 6502 takes interrupts: BRK, IRQ and NMI, which flags are pushed,
//! when an interrupt is allowed to happen (the delay after CLI and SEI), and an
//! NMI that arrives in the middle of another interrupt's entry. The SingleStep
//! tests and Klaus Dormann's suites do not raise interrupts at chosen cycles,
//! so these are what pin the behaviour down.

use std::collections::VecDeque;

use atom_rs::cpu::{Bus, Cpu, RamBus, flags};

const IRQ_HANDLER: u16 = 0x3000;
const NMI_HANDLER: u16 = 0x4000;
const START: u16 = 0x2000;
const STACK: u16 = 0x0100;

/// RAM plus an IRQ line and NMI edges that happen at chosen cycles. Every read
/// and write is one cycle, so "cycle 3" means "after the third bus access".
struct ScriptedBus {
    memory: Box<[u8; 0x10000]>,
    cycle: u64,
    irq: bool,
    /// The cycle each NMI edge appears on, soonest first.
    nmi_edges: VecDeque<u64>,
}

impl ScriptedBus {
    fn new(program: &[u8]) -> Self {
        let mut bus = ScriptedBus {
            memory: Box::new([0; 0x10000]),
            cycle: 0,
            irq: false,
            nmi_edges: VecDeque::new(),
        };
        bus.memory[START as usize..START as usize + program.len()].copy_from_slice(program);
        // Vectors, and a harmless RTI at each handler.
        bus.memory[0xFFFA..0xFFFC].copy_from_slice(&NMI_HANDLER.to_le_bytes());
        bus.memory[0xFFFE..0x10000].copy_from_slice(&IRQ_HANDLER.to_le_bytes());
        bus.memory[IRQ_HANDLER as usize] = 0x40; // RTI
        bus.memory[NMI_HANDLER as usize] = 0x40;
        bus
    }
}

impl Bus for ScriptedBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.cycle += 1;
        self.memory[addr as usize]
    }
    fn write(&mut self, addr: u16, value: u8) {
        self.cycle += 1;
        self.memory[addr as usize] = value;
    }
    fn irq_asserted(&mut self) -> bool {
        self.irq
    }
    fn take_nmi_edge(&mut self) -> bool {
        if self.nmi_edges.front().is_some_and(|&at| at <= self.cycle) {
            self.nmi_edges.pop_front();
            true
        } else {
            false
        }
    }
}

/// A CPU at `START` with interrupts enabled (I clear) and the stack at $01FF.
fn cpu() -> Cpu {
    let mut cpu = Cpu::new();
    cpu.regs.pc = START;
    cpu.regs.sp = 0xFF;
    cpu.regs.p = flags::UNUSED;
    cpu
}

/// What the stack holds, from the top: (status, return address).
fn pushed(cpu: &Cpu, bus: &ScriptedBus, bytes_pushed: u8) -> (u8, u16) {
    assert_eq!(0xFF - cpu.regs.sp, bytes_pushed, "bytes pushed");
    let at = |offset: u8| bus.memory[(STACK + u16::from(cpu.regs.sp) + u16::from(offset)) as usize];
    (at(1), u16::from_le_bytes([at(2), at(3)]))
}

// ---- BRK -------------------------------------------------------------------------

#[test]
fn brk_pushes_the_address_two_on_and_a_status_with_b_set_then_masks_irqs() {
    let mut bus = ScriptedBus::new(&[0x00, 0xEA]); // BRK, (signature byte)
    let mut cpu = cpu();
    let cycles = cpu.step(&mut bus);
    assert_eq!(cycles, 7);
    assert_eq!(cpu.regs.pc, IRQ_HANDLER);
    let (status, ret) = pushed(&cpu, &bus, 3);
    assert_eq!(
        ret,
        START + 2,
        "BRK is two bytes: the return skips the signature"
    );
    assert_eq!(
        status & (flags::BREAK | flags::UNUSED),
        flags::BREAK | flags::UNUSED
    );
    assert!(cpu.regs.flag(flags::IRQ_DISABLE));
    assert!(
        !cpu.took_interrupt(),
        "BRK is an instruction, not an interrupt"
    );
}

#[test]
fn rti_after_brk_returns_past_the_signature_byte_and_restores_the_flags() {
    let mut bus = ScriptedBus::new(&[0x00, 0xEA, 0xEA]);
    let mut cpu = cpu();
    cpu.regs.p |= flags::CARRY | flags::DECIMAL;
    cpu.step(&mut bus); // BRK
    cpu.step(&mut bus); // RTI
    assert_eq!(cpu.regs.pc, START + 2);
    assert!(cpu.regs.flag(flags::CARRY) && cpu.regs.flag(flags::DECIMAL));
    assert!(
        !cpu.regs.flag(flags::IRQ_DISABLE),
        "I is restored to what it was"
    );
    assert!(!cpu.regs.flag(flags::BREAK), "B is not a real flag");
    assert_eq!(cpu.regs.sp, 0xFF);
}

// ---- IRQ ------------------------------------------------------------------------

#[test]
fn an_irq_is_taken_between_instructions_with_b_clear_and_the_return_address_exact() {
    let mut bus = ScriptedBus::new(&[0xEA, 0xEA]);
    let mut cpu = cpu();
    cpu.step(&mut bus); // NOP
    bus.irq = true;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cycles, 7, "an interrupt entry is seven cycles");
    assert!(cpu.took_interrupt());
    assert_eq!(cpu.regs.pc, IRQ_HANDLER);
    let (status, ret) = pushed(&cpu, &bus, 3);
    assert_eq!(ret, START + 1, "the interrupted instruction has not run");
    assert_eq!(status & flags::BREAK, 0, "an IRQ pushes B clear");
    assert!(cpu.regs.flag(flags::IRQ_DISABLE));
}

#[test]
fn an_irq_is_ignored_while_the_i_flag_is_set_and_then_taken_when_it_clears() {
    let mut bus = ScriptedBus::new(&[0xEA, 0xEA, 0x58, 0xEA, 0xEA]); // NOP NOP CLI NOP NOP
    let mut cpu = cpu();
    cpu.regs.p |= flags::IRQ_DISABLE;
    bus.irq = true; // held low the whole time: level-sensitive
    cpu.step(&mut bus);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.pc, START + 2, "masked: nothing happened");
    cpu.step(&mut bus); // CLI
    assert!(!cpu.regs.flag(flags::IRQ_DISABLE));
    assert!(!cpu.took_interrupt());
    cpu.step(&mut bus); // the NOP after CLI still runs...
    assert_eq!(
        cpu.regs.pc,
        START + 4,
        "CLI takes effect one instruction late"
    );
    cpu.step(&mut bus); // ...and then the IRQ is taken
    assert!(cpu.took_interrupt());
    assert_eq!(cpu.regs.pc, IRQ_HANDLER);
    let (_, ret) = pushed(&cpu, &bus, 3);
    assert_eq!(ret, START + 4);
}

#[test]
fn sei_does_not_stop_an_irq_that_is_already_pending() {
    // The mirror image of the CLI delay: the I flag is set by SEI, but the
    // interrupt that was already waiting is still taken right after it.
    let mut bus = ScriptedBus::new(&[0x78, 0xEA]); // SEI NOP
    let mut cpu = cpu();
    bus.irq = true;
    cpu.step(&mut bus); // SEI (poll saw I clear, before the instruction's last cycle)
    assert!(cpu.regs.flag(flags::IRQ_DISABLE));
    cpu.step(&mut bus);
    assert!(
        cpu.took_interrupt(),
        "the pending IRQ is taken before the NOP"
    );
    let (status, ret) = pushed(&cpu, &bus, 3);
    assert_eq!(ret, START + 1);
    assert_ne!(
        status & flags::IRQ_DISABLE,
        0,
        "the pushed status already has I set"
    );
}

#[test]
fn a_still_asserted_irq_is_taken_again_at_once_after_rti_restores_i_clear() {
    let mut bus = ScriptedBus::new(&[0xEA, 0xEA]);
    let mut cpu = cpu();
    cpu.step(&mut bus); // a NOP first: a new CPU does not poll for interrupts yet
    bus.irq = true;
    cpu.step(&mut bus); // IRQ entry
    cpu.step(&mut bus); // RTI: the handler never cleared the source
    assert_eq!(cpu.regs.pc, START + 1, "back where it was interrupted");
    assert!(!cpu.regs.flag(flags::IRQ_DISABLE));
    cpu.step(&mut bus); // RTI changes I at once, so the IRQ comes straight back
    assert!(cpu.took_interrupt());
    let (_, ret) = pushed(&cpu, &bus, 3);
    assert_eq!(ret, START + 1, "before the next instruction has run");
}

// ---- NMI ------------------------------------------------------------------------

#[test]
fn an_nmi_is_taken_even_with_i_set_pushes_b_clear_and_is_edge_triggered() {
    let mut bus = ScriptedBus::new(&[0xEA, 0xEA, 0xEA]);
    let mut cpu = cpu();
    cpu.regs.p |= flags::IRQ_DISABLE;
    bus.nmi_edges.push_back(0);
    let cycles = cpu.step(&mut bus);
    assert_eq!(cycles, 7);
    assert!(cpu.took_interrupt());
    assert_eq!(cpu.regs.pc, NMI_HANDLER);
    let (status, ret) = pushed(&cpu, &bus, 3);
    assert_eq!(ret, START);
    assert_eq!(status & flags::BREAK, 0);
    cpu.step(&mut bus); // RTI
    cpu.step(&mut bus); // the NOP: the one edge is used up, so no second NMI
    assert_eq!(cpu.regs.pc, START + 1);
    assert!(!cpu.took_interrupt());
}

#[test]
fn nmi_wins_when_an_irq_is_also_waiting() {
    let mut bus = ScriptedBus::new(&[0xEA]);
    let mut cpu = cpu();
    bus.irq = true;
    bus.nmi_edges.push_back(0);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.pc, NMI_HANDLER, "NMI has priority");
    cpu.step(&mut bus); // RTI
    cpu.step(&mut bus); // the IRQ, still asserted, follows
    assert_eq!(cpu.regs.pc, IRQ_HANDLER);
}

// ---- An NMI arriving during another interrupt's entry ---------------------------------

#[test]
fn an_nmi_during_brk_hijacks_the_vector_but_the_pushed_status_still_has_b_set() {
    let mut bus = ScriptedBus::new(&[0x00, 0xEA]);
    let mut cpu = cpu();
    bus.nmi_edges.push_back(3); // after the opcode and signature reads, during the pushes
    let cycles = cpu.step(&mut bus);
    assert_eq!(cycles, 7, "the hijacked entry takes the same seven cycles");
    assert_eq!(cpu.regs.pc, NMI_HANDLER, "the NMI vector was used");
    let (status, ret) = pushed(&cpu, &bus, 3);
    assert_eq!(ret, START + 2);
    assert_ne!(
        status & flags::BREAK,
        0,
        "it was a BRK when the status was pushed"
    );
    assert!(cpu.regs.flag(flags::IRQ_DISABLE));
    // The NMI has been served, not left pending for later.
    cpu.step(&mut bus); // RTI
    assert_eq!(cpu.regs.pc, START + 2);
    cpu.step(&mut bus);
    assert!(!cpu.took_interrupt(), "the edge was used up by the hijack");
}

#[test]
fn an_nmi_during_an_irq_entry_hijacks_it_with_b_clear() {
    let mut bus = ScriptedBus::new(&[0xEA, 0xEA]);
    let mut cpu = cpu();
    cpu.step(&mut bus); // a NOP first: a new CPU does not poll for interrupts yet
    bus.irq = true;
    bus.nmi_edges.push_back(bus.cycle + 3); // part way through the seven cycles of the IRQ entry
    cpu.step(&mut bus);
    assert!(cpu.took_interrupt());
    assert_eq!(cpu.regs.pc, NMI_HANDLER, "the NMI vector was used");
    let (status, ret) = pushed(&cpu, &bus, 3);
    assert_eq!(ret, START + 1);
    assert_eq!(
        status & flags::BREAK,
        0,
        "it was an IRQ when the status was pushed"
    );
}

#[test]
fn an_nmi_that_arrives_after_the_vector_has_been_chosen_waits_for_the_next_step() {
    let mut bus = ScriptedBus::new(&[0x00, 0xEA]);
    let mut cpu = cpu();
    bus.nmi_edges.push_back(6); // during the vector fetch, too late to hijack
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.pc, IRQ_HANDLER, "the BRK went to its own vector");
    cpu.step(&mut bus); // so the NMI is taken at the next step
    assert!(cpu.took_interrupt());
    assert_eq!(cpu.regs.pc, NMI_HANDLER);
}

#[test]
fn a_second_nmi_during_an_nmi_entry_is_not_a_hijack_and_is_taken_straight_after() {
    let mut bus = ScriptedBus::new(&[0xEA]);
    let mut cpu = cpu();
    bus.nmi_edges.push_back(0);
    bus.nmi_edges.push_back(3);
    cpu.step(&mut bus); // first NMI entry; the second edge arrives during it
    assert_eq!(cpu.regs.pc, NMI_HANDLER);
    assert_eq!(0xFF - cpu.regs.sp, 3);
    cpu.step(&mut bus); // and now the second NMI is taken, before the handler's first instruction
    assert!(cpu.took_interrupt());
    assert_eq!(0xFF - cpu.regs.sp, 6, "two nested entries on the stack");
}

// ---- JAM and the defaults --------------------------------------------------------

#[test]
fn a_jam_opcode_locks_the_cpu_until_reset() {
    let mut bus = ScriptedBus::new(&[0x02, 0xEA]);
    let mut cpu = cpu();
    cpu.step(&mut bus); // JAM
    let pc = cpu.regs.pc;
    for _ in 0..3 {
        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, 1, "a jammed CPU just reads $FFFF");
        assert_eq!(cpu.regs.pc, pc, "and goes nowhere");
    }
    bus.irq = true; // not even an interrupt gets it out
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.pc, pc);
    cpu.reset(&mut bus);
    assert_ne!(cpu.regs.pc, pc, "reset frees it");
    cpu.regs.pc = START + 1;
    assert_eq!(cpu.step(&mut bus), 2, "and it runs instructions again");
}

#[test]
fn the_defaults_are_the_power_on_state() {
    let cpu = Cpu::default();
    assert_eq!(cpu.regs.sp, 0xFD);
    assert!(cpu.regs.flag(flags::IRQ_DISABLE));
    let bus = RamBus::default();
    assert!(bus.memory.iter().all(|&b| b == 0));
}
