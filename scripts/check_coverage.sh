#!/usr/bin/env bash
# Checks line coverage against the floors in coverage-floors.txt.
#
#   cargo llvm-cov --no-report            # run the tests, recording coverage
#   ./scripts/check_coverage.sh           # fail if any module is below its floor
#   ./scripts/check_coverage.sh --raise   # raise the floors to what has been reached
#
# Each line of coverage-floors.txt is a path prefix and the least percentage of
# its lines that must be covered:
#
#   src/cpu/               95
#   src/machine/via6522.rs 85
#
# A floor only goes up. When tests are added and a module gets better, run with
# --raise and commit the new file, so a later change cannot quietly undo it.
# The total is also written to $GITHUB_OUTPUT (as `total`) for the badge.
set -euo pipefail

floors=coverage-floors.txt
raise=false
[ "${1:-}" = "--raise" ] && raise=true

report=$(mktemp)
trap 'rm -f "$report"' EXIT
cargo llvm-cov report --json --summary-only > "$report"

python3 -I - "$report" "$floors" "$raise" <<'PY'
import json, os, sys

report_path, floors_path, raise_floors = sys.argv[1], sys.argv[2], sys.argv[3] == "true"
data = json.load(open(report_path))["data"][0]

# (covered, total) lines per file, with the path made relative to the project.
files = {}
for entry in data["files"]:
    path = entry["filename"]
    marker = path.rfind("/src/")
    relative = path[marker + 1:] if marker >= 0 else path
    lines = entry["summary"]["lines"]
    files[relative] = (lines["covered"], lines["count"])

total = data["totals"]["lines"]
total_percent = 100.0 * total["covered"] / total["count"]

def percent_for(prefix):
    covered = count = 0
    for name, (c, n) in files.items():
        if name.startswith(prefix):
            covered += c
            count += n
    return (100.0 * covered / count if count else None), count

rows = []
for line in open(floors_path):
    line = line.split("#")[0].strip()
    if line:
        prefix, floor = line.rsplit(None, 1)
        rows.append((prefix, float(floor)))

failed = False
new_rows = []
print(f"{'module':<34} {'covered':>8} {'floor':>6}")
for prefix, floor in rows:
    percent, count = percent_for(prefix)
    if percent is None:
        print(f"{prefix:<34} {'(no code found)':>8} {floor:>6.0f}  <-- no such module")
        failed = True
        new_rows.append((prefix, floor))
        continue
    status = ""
    if percent + 1e-9 < floor:
        status = "  <-- BELOW THE FLOOR"
        failed = True
    elif percent >= floor + 3:
        status = "  (could raise the floor)"
    print(f"{prefix:<34} {percent:>7.1f}% {floor:>6.0f}{status}")
    # Leave a little room (1 point) so a few lines of code do not flap the check.
    new_rows.append((prefix, max(floor, float(int(percent) - 1))))
print(f"{'TOTAL':<34} {total_percent:>7.1f}%")

if os.environ.get("GITHUB_OUTPUT"):
    with open(os.environ["GITHUB_OUTPUT"], "a") as out:
        out.write(f"total={total_percent:.0f}\n")

if raise_floors:
    comments = {}
    for line in open(floors_path):
        if "#" in line and not line.split("#")[0].strip():
            comments.setdefault("head", []).append(line.rstrip("\n"))
    with open(floors_path, "w") as out:
        for line in comments.get("head", []):
            out.write(line + "\n")
        width = max(len(prefix) for prefix, _ in new_rows)
        for prefix, floor in new_rows:
            out.write(f"{prefix:<{width}} {floor:.0f}\n")
    print(f"\nFloors raised in {floors_path}.")
    sys.exit(0)

if failed:
    print("\nCoverage fell below a floor. Add tests, or (if code was deliberately "
          "removed) lower the floor in coverage-floors.txt and say why in the commit.")
    sys.exit(1)
PY
