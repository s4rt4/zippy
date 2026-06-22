//! Deteksi format dari magic bytes (bukan ekstensi).
//!
//! Penting untuk file yang diganti nama / tanpa ekstensi. Deteksi berlapis:
//! format majemuk seperti `.tar.gz` dideteksi gzip dulu, lalu tar di dalamnya.
//! Magic `ustar` milik tar ada di **offset 257**, bukan offset 0
//! (Planning Doc §3.1).
//!
//! Status: **Sprint 0 — stub**. Tabel magic bytes sudah didefinisikan;
//! deteksi berlapis penuh + integrasi crate menyusul di v0.1.

/// Format archive yang dikenali Zippy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
    SevenZip,
    Rar,
    Unknown,
}

impl Format {
    /// Apakah format ini hanya bisa di-extract (tidak bisa di-compress).
    pub fn extract_only(self) -> bool {
        matches!(self, Format::Rar) // RAR proprietary — extract only (§3)
    }
}

// Magic bytes (offset 0 kecuali disebut) — Planning Doc §3.1.
const MAGIC_ZIP: &[u8] = &[0x50, 0x4B, 0x03, 0x04];
const MAGIC_7Z: &[u8] = &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
const MAGIC_RAR5: &[u8] = &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00];
const MAGIC_GZIP: &[u8] = &[0x1F, 0x8B];
const MAGIC_BZIP2: &[u8] = &[0x42, 0x5A, 0x68]; // "BZh"
const MAGIC_XZ: &[u8] = &[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00];
const MAGIC_ZSTD: &[u8] = &[0x28, 0xB5, 0x2F, 0xFD];
const MAGIC_TAR_USTAR: &[u8] = b"ustar"; // di offset 257
const TAR_USTAR_OFFSET: usize = 257;

/// Deteksi format dari buffer awal file (one-shot, top-level).
///
/// Catatan: ini hanya mengembalikan lapisan terluar. Deteksi berlapis penuh
/// (mis. gzip → tar di dalamnya) akan ditangani di `archive` pada v0.1.
pub fn detect(buf: &[u8]) -> Format {
    let starts_with = |m: &[u8]| buf.len() >= m.len() && &buf[..m.len()] == m;

    if starts_with(MAGIC_ZIP) {
        Format::Zip
    } else if starts_with(MAGIC_7Z) {
        Format::SevenZip
    } else if starts_with(MAGIC_RAR5) {
        Format::Rar
    } else if starts_with(MAGIC_GZIP) {
        Format::Gzip
    } else if starts_with(MAGIC_BZIP2) {
        Format::Bzip2
    } else if starts_with(MAGIC_XZ) {
        Format::Xz
    } else if starts_with(MAGIC_ZSTD) {
        Format::Zstd
    } else if buf.len() >= TAR_USTAR_OFFSET + MAGIC_TAR_USTAR.len()
        && &buf[TAR_USTAR_OFFSET..TAR_USTAR_OFFSET + MAGIC_TAR_USTAR.len()] == MAGIC_TAR_USTAR
    {
        Format::Tar
    } else {
        Format::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_zip() {
        assert_eq!(detect(&[0x50, 0x4B, 0x03, 0x04, 0, 0]), Format::Zip);
    }

    #[test]
    fn detects_gzip() {
        assert_eq!(detect(&[0x1F, 0x8B, 0x08]), Format::Gzip);
    }

    #[test]
    fn detects_tar_ustar_at_offset_257() {
        let mut buf = vec![0u8; 512];
        buf[257..262].copy_from_slice(b"ustar");
        assert_eq!(detect(&buf), Format::Tar);
    }

    #[test]
    fn unknown_for_empty() {
        assert_eq!(detect(&[]), Format::Unknown);
    }

    #[test]
    fn rar_is_extract_only() {
        assert!(Format::Rar.extract_only());
        assert!(!Format::Zip.extract_only());
    }
}
