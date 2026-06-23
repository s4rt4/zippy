#!/usr/bin/env bash
# Pasang Zippy + integrasi desktop/file-manager ke ~/.local (per-user, tanpa root).
#
#   ./data/install.sh           # build release + pasang
#   PREFIX=/usr/local sudo ./data/install.sh   # sistem-wide (binari + .desktop)
#
# Integrasi file manager dipasang sesuai DE yang terpasang:
#   - GNOME Files (Nautilus): butuh `nautilus-python` (Fedora: python3-nautilus)
#   - KDE Dolphin: ServiceMenu (tanpa dependensi tambahan)
#   - XFCE Thunar: Custom Actions di ~/.config/Thunar/uca.xml
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
DATA="$HOME/.local/share"   # integrasi FM selalu per-user (ikut $HOME)

echo "==> Build release"
( cd "$REPO" && cargo build --release -p zippy )

echo "==> Pasang biner → $PREFIX/bin/zippy"
install -Dm755 "$REPO/target/release/zippy" "$PREFIX/bin/zippy"

echo "==> Pasang ikon aplikasi"
install -Dm644 "$REPO/data/icons/io.github.s4rt4.Zippy.svg" \
    "$PREFIX/share/icons/hicolor/scalable/apps/io.github.s4rt4.Zippy.svg"
gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "==> Pasang .desktop + MIME"
install -Dm644 "$REPO/data/io.github.s4rt4.Zippy.desktop" \
    "$PREFIX/share/applications/io.github.s4rt4.Zippy.desktop"
update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true

# --- GNOME / Nautilus ---
echo "==> Nautilus (GNOME)"
install -Dm644 "$REPO/data/zippy-nautilus.py" \
    "$DATA/nautilus-python/extensions/zippy-nautilus.py"
echo "    terpasang (butuh paket nautilus-python; reload: nautilus -q)"

# --- KDE / Dolphin ---
if command -v dolphin >/dev/null 2>&1 || command -v plasmashell >/dev/null 2>&1; then
    echo "==> KDE Dolphin (ServiceMenu)"
    install -Dm755 "$REPO/data/kde/zippy-extract.desktop" \
        "$DATA/kio/servicemenus/zippy-extract.desktop"
    install -Dm755 "$REPO/data/kde/zippy-compress.desktop" \
        "$DATA/kio/servicemenus/zippy-compress.desktop"
    echo "    terpasang (restart Dolphin agar muncul)"
else
    echo "==> KDE Dolphin tidak terdeteksi — lewati (file ada di data/kde/)"
fi

# --- XFCE / Thunar ---
if command -v thunar >/dev/null 2>&1; then
    echo "==> XFCE Thunar (Custom Actions → uca.xml)"
    python3 - "$REPO/data/thunar/zippy-uca.xml" "$HOME/.config/Thunar/uca.xml" <<'PY'
import os, sys, xml.etree.ElementTree as ET

frag_path, target = sys.argv[1], sys.argv[2]
frag = ET.fromstring("<root>" + open(frag_path, encoding="utf-8").read() + "</root>")

os.makedirs(os.path.dirname(target), exist_ok=True)
if os.path.exists(target):
    root = ET.parse(target).getroot()
else:
    root = ET.Element("actions")

# Buang aksi Zippy lama (idempoten saat re-run), lalu sisipkan yang baru.
for a in list(root):
    if (a.findtext("unique-id") or "").startswith("zippy-"):
        root.remove(a)
for a in frag:
    root.append(a)

ET.indent(root, space="\t")
ET.ElementTree(root).write(target, encoding="UTF-8", xml_declaration=True)
print(f"    {target} diperbarui")
PY
    echo "    (tutup & buka lagi Thunar agar muncul)"
else
    echo "==> XFCE Thunar tidak terdeteksi — lewati (file ada di data/thunar/)"
fi

echo
echo "Selesai. Pastikan '$PREFIX/bin' ada di PATH."
echo "GNOME: sudo dnf install nautilus-python && nautilus -q"
