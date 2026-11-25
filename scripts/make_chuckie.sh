#!/usr/bin/env bash
# Builds testdata/games/chuckie.ssd (Chuckie Egg). Same as
#   ./scripts/make_game.sh RS/CHUCKIE chuckie
# and kept under this name because the README and tests refer to it.
set -euo pipefail
cd "$(dirname "$0")/.."
exec ./scripts/make_game.sh RS/CHUCKIE chuckie
