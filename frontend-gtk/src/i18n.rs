//! Internationalization ringan — dua bahasa (Indonesia / English) tanpa
//! dependensi (filosofi proyek = seringan mungkin, lihat [`crate::config`]).
//!
//! Teks **Indonesia adalah kanonik**: argumen `t("…")` selalu berupa teks
//! Indonesia, dan terjemahan Inggris di-overlay lewat tabel [`en`]. String yang
//! belum punya padanan Inggris otomatis jatuh kembali ke teks Indonesia, jadi
//! tidak ada yang "hilang" walau tabel belum lengkap.

use std::cell::Cell;

/// Bahasa konkret yang sedang aktif.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Id,
    En,
}

/// Preferensi bahasa user (disimpan di config). `Auto` = ikuti locale sistem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangPref {
    Auto,
    Id,
    En,
}

impl LangPref {
    pub fn as_str(self) -> &'static str {
        match self {
            LangPref::Auto => "auto",
            LangPref::Id => "id",
            LangPref::En => "en",
        }
    }

    pub fn parse(s: &str) -> LangPref {
        match s {
            "id" => LangPref::Id,
            "en" => LangPref::En,
            _ => LangPref::Auto,
        }
    }

    /// Resolusi ke bahasa konkret (`Auto` → deteksi locale sistem).
    pub fn resolve(self) -> Lang {
        match self {
            LangPref::Id => Lang::Id,
            LangPref::En => Lang::En,
            LangPref::Auto => detect_locale(),
        }
    }
}

/// Deteksi bahasa dari env locale (`LC_ALL`/`LC_MESSAGES`/`LANG`). Kode `id`
/// atau `in` (Indonesia) → [`Lang::Id`], selain itu → [`Lang::En`].
fn detect_locale() -> Lang {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        let Some(v) = std::env::var_os(key) else {
            continue;
        };
        let v = v.to_string_lossy().to_ascii_lowercase();
        if v.is_empty() || v == "c" || v == "posix" {
            continue;
        }
        let code = v.split(['_', '.', '@']).next().unwrap_or("");
        return if code == "id" || code == "in" {
            Lang::Id
        } else {
            Lang::En
        };
    }
    Lang::En
}

thread_local! {
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::Id) };
}

/// Set bahasa aktif (dipakai saat init & saat user mengganti di Preferensi).
pub fn set_lang(l: Lang) {
    CURRENT.with(|c| c.set(l));
}

/// Bahasa aktif saat ini.
pub fn lang() -> Lang {
    CURRENT.with(|c| c.get())
}

/// Inisialisasi bahasa aktif dari preferensi user.
pub fn init(pref: LangPref) {
    set_lang(pref.resolve());
}

