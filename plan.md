# Acorn Atom Emulator in Rust: Plan

A learning project. Readability beats cleverness everywhere. Every module gets a doc comment explaining the hardware it models.

## 1. Requirements (as understood)

| # | Requirement | Notes |
|---|-------------|-------|
| R1 | Acorn Atom emulator in Rust, standard tooling | `cargo new`, `cargo fmt`, `cargo clippy`, `cargo test` |
| R2 | CPU first, fully validated before anything else | Must pass SingleStepTests and Dormann tests |
| R3 | Memory map and peripherals modelled on Atomulator | Atomulator source is the reference |
| R4 | Cycle-accurate | Every CPU bus cycle ticks the rest of the machine |
| R5 | Separate debug windows | CPU/registers, memory, breakpoints, disassembly. Set registers, breakpoints, single-step |
| R6 | Rust-native graphics, sound, file dialogs | `eframe`/`egui`, `cpal`, `rfd` |
| R7 | Very readable code | Small functions, descriptive names, comments on the *why* |

### Points to confirm or note

- **6502 vs 6503.** The Atom uses a **6502** (6502A, 1 MHz). The 6503 is the same core in a 28-pin package with only 12 address lines. I'll write a 6502 core, which is what both the test suites and Atomulator target. The 6503 is just a 4 KB address mask, so it can be a trivial option later.
- **NMOS 6502 only** (no 65C02). The Atom never used one. Undocumented opcodes will be implemented because SingleStepTests covers them.
- **Decimal mode** is required. Dormann's decimal test and the Atom's BASIC both use it.
- **Test data** is external. SingleStepTests `65x02` is about 10k tests per opcode as JSON. Dormann's binaries are `6502_functional_test.bin` and `6502_interrupt_test.bin`, with the sources and `.lst` files. These are downloaded, not committed (see §5).
- **Dormann binaries.** The repo ships prebuilt binaries only for the functional test. The decimal and interrupt tests are shipped as `.a65` sources, so they must be assembled with Dormann's `as65` (included as `as65_142.zip`; it is an x86 build, so check it runs on this Mac) or with another assembler. This will be handled at the start of M2.

## 2. Architecture

A **single Cargo crate** with clearly separated modules. The `cpu` module must never import from `machine` or `ui`. That rule keeps the CPU testable on its own, and it means the code can be split into separate crates later if that is ever worthwhile.

```
atom-rs/
├── Cargo.toml
├── plan.md
├── scripts/fetch_tests.sh     # downloads external test data
├── testdata/                  # git-ignored: SingleStepTests, Dormann
├── roms/                      # git-ignored: user-supplied Atom ROMs
├── src/
│   ├── main.rs                # starts the app
│   ├── lib.rs                 # declares the modules below
│   ├── cpu/                   # pure 6502, no I/O, no UI
│   │   ├── mod.rs             # `Cpu`: tick()/step()
│   │   ├── bus.rs             # `Bus` trait: read(addr), write(addr, val)
│   │   ├── registers.rs       # A, X, Y, SP, PC, status flags
│   │   ├── opcodes.rs         # table: mnemonic, addressing mode, base cycles
│   │   ├── addressing.rs      # addressing modes, cycle by cycle
│   │   ├── instructions.rs    # ALU, loads, branches, etc.
│   │   └── disasm.rs          # shared with the debugger
│   ├── machine/               # the Atom: memory map and peripherals
│   │   └── {memory,ppi8255,via6522,vdg6847,keyboard,tape,sound,machine}.rs
│   └── ui/                    # eframe UI, cpal audio, rfd dialogs
│       └── {app,display,audio}.rs, debug/{cpu_window,memory_window,breakpoints,disasm_window}.rs
└── tests/
    ├── single_step.rs
    └── dormann.rs
```

### Key design decision: how to be cycle-accurate

The `Bus` trait is the only way the CPU touches the outside world. Each `read` or `write` is exactly one CPU cycle. The CPU does the real bus accesses of each instruction in order, including the dummy reads and writes (e.g. the extra read on a page-cross, the double write in read-modify-write instructions).

