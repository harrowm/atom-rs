#!/usr/bin/env bash
# Downloads the die photograph of the MC6847's character ROM that
# src/machine/font.rs was read from, into testdata/font/ (git-ignored).
#
# It is a Wikimedia Commons file by Robcfg, licensed CC BY-SA 4.0
# (https://commons.wikimedia.org/wiki/File:MC6847_Character_ROM_memory_organization.png),
# so it is not kept in this repository. tests/font_source.rs checks the font
# against it when it is there.
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p testdata/font
file=testdata/font/MC6847_Character_ROM_memory_organization.png
url=https://upload.wikimedia.org/wikipedia/commons/6/6c/MC6847_Character_ROM_memory_organization.png

if [ ! -f "$file" ]; then
    curl -sSL --fail -A "atom-rs (https://github.com/harrowm/atom-rs)" -o "$file" "$url"
fi
echo "ok: $file ($(wc -c < "$file") bytes)"
