# Pemetaan Fitur WinRAR → Zippy (Linux)

Sumber: 22 screenshot WinRAR di `~/Downloads/winrar/` (5 menu, 7 tab Settings, 7 tab Add, 3 view Extract).
Tujuan: catat **setiap** fitur yang terbaca, lalu pilah kelayakannya di Linux + status di Zippy.

## Legenda status

| Simbol | Arti |
|--------|------|
| ✅ | Sudah ada di Zippy |
| 🟢 | Layak — implementasi mudah/murah |
| 🟡 | Layak — butuh usaha / hanya sebagian format |
| 🔵 | Layak tapi nilai rendah / opsional |
| 🔴 | Tidak relevan / tak bisa di Linux (khas Windows / RAR-create) |
| 🆕 | Fitur baru yang diminta user (repair / virus scan) |

Catatan kunci Linux: **tak ada API menulis RAR** (unrar hanya baca), jadi semua fitur yang melekat ke pembuatan RAR (recovery record RAR, lock, solid-RAR, SFX .exe) tidak bisa direplikasi apa adanya. Padanan lintas-platform: **PAR2** (recovery), **shell-SFX** (self-extracting .sh), volume ZIP/7z.

---

## 1. Menu File

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Open archive (Ctrl+O) | ✅ | Sudah ada |
| Save archive copy as… | 🟢 | Copy file arsip biasa + file-chooser |
| Change drive (Ctrl+D) | 🔴 | Konsep "drive letter" Windows; di Linux N/A (pakai address bar / GVfs) |
| Set default password (Ctrl+P) | 🟢 | Simpan password sesi → dipakai otomatis saat open/extract/test |
| Copy files to clipboard (Ctrl+C) | 🟡 | GTK bisa taruh URI `text/uri-list` (ekstrak ke temp lalu copy) |
| Paste files from clipboard (Ctrl+V) | 🟡 | Paste = `add` file dari clipboard ke arsip |
| Copy full names to clipboard | ✅ | Sudah ada (Salin-nama) |
| Select all (Ctrl+A) | 🟢 | ColumnView select-all |
| Select / Deselect group (Gray +/-) | 🟢 | Pilih via pola wildcard |
| Invert selection (Gray *) | 🟢 | Toggle seleksi |
| Exit | ✅ | Sudah ada |

## 2. Menu Commands

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Add files to archive (Alt+A) | ✅ | Sudah ada |
| Extract to specified folder (Alt+E) | ✅ | Sudah ada |
| Test archived files (Alt+T) | ✅ | Sudah ada |
| View file (Alt+V) | ✅ | Sudah ada (GtkFileLauncher) |
| Delete files (Del) | ✅ | Sudah ada (delete in-place) |
| Rename file (F2) | 🟡 | Rename entry di dalam arsip (zip/7z/tar: rewrite; rar: ❌) |
| Print file (Ctrl+I) | 🔵 | Ekstrak ke temp → cetak via portal/`lp`; nilai rendah |
| Extract without confirmation (Alt+W) | 🟢 | "Extract Here" tanpa dialog |
| Add archive comment (Alt+M) | 🟡 | Komentar didukung ZIP & 7z & RAR(baca); tulis utk zip/7z |
| Protect archive from damage (Alt+P) | 🟡 | = recovery record. Padanan Linux = **PAR2** sidecar (lihat fitur Repair) |
| Lock archive (Alt+L) | 🔴 | Flag internal RAR; tak ada padanan di format Linux |

## 3. Menu Tools

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Wizard | ✅ | Sudah ada |
| **Scan archive for viruses (Alt+D)** | 🆕 | **Diminta** — via ClamAV (`clamscan`/`clamdscan`): ekstrak ke temp aman → scan → laporan. Pakai ikon good/bad-notif |
| Convert archives (Alt+Q) | 🟡 | Ekstrak lalu re-compress ke format lain (zip↔7z↔tar.*) |
| **Repair archive (Alt+R)** | 🆕 | **Diminta** — dua jalur: (a) `zip -FF`/`7z` recovery bawaan; (b) verifikasi+perbaiki via **PAR2** bila ada sidecar `.par2` |
| Convert archive to SFX (Alt+X) | 🟡 | Bukan .exe — buat **self-extracting `.sh`** (shell + payload), atau makeself |
| Find files (F3) | ✅ | Sudah ada (Find) |
| Show information (Alt+I) | ✅ | Sudah ada (Info/Properti) |
| Generate report (Alt+G) | 🟢 | Export daftar isi → .txt/.csv/.html |
| Benchmark (Alt+B) | 🔵 | Uji kecepatan kompres; nilai rendah, mudah |

