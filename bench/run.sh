#!/usr/bin/env bash
set -euo pipefail

# Benchmark steamroom vs DepotDownloader
# Usage: ./bench/run.sh [SCRATCH_DIR]
#
# Requires: hyperfine, steamroom (built), DepotDownloader (dotnet tool)
# Run inside nix develop shell for all deps.

SCRATCH="${1:-/mnt/g/tmp/steamroom-bench}"
STEAMROOM="${STEAMROOM:-steamroom}"
DD="${DD:-DepotDownloader}"

mkdir -p "$SCRATCH"
RESULTS="$SCRATCH/results"
mkdir -p "$RESULTS"

echo "=== Benchmark Configuration ==="
echo "  Scratch dir: $SCRATCH"
echo "  steamroom:   $STEAMROOM"
echo "  DD:          $DD"
echo ""

# Helper: clean a download directory before each run
clean() {
  rm -rf "$1" 2>/dev/null || true
}

# ──────────────────────────────────────────────────────────
# 1. App info query (no download, measures login + PICS speed)
# ──────────────────────────────────────────────────────────
echo "=== Benchmark: info query (app 480) ==="
hyperfine \
  --warmup 1 \
  --min-runs 5 \
  --export-json "$RESULTS/info.json" \
  --command-name "steamroom" \
    "$STEAMROOM info --app 480" \
  --command-name "DepotDownloader" \
    "$DD -app 480 -list-only -dir $SCRATCH/dd-info" \
  2>&1 | tee "$RESULTS/info.txt"
echo ""

# ──────────────────────────────────────────────────────────
# 2. Small download: Spacewar (app 480, depot 481, ~1.8 MB)
# ──────────────────────────────────────────────────────────
echo "=== Benchmark: download Spacewar (~1.8 MB) ==="
hyperfine \
  --warmup 0 \
  --min-runs 3 \
  --export-json "$RESULTS/spacewar.json" \
  --prepare "rm -rf $SCRATCH/sr-spacewar $SCRATCH/dd-spacewar" \
  --command-name "steamroom" \
    "$STEAMROOM download --app 480 --depot 481 -o $SCRATCH/sr-spacewar" \
  --command-name "DepotDownloader" \
    "$DD -app 480 -depot 481 -dir $SCRATCH/dd-spacewar" \
  2>&1 | tee "$RESULTS/spacewar.txt"
echo ""

# ──────────────────────────────────────────────────────────
# 3. File listing: Spacewar manifest (measures manifest fetch + parse + decrypt)
# ──────────────────────────────────────────────────────────
echo "=== Benchmark: file listing (app 480, depot 481) ==="
hyperfine \
  --warmup 1 \
  --min-runs 5 \
  --export-json "$RESULTS/files.json" \
  --command-name "steamroom" \
    "$STEAMROOM files --app 480 --depot 481 --format plain" \
  --command-name "DepotDownloader" \
    "$DD -app 480 -depot 481 -list-only -dir $SCRATCH/dd-files" \
  2>&1 | tee "$RESULTS/files.txt"
echo ""

# ──────────────────────────────────────────────────────────
# 4. Large download: CS2 pak01 subset (~2.5 GB, depot 2347770)
#    Uses a filelist with regex prefix for DD compatibility
# ──────────────────────────────────────────────────────────
echo "=== Benchmark: download CS2 pak01 subset (~2.5 GB) ==="
echo "  (This will take a while depending on your connection)"
DD_FILELIST="$SCRATCH/cs2-filelist.txt"
echo 'regex:pak01_0[01][0-9]\.vpk$' > "$DD_FILELIST"
hyperfine \
  --warmup 0 \
  --min-runs 1 \
  --export-json "$RESULTS/cs2.json" \
  --prepare "rm -rf $SCRATCH/sr-cs2 $SCRATCH/dd-cs2" \
  --command-name "steamroom" \
    "$STEAMROOM download --app 730 --depot 2347770 --file-regex 'pak01_0[01][0-9]\\.vpk$' -o $SCRATCH/sr-cs2" \
  --command-name "DepotDownloader" \
    "$DD -app 730 -depot 2347770 -filelist $DD_FILELIST -dir $SCRATCH/dd-cs2" \
  2>&1 | tee "$RESULTS/cs2.txt"
echo ""

# ──────────────────────────────────────────────────────────
# Summary
# ──────────────────────────────────────────────────────────
echo "=== Results saved to $RESULTS/ ==="
echo ""
for f in "$RESULTS"/*.txt; do
  echo "--- $(basename "$f" .txt) ---"
  tail -3 "$f"
  echo ""
done
