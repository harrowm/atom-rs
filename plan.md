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
| `BDC0–BDDF` | SID sound card (`machine/sid.rs`) |
| rest of `BC00–BFFF` | RAMROM / GDOS (not yet emulated) |
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

### Key mapping window
`ui/keymap.rs` is now a data table (`KeyMap`: host key name to `Target`, an Atom matrix key with optional Shift, or the separately wired SHIFT/CTRL/REPT), saved as JSON in `~/.config/atom-rs/keymap.json`. `ui/keyboard_window.rs` is a separate window listing every Atom key with the host keys bound to it, a Set button that captures the next key pressed, per-key removal, a filter and Reset to defaults. The host's Shift/Ctrl/Alt remain fixed (egui reports them as modifiers, not keys). Default added: the backtick key as a second `LOCK`, since Caps Lock is a toggle on a Mac.

### AtoMMC joystick port
Repton's Z, X, ; and . did nothing. The Retro Software ports read the joystick through the AtoMMC SD-card interface (write command `A2` to `B400`, read the answer from `B401`); with no AtoMMC the answer was open bus (`B4`), which decodes as right, left and up all held, so the keyboard was ignored. `machine/atommc.rs` now answers the port commands (`A0`-`A3`; no joystick reads as 0, as in Atomulator) and leaves every other command unanswered, so the DOS ROM's probing for an SD card behaves as before. The SD card itself is still not emulated. A joystick input (e.g. keys or a gamepad) could be wired to `AtoMmc::joystick` later.