## 4. Menu Favorites

| Fitur | Status | Catatan |
|-------|--------|---------|
| Add to favorites (Ctrl+F) | ✅ | Sudah ada |
| Organize favorites | ✅ | Sudah ada |

## 5. Menu Options

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Settings (Ctrl+S) | ✅ | Sudah ada (Preferences) |
| Import and export (config) | 🟢 | Export/import `config.ini` |
| Clear history | 🟢 | Bergantung fitur History dulu |
| File list ► (submenu) | 🟡 | Toggle kolom/tampilan (sebagian sudah di file_list) |
| Folder tree ► | 🟡 | Panel pohon folder kiri (opsional) |
| Themes ► | ✅ | Sudah ada (combo tema) |
| Name encoding (Ctrl+E) | 🟡 | Pilih encoding nama file legacy (CP437/Shift-JIS dll) |
| View log / Clear log | 🟢 | Log operasi in-memory/file |

## 6. Settings → General

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Low priority | 🟢 | `nice`/ionice untuk thread kerja |
| Threads (jumlah) | 🟡 | Terbatas dukungan lib (7z/zstd multi-thread) |
| Keep archives history / in dialogs | 🟢 | Daftar recent |
| Toolbar: ukuran tombol / show text / lock | 🟡 | Sebagian (label sudah ada) |
| Activate Wizard on start | 🟢 | Flag config |
| Enable sound | 🔵 | Bunyi notifikasi selesai |
| Show archive comment | 🟡 | Bergantung fitur komentar |
| Reuse existing window | 🟡 | Single-instance (GApplication) |
| Always on top | 🟢 | `set_keep_above`/hint |
| Full paths in title bar | 🟢 | Trivial |
| Windows/Taskbar progress bar | 🔴 | API taskbar Windows; padanan: badge Unity (terbatas) |
| Logging (errors to file, size limit) | 🟢 | Sejalan View log |

## 7. Settings → Compression

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Compression profiles (create/organize) | 🟡 | Simpan preset (format+level+password policy) |
| Volume size list | 🟡 | Daftar ukuran split siap-pakai |
| File types to open as archives first | 🔵 | Mis. `*.exe` SFX → buka sbg arsip; sebagian relevan |

## 8. Settings → Paths

| Fitur | Status | Catatan |
|-------|--------|---------|
| Folder temp + "use only for removable" | 🟢 | Set TMPDIR app |
| Start-up folder / restore last folder | 🟢 | Config |
| Default folder for archives | 🟢 | Config |
| Default folder for extracted files | 🟢 | Config |
| Append archive name to path | 🟢 | Sudah ada konsepnya di CLI `--extract-to-subfolder` |
| Remove redundant folders from path | 🟢 | Buang folder pembungkus tunggal saat ekstrak |

## 9. Settings → File list

| Fitur | Status | Catatan |
|-------|--------|---------|
| List view / Details | 🟡 | Sekarang Details (ColumnView) |
| Show grid lines / full row select / checkboxes | 🟢 | Properti GTK |
| Single/Double click to open | 🟢 | Gesture |
| Underline names (hover) | 🔵 | Kosmetik |
| Show archives first | 🟢 | Sorting |
| Color encrypted/compressed files | 🟢 | Sudah ada flag enkripsi → beri warna |
| Merge volumes contents | 🟡 | Bergantung dukungan volume |
| Show seconds / Exact sizes | 🟢 | Format kolom |
| Columns… / Set font… | 🟡 | Pilih kolom & font |

## 10. Settings → Viewer

