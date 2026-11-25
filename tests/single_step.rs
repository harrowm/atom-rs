//! Runs the SingleStepTests suite (https://github.com/SingleStepTests/65x02)
//! against the CPU core.
//!
//! Each test gives the full processor and memory state before and after ONE
//! instruction, plus the exact bus activity of every cycle. We check all
//! three, so this verifies cycle-accuracy and not just final results.
//!
//! Get the data with `scripts/fetch_tests.sh`. To run a single opcode:
//!
//! ```text
//! OPCODE=a9 cargo test --test single_step -- --nocapture
//! ```

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use atom_rs::cpu::{Bus, Cpu};
use serde::Deserialize;

const TEST_DIR: &str = "testdata/65x02/6502/v1";

/// The processor/memory state as stored in the JSON files.
#[derive(Deserialize)]
struct State {
    pc: u16,
    s: u8,
    a: u8,
    x: u8,
    y: u8,
    p: u8,
    ram: Vec<(u16, u8)>,
}

#[derive(Deserialize)]
struct TestCase {
    name: String,
    initial: State,
    #[serde(rename = "final")]
    expected: State,
    /// `(address, value, "read" | "write")` for every cycle.
    cycles: Vec<(u16, u8, String)>,
}

/// 64 KiB of RAM that records every bus access.
struct RecordingBus {
    memory: Vec<u8>,
    log: Vec<(u16, u8, &'static str)>,
}

impl Bus for RecordingBus {
    fn read(&mut self, addr: u16) -> u8 {
        let value = self.memory[addr as usize];
        self.log.push((addr, value, "read"));
        value
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.memory[addr as usize] = value;
        self.log.push((addr, value, "write"));
    }
}

/// Run one test case; `Err` describes the first difference found.
fn run_case(bus: &mut RecordingBus, case: &TestCase) -> Result<(), String> {
    let initial = &case.initial;
    for &(addr, value) in &initial.ram {
        bus.memory[addr as usize] = value;
    }
    bus.log.clear();

    let mut cpu = Cpu::new();
    cpu.regs.pc = initial.pc;
    cpu.regs.sp = initial.s;
    cpu.regs.a = initial.a;
    cpu.regs.x = initial.x;
    cpu.regs.y = initial.y;
    cpu.regs.p = initial.p;
    cpu.step(bus);

    let result = compare(&cpu, bus, case);

    // Put memory back to all zeroes for the next case.
    for &(addr, _) in initial.ram.iter().chain(&case.expected.ram) {
        bus.memory[addr as usize] = 0;
    }
    for &(addr, _, _) in &bus.log {
        bus.memory[addr as usize] = 0;
    }
    result
}

fn compare(cpu: &Cpu, bus: &RecordingBus, case: &TestCase) -> Result<(), String> {
    let r = &cpu.regs;
    let e = &case.expected;
    let actual = (r.pc, r.sp, r.a, r.x, r.y, r.p);
    let wanted = (e.pc, e.s, e.a, e.x, e.y, e.p);
    if actual != wanted {
        return Err(format!(
            "registers (pc,s,a,x,y,p): got {actual:?}, want {wanted:?}"
        ));
    }
    for &(addr, value) in &e.ram {
        if bus.memory[addr as usize] != value {
            return Err(format!(
                "memory[{addr:#06x}]: got {:#04x}, want {value:#04x}",
                bus.memory[addr as usize]
            ));
        }
    }
    let wanted_cycles: Vec<_> = case
        .cycles
        .iter()
        .map(|(a, v, kind)| (*a, *v, kind.as_str()))
        .collect();
    if bus.log != wanted_cycles {
        return Err(format!(
            "bus cycles:\n    got  {:?}\n    want {wanted_cycles:?}",
            bus.log
        ));
    }
    Ok(())
}

/// Run every case in one opcode's file. Returns (passed, first failure message).
fn run_opcode(opcode: u8) -> (usize, Option<String>) {
    let path = Path::new(TEST_DIR).join(format!("{opcode:02x}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let cases: Vec<TestCase> = serde_json::from_str(&text).expect("valid JSON");

    let mut bus = RecordingBus {
        memory: vec![0; 0x10000],
        log: Vec::new(),
    };
    let mut passed = 0;
    let mut first_failure = None;
    for case in &cases {
        match run_case(&mut bus, case) {
            Ok(()) => passed += 1,
            Err(why) => {
                first_failure.get_or_insert(format!("  {}: {why}", case.name));
            }
        }
    }
    (passed, first_failure)
}

#[test]
fn single_step_tests() {
    if !Path::new(TEST_DIR).exists() {
        eprintln!("SKIPPED: {TEST_DIR} not found. Run scripts/fetch_tests.sh first.");
        return;
    }

    let opcodes: Vec<u8> = match std::env::var("OPCODE") {
        Ok(hex) => vec![u8::from_str_radix(&hex, 16).expect("OPCODE must be hex, e.g. a9")],
        Err(_) => (0..=255).collect(),
    };

    // One thread per CPU core, each taking the next unclaimed opcode.
    let next = AtomicUsize::new(0);
    let failures = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some(&opcode) = opcodes.get(next.fetch_add(1, Ordering::SeqCst)) {
                    let (passed, first_failure) = run_opcode(opcode);
                    if let Some(message) = first_failure {
                        let summary = format!("opcode {opcode:02x}: {passed} passed\n{message}");
                        failures.lock().unwrap().push((opcode, summary));
                    }
                }
            });
        }
    });

    let mut failures = failures.into_inner().unwrap();
    failures.sort();
    for (_, summary) in &failures {
        eprintln!("{summary}");
    }
    assert!(
        failures.is_empty(),
        "{} of {} opcodes failed",
        failures.len(),
        opcodes.len()
    );
}
