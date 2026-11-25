//! Klaus Dormann's 6502 functional tests
//! (https://github.com/Klaus2m5/6502_65C02_functional_tests).
//!
//! Each test is a 64 KiB memory image containing a long program that
//! exercises every instruction and reports through its program counter: on
//! failure it jumps to itself forever at the address of the failed check; on
//! success it does the same at a known "success" address. So we run until the
//! PC stops changing and see where it stopped.
//!
//! Get the data with `scripts/fetch_tests.sh`.

use std::path::Path;

use atom_rs::cpu::{Bus, Cpu, RamBus};

const TEST_DIR: &str = "testdata/6502_65C02_functional_tests/bin_files";

/// Safety net so a broken CPU cannot loop forever without a trap.
const MAX_INSTRUCTIONS: u64 = 200_000_000;

fn load_image(memory: &mut [u8], image: &Path) {
    let bytes = std::fs::read(image).unwrap_or_else(|e| panic!("{image:?}: {e}"));
    memory.copy_from_slice(&bytes);
}

/// Start at `start`, and run until the CPU
/// traps. Returns the trap address and the number of cycles executed.
fn run_until_trap(bus: &mut impl Bus, cpu_start: u16) -> (u16, u64) {
    let start = cpu_start;
    let mut cpu = Cpu::new();
    cpu.regs.pc = start;
    for _ in 0..MAX_INSTRUCTIONS {
        let pc_before = cpu.regs.pc;
        cpu.step(bus);
        if cpu.regs.pc == pc_before {
            return (pc_before, cpu.cycles);
        }
    }
    panic!(
        "no trap after {MAX_INSTRUCTIONS} instructions; PC = {:#06x}",
        cpu.regs.pc
    );
}

#[test]
fn functional_test() {
    let image = Path::new(TEST_DIR).join("6502_functional_test.bin");
    if !image.exists() {
        eprintln!("SKIPPED: {image:?} not found. Run scripts/fetch_tests.sh first.");
        return;
    }
    // Entry point and success address come from the assembler listing
    // (6502_functional_test.lst).
    const START: u16 = 0x0400;
    const SUCCESS: u16 = 0x3469;

    let mut bus = RamBus::new();
    load_image(&mut bus.memory[..], &image);
    let (trap, cycles) = run_until_trap(&mut bus, START);
    assert_eq!(
        trap, SUCCESS,
        "stuck at {trap:#06x}; look this address up in the .lst file to see which check failed"
    );
    eprintln!("functional test passed after {cycles} cycles");
}

/// Bruce Clark's exhaustive decimal-mode test (every ADC and SBC combination
/// of operands and carry, checking A, N, V, Z and C).
///
/// The shipped Dormann repo only has the source, so `scripts/fetch_tests.sh`
/// builds a binary with ca65 from the ca65 port of the source.
#[test]
fn decimal_test() {
    let image = Path::new("testdata/amb5l_ca65/ca65/6502_decimal_test.bin");
    if !image.exists() {
        eprintln!("SKIPPED: {image:?} not found. Run scripts/fetch_tests.sh (needs ca65).");
        return;
    }
    const START: u16 = 0x0400;
    const DONE: u16 = 0x044B; // the `jmp *` at the DONE label in the .lst
    const ERROR_FLAG: usize = 0x000B; // zero page; 0 = passed, 1 = failed

    let mut bus = RamBus::new();
    load_image(&mut bus.memory[..], image);
    let (trap, cycles) = run_until_trap(&mut bus, START);
    assert_eq!(trap, DONE, "unexpected trap at {trap:#06x}");
    assert_eq!(
        bus.memory[ERROR_FLAG], 0,
        "decimal mode result differs from Bruce Clark's model"
    );
    eprintln!("decimal test passed after {cycles} cycles");
}

/// Dormann's interrupt test. It needs a "feedback register" at $BFFC: the
/// program writes there to raise and drop IRQ (bit 0) and NMI (bit 1), then
/// checks that the CPU services them correctly.
struct InterruptTestBus {
    memory: Box<[u8; 0x10000]>,
    feedback: u8,
    nmi_edge_seen: bool,
}

impl InterruptTestBus {
    const FEEDBACK_PORT: u16 = 0xBFFC;
    const IRQ_BIT: u8 = 0x01;
    const NMI_BIT: u8 = 0x02;
    /// Bit 7 is for "diagnostic stop" in emulators; ignore it.
    const FILTER: u8 = 0x7F;
}

impl Bus for InterruptTestBus {
    fn read(&mut self, addr: u16) -> u8 {
        if addr == Self::FEEDBACK_PORT {
            self.feedback & Self::FILTER
        } else {
            self.memory[addr as usize]
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if addr == Self::FEEDBACK_PORT {
            let new = value & Self::FILTER;
            // NMI is edge-triggered: remember a 0 -> 1 transition.
            if new & Self::NMI_BIT != 0 && self.feedback & Self::NMI_BIT == 0 {
                self.nmi_edge_seen = true;
            }
            self.feedback = new;
        } else {
            self.memory[addr as usize] = value;
        }
    }

    fn irq_asserted(&mut self) -> bool {
        self.feedback & Self::IRQ_BIT != 0
    }

    fn take_nmi_edge(&mut self) -> bool {
        std::mem::take(&mut self.nmi_edge_seen)
    }
}

/// This uses a prebuilt image from the mre/mos6502 project, a ca65 port of
/// Dormann's source. Its one change is that the "concurrent BRK and NMI" check
/// is switched off (the original is known to depend on real-chip timing).
#[test]
fn interrupt_test() {
    let image = Path::new("testdata/mre_interrupt/6502_interrupt_test.bin");
    if !image.exists() {
        eprintln!("SKIPPED: {image:?} not found. Run scripts/fetch_tests.sh first.");
        return;
    }
    const START: u16 = 0x0400;
    const SUCCESS: u16 = 0x06E8;

    let mut bus = InterruptTestBus {
        memory: Box::new([0; 0x10000]),
        feedback: 0,
        nmi_edge_seen: false,
    };
    load_image(&mut bus.memory[..], image);
    // The test's uninitialised data (zero page $0A-$0F, $0200-$0203) must start at zero.
    bus.memory[0x0A..=0x0F].fill(0);
    bus.memory[0x0200..=0x0203].fill(0);

    let (trap, cycles) = run_until_trap(&mut bus, START);
    assert_eq!(trap, SUCCESS, "stuck at {trap:#06x}");
    eprintln!("interrupt test passed after {cycles} cycles");
}