| Fitur | Status | Catatan |
|-------|--------|---------|
| Viewer: Internal/External/Associated/Ask | 🟡 | "Associated" sudah ada (GtkFileLauncher); internal viewer perlu widget teks |
| Autodetect encoding / Word wrap (internal) | 🟡 | Untuk internal viewer |
| Unpack everything for `*.htm…` | 🔵 | Ekstrak penuh sebelum lihat (HTML+aset) |
| Ignore modifications for | 🔵 | — |
| External viewer name (browse) | 🟢 | Set command |

## 11. Settings → Security

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Prohibited file types (exclude from extract) | 🟢 | **Berguna**: blok ekstrak `*.desktop *.sh` dll by default |
| Wipe temporary files (Never/Always/Encrypted) | 🟡 | Hapus aman file temp |
| Propose to select virus scanner | 🆕 | Terkait fitur Scan — pilih `clamscan` path |

## 12. Settings → Integration

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Associate with RAR/ZIP/7Z/TAR/GZ/… | 🟡 | = MIME default (`xdg-mime`); install.sh sudah sebagian |
| Add to Desktop / Start Menu | 🟢 | File `.desktop` |
| Shell integration / context menu | ✅ | Sudah ada (Nautilus/Dolphin/Thunar) |

## 13. Add → tab General (Archive name and parameters)

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Archive name + Browse | ✅ | Sudah ada |
| Default Profile / Profiles… | 🟡 | Preset kompresi |
| Archive format RAR / RAR4 / ZIP | 🟡 | RAR ❌ create; tawarkan ZIP/7z/TAR.* |
| Compression method (Store…Best) | ✅ | Sudah ada (Level enum) |
| Dictionary size | 🟡 | xz/7z/zstd expose dict; lainnya tidak |
| Split to volumes, size | 🟡 | ZIP & 7z bisa; TAR via split; RAR ❌ |
| Update mode (add & replace / update / fresh) | 🟡 | Logika add lanjutan |
| Delete files after archiving | 🟢 | Hapus sumber setelah sukses |
| Create SFX archive | 🟡 | self-extracting `.sh` |
| Create solid archive | 🟡 | 7z/tar.* inheren solid; zip ❌; rar ❌ create |
| Add recovery record | 🆕/🟡 | PAR2 sidecar |
| Test archived files | ✅ | Sudah ada (bisa dirangkai setelah compress) |
| Lock archive | 🔴 | RAR-only |
| Set password | ✅ | Sudah ada (AES-256) |

## 14. Add → tab Advanced

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| NTFS: Save file security | 🔴 | ACL Windows; padanan POSIX ACL berbeda |
| NTFS: Save file streams | 🔴 | ADS Windows; N/A |
| Store symbolic links as links | 🟡 | Relevan TAR (penting!), 7z sebagian |
| Store hard links as links | 🟡 | TAR mendukung hardlink |
| Recovery record percent | 🆕/🟡 | PAR2 redundancy % |
| Compression… (dialog detail) | 🟡 | Param lanjutan |
| SFX options | 🟡 | Untuk shell-SFX |
| When done (keep/shutdown/sleep…) | 🟡 | Shutdown/suspend via `systemctl`/portal |
| Background archiving | 🟢 | Sudah jalan di thread; tinggal opsi minimize |
| Wait if other copies active | 🔵 | Lock antar-instance |

## 15. Add → tab Options

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Delete mode: Delete / Recycle Bin / Wipe | 🟡 | "Recycle Bin" = Trash (gio trash); Wipe = shred |
| Wipe encrypted files | 🟡 | shred sumber |
| Quick open info (larger/all files) | 🔴 | Optimasi internal format RAR; N/A |
| Use BLAKE2 checksum | 🟡 | 7z mendukung; zip = CRC32 saja |
| Save identical files as references | 🟡 | Dedup (solid 7z) |
| Save original archive name and time | 🔵 | Metadata |
| Additional switches | 🔵 | Passthrough argumen CLI |

## 16. Add → tab Files

| Fitur | Status | Catatan |
|-------|--------|---------|
| Files to add / exclude / store-without-compression | 🟡 | Pola include/exclude/store |
| File paths (relative/full/absolute) | 🟢 | Mode path saat menambah |
| Put each file to separate archive | 🟢 | Sudah ada konsep `--add-quick`/extract-each |
| Use double extensions for archives | 🔵 | Penamaan |
| Create separate archives in subfolders | 🔵 | — |
| Send archive by email | 🔵 | `xdg-email` mailto attach |

