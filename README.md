# Zippy

> Archive manager untuk Linux — seringan & seresponsif WinRAR, dengan context
> menu klik-kanan yang kaya. Ditulis dengan **Rust + GTK4 / libadwaita**.

**Status:** Sprint 0 — *baseline & scaffold*. Belum fungsional; window kosong
untuk kalibrasi performa + kerangka workspace.

## Arsitektur

Core/frontend split (lihat Planning Doc §2):

```
zippy/
├── core/            # Pure Rust — zero UI dependency (cargo test -p zippy-core)
├── frontend-gtk/    # GTK4 frontend + dispatch verb CLI → binary `zippy`
├── integration/     # Aset desktop: MIME handler, Nautilus/Dolphin/Thunar
└── scripts/         # measure.sh (baseline RSS/startup)
```

## Build

```sh
# Frontend GTK4 (GNOME / KDE / XFCE)
cargo build --release -p zippy

# Core saja, tanpa UI
cargo test -p zippy-core
```

### Dependencies

```sh
# Fedora 43
sudo dnf install p7zip p7zip-plugins unrar gtk4-devel python3-nautilus

# Debian / Ubuntu
sudo apt install p7zip-full unrar libgtk-4-dev python3-nautilus
```

## Sprint 0 — mengukur baseline

```sh
cargo build --release -p zippy
./scripts/measure.sh          # butuh sesi desktop (Wayland/X11)
```

Angka RSS/startup bersifat **arah, bukan hard gate** — dikalibrasi dari hasil
ukur (Planning Doc §1.1, §9.1).

### Hasil kalibrasi awal (Fedora 43 · GNOME Wayland · n=5)

| Metrik | Terukur | Aspirasi (doc) | Catatan |
|--------|---------|----------------|---------|
| Ukuran binary | **432 KB** | — | profil release size-opt (`opt-level="z"`, lto, strip) |
| RSS idle (VmRSS) | **~150 MB** | ~30 MB | lantai ditentukan toolkit GTK4/Mesa, bukan Rust |
| Cold-start | **~700 ms** | ~200 ms | kasar (wall − 800ms settle delay) |

**Kesimpulan kalibrasi:** lantai memori GTK4+libadwaita (~150 MB VmRSS) jauh di
atas aspirasi ~30 MB dan di atas referensi File Roller (~80 MB) — konsisten
dengan prediksi doc bahwa angka ini ditentukan toolkit. RSS via VmRSS menghitung
halaman shared library; PSS kemungkinan lebih rendah. Target performa di-set
sebagai **arah**, bukan gerbang kelulusan rilis. Investigasi pengurangan memori
(mis. renderer GL vs cairo) bisa ditinjau pasca-v1.0 bila perlu.

## Roadmap

| Versi | Fokus | Sprint |
|-------|-------|--------|
| Sprint 0 | Baseline + scaffold workspace | **(ini)** |
| v0.1 | Core MVP: format detection, ZIP/TAR native, 7Z/RAR subprocess, safety, fuzzing | 1–3 |
| v0.2 | GTK4 basic: AdwApplicationWindow, GtkColumnView, extract/compress | 4–5 |
| v0.3 | GTK4 polish: drag & drop, password dialog, cancel, MIME handler | 6–7 |
| v0.4 | Context menu: verb CLI, Nautilus/Dolphin/Thunar | 8–9 |
| v1.0 | install.sh, README, screenshots, polish | 10 |

## Lisensi

TBD: MIT vs GPL-2.0 (Planning Doc §11.3).
