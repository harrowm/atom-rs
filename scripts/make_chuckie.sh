#!/usr/bin/env bash
# Builds testdata/games/chuckie.ssd: Chuckie Egg on a floppy disk image that the
# emulator can run. It downloads the Atom Software Archive (about 17 MB, from
# https://github.com/hoglet67/AtomSoftwareArchive), takes the Retro Software
# version of Chuckie Egg out of it and packs it onto a disk with atm2ssd.
#
# The game is not part of this repository; it is only downloaded to your machine.
#
# Run it, then:
#   cargo run --release -- --disk testdata/games/chuckie.ssd --run '*RUN RUNME\n'
set -euo pipefail

cd "$(dirname "$0")/.."

URL=https://github.com/hoglet67/AtomSoftwareArchive/releases/download/V13_01/AtomSoftwareArchive_20250717_1734_V13.01.zip
ARCHIVE_DIR=testdata/asa_v13_01
GAME_DIR="$ARCHIVE_DIR/files/ASA/RS/CHUCKIE"

mkdir -p "$ARCHIVE_DIR" testdata/games

if [ ! -f "$ARCHIVE_DIR/archive.zip" ]; then
    echo "Downloading the Atom Software Archive..."
    curl -fsSL -o "$ARCHIVE_DIR/archive.zip" "$URL"
fi
if [ ! -d "$GAME_DIR" ]; then
    unzip -q -o "$ARCHIVE_DIR/archive.zip" 'ASA/RS/CHUCKIE/*' -d "$ARCHIVE_DIR/files"
fi

# RUNME is the game's loader; it loads the other files itself.
cargo run --release --example atm2ssd -- testdata/games/chuckie.ssd \
    "$GAME_DIR/RUNME" "$GAME_DIR/ASM" "$GAME_DIR/FONT" "$GAME_DIR/ROTDATA" \
    "$GAME_DIR/MENU" "$GAME_DIR/MCHUCK" "$GAME_DIR/CCHUCK" \
    "$GAME_DIR/MCH-EGG" "$GAME_DIR/CCH-EGG"

echo
echo "Now run:"
echo "  cargo run --release -- --disk testdata/games/chuckie.ssd --run '*RUN RUNME\\n'"