- This design matches SingleStepTests, which record the exact bus activity of every cycle (`[address, value, "read"|"write"]`). Comparing the full cycle list gives accuracy testing for free.
- The `Machine` implements `Bus`. On every access it first advances the peripherals by one cycle (VIA timers, video position, audio), then does the access. So the CPU drives the clock and everything stays in lock-step.
- Implementation style: **instruction-level code that performs per-cycle bus accesses**, not a micro-op state machine. It is far easier to read and learn from, and it is still cycle-exact on the bus. A `tick`-per-cycle interface can be layered on later if the debugger needs mid-instruction stepping. Interrupt and reset sequences are handled as pseudo-instructions.

## 3. Milestones

Each milestone ends with `cargo test`, `cargo clippy`, and `cargo fmt --check` clean.

### M0: Project setup, DONE
- `cargo init` a single crate (library plus binary), and add `rustfmt` and `clippy` settings.
- Test-data fetch script (`scripts/fetch_tests.sh`) that downloads SingleStepTests and Dormann into a git-ignored `testdata/`.
- Basic CI-style `Makefile` or `cargo xtask`, kept simple.

### M1: 6502 core (library only), DONE
1. Registers and flags, the `Bus` trait, and a trivial 64 KB `RamBus` for tests.
2. Opcode table with mode, mnemonic, and cycle counts.
3. Addressing modes with the correct dummy cycles and page-crossing behaviour.
4. Official instructions in groups: loads/stores, transfers, ALU, shifts/RMW, branches, stack, flow.
5. Binary and **decimal** ADC/SBC (NMOS flag behaviour).
6. Interrupts: reset, IRQ, NMI, BRK, including NMI hijacking of BRK/IRQ and the I-flag latency of CLI/SEI/PLP.
7. Undocumented opcodes (LAX, SAX, DCP, ISC, SLO, RLA, SRE, RRA, ANC, ALR, ARR, XAA, AHX, TAS, SHX, SHY, LAS, SBX) and the JAM/KIL opcodes.

### M2: CPU validation (gate: must pass fully before M3)

*Status: DONE. SingleStepTests pass for all 256 opcodes (about 2.5 million cases, bus cycles included). Dormann functional test passes (96,241,367 cycles, matching the documented count), Bruce Clark's decimal test passes, and the interrupt test passes. The decimal test is built with ca65 (`brew install cc65`) because Dormann's `as65` is x86-only. The interrupt test is a prebuilt image from mre/mos6502 with the concurrent BRK+NMI check switched off; see `scripts/fetch_tests.sh`.*
- **SingleStepTests `65x02/6502/v1`**: a test harness loads each `XX.json`, sets the initial state, runs one instruction on a recording bus, and compares final registers, RAM, **and the per-cycle bus trace**. Report failures grouped by opcode. Handle the known JAM opcodes and anything that file's README says to skip.
- **Dormann `6502_functional_test`**: run until the PC stops advancing (a branch-to-self trap). Pass means reaching the success address from the `.lst`. Failure reports the trap address, so we can find the failing test in the listing.
- **Dormann decimal test** (`6502_decimal_test`) and **interrupt test** (`6502_interrupt_test`, which needs a bus with a feedback register to drive IRQ/NMI).
- A tiny per-instruction **cycle counter** check against known totals (Dormann functional run has a documented cycle count, handy as a regression check).
- Tests marked `#[ignore]` when the data is not downloaded, with a clear message.

### M3: Atom machine (headless), DONE
Based on Atomulator. I'll read its source (`atom.c`, `6502.c`, `via.c`, `8255.c`, `6847.c`) and pin the map down before writing it.

