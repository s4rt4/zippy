# Integrasi desktop & file manager

File di sini menghubungkan Zippy ke desktop Linux: asosiasi MIME ("Open With
Zippy"), dan menu klik-kanan di Nautilus (GNOME Files).

| Berkas | Fungsi |
|--------|--------|
| `io.github.s4rt4.Zippy.desktop` | Launcher + asosiasi MIME (double-click / "Open With") |
| `zippy-nautilus.py` | Ekstensi `nautilus-python`: submenu klik-kanan **Zippy** |
| `install.sh` | Build release + pasang semua ke `~/.local` (atau `PREFIX`) |

## Pasang

```sh
./data/install.sh
```

Lalu, agar menu klik-kanan muncul, pasang `nautilus-python` dan reload:

```sh
sudo dnf install nautilus-python   # Fedora
nautilus -q
```

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