### AtoMMC SD card
`machine/atommc.rs` emulates the AtoMMC interface at `B400-B7FF` (a port of Atomulator's firmware emulation): the register state machine, directory listing with wildcards, change/make/remove directory, rename, delete, open for reading/writing/random access with four file slots, read, write, seek, file info, the EEPROM, configuration byte and the joystick port. A folder on the host is the card: names are matched case-insensitively, must be 8.3, and cannot escape the folder. `Atom::use_sd_card` builds the machine: the AtoMMC ROM (instead of Atom DOS), kernel, BASIC and floating point come from the 128K `ramrom.rom` image, and the page at `0A00` becomes RAM. The real AtoMMC ROM boots (`ACORN ATOM + ATOMMC2`), lists, loads and saves, and the Atom Software Archive's own menu runs, browses and launches games from the unzipped archive (`tests/atommc.rs`, `tests/software.rs`). Options: `--sdcard FOLDER`, File > Use a folder as an SD card. Not done: SDDOS (disk images on the card); the Ramoth RAM/ROM board's bank registers (`BFFD-BFFF`) and banked utility ROMs; saving the card in a save state.

### One-command archive
`atom-rs --archive` (`src/archive.rs`) asks GitHub's releases API for the newest release of hoglet67/AtomSoftwareArchive (not `/latest`, which skips pre-releases, and every recent release is one), picks its plain zip (the shortest name; the others are variants for other hardware), and downloads and unpacks it into `~/.cache/atom-rs/archive/files` only if the recorded tag differs. It unpacks beside the old copy and swaps it in, so a failed download leaves the old copy working; zip names that would escape the folder are skipped; offline it falls back to the copy it has. The folder then becomes the SD card and `*MENU` is typed unless `--run` says otherwise. Dependencies added: `ureq` (HTTP) and `zip` (deflate only). Checked against the real V14BETA2 release: fresh download, up-to-date check, and offline fallback; its menu launches a game in the emulator. Not done: a File-menu "update archive" item, and picking the variant zips.

### SID sound card
`machine/sid.rs` maps a SID at `BDC0-BDDF` (as in Atomulator) and wraps the `sidera` crate. Chosen over the alternatives on licence and clock: `resid-rs` is GPL-3.0+, which cannot be linked into this GPL-2.0-only project; `mos6581` (MIT/Apache, new) only offers the C64's 985 kHz and 1.023 MHz clocks, while `sidera` (MIT) takes any clock, and the Atom's SID runs at 1 MHz, so the pitch is exact (tests measure 220/440/880 Hz to within 1.5 Hz). The chip is stepped once per audio sample from the sampler (`Sampler::tick` says when a sample is finished and the SID's is added to it), not per CPU cycle, and not at all while the master volume is zero (that kept the idle cost to nothing: 142x vs 143x without the card, against 71x if always stepped). A change of audio sample rate rebuilds the chip and replays the written registers. Tests: unit tests for pitch, release, reset and OSC3; a 6502 program playing a note through the bus; and a real tune from the archive's `SID` folder through its own player menu (silent in the menu, sounding when played; fails without the card). The player's own register writes (2,733 in 8 s) decode to a bass line and percussion. Listened to by the project owner on the real tunes from the archive's `SID` folder: it sounds right, so the 6581 model stays the default. Not done: saving the SID in states; cycle-exact register timing (digitised sample playback); the 8580 model option; the card's other address variants.

### Joystick
`machine/joystick.rs`: a `Joystick` (left, right, up, down, fire) seen through both of Atomulator's mechanisms. The AtoMMC port answers command `A2` with active-low bits (0 right, 1 left, 2 down, 3 up, 4 fire; a stick at rest reads `FF`), and the keyboard matrix interface pulls down row 0 bits as the keys 3 (left), minus (down), G (right), Q (up) and the empty column 0 (fire). `Atom::set_joystick(Some/None)` plugs it in or out (unplugged, the AtoMMC port answers 0, "no joystick", which Repton needs). The front end rebuilds the stick every frame like the keys: with Input > Joystick on, the arrow keys steer (and no longer type cursor keys) and Space fires (and still types). Tests: bit layouts, both interfaces at the Atom level, and Repton end to end: a stick at rest leaves it alone, and each direction produces the same picture as Repton's own key for it (a mutation check that swaps left and right fails it). Not done: gamepads (Atomulator has them; this would add a dependency such as gilrs), a second stick, remembering the setting between runs.

### The Ramoth RAM/ROM board
`machine/ramrom.rs` (+ `Memory`, `Atom`): the board's three registers at `BFFD-BFFF` (bank, four flags, jumpers; the flag in force is the register xor the jumpers), the 128K image with its eight utility banks and two ROM sets (DISKROM set or clear), and the flags' effects: `DISKROM` chooses the ROM set and (with `BLKA_RAM` clear) makes `0A00` RAM instead of the floppy controller, `EXTRAM` with bank 0 maps the RAM at `7000` into `A000`, writable. `use_sd_card` now installs a board with the `DISKROM` jumper instead of copying fixed ROM slices, so the machine can switch ROM sets at run time. The registers are NOT cleared by BREAK (a loaded ROM must survive the BREAK that starts it; Atomulator agrees) and are in saved states. Found by test: the archive's menu category F loaded nothing ("NO RAM BANKS FOUND AT #A000"); now all 13 titles on its first page load and verify, and A&F's Utilikit runs (`COLD` prints its copyright from A000). Utility ROMs hook BASIC keywords, not star commands. Not done: banks 8-15, BBC mode (the flag exists, has no effect).

### RAM presets
`RamPreset` (`machine/memory.rs`) replaces the old `RamLayout`: Atomulator's six main-RAM configurations (1K; +5K at 2800; +3K DOS = 2000-3FFF; +16K at 4000; the low pages except 0A00, our default; and all of 0000-7FFF). Hardware > Main RAM chooses one and restarts the Atom (RAM contents kept). Refused in SD card mode, where the board gives RAM everywhere. Saved in states. Observable: on 1K, BASIC puts programs in the video RAM above the text screen (`TOP` = 8202); with any larger size it is 2902 (tested). Not done: video RAM sizes (1K-8K; Atomulator's `vid_top`).

### SDDOS
`machine/sddos.rs` (the drives: up to four mounted image files, 256-byte sector read and write, the `BOOTDRV.CFG` record format) and eight more AtoMMC commands in `atommc.rs`: `12` mount (buffer: drive, name; replies the attribute byte or `40`+error), `40` set drive and sector (no reply, as on the firmware), `41` status and `47` unmount (both take the drive from the value latch, as the firmware does, not the buffer as its comments say), `42` names, `43`/`44` read and write a sector, `45` save and `46` restore the list. The SDDOS ROM was already in `ramrom.rom`; before these commands it hung waiting for replies at `ABF1`. Deviations from the firmware: a read-only image mounts read-only (the firmware cannot mount one); a missing image is "file not found". Mounted drives survive BREAK. Verified with the real SDDOS V3.25 ROM: `*DIN`, `*.`, `*LOAD`, `*SAVE` (into the image file) and remounting after a new power-on; large files load byte-for-byte the same as through the floppy controller. Chuckie Egg's `RUNME` failed under SDDOS with `ERROR 2 LINE 60`; see "Chuckie Egg and power-on RAM" below for the cause and fix.

### Video RAM size and the colour board
`Memory::set_video_ram_kb` (1-8, default 8; Atomulator's `vid_top`): CPU accesses above the fitted video RAM are unmapped (writes lost, reads open bus); the picture is still drawn from the whole array, as Atomulator does. Saved in states. `Framebuffer::monochrome` switches to Atomulator's grey palette (the colour board off); display only, not saved. Not done: the colour board's 50 Hz refresh (the default stays 60 Hz; PAL/NTSC is a separate item).

### BBC BASIC board
The RAM/ROM board's `BBCMODE` flag (a jumper, toggled by `Atom::set_bbc_basic`, which restarts the machine): the five sockets show BBC BASIC (image `1A000`, `1C000`, `1D000`, `1E000`) and the BBC OS (`1F000`), `EXTRAM`'s RAM window is off, and `Memory::load_bbc_ram` copies two image blocks into RAM at `6000` (block chosen by the bank register) and `7000` whenever the board registers change and on reset, as Atomulator's `set_rr_ptrs` does. Works with or without the SD card. Boots to "ATOM MOS V3.0 HOGLET, BASIC 2, EXTROM1, BBCMMC 1.00" with working integer and floating point arithmetic and programs (`tests/bbc.rs`; the screen is graphics, so tests make BBC BASIC write bytes with `?&2000=...` and read them back). Not done: the floppy system in BBC mode (the real board has none either).

### GDOS / the WD1770
`machine/fdc1770.rs`: the WD1770 at `BC10-BC14` (command/status, track, sector, data, and the board's control register: bit 0 side, bit 1 drive 0, bit 3 single density, bit 4 not-reset, bit 6 index (alternates), bit 7 data request on reads), written from Atomulator's `1770.c` (from Elkulator) but at sector level with simple timing instead of its raw-track layer: restore/seek/step/step in/out, read and write sector (and multiple), read address, force interrupt, write track (clears the track), 32/64 cycles per byte for double/single density, the interrupt request is the Atom's NMI, drives are the 8271's disk images (`fdc.drives`). Deliberately correct where Atomulator is not: TR00 is set when the head IS at track zero (Atomulator's macro is inverted). `Memory` holds the 64K GDOS ROM (16 banks of 4K, chosen from the menu as in Atomulator) which replaces the disk ROM when fitted and no board is. Verified with the real ROM: `*DOS` answers `ADOS-1770 BC10:1000`, `*.` lists a disk, `*LOAD`, `*SAVE` (into the image), and Chuckie Egg's loader reaches its menu. Not done: rotation, CRC errors, track layout on format, read track, saving the controller in states (it starts idle after a load).

### Machine types (Hardware > Machine)
`Machine` (`machine/atom.rs`): FloppyAtom, GdosAtom, SdCard, BbcBasic, with `Atom::machine()` (derived from what is fitted) and `Atom::set_machine(machine, rom_dir, card_folder)`, which gives up the old kind's settings cleanly (RAM size, card, GDOS, board) and restarts. The Hardware menu now leads with the machine choice (radio buttons, each with a description), then only the settings that apply to it (Main RAM only on the two floppy machines, GDOS ROM bank only for GDOS, card folder for SD), then the display settings. The status line always names the machine. Added because Main RAM was greyed out in SD card mode with only a small note, which nobody could discover. The last card folder is remembered so switching back to SD card is one click. `tests/machines.rs` switches between every pair of machines (16 combinations) with the real ROMs and checks each really is what it says (banner, `*DOS` answers ADOS-1770, `?&2000=6*7` in BBC BASIC, and so on).

### Menu tidy-up and restart clicks
Menus now follow one rule: anything about the machine's hardware is under **Hardware** (the machine type, RAM, video RAM, colour board, SID card, GDOS bank, card folder, ROMs). The old File items for the SD card and ROMs, and the Sound menu's SID switch, moved or went (they duplicated Hardware > Machine). The top-level "Machine" menu (Reset, Pause, speed) is now **Run**, and the debugger's "Hardware" window is **Chip registers**, so each name means one thing. Restarting the Atom from a menu, BREAK, reset, loading a ROM or a state caused a click (the speaker and SID levels jump while samples keep flowing, and the DC blocker turns the step into a pop): `AtomApp::restart_audio` now clears the audio queue, so the output fades out through the existing run-dry path, and holds new sound back for 150 ms so it fades in again.

The Disk menu's drives are greyed out on machines with no floppy drives (SD card, BBC BASIC) and when no disk ROM is loaded, with a hover explanation (for SD card it points to SDDOS and *DIN).

### Help and About
`ui/help_window.rs`: Help (F1, Help menu) and About open as separate windows like the Keyboard mapping window. The words are plain functions (`help_sections`, `key_table`, `credits`, ...) so they can be tested: every menu and every `Machine` is mentioned, the special keys are in the key table, and both windows are laid out headlessly through egui to catch layout panics. The text is written by hand, so it must be updated with the features (the tests catch missing menus and machines, not stale wording).

### Chuckie Egg and power-on RAM
Chuckie Egg's loader failed under SDDOS (and only there). Bisecting found the trigger: BASIC lines that are a single space (`30 `, `50 `), followed by a `*` command. Such a statement is offered to the utility ROMs, which runs the AtoMMC ROM's service entry (`EB22`): it scans the ROM banks (`INC $FD`, write it to the bank register `BFFF`, look for the `40 BF` signature, call `A002`) and wraps (`ED8E`) every OS vector that points into the banked `A000` area in a trampoline that saves the vector and the bank number from `$FD`, and later switches to that bank (`STA $BFFF`) before jumping to the original. So **`$FD` is the ROM's current bank and the ROM assumes it starts at 0.** My power-on RAM (junk everywhere, added to make Asteroids' `RND` work) made it `2C`, SDDOS's vectors were saved as living in bank `2C & 0F` = 12 (BBC ROM data), and every `*` command after a blank statement ran the wrong ROM and failed. Atomulator, which these ROMs are known to work with, powers RAM up as zeros with only the `RND` seed (`08-0B`) random, so that is what `power_on_ram` does now (Asteroids still works). Regression tests: a blank BASIC line then `*NOMON` under SDDOS, the real Chuckie Egg `RUNME` through SDDOS, and Chuckie Egg from the archive on the SD card machine (menu, then colour, then the game screen); both SDDOS tests fail with junk RAM. The general lesson: a ROM's assumptions about uninitialised RAM are part of its contract, so match the reference emulator's start-up state unless there is a reason not to.

## To do / ideas (not started)
- Gamepad input (Atomulator has it; would add a dependency such as `gilrs`) and a second stick.
- PAL/NTSC refresh (the colour board's 50 Hz); `.wav` tapes; a File-menu "update archive" item.
- Atomulator-style 8271 extras that nothing needs: the 128-byte single-record commands (`0A 0E 12 16 1E`), scan commands, the late-DMA error.
- More play-testing: Elite, Jet Set Willy, Snapper.

### Quick wins: write protect, SID model, screenshot, full screen
Disk > Drive n > Write protected (the chips already refused writes to a protected `DiskImage`; only the switch was missing). `SidModel` (6581 default, 8580) in `machine/sid.rs`, kept across rate changes and resets; a plain wave is identical on both in sidera, the models differ in combined waveforms and the filter (tested with saw+triangle). `ui/screenshot.rs`: PNG via the `png` crate, with dated file names (`atom-YYYYMMDD-HHMMSS.png`, date maths checked against known timestamps including a leap day); F10 saves at once, File > Save screenshot... asks where; the colour board setting is respected. Full screen with F11 and View > Full screen (`ViewportCommand::Fullscreen`; Atomulator uses Alt+Enter, but Alt is REPT here).

### CSW tapes and the tape catalogue
`machine/csw.rs`: CSW versions 1 and 2 (RLE, and zlib for version 2) are decoded into the same `TapeChunk`s the UEF player uses, so there is one player and one catalogue: pulses are sorted short (about 208 us, a 2400 Hz half cycle) or long (417 us, 1200 Hz) by the nearer length, a start bit is 8 long pulses, a data bit 16 short (1) or 8 long (0), the stop bit 16 short, extra short pulses are carrier, very long ones silence. Tolerates a tape 15% fast or slow and a stray noise pulse (tested). `csw::encode` writes version 1 (used by the tests; no menu item). `TapeImage::from_file` picks the format, and is what Insert tape and `--tape` use. `TapeImage::catalogue` reads the Atom tape block layout (`****`, name, CR, flags, block number big-endian, length-1, exec address, this block's load address, data, checksum), which was read off a real `*SAVE` from the real OS: flags bit 7 = more blocks follow, bit 6 = has data, bit 5 = not the first block. Tests: the real OS's save is catalogued correctly, and a real save turned into a CSW file and read back loads on the real Atom (fails if the pulse decoder is wrong). Tape > Catalogue... shows it. Not done: saving CSW, hearing the tape, a CSW inside a gzip file.

### Snow (experimental, off by default)
The 6847 and the CPU share the video RAM bus. `vdg6847::fetch_slot` says which of the line's 32 byte fetches is happening at a given cycle (window of 36 cycles starting at cycle 20: from the datasheet's 128 colour clocks of display out of 227.5, approximate). With `AtomBus::snow` on, a line is drawn one line late; any CPU read or write of video RAM during a fetch puts the CPU's byte in that fetch's place (`video::fetch_address` maps slot to RAM address; the byte is swapped in for the draw and restored). This is an approximation: Atomulator's snow is a no-op, and the real hand-over timing is not known exactly. **Checked against a video of a real Atom** (Early Warning, loaded from tape, 20 Feb 2025): the real snow is single-scanline white dashes about one byte (8 pixels) wide, scattered over the dark parts of the picture, roughly 10-15 per camera frame (a camera frame spans about 1.7 Atom frames); the model, running the same game from the archive, makes the same kind of dashes at about 7 per Atom frame (the game makes about 18 video RAM accesses a frame, and about 40% of them land in a fetch). Only a visual and rough-rate comparison: the exact positions and byte values were not compared. `AtomBus::video_ram_accesses` and `snow_events` count these for such comparisons. Hardware > Video snow (experimental). Tests: no snow when off or when the CPU uses other RAM; some, but not much, when on.

### Remembered settings
`ui/settings.rs`: a `Settings` struct (machine, SD card folder, RAM preset, video RAM, colour board, snow, SID fitted and model, volume, mute, integer scaling, fast tape, keyboard joystick) saved as JSON in `~/.config/atom-rs/settings.json`, beside `keymap.json` (both now use `settings::config_folder`). `#[serde(default)]` means a missing setting takes its default and an older or hand-edited file still loads; an unreadable file gives a status-bar message and the defaults. `AtomApp::settings()` reads the current state, `apply_settings()` puts a saved one into effect (the machine first, since choosing one restarts the Atom and sets its hardware; the RAM preset is skipped on the RAM/ROM board machines), and `save_settings_if_changed()` runs every frame, compares with what was last written and writes only on a change, and not while a mouse button is down so a slider drag writes once at the end. Command line wins: `--sdcard` or `--state` stop the saved machine being restored, and `--joystick` and `--fast-tape` only switch things on. If the saved machine is the SD card but its folder is gone, it falls back to the floppy Atom with a message. Not remembered on purpose: loaded disks, tapes, the debugger windows, window size. Tests: JSON round trip, partial and bad files, the defaults equal a fresh machine's, and applying settings really changes the machine.

### Screen border
`video.rs`: `BORDER_TOP` 25 and `BORDER_BOTTOM` 26 lines (datasheet Figure 13: 25 + 192 + 26, with 6 lines of retrace and 13 of blanking making 262), and `BORDER_SIDE` 65 pixels each side: the datasheet gives 193.1 colour clocks from blanking to blanking, the picture takes 128, and a colour clock is 2 pixels, so 65.1 pixels are left over, split evenly so the picture is centred (its text says the display window starts "283 periods" in, which cannot be right as the whole line is 193.1; probably 28.3, which would put 57 on the left and 73 on the right, but a monitor's overscan decides that in practice). The full picture is 386 x 243. Colour: black with text and semigraphics, green (colour set 0) or buff (set 1) with any graphics mode (Figure 14). Checked against a real Atom: a video of Early Warning shows a bright surround in graphics and a dark one in text; a screenshot of a real Atom's text screen shows a dark border round the picture (and a dark green text background, see below). `Framebuffer::note_border` is called as each scanline starts, so the border colour is recorded per row as the beam passes: a program that changes mode mid-frame changes the border at the same line (`border_row` maps scanlines to rows; the last 25 lines of a frame are the top border of the next). `Framebuffer::show_border` (Hardware > Screen border, remembered in the settings) chooses between the 386 x 243 and the plain 256 x 192 picture; `to_rgba_shown` and `shown_size` are what the window and screenshots use, `to_rgba` is still the picture alone. Tests: the sizes, the scanline mapping for NTSC and PAL, colours by mode, the composed picture, and mid-frame mode changes.
**Text background:** a real Atom's text screen is bright green characters on dark green (colour set 1: orange on dark orange), not on black, as a screenshot of a real Atom shows, so the border is visible round it. `DARK_GREEN` (0, 130, 0) and `DARK_ORANGE` (130, 55, 0) are approximate, picked to look like the screenshot (the datasheet gives no RGB values); semigraphics blocks keep a black background, and inverse text swaps the two. Tests that counted "non-black" as text now count the bright colour.

### Releases
`.github/workflows/release.yml`: pushing a tag `v*` builds six targets in a matrix (Linux x86_64 and arm64, macOS arm64 and x86_64, Windows x86_64 and arm64), each packaged with `roms/`, the README and the licence (`.tar.gz`, `.zip` on Windows), then a second job downloads them all, adds `SHA256SUMS.txt` and runs `gh release create --generate-notes` (no third-party release action). The macOS Intel build and the Windows arm64 build are cross-compiled on the macOS arm64 and Windows x86_64 runners, so they are built but not run in CI; the others also run `cargo test --release` (the tests that need downloaded data skip themselves). Run by hand it builds and keeps the archives for 14 days without publishing, for testing in VMs. The ROM lookup already tries the folder beside the executable, so a release works from anywhere. Not signed or notarised (macOS Gatekeeper and Windows SmartScreen warnings are documented in the README). Linux arm64 uses `ubuntu-24.04-arm`, free only on public repositories.

