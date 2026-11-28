#!/usr/bin/env bash
# Writes a "tests | N passed" badge as an SVG, in the style of shields.io.
#
#   ./scripts/make_badge.sh 157 > badges/tests.svg
#   ./scripts/make_badge.sh 91% coverage > badges/coverage.svg
#
# With only a number it makes the tests badge ("157 passed"). Given a label as
# well, the value is shown as it is, and the right-hand colour reflects it when
# the value is a percentage (green from 80, yellow from 60, red below).
#
# CI runs this after the tests pass on master, with the real count, and commits
# the result if it changed. (shields.io cannot read a private repository, and
# this needs no secrets.)
set -euo pipefail

count=${1:?usage: make_badge.sh PASSED_COUNT | VALUE LABEL}
if [ $# -ge 2 ]; then
    label="$2"
    value="$count"
else
    label="tests"
    value="$count passed"
fi
colour="#4c1"
case "$value" in
    *%)
        percent=${value%\%}
        percent=${percent%.*}
        if [ "$percent" -lt 60 ]; then colour="#e05d44"
        elif [ "$percent" -lt 80 ]; then colour="#dfb317"
        fi
        ;;
esac

# Text widths are approximate: about 6.2 pixels per character at this size,
# plus padding either side.
awk -v label="$label" -v value="$value" -v colour="$colour" 'BEGIN {
    lw = int(length(label) * 6.2 + 10)
    rw = int(length(value) * 6.2 + 10)
    w = lw + rw
    printf "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"%d\" height=\"20\" role=\"img\" aria-label=\"%s: %s\">\n", w, label, value
    printf "<title>%s: %s</title>\n", label, value
    printf "<linearGradient id=\"s\" x2=\"0\" y2=\"100%%\"><stop offset=\"0\" stop-color=\"#bbb\" stop-opacity=\".1\"/><stop offset=\"1\" stop-opacity=\".1\"/></linearGradient>\n"
    printf "<clipPath id=\"r\"><rect width=\"%d\" height=\"20\" rx=\"3\" fill=\"#fff\"/></clipPath>\n", w
    printf "<g clip-path=\"url(#r)\"><rect width=\"%d\" height=\"20\" fill=\"#555\"/><rect x=\"%d\" width=\"%d\" height=\"20\" fill=\"%s\"/><rect width=\"%d\" height=\"20\" fill=\"url(#s)\"/></g>\n", lw, lw, rw, colour, w
    printf "<g fill=\"#fff\" text-anchor=\"middle\" font-family=\"Verdana,Geneva,DejaVu Sans,sans-serif\" text-rendering=\"geometricPrecision\" font-size=\"11\">\n"
    printf "<text x=\"%.1f\" y=\"14\">%s</text><text x=\"%.1f\" y=\"14\">%s</text>\n", lw / 2, label, lw + rw / 2, value
    printf "</g></svg>\n"
}'
