"""Ekstensi context-menu Nautilus untuk Zippy (Planning Doc §6.1–6.2).

Menu kondisional per seleksi, dengan label dinamis (mis. Extract to "X/"):

  A. file/folder biasa  → Add to archive… · Add to "<nama>.zip"
  B. SATU archive       → Extract Here · Extract to "<nama>/" · Extract to… ·
                          Open with Zippy · Test integrity · Add to archive…
  C. BANYAK archive     → Add to archive… · Add to "<folder-induk>.zip" ·
                          Extract Here · Extract each to separate folder ·
                          Test integrity
  D. seleksi campuran   → Add to archive… · Add to "<folder-induk>.zip"

Tiap entri memanggil `zippy <verb> <paths>` (lihat src/cli.rs); semua logika
berat ada di binary, definisi menu setipis mungkin.

Butuh paket `nautilus-python` (Fedora: python3-nautilus). Pasang ke
~/.local/share/nautilus-python/extensions/ lalu `nautilus -q`.
"""

import os
import subprocess

import gi

# Versi namespace Nautilus berbeda antar-distro (Fedora 43 = 4.1). Pilih versi
# yang tersedia / sudah ter-load oleh host.
for _v in ("4.1", "4.0"):
    try:
        gi.require_version("Nautilus", _v)
        break
    except ValueError:
        continue
from gi.repository import GObject, Nautilus  # noqa: E402

ARCHIVE_MIMES = {
    "application/zip",
    "application/x-7z-compressed",
    "application/vnd.rar",
    "application/x-rar",
    "application/x-rar-compressed",
    "application/x-tar",
    "application/gzip",
    "application/x-gzip",
    "application/x-bzip2",
    "application/x-xz",
    "application/zstd",
    "application/x-zstd",
    "application/x-compressed-tar",
    "application/x-bzip-compressed-tar",
    "application/x-xz-compressed-tar",
    "application/x-zstd-compressed-tar",
}

ARCHIVE_EXTS = (
    ".zip", ".7z", ".rar", ".tar",
    ".gz", ".tgz", ".bz2", ".tbz2", ".xz", ".txz", ".zst",
    ".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst",
)

# Ekstensi dua-lapis (di-strip duluan agar "foo.tar.gz" → "foo").
_MULTI_EXTS = (".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".txz")


def _path(file_info):
    loc = file_info.get_location()
    return loc.get_path() if loc is not None else None


def _is_archive(file_info):
    if file_info.get_mime_type() in ARCHIVE_MIMES:
        return True
    p = _path(file_info)
    return bool(p) and p.lower().endswith(ARCHIVE_EXTS)


def _strip_archive_ext(name):
    low = name.lower()
    for ext in _MULTI_EXTS:
        if low.endswith(ext):
            return name[: -len(ext)]
    stem, dot, _ = name.rpartition(".")
    return stem if dot and stem else name


def _strip_ext(name):
    """Buang satu ekstensi terakhir; dotfile/.tanpa-titik dibiarkan."""
    stem, dot, _ = name.rpartition(".")
    return stem if dot and stem else name


def _quick_zip_name(files):
    """Nama .zip auto (gaya WinRAR): satu input → ekstensi dibuang dulu
    ('photo.png' → 'photo.zip'); banyak input → '<folder-induk>.zip'."""
    if len(files) == 1:
        return _strip_ext(os.path.basename(_path(files[0]))) + ".zip"
    parent = os.path.dirname(_path(files[0]))
    folder = os.path.basename(parent) or "archive"
    return folder + ".zip"


class ZippyMenuProvider(GObject.GObject, Nautilus.MenuProvider):
    # ---- helper ----
    def _run(self, _menu, verb, files):
        paths = [p for p in (_path(f) for f in files) if p]
        if paths:
            subprocess.Popen(["zippy", verb, *paths])

    def _item(self, name, label, verb, files):
        item = Nautilus.MenuItem(name=name, label=label)
        item.connect("activate", self._run, verb, files)
        return item

    def _add_items(self, files):
        """Dua entri Add (konteks A & D, juga ekor konteks B/C)."""
        return [
            self._item("Zippy::add", "Add to archive…", "--add", files),
            self._item(
                "Zippy::add_quick",
                f'Add to "{_quick_zip_name(files)}"',
                "--add-quick",
                files,
            ),
        ]

    # ---- entry point ----
    def get_file_items(self, files):
        if not files or any(_path(f) is None for f in files):
            return []

        archives = [f for f in files if _is_archive(f)]
        all_archives = len(archives) == len(files)

        if not all_archives:
            # Konteks A (1 non-archive) / D (campuran) → prioritaskan Add.
            return self._add_items(files)

        if len(files) == 1:
            # Konteks B — satu archive.
            f = files[0]
            base = _strip_archive_ext(os.path.basename(_path(f)))
            return [
                self._item("Zippy::xhere", "Extract Here", "--extract-here", files),
                self._item(
                    "Zippy::xsub", f'Extract to "{base}/"', "--extract-to-subfolder", files
                ),
                self._item("Zippy::xto", "Extract to…", "--extract-to", files),
                self._item("Zippy::open", "Open with Zippy", "--open", files),
                self._item("Zippy::test", "Test integrity", "--test", files),
                *self._add_items(files),
            ]

        # Konteks C — banyak archive.
        return [
            *self._add_items(files),
            self._item("Zippy::xhere_all", "Extract Here (semua)", "--extract-here", files),
            self._item(
                "Zippy::xeach",
                "Extract each to separate folder",
                "--extract-to-subfolder",
                files,
            ),
            self._item("Zippy::test_all", "Test integrity (semua)", "--test", files),
        ]
