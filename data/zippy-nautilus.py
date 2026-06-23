"""Ekstensi context-menu Nautilus untuk Zippy (Planning Doc §6.1).

Menambah submenu "Zippy" pada klik-kanan di file manager:
  - bila semua item terpilih adalah archive → Extract Here / Extract to
    Subfolder / Test Archive / Open with Zippy
  - selain itu → Compress with Zippy

Tiap aksi memanggil biner `zippy <verb> <paths>` (lihat src/cli.rs).

Butuh paket `nautilus-python` (Fedora: python3-nautilus). Pasang ke
~/.local/share/nautilus-python/extensions/ lalu `nautilus -q` untuk reload.
"""

import subprocess

import gi

gi.require_version("Nautilus", "4.0")
from gi.repository import GObject, Nautilus  # noqa: E402

ARCHIVE_EXTS = (
    ".zip", ".7z", ".rar", ".tar",
    ".gz", ".tgz", ".bz2", ".tbz2", ".xz", ".txz", ".zst",
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst",
)


def _path(file_info):
    loc = file_info.get_location()
    return loc.get_path() if loc is not None else None


def _is_archive(file_info):
    p = _path(file_info)
    return bool(p) and p.lower().endswith(ARCHIVE_EXTS)


class ZippyMenuProvider(GObject.GObject, Nautilus.MenuProvider):
    def _run(self, verb, files):
        paths = [p for p in (_path(f) for f in files) if p]
        if paths:
            subprocess.Popen(["zippy", verb, *paths])

    def _item(self, name, label, verb, files):
        item = Nautilus.MenuItem(name=name, label=label)
        item.connect("activate", lambda _m: self._run(verb, files))
        return item

    def get_file_items(self, files):
        if not files:
            return []
        # Hanya berkas lokal (punya path filesystem).
        if any(_path(f) is None for f in files):
            return []

        top = Nautilus.MenuItem(name="ZippyMenuProvider::top", label="Zippy")
        submenu = Nautilus.Menu()
        top.set_submenu(submenu)

        if all(_is_archive(f) for f in files):
            submenu.append_item(
                self._item("Zippy::extract_here", "Extract Here", "--extract-here", files)
            )
            submenu.append_item(
                self._item(
                    "Zippy::extract_sub",
                    "Extract to Subfolder",
                    "--extract-to-subfolder",
                    files,
                )
            )
            submenu.append_item(
                self._item("Zippy::test", "Test Archive", "--test", files)
            )
            submenu.append_item(
                self._item("Zippy::open", "Open with Zippy", "--open", files[:1])
            )
        else:
            submenu.append_item(
                self._item("Zippy::add", "Compress with Zippy", "--add", files)
            )

        return [top]

    # Klik-kanan di area kosong folder → tawarkan "Compress" untuk folder ini.
    def get_background_items(self, current_folder):
        return []
