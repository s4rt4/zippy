#!/usr/bin/env python3
# Nautilus (GNOME) context-menu extension — Planning Doc §6.5.
# Pasang ke ~/.local/share/nautilus-python/extensions/
# Butuh paket python3-nautilus. Opsional: app tetap jalan tanpanya (MIME handler).
#
# Status: Sprint 0 scaffold — kerangka menu; verb di-dispatch ke binary `zippy`.
# Menu kondisional penuh + label dinamis difinalkan di v0.4 (Sprint 8-9).

import os
import subprocess

from gi.repository import GObject, Nautilus

ARCHIVE_MIMES = {
    "application/zip",
    "application/x-7z-compressed",
    "application/x-rar",
    "application/x-tar",
    "application/gzip",
    "application/x-xz",
    "application/zstd",
}


class ZippyMenu(GObject.GObject, Nautilus.MenuProvider):
    # NB: signature Nautilus baru — get_file_items(self, files) tanpa arg window.
    def get_file_items(self, files):
        archives = [f for f in files if f.get_mime_type() in ARCHIVE_MIMES]
        if archives and len(archives) == len(files):
            return self._archive_menu(files)  # konteks B / C
        return self._compress_menu(files)     # konteks A / D

    def _run(self, _menu, verb, files):
        paths = [f.get_location().get_path() for f in files]
        subprocess.Popen(["zippy", verb, *paths])

    def _item(self, name, label, verb, files):
        item = Nautilus.MenuItem(name=name, label=label)
        item.connect("activate", self._run, verb, files)
        return item

    def _archive_menu(self, files):
        if len(files) == 1:
            base = os.path.basename(files[0].get_location().get_path())
            stem = base.rsplit(".", 1)[0]
            return [
                self._item("Zippy::ExtractHere", "Extract Here", "--extract-here", files),
                self._item("Zippy::ExtractSub", f'Extract to "{stem}/"', "--extract-to-subfolder", files),
                self._item("Zippy::ExtractTo", "Extract to…", "--extract-to", files),
                self._item("Zippy::Open", "Open with Zippy", "--open", files),
                self._item("Zippy::Test", "Test integrity", "--test", files),
            ]
        return [
            self._item("Zippy::ExtractHere", "Extract Here (all)", "--extract-here", files),
            self._item("Zippy::ExtractEach", "Extract each to separate folder", "--extract-each", files),
            self._item("Zippy::Test", "Test integrity (all)", "--test", files),
        ]

    def _compress_menu(self, files):
        return [
            self._item("Zippy::Add", "Add to archive…", "--add", files),
            self._item("Zippy::AddQuick", "Add to archive (quick)", "--add-quick", files),
        ]
