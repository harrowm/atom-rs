#!/usr/bin/env bash
# Downloads and unpacks the Atom Software Archive into testdata/asa_v13_01/
# (about 17 MB to download, 5,000 files). Safe to run again: it only fetches
# what is missing.
#
# The archive is a huge library of Atom software gathered at stardot.org.uk and
# maintained at https://github.com/hoglet67/AtomSoftwareArchive. It is not part
# of this repository; it is only downloaded to your machine.
set -euo pipefail

cd "$(dirname "$0")/.."

URL=https://github.com/hoglet67/AtomSoftwareArchive/releases/download/V13_01/AtomSoftwareArchive_20250717_1734_V13.01.zip
DIR=testdata/asa_v13_01

mkdir -p "$DIR"
if [ ! -f "$DIR/archive.zip" ]; then
    echo "Downloading the Atom Software Archive..."
    curl -fsSL -o "$DIR/archive.zip" "$URL"
fi
if [ ! -d "$DIR/files/ASA" ]; then
    echo "Unpacking it..."
    unzip -q -o "$DIR/archive.zip" -d "$DIR/files"
fi
echo "Archive ready: browse $DIR/files/ASA/"
