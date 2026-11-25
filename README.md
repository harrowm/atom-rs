# atom-rs: an Acorn Atom emulator in Rust

[![CI](https://github.com/harrowm/atom-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/harrowm/atom-rs/actions/workflows/ci.yml)
[![Licence: GPL v2](https://img.shields.io/badge/licence-GPL--2.0--only-blue.svg)](LICENSE)
![Rust 2024](https://img.shields.io/badge/rust-2024%20edition-orange.svg)

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
* **Memory:** the Atom is emulated with its RAM expansion (about 31K: `0000-09FF`
  and `0B00-7FFF`, with the disk controller at `0A00`), since a lot of software
  keeps its variables in the low pages.
* **Disks:** an Intel 8271 floppy controller with `.ssd` and `.dsd` disk
  images, working with the real Atom DOS ROM (`*DOS`, `*CAT`, `*LOAD`, `*SAVE`).
* **Software archive in one command:** `atom-rs --archive` downloads the latest
  Atom Software Archive (about 1,100 titles), keeps it up to date, and opens its
  menu. It works through an emulated AtoMMC SD card interface, so any folder of
  Atom programs can be used as a card too.
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
* The Atom ROM images are in `roms/` (see [the ROMs](#the-roms) below), so
  there is nothing to fetch. A different set can be used with
  `cargo run --release -- /path/to/roms`. `akernel.rom`, `abasic.rom` and
  `afloat.rom` are required; `axr1.rom` (utility) and `dosrom.rom` (disk) are
  optional, and without `dosrom.rom` there is no disk interface. `ramrom.rom`
  and `mmc_eeprom.rom` are only needed for the SD card mode.
* Optional, for the full test suite: `git`, `curl`, and `cc65`
  (`brew install cc65`) to build the decimal-mode test.

## Build and run

```sh
cargo run --release                   # build and start the emulator
cargo run --release -- --archive      # fetch/update the software archive and browse it
cargo run --release -- --debug        # ...with all the debugger windows open
cargo run --release -- /path/to/roms  # use a different ROM folder
cargo build --release                 # just build; binary is target/release/atom-rs
```

### Quick start: browse the whole Atom Software Archive

The easiest way to try the archive's ~1,100 titles is one command:

```sh
cargo run --release -- --archive
```

The first time, it downloads the latest Atom Software Archive from GitHub (about
17 MB) and keeps it in `~/.cache/atom-rs/archive`. Every time after that it asks
GitHub whether there is a newer release, downloads it only if there is, and
starts the archive's menu. With no network it uses the copy it already has.
(It takes the newest release, which is currently a pre-release, as the
maintainers flag every recent one that way.)

Under the hood the emulator treats the archive's folder as an **SD card**. The
archive is laid out exactly as the contents of an AtoMMC card, with its own menu
program. To use a copy you already have, or any other folder:

```sh
cargo run --release -- --sdcard path/to/folder --run '*MENU\n'
```

You get "ACORN ATOM software archive, maintained by Hoglet": press **A** to **G**
for a category (Commercial, Modern Creations, Arcade Game Designer, Non
Commercial, Books and Magazines, Utility ROMs, All Titles), then pick a title
from the list ("ATOMMC BY TITLE, page 1/23") with the keys it shows. When I
tried it, **Return** loaded the highlighted first title, 3D Asteroids, from the
folder and ran it.

This needs `roms/ramrom.rom` (the 128K ROM image Atomulator ships, which holds
the AtoMMC ROM, the kernel and BASIC to match). It can also be chosen from the
**File > Use a folder as an SD card...** menu, and **Back to floppy disks** goes
back to Atom DOS. In this mode the Atom reports `ACORN ATOM + ATOMMC2` and there
are no floppy drives.

Any folder works as a card, not just the archive: put programs in it (in the
`.atm` format, which is what `*SAVE` writes) and use `*CAT`, `*LOAD NAME`,
`*SAVE NAME start end` and `*NAME` to run one. File names must be valid 8.3
names, are matched without regard to case, and nothing outside the folder can
be reached. What you save from the Atom appears in the folder as an ordinary
file.

Not emulated yet: SDDOS (disk images stored on the card), so games that insist
on a floppy still use the disk method below.

### Running single games from disk images (the manual way)

If you prefer floppy disk images, or want a game that insists on one, you can
extract a game from the archive and pack it into an `.ssd` yourself.

The [Atom Software Archive](https://github.com/hoglet67/AtomSoftwareArchive)
is a huge library of Atom software, kept at stardot.org.uk. Each program in it
is stored as a file with a small header (the `.atm` format). Many of the games
are conversions by Kees van Oss for Retro Software (the `RS/` folders), made to
run from an SD card interface. This emulator reads them from a floppy disk
instead, so there are two steps: **make a disk**, then **run it**.

**1. Make the disks** (once). This downloads the archive (about 17 MB, kept in
`testdata/`, not in this repository) and packs each game onto a disk image in
`testdata/games/`:

```sh
./scripts/make_all_games.sh                      # every game in the table below
./scripts/make_game.sh RS/JSW jsw                # or just one
```

**2. Run one.** Each game is started by typing a command, which `--run` does for
you (`\n` is the RETURN key; `*DOS` is typed first automatically):

```sh
cargo run --release -- --disk testdata/games/<disk>.ssd --run '<text>'
```

For example, Jet Set Willy:

```sh
cargo run --release -- --disk testdata/games/jsw.ssd --run 'LOAD "JSWRUN"\nRUN\n'
```

| Game | Disk | `--run` text | What to do next |
|------|------|--------------|-----------------|
| Chuckie Egg | `chuckie` | `'*RUN RUNME\n'` | Press **2** for colour (1 for monochrome), then **S** to start and **1** for one player. Press **K** on the high-scores screen to choose your own keys. |
| Jet Set Willy | `jsw` | `'LOAD "JSWRUN"\nRUN\n'` | Press any key through the instructions and the controls screen (**1** = keyboard, the default), then any key at the title to start. **Z** left, **X** right, **RETURN** jump. |
| Repton | `repton` | `'*RUN LOADER\n'` | Press **Space** three times (menu, title, high scores). **Z** left, **X** right, **;** up, **.** down, **Esc** kill, **R** restart, **Q**/**S** sound, **End** or **Backspace** pause. |
| Elite | `elite` | `'*RUN ELITE\n'` | Starts with an intro screen. In flight: **<** and **>** roll, **S** dive, **X** climb, **Space** faster, **/** slower, **H** hyperspace, **Shift+1** to **Shift+0** the views and screens. The full list is in `RS/ELITE/KEYS.TXT` in the archive. |
| Snapper | `snapper` | `'LOAD "SNAPPER"\nRUN\n'` | A Pac-Man style game; it starts straight away. |
| Pinball | `pinball` | `'LOAD "PINBALL"\nRUN\n'` | Answer "1 OR 2 PLAYERS?". |
| Space Panic | `panic` | `'*RUN PANIC\n'` | The instructions screen lists the keys. |
| Frogger | `frogger` | `'*RUN FROGGER\n'` | **T** up, **B** down, **F** left, **H** right. |
| Galaxians | `galaxian` | `'*RUN GALAXIA\n'` | Press **A** for one player or **B** for two. Then **LOCK** moves left (the **`** key, or Caps Lock), **Shift** moves right, **Alt/Option** (the Atom's REPT) fires. |
| Space Invaders | `invaders` | `'*RUN INVADER\n'` | Starts at a "PLAY SPACE INVADERS" screen. |
| Asteroids (Acornsoft) | `asteroid` | `'*RUN ASTEROI\n'` | Left Shift and Ctrl turn the ship, Return fires, Rept thrusts (checked by holding each key and watching the picture change) |
| Breakout | `breakout` | `'*RUN BREAKOU\n'` | **Shift** left, **Alt/Option** (REPT) right, **Ctrl** to serve and to start. |

A few notes:

* Every game in the table loads and reaches its first screens in the emulator
  (checked by running it and looking at the picture, and for Chuckie Egg and
  Repton by tests that press the keys). Only Chuckie Egg has been played
  properly, so expect rough edges and please report them.
* **BASIC or machine code?** A BASIC program is started by BASIC's own `LOAD`
  and `RUN`; machine code (and BASIC programs that start themselves) use
  `*RUN`. `make_game.sh` reads the program's header and prints which to use.
* A disk catalogue allows names of 7 characters, so `GALAXIANS` becomes
  `GALAXIA` and so on. That is why the table says `*RUN GALAXIA`.
* The keys are the ones on the Atom, found by position: see *Changing the
  keys* if a game wants a key that is awkward on your keyboard.
* **Other games:** browse `testdata/asa_v13_01/files/ASA/` for more. A game that
  is one file or a folder works with `./scripts/make_game.sh AF/PANIC panic`
  (a path under `ASA/`, then a disk name). For a folder the loader is usually
  called `RUNME` or `LOADER`. Some games may need hardware the emulator does not
  have yet (SID sound, the SD card itself); the Hardware debug window's
  *Unmapped memory* log shows what a game reaches for.

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
| letters, digits, punctuation | the Atom key in the same position (US layout); change in *View > Keyboard mapping* |
| Shift, Ctrl | Shift, Ctrl |
| Alt | REPT |
| Enter, Backspace | RETURN, DELETE |
| Arrow keys | cursor keys (Down and Left are Shift+Up and Shift+Right, as on the Atom) |
| End, Tab | COPY, `^` |
| F12 | BREAK (reset) |
| F6, F7 | quick save / quick load of the whole machine |

### Changing the keys

**View > Keyboard mapping...** opens a window listing the Atom keys and the
keys on your keyboard that press them, in three folding sections: *Symbols and
special keys* (open), then *Letters* and *Numbers* (folded away, since few
people rebind those; typing in the filter box opens any section with a match). Click **Set** next to an Atom key, then
press the key you want; click a key's name to remove it; **Reset all to
defaults** puts everything back. A key can only do one thing, so binding it to
a new Atom key takes it off the old one (the window tells you what it
replaced). Esc cancels. Your Shift, Ctrl and Alt keys are always the Atom's
SHIFT, CTRL and REPT, and F6, F7 and F12 belong to the emulator.

Changes are saved to `~/.config/atom-rs/keymap.json`, which you can also edit
or delete by hand.

Menus: **File** loads a ROM into a socket, chooses a ROM folder, or loads an
`.atm` program into memory. **Tape** inserts, plays, rewinds, records and saves
tapes. **Sound** has mute and volume. **Machine**
has reset, pause and a speed slider from 0.1x to 100x (the status bar shows
the speed actually achieved; click `1x` to go back to real speed). **View**
toggles integer scaling. **Debug** opens the debugger windows.

The **Hardware** debug window has an *Unmapped memory* log: switch it on and it
lists every address the running program touched that the Atom has nothing at.
That is how a missing-RAM bug in Chuckie Egg was found, and it is a quick way
to see what a misbehaving program expects.

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
cargo test --test software -- --nocapture                # Chuckie Egg and Repton (needs ./scripts/make_all_games.sh first)
```

The big CPU tests need external data, which is not stored in this repository:

```sh
./scripts/fetch_tests.sh
```

This downloads the plain-6502 part of the SingleStepTests (about 190 MB to
download, 1 GB on disk) and Dormann's tests into `testdata/`, builds the decimal
test with `ca65` if it is installed, and fetches a prebuilt interrupt test.
Tests that cannot find their data print `SKIPPED` and pass, so a plain
`cargo test` always works. Set `REQUIRE_TEST_DATA=1` to make them fail instead.

### Continuous integration

GitHub Actions (`.github/workflows/ci.yml`) runs `cargo fmt --check`, clippy and
the whole test suite on every push and pull request, with `REQUIRE_TEST_DATA=1`
so that a green tick always means the tests really ran. It downloads the test
data as above (the 1 GB of SingleStepTests is kept in the Actions cache), and
also fetches the software archive and builds the game disks, so the tests that
play Chuckie Egg, Repton and Asteroids and browse the SD card run there too.

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
scripts/        fetch_tests.sh, get_archive.sh, make_game.sh, make_all_games.sh
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
repository and keep their own licences.

### The ROMs

The files in `roms/` are **not** covered by this project's licence and are not
my work. They are the Acorn Atom's system ROMs (operating system, BASIC,
floating point, disk filing system and utility ROM), which are copyright
Acorn Computers, and the AtoMMC and SDDOS ROMs (SDDOS is by Kees van Oss) in
the 128K `ramrom.rom` image. All of them are copied unchanged from the
[Atomulator](https://github.com/hoglet67/Atomulator) repository, which has
distributed them for years so that the emulator can run. They are here for the
same reason: without them an Atom emulator cannot start. If you hold the rights
to any of them and would like one removed, please open an issue and it will be.