| Range | Device |
|-------|--------|
| `0000–03FF` | RAM (always present) |
| `2000–7FFF` | RAM (Atomulator's default 25K configuration) |
| `8000–9FFF` | Video RAM (6847) |
| `A000–AFFF` | Utility ROM socket |
| `B000–B3FF` | 8255 PPI (decoded on `addr & 3`) |
| `B400–B7FF` | AtoMMC (not yet emulated) |
| `B800–BBFF` | 6522 VIA (decoded on `addr & 15`) |
| `BC00–BFFF` | SID / RAMROM / GDOS (not yet emulated) |
| `C000–CFFF` | BASIC ROM |
| `D000–DFFF` | Floating point ROM |
| `E000–EFFF` | Disk ROM socket |
| `F000–FFFF` | MOS kernel ROM |

*Verified against Atomulator's `6502.c`. Unmapped reads return the high byte of the address; unmapped writes are ignored.*

- Memory with RAM/ROM regions and ROM loading from files.
- **8255 PPI**: keyboard matrix scan, VDG mode lines, 2.4 kHz tone, cassette out.
- **6522 VIA**: timers, interrupts, registers.
- **6847 VDG** timing model: 60 Hz frame, per-scanline fetch (and the FS/HS signals the PPI reads).
- Boot to the Atom `>` prompt in a headless test and check video RAM contents.

*Status: DONE. The Atom boots to `ACORN ATOM` and `>` from the real ROMs, and BASIC runs a command typed through the keyboard matrix (`tests/boot.rs`). The ROMs come from Atomulator's `roms/` folder, copied into the git-ignored `roms/`. The 8255, 6522 VIA (timers, ports, interrupts) and 6847 timing (262 lines x 64 cycles, FS low on lines 192-223) are in `src/machine/`. Still to come: the Atom's 6847 pixel rendering (M4), tape (M6), and the optional disk and AtoMMC interfaces.*

### M4: Frontend (eframe/egui), mostly DONE
- Main window rendering the 6847 output as a texture, scaled and pixel-accurate, with all 8 modes (text, semigraphics, CLEAR 1–4 graphics).
- Keyboard mapping from host keys to the Atom matrix (Shift, Ctrl, Rept, Break).
- Emulation loop paced to 1 MHz against wall-clock, run per video frame. Pause, reset, and speed control.
- Menu with `rfd` file dialogs: load ROM, load tape/`.tap`/`.uef`/`.atm` files, and load a snapshot.

*Status: the window works (confirmed by running it): the 6847 picture is drawn per scanline in `src/machine/video.rs` (text, semigraphics and all 8 graphics modes), shown at integer scale, with the host keyboard mapped by position in `src/ui/keymap.rs` (F12 = BREAK/reset, Alt = REPT). Menu: load a ROM into a socket or choose a ROM folder (rfd), reset, pause, speed 0.1x-10x. Not done yet: loading tape/`.uef`/`.atm`/snapshot files, which belong with the tape work in M6.*

### M5: Debugger windows (egui viewports, so they are real separate windows), DONE
- **CPU window**: registers and flags, editable inline; run, pause, step instruction, step cycle, and reset/IRQ/NMI buttons; cycle counter.
- **Disassembly window**: follows PC; click a line to toggle a breakpoint.
- **Memory window**: hex and ASCII, editable, jump to address, RAM/ROM/IO colouring.
- **Breakpoints window**: exec, read/write watchpoints, and optional conditions. The `Bus` wrapper checks watchpoints.
- **Peripheral windows**: VIA, PPI, and VDG state, added as peripherals arrive.
- Debug state lives in the `Machine` and is only *read* by the UI, to keep the borrow story simple.

*Status: DONE, except single-cycle stepping. Five separate OS windows (CPU/controls, disassembly, memory, breakpoints, hardware) opened from the Debug menu or with `--debug`. Stepping is per instruction (Step, Step over, Run to here, one frame), because the CPU runs whole instructions; stepping a single cycle needs a CPU that can pause mid-instruction (a micro-op or coroutine rewrite), which is left as a possible later task. Breakpoints are checked between instructions; watchpoints are checked on every bus access and stop after the instruction that touched the address.*

### M6: Sound and tape, DONE
- `cpal` output stream with a ring buffer. The 1-bit speaker is sampled per CPU cycle, then filtered and resampled to the device rate.
- Cassette: read UEF/WAV to produce the tape-in bit, with 1200/2400 Hz tone generation for save. Fast-load option.

*Status: DONE. Sound: the speaker bit is averaged per output sample (`machine/sound.rs`) and played through cpal with a small queue between the emulator and the sound card (`ui/audio.rs`); muted above about 1x. Tape: UEF playback (plain or gzip) at 300 baud, tape recording by decoding what the OS writes to the cassette output (the OS makes 1 bits and the leader with the hardware 2400 Hz tone, 0 bits by toggling port C bit 0 at 1200 Hz), UEF writing, fast-tape option, and `.atm` loading into RAM. `tests/tape.rs` proves the whole chain with the real ROMs: `*SAVE`, wipe memory, `*LOAD`, memory restored. Not done: WAV/CSW tape files, and hearing the tape signal through the speaker (Atomulator can mix it in). Audio output could only be checked to the point of opening the device without errors, not by ear.*

### M7: Polish and extras, mostly DONE
- Disk interface (the 8271 and `.ssd`/`.dsd` images): DONE.
- Save states: DONE.
- Not done: SID and RAM-expansion variants, single-cycle CPU stepping, and finer 6847 artefacts (video is already drawn per scanline from the mode bits at that moment, but not per character clock).

*Status: the disk system (`machine/fdc8271.rs`, `machine/disk.rs`) models the 8271's command set and its NMI-per-byte data transfers, with one byte every 64 cycles. Disk rotation is not modelled (a sector starts a fixed time after the head arrives) and CRC errors cannot happen. `tests/disk.rs` proves it with the real DOS ROM: `*CAT`, `*LOAD` from a disk built in the test, and `*SAVE` then `*LOAD` on a blank disk. Save states (`machine/snapshot.rs`) are gzipped JSON of the CPU, RAM, chips, disks and tape; a restored machine runs identically to the original (tested). Note the `cpu` module now uses `serde` for its derives, so it is no longer dependency-free.*

### Command-line start-up, DONE
`--disk`, `--disk1`, `--tape`, `--state`, `--run` and `--fast-tape` (`src/cli.rs`). A typist (`machine/typist.rs`) presses keys by counting emulated frames, and `machine/autoplay.rs` presses play and a key when the Atom prints `PLAY TAPE`. Both are in the library so `tests/tape.rs` and `tests/disk.rs` can run the same hands-free scenarios against the real ROMs. The window itself was only smoke-tested (it starts and stays running); I could not watch it type.

### Chuckie Egg fixes
Chuckie Egg showed a score of 555555, a high-score table of `dddddddd` and a man who died at once. Cause: no RAM at `0400-09FF` and `0B00-1FFF`, where the game keeps its tables. The default layout is now `RamLayout::expanded()` (about 31K, with the disk controller's page at `0A00` left free). Found with a new debugger feature: a log of accesses to unmapped memory (`Debugger::log_unmapped`, shown in the Hardware window). `tests/software.rs` fails if the game ever touches unmapped memory below `B000`.

### Sound fixes
A speaker held still sounded like a faint whine and pausing thumped. Causes: (1) the sampler carried the fractional part of a sample period (22.68 cycles at 44.1 kHz) wrongly, so a still speaker gave a beat pattern instead of a flat line; it is now an exact box average (`machine/sound.rs`, with a test). (2) Reset only reset the CPU, so a program's speaker, timer and disk-controller state survived it; `Atom::reset` now resets the 8255, 6522 and 8271 like the real RESET line. (3) The audio output jumped between a steady level and silence; it now has a DC blocker, a fade in and out, and re-primes after a gap (`ui/audio.rs`). Samples made while paused or single-stepping are discarded. I could not listen to the result, only check the numbers.
