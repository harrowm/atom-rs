# atom-rs: an Acorn Atom emulator in Rust

A cycle-accurate emulator of the Acorn Atom (1980), written as a learning
project. The code is meant to be read: small modules, one idea per function,
and comments that explain the hardware being modelled.

* **CPU:** a 6502 core that makes every bus access (including the "dummy"
  ones) in the same order as the real chip. It passes all of Tom Harte's
  SingleStepTests, Klaus Dormann's functional and interrupt tests, and Bruce
  Clark's decimal-mode test.
* **Machine:** 8255 PPI (keyboard, video mode, speaker), 6522 VIA, 6847 video
  (drawn scanline by scanline, all text and graphics modes), memory map
  modelled on [Atomulator](https://github.com/hoglet67/Atomulator).
* **Front end:** [egui/eframe](https://github.com/emilk/egui) window with a
  debugger made of separate windows.
* **Sound and tape:** the speaker plays through your sound card (cpal). Tapes
  load from `.uef` files (also gzipped) and the Atom can record `*SAVE`s to a
  new `.uef`. `.atm` programs can be loaded straight into memory.
* **Disks:** an Intel 8271 floppy controller with `.ssd` and `.dsd` disk
  images, working with the real Atom DOS ROM (`*DOS`, `*CAT`, `*LOAD`, `*SAVE`).
* **Save states:** freeze and restore the whole machine, in a file or a quick
  slot (F6 / F7).

See [`plan.md`](plan.md) for the roadmap and design notes.

## Requirements

* Rust (stable). On macOS with Homebrew:

  ```sh
  brew install rustup
  rustup default stable
  echo 'export PATH=/opt/homebrew/opt/rustup/bin:$PATH' >> ~/.zshrc
  source ~/.zshrc
  ```
* Atom ROM images in `roms/` (git-ignored): `akernel.rom`, `abasic.rom` and
  `afloat.rom` are required; `axr1.rom` (utility) and `dosrom.rom` (disk) are
  optional. Without `dosrom.rom` there is no disk interface. Atomulator's repository ships a set in its `roms/` folder.
* Optional, for the full test suite: `git`, `curl`, and `cc65`
  (`brew install cc65`) to build the decimal-mode test.

## Build and run

```sh
cargo run --release                   # build and start the emulator
cargo run --release -- --debug        # ...with all the debugger windows open
cargo run --release -- /path/to/roms  # use a different ROM folder
cargo build --release                 # just build; binary is target/release/atom-rs
```

### Starting a program from the command line

Give the emulator a disk or tape and the command to start it, and it types the
command for you once the Atom has started:

```sh
# a disk: "*DOS" is typed first for you, then the command
cargo run --release -- --disk games.ssd --run '*RUN CHUCKIE\n'

# a tape: it presses play and a key by itself when the Atom says PLAY TAPE
cargo run --release -- --tape chuckie.uef --fast-tape --run 'LOAD "CHUCKIE"\nRUN\n'
```

| Option | Meaning |
|--------|---------|
| `--disk FILE`, `--disk1 FILE` | `.ssd` / `.dsd` image in drive 0 / 1 |
| `--tape FILE` | `.uef` tape (plain or gzipped); plays itself when asked |
| `--run TEXT` | text to type after start-up; `\n` is RETURN |
| `--fast-tape` | maximum speed while the tape is playing |
| `--state FILE` | start from a saved state |
| `--debug` | open the debugger windows |
| `--help` | list the options |
| `ROM_FOLDER` | where the ROMs are (default `./roms`) |

What to type depends on the program: BASIC programs are `LOAD "NAME"` then
`RUN`; machine-code programs are usually `*RUN "NAME"`. Atomulator's readme
has examples. The Atom prints nothing while a tape loads, so be patient (or use
`--fast-tape`). Note that `--run` only types: you still need your own copy of
any game.

`--release` is recommended, but a plain `cargo run` is fast enough too (the dev
profile is lightly optimised). The ROM folder is found automatically from
wherever you start the program.

## Using the emulator

| Key | Action |
|-----|--------|
| letters, digits, punctuation | the Atom key in the same position (US layout) |
| Shift, Ctrl | Shift, Ctrl |
| Alt | REPT |
| Enter, Backspace | RETURN, DELETE |
| Arrow keys | cursor keys (Down and Left are Shift+Up and Shift+Right, as on the Atom) |
| End, Tab | COPY, `^` |
| F12 | BREAK (reset) |
| F6, F7 | quick save / quick load of the whole machine |

Menus: **File** loads a ROM into a socket, chooses a ROM folder, or loads an
`.atm` program into memory. **Tape** inserts, plays, rewinds, records and saves
tapes. **Sound** has mute and volume. **Machine**
has reset, pause and a speed slider from 0.1x to 100x (the status bar shows
the speed actually achieved; click `1x` to go back to real speed). **View**
toggles integer scaling. **Debug** opens the debugger windows.

Try: `PRINT 2+2`, or `10 PRINT "HELLO"` then `RUN`.

### Tape

The Atom works like the real machine: `*SAVE "NAME" 2900 2A00` (start and end
address) prints `RECORD TAPE` and waits for a key; `*LOAD "NAME"` prints
`PLAY TAPE` and waits.

* **Saving:** choose *Tape > Record*, run the `*SAVE`, press a key at `RECORD
  TAPE`, wait for the `>` prompt, then *Tape > Stop recording*. The recording
  is now in the player; *Tape > Save tape as UEF...* writes it to a file.
* **Loading:** *Tape > Insert tape (UEF)...*, type `*LOAD "NAME"` (or just
  `*LOAD ""` for the next file), then choose *Tape > Play* and press a key at
  `PLAY TAPE`. Tick *Fast tape* to run at maximum speed while the tape plays.
* **`.atm` files** (a program saved straight from memory) go into RAM with
  *File > Load .atm file into memory*; start them with *File > Run loaded
  program* or by typing `LINK #xxxx`.

### Disks

Put `dosrom.rom` in `roms/`, then use **Disk > Drive 0 > Insert disk image...**
(`.ssd` single-sided or `.dsd` double-sided) or **New blank disk**. On the Atom
type `*DOS` to switch to the disk filing system, then the usual commands work:

```
*CAT                  list the disk
*LOAD NAME            load a file to the address it was saved from
*SAVE NAME 2900 2A00  save memory from start address to end address
```

Changes are made in memory only. **Disk > Drive 0 > Save** writes the image
file; the status bar shows `D0*` while a disk has unsaved changes.

### Save states

**File > Save state...** and **Load state...** use a file; **F6** and **F7**
use a quick slot in memory. A state holds the CPU, all RAM, every chip, the
disks in the drives and the tape, but not the ROMs or breakpoints.

### Sound

The speaker is a single bit, sampled every CPU cycle and averaged down to your
sound card's rate. Sound plays at normal speed and slower; above about 1x it
is muted rather than squealing.

### The debugger

Each tool is a separate window, opened from the **Debug** menu.

* **CPU and controls:** registers and flags (editable while stopped), the
  stack, and Run / Pause / Step / Step over / Frame buttons, plus Reset, NMI
  and a held-IRQ toggle.
* **Disassembly:** follows the PC. Click the dot to set a breakpoint;
  right-click a line for "Run to here" or "Set PC here".
* **Memory:** hex dump of all 64K, colour-coded (blue ROM, green video RAM,
  yellow I/O), jump-to buttons, and a box to write bytes into RAM.
* **Breakpoints:** execution breakpoints and read/write watchpoints.
* **Hardware:** video position and mode, 8255 ports, 6522 registers and timers.

Stepping is per instruction. Single-cycle stepping needs a CPU that can pause
mid-instruction and is not implemented yet.

## Tests

```sh
cargo test                            # everything (needs the test data below)
cargo test --lib                      # fast unit tests only
cargo test --test single_step         # SingleStepTests: ~2.5 million cases
OPCODE=a9 cargo test --test single_step -- --nocapture   # just one opcode
cargo test --test dormann -- --nocapture                 # Dormann + decimal + interrupt
cargo test --test boot -- --nocapture                    # boot the ROMs, type BASIC lines, save states
cargo test --test tape -- --nocapture                    # *SAVE to tape, wipe memory, *LOAD it back
cargo test --test disk -- --nocapture                    # the real DOS lists, loads and saves on a disk image
```

The big CPU tests need external data, which is not stored in this repository:

```sh
./scripts/fetch_tests.sh
```

This downloads the SingleStepTests (about 5 GB) and Dormann's tests into
`testdata/`, builds the decimal test with `ca65` if it is installed, and fetches
a prebuilt interrupt test. Tests that cannot find their data print `SKIPPED`
and pass, so a plain `cargo test` always works.

## Benchmarks

```sh
cargo run --release --example bench
```

Runs 200 million emulated cycles with the Atom sitting at the BASIC prompt and
reports how many times faster than a real Atom (1 MHz) it ran. On an Apple
silicon MacBook Air it manages about 160-175x.

## Code quality checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## Project layout

```
src/
  cpu/          the 6502: bus trait, opcode table, addressing, instructions,
                interrupts, disassembler. Knows nothing about the Atom.
  machine/      the Atom: memory map, 8255, 6522, 6847 timing and rendering,
                keyboard, sound sampler, tape, 8271 disk controller, disk images,
                save states, .atm files, debugger support,
                and the Atom that ties it together.
  ui/           egui front end: main window, key mapping, audio output,
                debugger windows.
  lib.rs        the library (cpu + machine); main.rs is the application
tests/          SingleStepTests, Dormann tests, boot, tape and disk tests
examples/       bench.rs
scripts/        fetch_tests.sh
roms/           your Atom ROMs (git-ignored)
testdata/       downloaded test suites (git-ignored)
```

Dependencies point one way: `ui` uses `machine`, which uses `cpu`.

## Credits

The memory map, character font and video behaviour follow Tom Walker's
Atomulator. CPU tests are by Tom Harte and others (SingleStepTests), Klaus
Dormann, and Bruce Clark.

## Licence

Licensed under the **GNU General Public License, version 2 only** (see
[`LICENSE`](LICENSE)). This follows from Atomulator: the character font in
`src/machine/font.rs` is copied from it, and the memory map, peripheral
behaviour and key layout were worked out from reading its source. Atomulator
states "licensed under the GPL" and ships the version 2 text, with no
"or later" grant, so this project does the same.

The test suites downloaded by `scripts/fetch_tests.sh` are not part of this
repository and keep their own licences. The Atom ROMs are Acorn's and are not
included.
