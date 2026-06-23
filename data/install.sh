#!/usr/bin/env bash
# Pasang Zippy + integrasi desktop/Nautilus ke ~/.local (per-user, tanpa root).
#
#   ./data/install.sh           # build release + pasang
#   PREFIX=/usr/local sudo ./data/install.sh   # sistem-wide
#
# Butuh paket `nautilus-python` agar menu klik-kanan muncul
# (Fedora: sudo dnf install nautilus-python).
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"

echo "==> Build release"
( cd "$REPO" && cargo build --release -p zippy )

echo "==> Pasang biner → $PREFIX/bin/zippy"
install -Dm755 "$REPO/target/release/zippy" "$PREFIX/bin/zippy"

echo "==> Pasang .desktop + MIME"
install -Dm644 "$REPO/data/io.github.s4rt4.Zippy.desktop" \
    "$PREFIX/share/applications/io.github.s4rt4.Zippy.desktop"
update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true

echo "==> Pasang ekstensi Nautilus"
install -Dm644 "$REPO/data/zippy-nautilus.py" \
    "$PREFIX/share/nautilus-python/extensions/zippy-nautilus.py"

echo
echo "Selesai."
echo " - Pastikan '$PREFIX/bin' ada di PATH."
echo " - Pasang nautilus-python bila menu belum muncul (Fedora: sudo dnf install nautilus-python)."
echo " - Reload file manager:  nautilus -q"
