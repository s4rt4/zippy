# Integrasi desktop & file manager

File di sini menghubungkan Zippy ke desktop Linux: asosiasi MIME ("Open With
Zippy"), dan menu klik-kanan di Nautilus (GNOME Files).

| Berkas | Fungsi |
|--------|--------|
| `io.github.s4rt4.Zippy.desktop` | Launcher + asosiasi MIME (double-click / "Open With") |
| `zippy-nautilus.py` | GNOME Files: ekstensi `nautilus-python`, submenu **Zippy** |
| `kde/zippy-*.desktop` | KDE Dolphin: ServiceMenu (extract + compress) |
| `thunar/zippy-uca.xml` | XFCE Thunar: Custom Actions |
| `install.sh` | Build release + pasang sesuai DE yang terpasang |

## Pasang

```sh
./data/install.sh
```

`install.sh` mendeteksi DE yang ada dan memasang integrasi yang sesuai. Lalu
aktifkan per file manager:

| DE | Aktivasi |
|----|----------|
| **GNOME Files** | `sudo dnf install nautilus-python` lalu `nautilus -q` |
| **KDE Dolphin** | Restart Dolphin (ServiceMenu, tanpa dependensi) |
| **XFCE Thunar** | Tutup & buka lagi Thunar (aksi di-merge ke `~/.config/Thunar/uca.xml`, aksi user lain dipertahankan) |

## Menu klik-kanan (Nautilus)

- **Semua arsip terpilih** → Extract Here · Extract to Subfolder · Test
  Archive · Open with Zippy
- **Berkas lain** → Compress with Zippy

Tiap aksi memanggil `zippy <verb> <paths>` (lihat `../frontend-gtk/src/cli.rs`).

## Verb CLI (bisa dipakai langsung / file manager lain)

```
zippy --extract-here <arsip…>            # extract ke folder arsip
zippy --extract-to-subfolder <arsip…>    # extract ke sub-folder senama
zippy --extract-to <arsip>               # extract via dialog GUI
zippy --test <arsip…>                    # uji integritas
zippy --add <berkas…>                    # kompres jadi .zip
zippy --open <arsip> | zippy <arsip>     # buka di GUI
```
