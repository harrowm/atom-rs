#!/usr/bin/env bash
# Downloads the external 6502 test suites into testdata/ (git-ignored).
#
#   testdata/65x02/            SingleStepTests (6502/v1 only): per-opcode JSON,
#                              cycle-by-cycle
#   testdata/6502_65C02_functional_tests/   Klaus Dormann's functional tests
#   testdata/amb5l_ca65/       ca65 port of Dormann's sources; we build the
#                              decimal test from it (needs `brew install cc65`)
#   testdata/mre_interrupt/    prebuilt interrupt test (from the mre/mos6502 repo)
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p testdata

clone() { # clone <url> <dir>: shallow clone, or update if already present
    if [ -d "$2/.git" ]; then
        git -C "$2" pull --ff-only
    else
        git clone --depth 1 "$1" "$2"
    fi
}

# The SingleStepTests repo is 5.7 GB, covering many CPU variants, but only the
# plain 6502 folder (1 GB, about 190 MB to download) is used, so fetch only that.
if [ ! -d testdata/65x02/.git ]; then
    git clone --depth 1 --filter=blob:none --sparse \
        https://github.com/SingleStepTests/65x02.git testdata/65x02
    git -C testdata/65x02 sparse-checkout set 6502/v1
else
    git -C testdata/65x02 pull --ff-only
fi
clone https://github.com/Klaus2m5/6502_65C02_functional_tests.git \
    testdata/6502_65C02_functional_tests

# Dormann's repo only ships a binary for the functional test. Bruce Clark's
# decimal test has to be assembled; as65 (Dormann's assembler) is x86-only, so
# use the ca65 port with cc65's ca65/ld65.
clone https://github.com/amb5l/6502_65C02_functional_tests.git testdata/amb5l_ca65
if command -v ca65 >/dev/null && command -v ld65 >/dev/null; then
    (cd testdata/amb5l_ca65/ca65 &&
        ca65 -l 6502_decimal_test.lst 6502_decimal_test.ca65 &&
        ld65 6502_decimal_test.o -o 6502_decimal_test.bin -C example.cfg 2>/dev/null)
else
    echo "NOTE: ca65 not found (brew install cc65); decimal test will be skipped"
fi

# Prebuilt interrupt test image (the "concurrent BRK and NMI" check is disabled).
mkdir -p testdata/mre_interrupt
curl -fsSL -o testdata/mre_interrupt/6502_interrupt_test.bin \
    https://raw.githubusercontent.com/mre/mos6502/master/tests/assets/6502_interrupt_test.bin

echo "Test data ready in testdata/"
