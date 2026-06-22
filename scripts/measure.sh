#!/usr/bin/env bash
# Sprint 0 — ukur baseline cold-start & idle RSS window kosong GTK4+libadwaita.
# (Planning Doc §9.1). Membutuhkan display aktif (Wayland/X11).
#
# Pemakaian:
#   cargo build --release -p zippy
#   ./scripts/measure.sh [jumlah_run]
set -euo pipefail

BIN="${BIN:-target/release/zippy}"
RUNS="${1:-5}"

if [[ ! -x "$BIN" ]]; then
  echo "Binary tidak ditemukan: $BIN" >&2
  echo "Build dulu: cargo build --release -p zippy" >&2
  exit 1
fi

if [[ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]]; then
  echo "Tidak ada display (WAYLAND_DISPLAY/DISPLAY). Jalankan di sesi desktop." >&2
  exit 1
fi

echo "Mengukur '$BIN' sebanyak $RUNS run (ZIPPY_BENCH=1)..."
echo

total_ms=0
total_rss=0
for i in $(seq 1 "$RUNS"); do
  start=$(date +%s%N)
  rss_line=$(ZIPPY_BENCH=1 "$BIN" 2>/dev/null | grep -m1 ZIPPY_BENCH || true)
  end=$(date +%s%N)

  wall_ms=$(( (end - start) / 1000000 ))
  rss_mb=$(echo "$rss_line" | sed -n 's/.*rss_mb=\([0-9.]*\).*/\1/p')
  rss_mb=${rss_mb:-0}

  printf "  run %d: wall=%4d ms  rss=%6s MB\n" "$i" "$wall_ms" "$rss_mb"
  total_ms=$(( total_ms + wall_ms ))
  total_rss=$(awk -v a="$total_rss" -v b="$rss_mb" 'BEGIN{printf "%.1f", a+b}')
done

echo
awk -v ms="$total_ms" -v rss="$total_rss" -v n="$RUNS" \
  'BEGIN{printf "Rata-rata: wall=%.0f ms  rss=%.1f MB  (n=%d)\n", ms/n, rss/n, n}'
echo
echo "Catatan: wall mencakup ~800ms settle delay dari ZIPPY_BENCH."
echo "Gunakan angka ini untuk kalibrasi 'arah' performa (bukan hard gate)."
