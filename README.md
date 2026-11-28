# atom-rs: an Acorn Atom emulator in Rust

[![CI](https://github.com/harrowm/atom-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/harrowm/atom-rs/actions/workflows/ci.yml)
[![Licence: GPL v2](https://img.shields.io/badge/licence-GPL--2.0--only-blue.svg)](LICENSE)
![Tests](badges/tests.svg)
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
  load from `.uef` files (also gzipped) or `.csw` files (a recording of the
  actual signal), there is a tape catalogue window, and the Atom can record
  `*SAVE`s to a new `.uef`. `.atm` programs can be loaded straight into memory.
* **Memory:** the Atom is emulated with its RAM expansion by default (about 31K:
  `0000-09FF` and `0B00-7FFF`, with the disk controller at `0A00`), since a lot
  of software keeps its variables in the low pages. Six RAM sizes, from a bare
  1K Atom up, can be chosen from the **Hardware** menu. The Ramoth RAM/ROM board
  (switchable ROMs and RAM at `A000`) is emulated in SD card mode.
* **Disks:** an Intel 8271 floppy controller with `.ssd` and `.dsd` disk
  images, working with the real Atom DOS ROM (`*DOS`, `*CAT`, `*LOAD`, `*SAVE`).
* **Software archive in one command:** `atom-rs --archive` downloads the latest
  Atom Software Archive (about 1,100 titles), keeps it up to date, and opens its
  menu. It works through an emulated AtoMMC SD card interface, so any folder of
  Atom programs can be used as a card too.
* **Save states:** freeze and restore the whole machine, in a file or a quick
  slot (F6 / F7).

See [`plan.md`](plan.md) for the roadmap and design notes.

## Download

Ready-made builds for Linux, macOS and Windows (Intel/AMD and ARM) are on the
[Releases page](https://github.com/harrowm/atom-rs/releases). Unpack the
archive and run `atom-rs` (the ROMs are in the `roms` folder beside it); there
is nothing to install.

* **macOS:** the program is not signed, so macOS refuses a downloaded copy at
  first. Run `xattr -d com.apple.quarantine atom-rs` in the unpacked folder
  once (or right-click it and choose Open).
* **Windows:** if SmartScreen warns about an unrecognised app, choose *More
  info* then *Run anyway*.
* **Linux:** needs **glibc 2.35 or newer** (Ubuntu 22.04 or later, Debian 12 or
  later, Fedora 36 or later; `ldd --version` shows yours), ALSA (the sound
  library, `libasound2`) and a desktop with X11 or Wayland; `chmod +x atom-rs`
  if the archive did not keep the permission.

**Running a download from a terminal.** The command lines in this README are
written for building from source (`cargo run --release -- OPTIONS`). With a
download, run the program itself instead, from the unpacked folder:

| From source | macOS / Linux download | Windows download |
|---|---|---|
| `cargo run --release -- --archive` | `./atom-rs --archive` | `.\atom-rs.exe --archive` |
| `cargo run --release -- --tape x.uef` | `./atom-rs --tape x.uef` | `.\atom-rs.exe --tape x.uef` |

On Windows use Command Prompt or PowerShell (`cd` to the unpacked folder first).
Double-clicking the program starts it with no options. Your settings and the
archive are kept in `.config\atom-rs` and `.cache\atom-rs` in your home folder
(`C:\Users\you` on Windows).

To make a release (for the maintainer): `git tag v0.1.0 && git push --tags`.
The *Release* workflow builds every platform and publishes the archives with
checksums.

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
**Hardware > Machine > SD card (AtoMMC)** menu: the first time, the standard
archive (the same one `--archive` keeps in `~/.cache/atom-rs/archive`) is
downloaded in the background, with progress in the status bar, and the Atom
restarts as an SD card machine when it arrives, so you can just type `*MENU`.
*Choose another card folder...* in the same menu switches to a different
folder, and the choice is remembered. Choosing **Floppy disk Atom** goes back
to Atom DOS. In this mode the Atom reports `ACORN ATOM + ATOMMC2` and there
are no floppy drives.

Any folder works as a card, not just the archive: put programs in it (in the
`.atm` format, which is what `*SAVE` writes) and use `*CAT`, `*LOAD NAME`,
`*SAVE NAME start end` and `*NAME` to run one. File names must be valid 8.3
names, are matched without regard to case, and nothing outside the folder can
be reached. What you save from the Atom appears in the folder as an ordinary
file.

**Utility ROMs.** The menu's category **F** ("Utility ROMs": Acorn's Atom Calc,
the ACE and AEK ROMs, A&F's Utilikit and more) loads a ROM image from the card
into RAM and switches it in at `A000`, which needs the **Ramoth RAM/ROM board**.
That board is part of the SD card mode, so these work. After a ROM says it has
loaded, press **F12** (BREAK) to start it. Most of these ROMs add BASIC
keywords, so you type them **without a star**: for example, after loading A&F's
Utilikit and pressing BREAK, `COLD` prints its copyright line. The ROM
survives BREAK; it goes when you leave SD card mode or reset the whole machine.
Bank 8 onwards of the board's ROM and its BBC BASIC mode are not emulated.

**Disk images on the card (SDDOS).** Floppy disk images (`.ssd`) can live on the
card and be used as drives, with Kees van Oss's SDDOS, which is in the ROM
image. Put the image in the card folder (and `*CWD` to its folder), then:

```
SDDOS                  switch it on (no star)
*DIN 0 GAME.SSD       mount GAME.SSD as drive 0 (drives 0-3)
*.                    the usual disk commands now use it: *LOAD, *RUN, *SAVE...
*DHELP                list SDDOS's commands (*DOUT unmounts, *DCAT lists images)
```

What you `*SAVE` is written into the image file itself. The mounted images are
remembered in a small file, `BOOTDRV.CFG`, in the card's top folder, and come
back the next time you type `SDDOS`. (That means the emulator writes that one
file into whatever folder you use as the card.) A read-only image mounts but
refuses writes. Chuckie Egg loads and runs from the SD card, from a floppy
image, with GDOS, and from an image on the card through SDDOS.

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
  have yet (a joystick, say); the *Chip registers* debug window's
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
| `--tape FILE` | `.uef` tape (plain or gzipped) or `.csw` tape; plays itself when asked |
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
| letters, digits, punctuation | the Atom key in the same position (US layout); change in *Input > Keyboard mapping* |
| Shift, Ctrl | Shift, Ctrl |
| Alt | REPT |
| Enter, Backspace | RETURN, DELETE |
| Arrow keys | cursor keys (Down and Left are Shift+Up and Shift+Right, as on the Atom), or the joystick if *Input > Joystick* is on |
| End, Tab | COPY, `^` |
| F12 | BREAK (reset) |
| F6, F7 | quick save / quick load of the whole machine |
| F1 | the built-in help |
| F10 | save a screenshot (a PNG named by the date and time, in the current folder; **File > Save screenshot...** lets you choose where) |
| F11 | full screen on and off (also **View > Full screen**; on a Mac the system may keep F11, so use the menu) |

### Help and About

The **Help** menu (or **F1**) opens a built-in guide: getting started, how to
load tapes, disks and SD cards, what the four machines are, what each menu does,
the keys, and what to try when something is not working. **About** shows the
version, the licence, the credits and the note on the ROMs.

### Joystick

The Atom has no joystick port of its own, so games used add-ons. Two are
emulated, as in Atomulator, so software written for either works: the
**AtoMMC card's port** (Repton reads its joystick this way) and a stick wired
into the **keyboard matrix** (it looks like the keys 3, minus, G and Q, plus a
fire line, on keyboard row 0).

Turn it on with **Input > Joystick on the keyboard** (or start with
`--joystick`): the **arrow keys** then steer and **Space** fires. While it is
on the arrow keys no longer type the Atom's cursor keys, and Space still types a
space as well, so menus that wait for it keep working. Both interfaces are fed
at once. A gamepad is not supported yet.

With the joystick off, the AtoMMC port reports "no joystick" (0). Checked with
Repton: a stick at rest leaves it alone and each direction does exactly what
Repton's own key for that direction does.

### Changing the keys

**Input > Keyboard mapping...** opens a window listing the Atom keys and the
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

### Remembered settings

The machine you chose (and the SD card folder), RAM and video RAM, the colour
board, snow, the SID card and model, volume and mute, scaling, fast tape and
the keyboard joystick are remembered between runs in
`~/.config/atom-rs/settings.json` (plain JSON; delete it to go back to the
defaults). Options on the command line win: `--sdcard` or `--state` decide the
machine, and `--joystick` and `--fast-tape` only switch things on. Loaded
disks and tapes are not remembered.

Menus: **File** loads a ROM into a socket, chooses a ROM folder, or loads an
`.atm` program into memory. **Tape** inserts, plays, rewinds, records and saves
tapes. **Sound** has mute and volume. **Machine**
has reset, pause and a speed slider from 0.1x to 100x (the status bar shows
the speed actually achieved; click `1x` to go back to real speed). **View**
toggles integer scaling. **Debug** opens the debugger windows.

The **Chip registers** debug window has an *Unmapped memory* log: switch it on and it
lists every address the running program touched that the Atom has nothing at.
That is how a missing-RAM bug in Chuckie Egg was found, and it is a quick way
to see what a misbehaving program expects.

Try: `PRINT 2+2`, or `10 PRINT "HELLO"` then `RUN`.

### The machine: Hardware > Machine

The emulator can be four different kinds of Atom, chosen at the top of the
**Hardware** menu (the current one is also shown in the status line). Choosing
one restarts the Atom, and the settings below it in the menu change to the ones
that apply to it:

| Machine | What it is | Settings that apply |
|---|---|---|
| **Floppy disk Atom** | the standard disk Atom: 8271 controller, Atom DOS (the default) | Main RAM |
| **Floppy disk Atom with GDOS/ADOS** | the WD1770 disk system | Main RAM, GDOS ROM bank |
| **SD card (AtoMMC)** | a folder is the SD card; includes the RAM/ROM board | card folder |
| **BBC BASIC** | BBC BASIC 2, through the RAM/ROM board | none |

*Video RAM* and *Colour board* apply to all of them. Main RAM is only
choosable on the two floppy machines: the SD card and BBC BASIC machines have the
RAM/ROM board, which provides all the RAM, so the menu says so and points you to
a floppy machine. The sections below describe each in turn.

### Memory

**Hardware > Main RAM** (on the floppy disk machines) fits one of Atomulator's six RAM configurations and
restarts the Atom (what is in RAM is kept, as when swapping chips on a real
Atom):

| Configuration | RAM |
|---|---|
| Minimum 1K in base | `0000-03FF` |
| 1K in base + 5K | and `2800-3BFF` |
| 6K on motherboard + 3K DOS | `0000-03FF`, `2000-3FFF` |
| ... + 16K expansion | `0000-03FF`, `2000-7FFF` |
| ... + 22K expansion (hole at `0A00`) | `0000-09FF`, `0B00-7FFF` (the default) |
| ... + 23K expansion | `0000-7FFF` |

**Hardware > Video RAM** chooses 1K to 8K of video RAM (`8000-9FFF`): beyond what
is fitted, writes are lost and reads see open bus, as in Atomulator. The text
screen needs only half a K, but the graphics modes need more (6K for the biggest).
**Hardware > Colour board** off shows the picture in shades of grey, as an Atom
without the colour board would (the refresh rate stays as it is).

Address ranges with no RAM read as open bus. You can see the difference: on a
bare 1K Atom BASIC keeps its programs in the spare video RAM (type `PRINT TOP`:
33282, which is `8202`), while with any of the larger sizes it is 10498 (`2902`).
Many programs need the larger sizes. On the SD card and BBC BASIC machines the choice is not available: the
RAM/ROM board provides RAM everywhere.

### GDOS/ADOS 2015 (WD1770 disks)

**Hardware > Machine > Floppy disk Atom with GDOS/ADOS** fits the other disk system the Atom had: a
Western Digital WD1770 controller (at `BC10`) with the GDOS/ADOS ROM, instead of
the standard Intel 8271 and Atom DOS. The same disk images (`--disk`, or *Disks*
below) work in it. Type `*DOS` to start it (it answers `ADOS-1770 BC10:1000`),
then `*.`, `*LOAD`, `*SAVE` and so on as usual; a game such as Chuckie Egg loads
through it. The ROM has 16 banks of 4K, chosen under **GDOS ROM bank**; this is
chosen from the menu as in Atomulator. It needs `roms/gdos2015.rom`, and is only
for the floppy disk Atom (not SD card or BBC mode, which supply their own disk
ROM). Written at the level of whole sectors with simple timing: it does not model
the disk's rotation or CRC errors, and formatting clears the track without
laying out the sectors.

### BBC BASIC

**Hardware > Machine > BBC BASIC** restarts the Atom as a BBC BASIC machine. The
Ramoth RAM/ROM board's BBC mode swaps in BBC BASIC 2 and a small BBC operating
system ("ATOM MOS V3.0 HOGLET"), using the ROM images in `ramrom.rom`. The
screen is a graphics screen the BBC OS writes its own text into. It works in
either machine mode (in SD card mode only the board's jumper changes). As on the
real board, the floppy disk system is not available in BBC mode. Choose another machine to go back. Quick check: `PRINT PI` gives 3.14159265.

### Tape formats and the catalogue

Tapes can be **UEF** (describes what is on the tape: carrier, bytes, gaps; the
usual emulator format) or **CSW** ("Compressed Square Wave": a list of pulse
lengths, a copy of the sound). A CSW file is turned back into bytes by
measuring its pulses against the Atom's two tones (1200 Hz and 2400 Hz), so a
tape running a little fast or slow still reads. **Tape > Catalogue...** lists
the files on the inserted tape (name, load and run addresses, size, and
whether all its blocks are there), as in the Atom tape format
`****`, name, header, data, checksum; it was checked against what the real OS
writes. CSW version 1 and 2 (including zlib-compressed) are read; only UEF
can be saved.

### Screen border

Text is drawn as the 6847 does it: bright green characters on a dark green
background (orange on dark orange in the second colour set), not on black.

The 6847 draws a border round the picture (25 lines above, 26 below, and about
65 pixels at each side, as in its datasheet): black with text, and green or
buff (off-white) with graphics, following the colour-set bit. It changes at
the scanline where a program changes mode. *Hardware > Screen border* turns it
off, which gives the plain 256 x 192 picture; screenshots show what the window
shows.

### Video snow

Real Atoms flicker a little when the CPU uses video RAM while the 6847 is
reading it. *Hardware > Video snow (experimental)* models this: if the CPU
reads or writes video RAM at the moment the 6847 is fetching a byte, the
picture shows the CPU's byte there. It is an approximation, off by default;
it was compared with a video of a real Atom running Early Warning and makes the
same short, byte-wide dashes at about the same rate.

### Disk write protection

Each drive's menu (**Disk > Drive 0**) has a **Write protected** tick, like the
notch on a real floppy: a protected disk can be read but the Atom's attempts to
write to it fail with the disk system's "write protected" error. It works on both
the 8271 and the GDOS (1770) disk systems.

### Tape

The Atom works like the real machine: `*SAVE "NAME" 2900 2A00` (start and end
address) prints `RECORD TAPE` and waits for a key; `*LOAD "NAME"` prints
`PLAY TAPE` and waits.

* **Saving:** choose *Tape > Record*, run the `*SAVE`, press a key at `RECORD
  TAPE`, wait for the `>` prompt, then *Tape > Stop recording*. The recording
  is now in the player; *Tape > Save tape as UEF...* writes it to a file.
* **Loading:** *Tape > Insert tape (UEF or CSW)...*, type `*LOAD "NAME"` (or just
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

**SID music.** A SID sound card (the Commodore 64's sound chip, which several
add-on cards put in the Atom) is emulated at `BDC0-BDDF` (or at `9FC0-9FDF`,
see below), clocked at the Atom's
1 MHz, and mixed in with the speaker. The chip itself is the
[`sidera`](https://crates.io/crates/sidera) crate; the rest (the memory map,
the clock, the mixing) is in `src/machine/sid.rs`. The archive's `SID` folder
has tunes with a player, which you can try with the archive and SD card mode:

```sh
cargo run --release -- --archive --run '*CWD ASA/SID/ATMSID1\n*RUN SIDMENU\n'
```

then press **A** or **Z** to choose a tune and **Space** to play it. There is a
**Hardware > SID sound card** switch (and **SID model**: the original 6581 or the later 8580, which sound different) in the menu.

**Two addresses.** The archive has two copies of its SID tunes: `ASA/SID/ATOMSID`
(and `ATMSID1` to `19`) are written for a card at `BDC0-BDDF`, which is the
default, and `ASA/SID/DAVESID` is written for a card at `9FC0-9FDF`, the other
position the archive's own manual page for the SID lists. Each set is silent
with the card at the other address, so choose **Hardware > SID address** to
match the tunes you are playing (the choice is remembered). At `9FC0` the card
sits in the top of the video RAM space and hides the 32 bytes of video RAM
there, which the screen never uses. When a tune plays it costs about
half the emulator's speed; when nothing uses the SID it costs nothing. Things to
know: a saved state does not include the SID (it restarts silent), and the chip
is stepped once per audio sample, so a register write takes effect up to about
22 microseconds late, which tunes cannot hear but sample playback through the
volume register would lose a little detail through.

### The debugger

Each tool is a separate window, opened from the **Debug** menu.

* **CPU and controls:** registers and flags (editable while stopped), the
  stack, and Run / Pause / Step / Step over / Step out / Frame buttons, plus
  Reset, NMI and a held-IRQ toggle.
* **Disassembly:** follows the PC. Click the dot to set a breakpoint;
  right-click a line for "Run to here" or "Set PC here".
* **Memory:** hex dump of all 64K, colour-coded (blue ROM, green video RAM,
  yellow I/O), jump-to buttons, and a box to write bytes into RAM.
* **Breakpoints:** execution breakpoints (optionally *conditional*, see below)
  and read/write watchpoints.
* **Instruction history:** a flight recorder. Tick *Record*, run or step, and it
  lists the last instructions that ran with the registers before each, so
  after a breakpoint or a crash you can see how the machine got there. It can
  be saved as a text file.
* **Chip registers:** video position and mode, 8255 ports, 6522 registers and timers.
* **Command box:** type commands instead of clicking (below).

Stepping is per instruction. Single-cycle stepping needs a CPU that can pause
mid-instruction and is not implemented yet.

#### The command box

*Debug > Command box* takes typed commands, in the style of Atomulator's
debugger plus a few more. Numbers are hexadecimal (`2900`, `$2900` or
`0x2900`), except the number of steps. Pressing Enter on an empty line repeats
the last command, and the Up and Down keys bring back earlier ones.

| Command | What it does |
|---|---|
| `r` | show the registers and flags |
| `set REG VALUE` | change `a x y sp pc p`, or a flag (`n v d i z c`, 0 or 1) while stopped |
| `s [n]` | step n instructions (n is decimal), then show where the machine is |
| `over` | step over a `JSR` as one step |
| `out` | run until the current subroutine returns to its caller |
| `c`, `p` | continue, pause |
| `g ADDR` | run until the PC reaches ADDR |
| `f` | run for one video frame |
| `d [ADDR]` | disassemble from the PC or ADDR, carrying on if repeated |
| `m [ADDR]` | dump memory, carrying on if repeated |
| `writem ADDR V...` | write one or more bytes to RAM |
| `break` / `bclear ADDR`, `blist` | set, clear and list execution breakpoints |
| `break ADDR if COND` | a conditional breakpoint: stop only when COND is true |
| `trace on` / `off` / `clear` | record every instruction that runs (or stop, or forget) |
| `history [n]` | show the last n recorded instructions (n decimal, 16 if none) |
| `breakr` / `breakw ADDR` | stop when ADDR is read / written (`bclearr`, `bclearw` clear) |
| `watchr` / `watchw ADDR` | log reads / writes of ADDR without stopping (`wclearr`, `wclearw`, `wlist`) |
| `help` | the list |

**Conditions.** `break 2906 if x==3`, `break FFF4 if a==21 && [0200]>2`. A
condition compares registers (`a x y sp pc p`), flags (`n v d i z c`, which are
0 or 1), memory (`[2900]`) and hex numbers, with `==  !=  <  <=  >  >=`,
joined by `&&`. A bare letter means the register or flag, so write the number
C as `$c`. In the Breakpoints window the condition goes in the *if* box, and
the dot beside the line in the disassembly turns orange (hover for the
condition).

**Step out** (`out`, or the button) is the one to use after stepping *into* a
routine: it runs the rest of the routine, calls it makes included, and stops
just after the `JSR` that called it. An interrupt that happens on the way is
not mistaken for the routine returning.

**History.** `trace on`, run, and after a stop `history 20` shows what led
there, for example:

```
  2012161  2900  A9 5A     LDA #$5A      A=02 X=01 Y=09 S=F6 .V-.....
  2012163  2902  20 F4 FF  JSR $FFF4     A=5A X=01 Y=09 S=F6 .V-.....
```

(cycle, address, bytes, instruction, then the registers *before* it ran). An
interrupt taken shows as `interrupt taken`. The last 10,000 are kept, and
recording costs a little speed only while it is on.

A session might go: `writem 2900 A9 5A 20 F4 FF 4C 05 29` (a program that prints
a `Z` through the operating system), `set pc 2900`, `break FFF4`, `c` (it
stops as the OS is about to print), `set a 41` (change the letter to `A`),
`c` (the screen shows `A`). Breakpoints stop *before* the instruction;
read/write breakpoints stop just *after* the instruction that used the
address. A `watchw` log line shows the byte written and the cycle it happened
on; a read-modify-write instruction such as `INC` shows a read and then two
writes (the 6502 writes the old value back before the new one).

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
roms/           the Atom ROMs (see "The ROMs" below)
testdata/       downloaded test suites (git-ignored)
```

Dependencies point one way: `ui` uses `machine`, which uses `cpu`.

## Credits

The memory map, peripheral behaviour and video behaviour follow Tom Walker's
Atomulator. The character font is read from a die photograph of the 6847's ROM
by Robcfg (see the licence section). CPU tests are by Tom Harte and others (SingleStepTests), Klaus
Dormann, and Bruce Clark. The SID chip is the `sidera` crate (MIT).

## Licence

Licensed under the **GNU General Public License, version 2 only** (see
[`LICENSE`](LICENSE)). This follows from Atomulator, which states "licensed
under the GPL" and ships the version 2 text with no "or later" grant. Some of
this code was adapted from Atomulator (the WD1770 disk controller in
`src/machine/fdc1770.rs`, the keyboard matrix table) and more was written
after reading its source (the AtoMMC and RAM/ROM board behaviour, the memory
map), so the project stays GPL until those are redone from independent
sources. The character font was a copy of Atomulator's table; it is now read
from a photograph of the ROM itself (see below), which gives the same table.

The test suites downloaded by `scripts/fetch_tests.sh` are not part of this
repository and keep their own licences.

The character font in `src/machine/font.rs` was read from the die photograph of
the 6847's character ROM in Wikipedia's article on the Motorola 6847 (by Robcfg,
CC BY-SA 4.0; the photograph is not part of this repository). It used to be a
copy of Atomulator's table; the two are identical, as they describe the same ROM.

### The ROMs

The files in `roms/` are **not** covered by this project's licence and are not
my work. They are the Acorn Atom's system ROMs (operating system, BASIC,
floating point, disk filing system and utility ROM), which are copyright
Acorn Computers, the AtoMMC and SDDOS ROMs (SDDOS is by Kees van Oss) and the
BBC BASIC ROMs in the 128K `ramrom.rom` image, and the GDOS/ADOS 2015 ROM
(`gdos2015.rom`). All of them are copied unchanged from the
[Atomulator](https://github.com/hoglet67/Atomulator) repository, which has
distributed them for years so that the emulator can run. They are here for the
same reason: without them an Atom emulator cannot start. If you hold the rights
to any of them and would like one removed, please open an issue and it will be.
