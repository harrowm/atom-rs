#!/usr/bin/env bash
# Builds a disk image for every game listed in the README's games table, into
# testdata/games/. Downloads the Atom Software Archive first if needed.
#
#   ./scripts/make_all_games.sh
set -euo pipefail

cd "$(dirname "$0")/.."
./scripts/get_archive.sh

# archive path (under ASA/)    disk name
while read -r path name; do
    ./scripts/make_game.sh "$path" "$name" | grep -E '^(wrote|note)' || true
done <<'GAMES'
RS/CHUCKIE            chuckie
RS/JSW                jsw
RS/REPTON             repton
RS/ELITE              elite
AS/GAMES09/SNAPPER    snapper
BB/PINBALL            pinball
AF/PANIC              panic
AF/FROGGER            frogger
AA/GALAXIAN           galaxian
AA/INVADER            invaders
AS/GAMES01/ASTEROID   asteroid
AS/GAMES01/BREAKOUT   breakout
GAMES

echo
echo "Disks are in testdata/games/. See the README for how to run each one."
