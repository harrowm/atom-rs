#!/usr/bin/env bash
# Writes a "tests | N passed" badge as an SVG, in the style of shields.io.
#
#   ./scripts/make_badge.sh 157 > badges/tests.svg
#
# CI runs this after the tests pass on master, with the real count, and commits
# the result if it changed. (shields.io cannot read a private repository, and
# this needs no secrets.)
set -euo pipefail

count=${1:?usage: make_badge.sh PASSED_COUNT}
label="tests"
value="$count passed"

# Text widths are approximate: about 6.2 pixels per character at this size,
# plus padding either side.
awk -v label="$label" -v value="$value" 'BEGIN {
    lw = int(length(label) * 6.2 + 10)
    rw = int(length(value) * 6.2 + 10)
    w = lw + rw
    printf "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"%d\" height=\"20\" role=\"img\" aria-label=\"%s: %s\">\n", w, label, value
    printf "<title>%s: %s</title>\n", label, value
    printf "<linearGradient id=\"s\" x2=\"0\" y2=\"100%%\"><stop offset=\"0\" stop-color=\"#bbb\" stop-opacity=\".1\"/><stop offset=\"1\" stop-opacity=\".1\"/></linearGradient>\n"
    printf "<clipPath id=\"r\"><rect width=\"%d\" height=\"20\" rx=\"3\" fill=\"#fff\"/></clipPath>\n", w
    printf "<g clip-path=\"url(#r)\"><rect width=\"%d\" height=\"20\" fill=\"#555\"/><rect x=\"%d\" width=\"%d\" height=\"20\" fill=\"#4c1\"/><rect width=\"%d\" height=\"20\" fill=\"url(#s)\"/></g>\n", lw, lw, rw, w
    printf "<g fill=\"#fff\" text-anchor=\"middle\" font-family=\"Verdana,Geneva,DejaVu Sans,sans-serif\" text-rendering=\"geometricPrecision\" font-size=\"11\">\n"
    printf "<text x=\"%.1f\" y=\"14\">%s</text><text x=\"%.1f\" y=\"14\">%s</text>\n", lw / 2, label, lw + rw / 2, value
    printf "</g></svg>\n"
}'