## 17. Add → tab Backup

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Erase destination before archiving | 🔵 | — |
| Add only files with "Archive" attribute | 🔴 | Atribut arsip Windows; N/A |
| Clear "Archive" attribute after | 🔴 | N/A |
| Open shared files | 🔵 | — |
| Generate archive name by mask (yyyymmddhhmm) | 🟢 | Penamaan berbasis tanggal (berguna utk backup) |
| Keep previous file versions | 🔵 | — |

## 18. Add → tab Time

| Fitur | Status | Catatan |
|-------|--------|---------|
| Store modification time | ✅/🟢 | Default tersimpan |
| Store creation / last access time | 🟡 | TAR pax / 7z; ctime POSIX terbatas |
| High precision time format | 🟡 | Sub-detik (pax/7z) |
| Preserve source last access time | 🟢 | Set atime balik |
| Include files of time (any/after…) | 🟡 | Filter berdasar mtime |
| Set archive time to | 🔵 | Stempel waktu arsip |

## 19. Add → tab Comment

| Fitur | Status | Catatan |
|-------|--------|---------|
| Load comment from file / enter manually | 🟡 | ZIP & 7z mendukung komentar arsip |

## 20. Extract → General / Advanced / Options

| Fitur | Status | Catatan Linux |
|-------|--------|----------------|
| Destination path + tree picker + New folder | ✅/🟡 | Dialog folder ada; pohon inline opsional |
| Update mode (replace/update/fresh) | 🟡 | Logika ekstrak |
| Overwrite mode (ask/overwrite/skip/rename) | 🟢 | **Penting** — sekarang perlu eksplisit |
| Keep broken files | 🟢 | Pertahankan output parsial |
| Display files in file manager | 🟢 | `xdg-open` folder tujuan |
| File time (mod/creation/access) | 🟡 | Set waktu hasil |
| Attributes (clear archive / file security / compressed) | 🔴 | Atribut NTFS; N/A |
| File paths (relative/full/absolute/none) | 🟢 | Mode path ekstrak |
| Delete archive after (never/ask/always/trash) | 🟢 | + Trash via gio |
| Background extraction | 🟢 | Sudah di thread |
| Allow absolute paths in symlinks | 🟡 | **Keamanan** — default OFF (anti path-escape) |
| Allow potentially incompatible names | 🟡 | Sanitasi nama (`:` `\` dll) |
| Extract archives to (dest/subfolder/archive folder) | 🟢 | Sudah ada via CLI verbs |
| When done | 🟡 | Shutdown/suspend |

---

## Ringkasan prioritas implementasi

**Gelombang 1 — diminta user (🆕) — ✅ SELESAI:**
1. ✅ **Repair archive** — `zip -FF` (ZIP) + `par2 repair` (sidecar `.par2`); `core::tools::repair`, tombol+menu Tools.
2. ✅ **Scan virus** — ClamAV (`clamdscan`/`clamscan`), pindai arsip langsung (ClamAV buka isi sendiri) → laporan good/bad-notif; `core::tools::scan`.
3. ✅ **Ganti ikon toolbar** ke 12 SVG baru (add/extract/test/view/delete/find/wizard/info/repair/scan + good/bad-notif), di-embed via `ACTION_ICONS` + `setup_icon_theme`.

**Gelombang 2 — menang besar, murah (🟢):**
Overwrite mode di Extract, Set default password, Select/Invert selection, Generate report, View log, Delete-after-archiving, Display in file manager, Prohibited file types (security), Delete-archive-to-Trash, Save copy as.

**Gelombang 3 — fitur format menengah (🟡):**
Convert archives, Rename in-archive, Archive comment (zip/7z), Split to volumes, SFX shell, Compression profiles, symlink/hardlink handling (tar), Name encoding.

**Tidak dikerjakan (🔴) — khas Windows / RAR-create:**
Change drive, Lock archive, Quick-open-info, NTFS streams/security, atribut "Archive", taskbar progress, Convert-to-SFX-.exe.
