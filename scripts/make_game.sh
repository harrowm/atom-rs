#!/usr/bin/env bash
# Packs a program from the Atom Software Archive onto a disk image.
#
#   ./scripts/make_game.sh AF/PANIC         # one program (a single file)
#   ./scripts/make_game.sh RS/CHUCKIE       # a folder of files
#   ./scripts/make_game.sh AA/GALAXIAN galaxian   # choose the disk's name
#
# The path is relative to the archive's ASA folder: browse
# testdata/asa_v13_01/files/ASA/ to see what is there. If you have not
# downloaded the archive yet, run ./scripts/make_chuckie.sh once first.
#
# The disk is written to testdata/games/NAME.ssd and the command to run it is
# printed.
set -euo pipefail

cd "$(dirname "$0")/.."

if [ $# -lt 1 ]; then
    sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
fi

ROOT=testdata/asa_v13_01/files/ASA
SOURCE="$ROOT/$1"
NAME=${2:-$(basename "$1" | tr 'A-Z' 'a-z')}
OUT="testdata/games/$NAME.ssd"

if [ ! -e "$SOURCE" ]; then
    echo "Not found: $SOURCE" >&2
    echo "(download the archive first with ./scripts/make_chuckie.sh)" >&2
    exit 1
fi
mkdir -p testdata/games

if [ -d "$SOURCE" ]; then
    FILES=$(find "$SOURCE" -maxdepth 1 -type f ! -name '*.TXT' ! -name '*.txt' | sort)
else
    FILES=$SOURCE
fi

# shellcheck disable=SC2086
cargo run --release --quiet --example atm2ssd -- "$OUT" $FILES

echo
echo "Run it with:"
echo "  cargo run --release -- --disk $OUT --run <the text above>"
echo "(for a folder of files the loader is usually named RUNME or LOADER; use"
echo "'*RUN RUNME\\n' or, for a BASIC loader that stops at the prompt,"
echo "'LOAD \"JSWRUN\"\\nRUN\\n')."