/// Terjemahkan teks Indonesia kanonik `id` ke bahasa aktif.
pub fn t(id: &'static str) -> &'static str {
    match lang() {
        Lang::Id => id,
        Lang::En => en(id).unwrap_or(id),
    }
}

/// Seperti [`t`] tapi mengisi placeholder `{}` (berurutan) dengan `args`.
/// Template diterjemahkan dulu, baru argumen disisipkan — jadi urutan kata bisa
/// berbeda antar bahasa selama jumlah `{}` sama.
pub fn tf(id: &'static str, args: &[&str]) -> String {
    let tmpl = t(id);
    let mut out = String::with_capacity(tmpl.len() + 16);
    let mut parts = tmpl.split("{}");
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    let mut args = args.iter();
    for part in parts {
        if let Some(a) = args.next() {
            out.push_str(a);
        }
        out.push_str(part);
    }
    out
}

/// Overlay Inggris untuk tiap teks Indonesia kanonik. `None` = belum
/// diterjemahkan (pemanggil jatuh ke teks Indonesia).
fn en(id: &str) -> Option<&'static str> {
    Some(match id {
        // — Menu bar —
        "Berkas" => "File",
        "Perintah" => "Commands",
        "Alat" => "Tools",
        "Favorit" => "Favorites",
        "Opsi" => "Options",
        "Bantuan" => "Help",
        "Buka Archive…" => "Open Archive…",
        "Simpan Salinan Archive…" => "Save Archive Copy…",
        "Set Password Default…" => "Set Default Password…",
        "Tutup Archive" => "Close Archive",
        "Pilih Semua" => "Select All",
        "Balik Seleksi" => "Invert Selection",
        "Keluar" => "Quit",
        "Tambah Berkas…" => "Add Files…",
        "Extract Ke…" => "Extract To…",
        "Ganti Nama…" => "Rename…",
        "Komentar Archive…" => "Archive Comment…",
        "Pindai Virus…" => "Scan for Viruses…",
        "Perbaiki Arsip…" => "Repair Archive…",
        "Convert Archive…" => "Convert Archive…",
        "Buat SFX (.sh)…" => "Create SFX (.sh)…",
        "Buat Laporan…" => "Generate Report…",
        "Cari…" => "Find…",
        "Folder Tree (tampil/sembunyi)" => "Folder Tree (show/hide)",
        "Preferensi…" => "Preferences…",
        "Profil Kompresi…" => "Compression Profiles…",
        "Penyandian Nama…" => "Name Encoding…",
        "Lihat Log…" => "View Log…",
        "Tentang Zippy" => "About Zippy",

        // — Favorit —
        "Tambah arsip saat ini" => "Add current archive",
        "Hapus arsip saat ini" => "Remove current archive",
        "Kelola Favorit…" => "Manage Favorites…",
        "Tersimpan" => "Saved",
        "Buka arsip dulu sebelum menambah ke Favorit" => {
            "Open an archive first before adding to Favorites"
        }
        "Ditambahkan ke Favorit" => "Added to Favorites",
        "Arsip ini tidak ada di Favorit" => "This archive is not in Favorites",
        "Dihapus dari Favorit" => "Removed from Favorites",
        "Kelola Favorit" => "Manage Favorites",
        "Arsip Favorit" => "Favorite Archives",
        "Klik baris untuk membuka, atau tombol hapus untuk membuang." => {
            "Click a row to open, or the remove button to discard."
        }
        "Belum ada favorit" => "No favorites yet",
        "Tambahkan lewat menu Favorit → Tambah arsip saat ini" => {
            "Add via the Favorites menu → Add current archive"
        }
        "Hapus dari Favorit" => "Remove from Favorites",

        // — Toolbar —
        "Tambah" => "Add",
        "Extract" => "Extract",
        "Lihat" => "View",
        "Hapus" => "Delete",
        "Cari" => "Find",
        "Perbaiki" => "Repair",
        "Pindai" => "Scan",
        "Batalkan operasi" => "Cancel operation",
        "Filter berkas di folder ini…" => "Filter files in this folder…",
        "Membatalkan…" => "Cancelling…",

        // — Status & address bar —
        "Total {} folder dan {} bita dalam {} berkas" => "Total {} folders and {} bytes in {} files",
        "Tidak ada archive terbuka" => "No archive open",
        "{} - arsip {}, ukuran asli {} bita" => "{} - {} archive, unpacked size {} bytes",

        // — Umum (toast/dialog generik) —
        "Belum ada archive terbuka" => "No archive open yet",
        "Perhatian" => "Warning",
        "Tutup" => "Close",
        "Batal" => "Cancel",
        "Simpan" => "Save",
        "Buka" => "Open",
        "Buat" => "Create",
        "Password kosong" => "Empty password",
        "Archive Terenkripsi" => "Encrypted Archive",
        "Archive kosong" => "Empty archive",

        // — Set password default —
        "Password Default" => "Default Password",
        "Dipakai otomatis untuk extract/test/view arsip terenkripsi (sesi ini saja)." => {
            "Used automatically for extract/test/view of encrypted archives (this session only)."
        }
        "Password default diset" => "Default password set",
        "Password default dikosongkan" => "Default password cleared",

        // — Simpan salinan & laporan —
        "Simpan salinan archive sebagai…" => "Save archive copy as…",
        "Tujuan sama dengan sumber" => "Destination is the same as source",
        "Salinan archive disimpan" => "Archive copy saved",
        "Gagal menyimpan: {}" => "Failed to save: {}",
        "Simpan laporan…" => "Save report…",
        "Laporan disimpan" => "Report saved",
        "Gagal menulis laporan: {}" => "Failed to write report: {}",
        "Laporan Archive — Zippy v{}\n" => "Archive Report — Zippy v{}\n",
        "Archive : {}\n" => "Archive : {}\n",
        "Berkas  : {}   Folder: {}\n" => "Files   : {}   Folders: {}\n",
        "Ukuran  : {} bytes (packed {} bytes" => "Size    : {} bytes (packed {} bytes",
        "Nama\tUkuran\tPacked\tModified\tCRC32\n" => "Name\tSize\tPacked\tModified\tCRC32\n",

        // — Dialog kompres (Add) —
        "Pilih berkas/folder untuk diarsipkan" => "Choose files/folders to archive",
        "Simpan archive sebagai…" => "Save archive as…",
        "Simpan (tanpa kompresi)" => "Store (no compression)",
        "Cepat" => "Fastest",
        "Maksimal" => "Maximum",
        "Tingkat kompresi:" => "Compression level:",
        "(Custom)" => "(Custom)",
        "Profil:" => "Profile:",
        "Password AES-256 (opsional)" => "AES-256 password (optional)",
        "Split ukuran volume mis. 100m (opsional)" => "Split volume size e.g. 100m (optional)",
        "Simpan symlink sebagai link (bukan isinya)" => "Store symlinks as links (not their contents)",
        "Hapus berkas sumber setelah arsip dibuat" => "Delete source files after the archive is created",
        "Buat Archive" => "Create Archive",
        "Mengompres…" => "Compressing…",
        "Arsip dibuat: {}" => "Archive created: {}",
        "Archive dibuat" => "Archive created",
        "Kompres dibatalkan" => "Compression cancelled",
        "Kompres Gagal" => "Compression Failed",
        "{} sumber dipindah ke Trash" => "{} sources moved to Trash",
        "Berkas sumber dipindahkan ke Trash" => "Source files moved to Trash",
        "{} sumber gagal dipindah ke Trash" => "{} sources could not be moved to Trash",

        // — Test —
        "Menguji…" => "Testing…",
        "Test Selesai" => "Test Complete",
        "Tidak ada kesalahan ditemukan — arsip utuh." => "No errors found — archive is intact.",
        "Test dibatalkan" => "Test cancelled",
        "Masukkan password untuk menguji." => "Enter the password to test.",
        "Uji" => "Test",
        "Test Gagal" => "Test Failed",
        "Arsip rusak atau tidak valid:\n{}" => "Archive is corrupt or invalid:\n{}",

        // — Repair —
        "Memperbaiki arsip…" => "Repairing archive…",
        "Perbaikan dibatalkan" => "Repair cancelled",
        "Repair Gagal" => "Repair Failed",
        "Metode: {}\n" => "Method: {}\n",
        "Output: {}\n" => "Output: {}\n",
        "\nStatus: berhasil ✓" => "\nStatus: succeeded ✓",
        "\nStatus: tidak dapat diperbaiki sepenuhnya" => "\nStatus: could not be fully repaired",
        "Perbaikan Berhasil" => "Repair Succeeded",
        "Perbaikan Tidak Tuntas" => "Repair Incomplete",

        // — Scan virus —
        "ClamAV Tidak Terpasang" => "ClamAV Not Installed",
        "Pemindaian virus butuh ClamAV. Pasang paket `clamav` lalu coba lagi." => {
            "Virus scanning requires ClamAV. Install the `clamav` package and try again."
        }
        "Memindai virus…" => "Scanning for viruses…",
        "Pemindaian dibatalkan" => "Scan cancelled",
        "Scan Gagal" => "Scan Failed",
        "Scanner: {}\n" => "Scanner: {}\n",
        "\nArsip bersih ✓" => "\nArchive is clean ✓",
        "\n{} berkas terinfeksi:\n" => "\n{} infected files:\n",
        "… dan {} lagi\n" => "… and {} more\n",
        "Tidak Ada Virus" => "No Viruses",
        "Virus Terdeteksi!" => "Viruses Detected!",

        // — Convert —
        "Convert ke… (format dari ekstensi)" => "Convert to… (format from extension)",
        "Format Tidak Dikenali" => "Unrecognized Format",
        "Ekstensi tujuan tidak didukung." => "The destination extension is not supported.",
        "Password AES-256 hasil (opsional)" => "Resulting AES-256 password (optional)",
        "Convert Archive" => "Convert Archive",
        "Convert" => "Convert",
        "Mengonversi…" => "Converting…",
        "Convert: {} → {}" => "Convert: {} → {}",
        "Konversi Selesai" => "Conversion Complete",
        "Arsip dibuat:\n{}" => "Archive created:\n{}",
        "Konversi dibatalkan" => "Conversion cancelled",
        "Sumber Terenkripsi" => "Encrypted Source",
        "Masukkan password untuk membuka arsip sumber." => "Enter the password to open the source archive.",
        "Konversi Gagal" => "Conversion Failed",

        // — SFX —
        "Buat SFX (.sh) ke…" => "Create SFX (.sh) to…",
        "Membuat SFX…" => "Creating SFX…",
        "SFX dibuat: {}" => "SFX created: {}",
        "SFX Dibuat" => "SFX Created",
        "Self-extracting script:\n{}\n\nJalankan: sh {} [folder-tujuan]" => {
            "Self-extracting script:\n{}\n\nRun: sh {} [target-folder]"
        }
        "Pembuatan SFX dibatalkan" => "SFX creation cancelled",
        "Buat SFX" => "Create SFX",
        "SFX Gagal" => "SFX Failed",

        // — Komentar —
        "Tidak Didukung" => "Not Supported",
        "Komentar arsip hanya tersedia untuk ZIP." => "Archive comments are only available for ZIP.",
        "Komentar Archive" => "Archive Comment",
        "Komentar disimpan di arsip ZIP." => "The comment is stored in the ZIP archive.",
        "Menyimpan komentar…" => "Saving comment…",
        "Komentar disimpan" => "Comment saved",
        "Gagal Simpan Komentar" => "Failed to Save Comment",

        // — View / launch —
        "Pilih berkas dulu" => "Select a file first",
        "Pilih berkas, bukan folder" => "Select a file, not a folder",
        "Membuka {}…" => "Opening {}…",
        "Masukkan password untuk membuka berkas." => "Enter the password to open the file.",
        "Gagal membuka: {}" => "Failed to open: {}",
        "Gagal membuka archive" => "Failed to open archive",

        // — Context menu —
        "Naik ke folder induk" => "Up to parent folder",
        "Buka folder" => "Open folder",
        "Extract folder ini…" => "Extract this folder…",
        "Ganti nama…" => "Rename…",
        "Salin nama" => "Copy name",
        "Hapus folder ini" => "Delete this folder",
        "Properti…" => "Properties…",
        "Buka (View)" => "Open (View)",
        "Extract berkas ini…" => "Extract this file…",
        "Extract {} item terpilih…" => "Extract {} selected items…",
        "Hapus {} item terpilih" => "Delete {} selected items",
        "Extract Semua…" => "Extract All…",
        "Test Archive" => "Test Archive",
        "{} nama disalin" => "{} names copied",

        // — Properti —
        "Nama: {}\nPath: {}\nTipe: {}\nUkuran: {} bita\nPacked: {} bita\nRasio: {}\nModified: {}\nCRC32: {}" => {
            "Name: {}\nPath: {}\nType: {}\nSize: {} bytes\nPacked: {} bytes\nRatio: {}\nModified: {}\nCRC32: {}"
        }
        "Folder" => "Folder",
        "Properti" => "Properties",

        // — Extract —
        "Tidak ada berkas untuk di-extract" => "No files to extract",
        "Extract ke folder…" => "Extract to folder…",
        "Memulai…" => "Starting…",
        "Extract selesai" => "Extraction complete",
        "Extract dibatalkan" => "Extraction cancelled",
        "Masukkan password untuk extract." => "Enter the password to extract.",
        "Gagal extract: {}" => "Extraction failed: {}",

        // — Delete —
        "RAR tidak mendukung hapus (extract-only)" => "RAR does not support deletion (extract-only)",
        "Format stream tunggal tak punya entri untuk dihapus" => {
            "Single-stream formats have no entries to delete"
        }
        "Pilih entri yang akan dihapus" => "Select the entries to delete",
        "Hapus \"{}\" dari archive? Tindakan ini tidak bisa dibatalkan." => {
            "Delete \"{}\" from the archive? This action cannot be undone."
        }
        "Hapus {} item dari archive? Tindakan ini tidak bisa dibatalkan." => {
            "Delete {} items from the archive? This action cannot be undone."
        }
        "Hapus dari Archive" => "Delete from Archive",
        "Menghapus…" => "Deleting…",
        "Entri dihapus" => "Entries deleted",
        "Hapus dibatalkan" => "Deletion cancelled",
        "Masukkan password untuk menghapus entri." => "Enter the password to delete entries.",
        "Gagal hapus: {}" => "Deletion failed: {}",

        // — Rename —
        "RAR tidak mendukung rename (extract-only)" => "RAR does not support rename (extract-only)",
        "Format stream tunggal tak punya entri untuk di-rename" => {
            "Single-stream formats have no entries to rename"
        }
        "Pilih entri yang akan di-rename" => "Select the entry to rename",
        "Ganti Nama" => "Rename",
        "Masukkan nama baru (tetap di folder yang sama)." => {
            "Enter the new name (stays in the same folder)."
        }
        "Nama baru kosong" => "New name is empty",
        "Mengganti nama…" => "Renaming…",
        "Rename: {} → {}" => "Rename: {} → {}",
        "Nama diubah" => "Renamed",
        "Rename dibatalkan" => "Rename cancelled",
        "Masukkan password untuk mengganti nama entri." => "Enter the password to rename entries.",
        "Rename Gagal" => "Rename Failed",

        // — About —
        "© 2026 s4rt4" => "© 2026 s4rt4",
        "Repositori GitHub" => "GitHub Repository",
        "Laporkan Masalah" => "Report an Issue",

        // — Preferensi —
        "Preferensi" => "Preferences",
        "Umum" => "General",
        "Bahasa" => "Language",
        "Ikuti sistem (locale)" => "Follow system (locale)",
        "Bahasa Indonesia" => "Indonesian",
        "English" => "English",
        "Tema" => "Theme",
        "Ikuti sistem" => "Follow system",
        "Terang" => "Light",
        "Gelap" => "Dark",
        "Tingkat kompresi default" => "Default compression level",
        "Dipakai sebagai pilihan awal di dialog Add" => "Used as the initial choice in the Add dialog",
        "Konfirmasi sebelum hapus" => "Confirm before deleting",
        "Tampilkan dialog konfirmasi saat menghapus entri arsip" => {
            "Show a confirmation dialog when deleting archive entries"
        }
        "Hapus arsip setelah extract" => "Delete archive after extract",
        "Pindahkan arsip ke Trash setelah extract berhasil" => {
            "Move the archive to Trash after a successful extract"
        }
        "Tipe berkas dilarang di-extract" => "File types excluded from extracting",
        "Ekstensi dipisah spasi (mis. \"desktop sh exe\"). Kosong = tanpa filter." => {
            "Extensions separated by spaces (e.g. \"desktop sh exe\"). Empty = no filter."
        }
        "Penyandian nama (ZIP legasi)" => "Name encoding (legacy ZIP)",
        "Untuk arsip lama dengan nama non-UTF8" => "For old archives with non-UTF-8 names",
        "Profil kompresi" => "Compression profiles",
        "Simpan preset level untuk dialog Add" => "Save level presets for the Add dialog",

        // — Profil kompresi —
        "Profil Kompresi" => "Compression Profiles",
        "Tambah Profil" => "Add Profile",
        "Nama profil" => "Profile name",
        "Tingkat" => "Level",

        // — file_list kolom & tipe —
        "Nama" => "Name",
        "Ukuran" => "Size",
        "Packed" => "Packed",
        "Tipe" => "Type",
        "Modified" => "Modified",
        // "Berkas" (tipe entri = file) sudah dipetakan ke "File" di atas (menu).
        "File HTML" => "HTML File",
        "File {}" => "{} File",
        "Aplikasi" => "Application",
        "Dokumen Teks" => "Text Document",

        // — File chooser & overwrite —
        "Buka Archive" => "Open Archive",
        "Berkas Sudah Ada" => "Files Already Exist",
        "{} berkas sudah ada di folder tujuan.\nPilih cara menanganinya:" => {
            "{} files already exist in the destination folder.\nChoose how to handle them:"
        }
        "Lewati" => "Skip",
        "Beri Nama Baru" => "Rename",
        "Timpa Semua" => "Overwrite All",
        "Extract Gagal" => "Extraction Failed",
        "Membaca {}…" => "Reading {}…",
        "Extract: {} → {}" => "Extract: {} → {}",
        "Masukkan password untuk \"{}\"." => "Enter the password for \"{}\".",
        ", rasio {}%" => ", ratio {}%",
        "• {} — {}\n" => "• {} — {}\n",
        "⚠ Archive memakai enkripsi ZipCrypto lemah — pertimbangkan ulang dengan AES-256" => {
            "⚠ Archive uses weak ZipCrypto encryption — consider AES-256 instead"
        }
        "Arsip dipindahkan ke Trash" => "Archive moved to Trash",
        "Gagal memindah arsip ke Trash: {}" => "Failed to move archive to Trash: {}",
        "Buka Folder" => "Open Folder",
        "Nama profil tidak valid (tanpa '.' atau '=')" => "Invalid profile name (no '.' or '=')",

        // — Encoding dialog —
        "Penyandian Nama" => "Name Encoding",
        "Untuk arsip ZIP lama dengan nama non-UTF8." => "For old ZIP archives with non-UTF-8 names.",
        "Terapkan" => "Apply",

        // — Wizard —
        "Wizard Zippy" => "Zippy Wizard",
        "Apa yang ingin Anda lakukan?" => "What would you like to do?",
        "Buka arsip" => "Open archive",
        "Tampilkan isi arsip yang sudah ada" => "Show the contents of an existing archive",
        "Buat arsip baru" => "Create a new archive",
        "Pilih berkas/folder lalu kompres" => "Choose files/folders then compress",
        "Extract arsip" => "Extract archive",
        "Pilih arsip lalu folder tujuan" => "Choose an archive then a destination folder",
        "Uji arsip" => "Test archive",
        "Verifikasi integritas isi arsip" => "Verify the archive's contents",
        "Pilih arsip untuk di-extract" => "Choose an archive to extract",
        "Pilih arsip untuk diuji" => "Choose an archive to test",

        // — Log dialog —
        "(Belum ada aktivitas)" => "(No activity yet)",
        "Log Aktivitas" => "Activity Log",
        "Bersihkan" => "Clear",
        "Log dibersihkan" => "Log cleared",

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_canonical_passthrough() {
        set_lang(Lang::Id);
        assert_eq!(t("Berkas"), "Berkas");
        assert_eq!(t("Tutup"), "Tutup");
    }

    #[test]
    fn en_overlay_translates() {
        set_lang(Lang::En);
        assert_eq!(t("Berkas"), "File");
        assert_eq!(t("Perintah"), "Commands");
        assert_eq!(t("Tutup"), "Close");
    }

    #[test]
    fn unknown_falls_back_to_indonesian() {
        set_lang(Lang::En);
        // Tidak ada di tabel → kembalikan teks Indonesia apa adanya.
        assert_eq!(t("ZIP"), "ZIP");
        assert_eq!(
            t("Teks yang belum diterjemahkan"),
            "Teks yang belum diterjemahkan"
        );
    }

    #[test]
    fn tf_fills_placeholders_in_order() {
        set_lang(Lang::Id);
        assert_eq!(
            tf(
                "Total {} folder dan {} bita dalam {} berkas",
                &["1", "2", "3"]
            ),
            "Total 1 folder dan 2 bita dalam 3 berkas"
        );
        // Urutan kata berbeda di Inggris, jumlah {} tetap sama.
        set_lang(Lang::En);
        assert_eq!(
            tf(
                "Total {} folder dan {} bita dalam {} berkas",
                &["1", "2", "3"]
            ),
            "Total 1 folders and 2 bytes in 3 files"
        );
    }

    #[test]
    fn langpref_roundtrips_and_resolves() {
        assert_eq!(LangPref::parse("id"), LangPref::Id);
        assert_eq!(LangPref::parse("en"), LangPref::En);
        assert_eq!(LangPref::parse("xx"), LangPref::Auto);
        assert_eq!(LangPref::Auto.as_str(), "auto");
        assert_eq!(LangPref::Id.resolve(), Lang::Id);
        assert_eq!(LangPref::En.resolve(), Lang::En);
    }
}
